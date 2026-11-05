//! Reading and validating a policy (`warden.toml`, schema `forge-warden.policy.v0`).
//!
//! A policy is held to the same standard as code. Loading it either yields a fully typed
//! [`Policy`] or refuses with **every** problem found, each with a location, so an author fixes
//! them in one pass. Required fields have no defaults: a rule says what it means.
//!
//! ```toml
//! schema = "forge-warden.policy.v0"
//!
//! [[rule]]
//! id = "no-secrets"
//! why = "A credential committed once is compromised forever."
//! kind = "forbid_signal"
//! severity = "fail"
//! signals = ["secret_like"]
//! ```

use crate::date::Date;
use crate::exit::Outcome;
use crate::vocab::{is_known_signal, suggest_signal};
use globset::{Glob, GlobBuilder, GlobSet, GlobSetBuilder};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::fmt;

/// The only policy schema this version reads.
pub const POLICY_SCHEMA: &str = "forge-warden.policy.v0";

/// A rationale shorter than this is refused: "because" is not a reason.
pub const MIN_WHY_CHARS: usize = 12;

/// The longest rule id accepted.
pub const MAX_ID_CHARS: usize = 64;

/// A validated policy.
#[derive(Debug, Clone)]
pub struct Policy {
    /// Rules, in the order the file declares them (and the verdict reports them).
    pub rules: Vec<Rule>,
    /// Waivers, in file order.
    pub waivers: Vec<Waiver>,
}

/// One rule.
#[derive(Debug, Clone)]
pub struct Rule {
    /// Unique kebab-case id.
    pub id: String,
    /// Why the rule exists, for the person it stops.
    pub why: String,
    /// What a violation means.
    pub severity: Severity,
    /// What it checks.
    pub check: Check,
}

/// What a violated rule does to the verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The change must not merge as it is.
    Fail,
    /// A person must look before it merges.
    Hold,
}

impl Severity {
    /// The outcome a violation of this severity produces.
    #[must_use]
    pub const fn outcome(self) -> Outcome {
        match self {
            Self::Fail => Outcome::Fail,
            Self::Hold => Outcome::Hold,
        }
    }

    /// Lowercase name.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Fail => "fail",
            Self::Hold => "hold",
        }
    }
}

/// The check a rule performs. Each variant is one `kind`.
#[derive(Debug, Clone)]
pub enum Check {
    /// `forbid_signal`: no file in scope may carry any of `signals`.
    ForbidSignal {
        /// Signals that are not allowed.
        signals: Vec<String>,
        /// Files the rule looks at.
        scope: Scope,
    },
    /// `require_companion`: if a file in `when` changes, some file in `require` must change too.
    RequireCompanion {
        /// Files whose change triggers the rule.
        when: Scope,
        /// Files of which at least one must also be in the change.
        require: PathSet,
    },
    /// `protect_paths`: any change to a file in scope is a violation.
    ProtectPaths {
        /// Files that are protected.
        scope: Scope,
    },
    /// `max_score`: a risk-score budget.
    MaxScore {
        /// The largest allowed score.
        limit: u32,
        /// Per file, or summed over the change.
        per: ScorePer,
        /// Files the rule looks at.
        scope: Scope,
    },
    /// `max_files`: at most `limit` files in scope may change.
    MaxFiles {
        /// The largest allowed number of files.
        limit: u32,
        /// Files the rule looks at.
        scope: Scope,
    },
}

impl Check {
    /// The `kind` string this check is written as.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::ForbidSignal { .. } => "forbid_signal",
            Self::RequireCompanion { .. } => "require_companion",
            Self::ProtectPaths { .. } => "protect_paths",
            Self::MaxScore { .. } => "max_score",
            Self::MaxFiles { .. } => "max_files",
        }
    }
}

/// Whether a score budget applies to each file or to the change as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScorePer {
    /// Each file's score.
    File,
    /// The sum of every in-scope file's score.
    Change,
}

/// A compiled set of globs, keeping the patterns for reporting.
#[derive(Debug, Clone)]
pub struct PathSet {
    patterns: Vec<String>,
    set: GlobSet,
}

impl PathSet {
    /// Whether `path` matches any pattern.
    #[must_use]
    pub fn matches(&self, path: &str) -> bool {
        self.set.is_match(path)
    }

    /// The patterns as written.
    #[must_use]
    pub fn patterns(&self) -> &[String] {
        &self.patterns
    }
}

/// Files a rule looks at: in `paths` and not in `except`.
#[derive(Debug, Clone)]
pub struct Scope {
    paths: PathSet,
    except: Option<PathSet>,
}

impl Scope {
    /// Whether `path` is in scope.
    #[must_use]
    pub fn contains(&self, path: &str) -> bool {
        self.paths.matches(path) && !self.except.as_ref().is_some_and(|ex| ex.matches(path))
    }
}

/// A time-boxed, owned exemption from one rule for some paths.
#[derive(Debug, Clone)]
pub struct Waiver {
    /// The rule it exempts from.
    pub rule: String,
    /// Paths it covers.
    pub paths: PathSet,
    /// Why the exemption is acceptable.
    pub reason: String,
    /// Who answers for it.
    pub owner: String,
    /// The last day it applies. From the next day it is expired, and an expired waiver fails the
    /// verdict until it is removed or renewed.
    pub expires: Date,
}

/// A policy that could not be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    /// More than [`crate::MAX_INPUT_BYTES`].
    #[error("policy is {0} bytes, over the {max} byte budget", max = crate::MAX_INPUT_BYTES)]
    TooLarge(usize),
    /// Not UTF-8, or not TOML of the policy shape (including unknown keys).
    #[error("policy is not valid TOML of the forge-warden.policy.v0 shape: {0}")]
    Malformed(String),
    /// Well-formed, but wrong. Every issue found is listed.
    #[error("policy refused with {} issue(s):\n{}", .0.len(), IssueList(.0))]
    Invalid(Vec<PolicyIssue>),
}

/// One problem in a policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyIssue {
    /// Where: `rule[2] "no-secrets"`, `waiver[0]`, or `policy`.
    pub location: String,
    /// What is wrong, and how to fix it when that is knowable.
    pub message: String,
}

impl fmt::Display for PolicyIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.location, self.message)
    }
}

struct IssueList<'a>(&'a [PolicyIssue]);

impl fmt::Display for IssueList<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, issue) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "  - {issue}")?;
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPolicy {
    schema: String,
    #[serde(default, rename = "rule")]
    rules: Vec<RawRule>,
    #[serde(default, rename = "waiver")]
    waivers: Vec<RawWaiver>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    id: String,
    why: String,
    kind: String,
    severity: String,
    signals: Option<Vec<String>>,
    paths: Option<Vec<String>>,
    except: Option<Vec<String>>,
    when: Option<Vec<String>>,
    require: Option<Vec<String>>,
    limit: Option<u32>,
    per: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWaiver {
    rule: String,
    paths: Vec<String>,
    reason: String,
    owner: String,
    expires: String,
}

/// Every optional field a rule may carry, and the kinds that accept it. (`id`, `why`, `kind` and
/// `severity` are required on every rule.)
pub const RULE_FIELD_KINDS: [(&str, &[&str]); 7] = [
    ("signals", &["forbid_signal"]),
    (
        "paths",
        &["forbid_signal", "protect_paths", "max_score", "max_files"],
    ),
    (
        "except",
        &[
            "forbid_signal",
            "require_companion",
            "protect_paths",
            "max_score",
            "max_files",
        ],
    ),
    ("when", &["require_companion"]),
    ("require", &["require_companion"]),
    ("limit", &["max_score", "max_files"]),
    ("per", &["max_score"]),
];

/// The kinds a policy may use.
pub const KINDS: [&str; 5] = [
    "forbid_signal",
    "require_companion",
    "protect_paths",
    "max_score",
    "max_files",
];

/// Parse and validate a policy from its exact bytes.
///
/// # Errors
///
/// [`PolicyError`]: over budget, not UTF-8 TOML of the right shape, or invalid, with every issue
/// listed.
pub fn parse_policy(bytes: &[u8]) -> Result<Policy, PolicyError> {
    if bytes.len() > crate::MAX_INPUT_BYTES {
        return Err(PolicyError::TooLarge(bytes.len()));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|e| PolicyError::Malformed(format!("not UTF-8: {e}")))?;
    let raw: RawPolicy =
        toml::from_str(text).map_err(|e| PolicyError::Malformed(e.message().to_owned()))?;
    let mut issues = Vec::new();
    if raw.schema != POLICY_SCHEMA {
        issues.push(issue(
            "policy",
            format!("schema is {:?}; expected {POLICY_SCHEMA:?}", raw.schema),
        ));
    }
    if raw.rules.is_empty() {
        issues.push(issue(
            "policy",
            "declares no [[rule]]; an empty policy would pass every change".to_owned(),
        ));
    }
    let mut seen = BTreeSet::new();
    let mut rules = Vec::with_capacity(raw.rules.len());
    for (index, raw_rule) in raw.rules.into_iter().enumerate() {
        let location = format!("rule[{index}] {:?}", raw_rule.id);
        if !seen.insert(raw_rule.id.clone()) {
            issues.push(issue(
                &location,
                "duplicate id; ids must be unique".to_owned(),
            ));
        }
        if let Some(rule) = validate_rule(&location, raw_rule, &mut issues) {
            rules.push(rule);
        }
    }
    let mut waivers = Vec::with_capacity(raw.waivers.len());
    for (index, raw_waiver) in raw.waivers.into_iter().enumerate() {
        let location = format!("waiver[{index}]");
        if let Some(waiver) = validate_waiver(&location, raw_waiver, &seen, &mut issues) {
            waivers.push(waiver);
        }
    }
    if issues.is_empty() {
        Ok(Policy { rules, waivers })
    } else {
        Err(PolicyError::Invalid(issues))
    }
}

fn issue(location: &str, message: String) -> PolicyIssue {
    PolicyIssue {
        location: location.to_owned(),
        message,
    }
}

fn is_kebab_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_CHARS
        && id.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
        && id.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
}

fn validate_rule(location: &str, raw: RawRule, issues: &mut Vec<PolicyIssue>) -> Option<Rule> {
    let before = issues.len();
    if !is_kebab_id(&raw.id) {
        issues.push(issue(
            location,
            format!("id must be kebab-case (a-z, 0-9, single hyphens, starting with a letter), at most {MAX_ID_CHARS} chars"),
        ));
    }
    if raw.why.trim().chars().count() < MIN_WHY_CHARS {
        issues.push(issue(
            location,
            format!("`why` must explain the rule to the person it stops (at least {MIN_WHY_CHARS} characters)"),
        ));
    }
    let severity = match raw.severity.as_str() {
        "fail" => Some(Severity::Fail),
        "hold" => Some(Severity::Hold),
        other => {
            issues.push(issue(
                location,
                format!("severity {other:?} is not \"fail\" or \"hold\""),
            ));
            None
        }
    };
    if !KINDS.contains(&raw.kind.as_str()) {
        issues.push(issue(
            location,
            format!("kind {:?} is not one of: {}", raw.kind, KINDS.join(", ")),
        ));
        return None;
    }
    check_fields_apply(location, &raw, issues);
    let check = build_check(location, &raw, issues);
    match (severity, check) {
        (Some(severity), Some(check)) if issues.len() == before => Some(Rule {
            id: raw.id,
            why: raw.why.trim().to_owned(),
            severity,
            check,
        }),
        _ => None,
    }
}

fn check_fields_apply(location: &str, raw: &RawRule, issues: &mut Vec<PolicyIssue>) {
    let present = [
        ("signals", raw.signals.is_some()),
        ("paths", raw.paths.is_some()),
        ("except", raw.except.is_some()),
        ("when", raw.when.is_some()),
        ("require", raw.require.is_some()),
        ("limit", raw.limit.is_some()),
        ("per", raw.per.is_some()),
    ];
    for (field, is_present) in present {
        let accepts = RULE_FIELD_KINDS
            .iter()
            .find(|(name, _)| *name == field)
            .is_some_and(|(_, kinds)| kinds.contains(&raw.kind.as_str()));
        if is_present && !accepts {
            issues.push(issue(
                location,
                format!("field `{field}` does not apply to kind {:?}", raw.kind),
            ));
        }
    }
}

fn build_check(location: &str, raw: &RawRule, issues: &mut Vec<PolicyIssue>) -> Option<Check> {
    let except = raw
        .except
        .as_ref()
        .and_then(|globs| compile(location, "except", globs, issues));
    // `paths` defaults to everything only where the kind makes that meaningful; `protect_paths`
    // must say what it protects.
    let scope_from = |paths: Option<PathSet>| {
        paths.map(|paths| Scope {
            paths,
            except: except.clone(),
        })
    };
    let optional_paths = |sink: &mut Vec<PolicyIssue>| match &raw.paths {
        Some(globs) => compile(location, "paths", globs, sink),
        None => compile(location, "paths", &["**".to_owned()], sink),
    };
    match raw.kind.as_str() {
        "forbid_signal" => {
            let signals = required(location, "signals", raw.signals.as_ref(), issues)?;
            for signal in signals {
                if !is_known_signal(signal) {
                    let hint = suggest_signal(signal)
                        .map(|s| format!(" (did you mean {s:?}?)"))
                        .unwrap_or_default();
                    issues.push(issue(location, format!("unknown signal {signal:?}{hint}")));
                }
            }
            let scope = scope_from(optional_paths(issues))?;
            Some(Check::ForbidSignal {
                signals: signals.clone(),
                scope,
            })
        }
        "require_companion" => {
            let when_globs = required(location, "when", raw.when.as_ref(), issues)?;
            let require_globs = required(location, "require", raw.require.as_ref(), issues)?;
            let when = scope_from(compile(location, "when", when_globs, issues))?;
            let require = compile(location, "require", require_globs, issues)?;
            Some(Check::RequireCompanion { when, require })
        }
        "protect_paths" => {
            let globs = required(location, "paths", raw.paths.as_ref(), issues)?;
            let scope = scope_from(compile(location, "paths", globs, issues))?;
            Some(Check::ProtectPaths { scope })
        }
        "max_score" => {
            let Some(limit) = raw.limit else {
                issues.push(issue(
                    location,
                    "kind \"max_score\" requires `limit`".to_owned(),
                ));
                return None;
            };
            let per = match raw.per.as_deref() {
                Some("file") => ScorePer::File,
                Some("change") => ScorePer::Change,
                Some(other) => {
                    issues.push(issue(
                        location,
                        format!("per {other:?} is not \"file\" or \"change\""),
                    ));
                    return None;
                }
                None => {
                    issues.push(issue(
                        location,
                        "kind \"max_score\" requires `per` (\"file\" or \"change\")".to_owned(),
                    ));
                    return None;
                }
            };
            let scope = scope_from(optional_paths(issues))?;
            Some(Check::MaxScore { limit, per, scope })
        }
        "max_files" => {
            let Some(limit) = raw.limit else {
                issues.push(issue(
                    location,
                    "kind \"max_files\" requires `limit`".to_owned(),
                ));
                return None;
            };
            if limit == 0 {
                issues.push(issue(
                    location,
                    "limit 0 forbids every change; use protect_paths to say that".to_owned(),
                ));
                return None;
            }
            let scope = scope_from(optional_paths(issues))?;
            Some(Check::MaxFiles { limit, scope })
        }
        _ => None,
    }
}

fn required<'a>(
    location: &str,
    field: &str,
    value: Option<&'a Vec<String>>,
    issues: &mut Vec<PolicyIssue>,
) -> Option<&'a Vec<String>> {
    match value {
        Some(list) if !list.is_empty() => Some(list),
        Some(_) => {
            issues.push(issue(location, format!("`{field}` must not be empty")));
            None
        }
        None => {
            issues.push(issue(location, format!("this kind requires `{field}`")));
            None
        }
    }
}

/// Compile globs with `/`-aware semantics: `*` stays inside one path segment, `**` crosses them.
fn compile(
    location: &str,
    field: &str,
    globs: &[String],
    issues: &mut Vec<PolicyIssue>,
) -> Option<PathSet> {
    if globs.is_empty() {
        issues.push(issue(location, format!("`{field}` must not be empty")));
        return None;
    }
    let mut builder = GlobSetBuilder::new();
    let mut ok = true;
    for pattern in globs {
        match build_glob(pattern) {
            Ok(glob) => {
                builder.add(glob);
            }
            Err(message) => {
                issues.push(issue(
                    location,
                    format!("`{field}` pattern {pattern:?}: {message}"),
                ));
                ok = false;
            }
        }
    }
    if !ok {
        return None;
    }
    match builder.build() {
        Ok(set) => Some(PathSet {
            patterns: globs.to_vec(),
            set,
        }),
        Err(e) => {
            issues.push(issue(location, format!("`{field}`: {e}")));
            None
        }
    }
}

fn build_glob(pattern: &str) -> Result<Glob, String> {
    if pattern.is_empty() {
        return Err("empty pattern".to_owned());
    }
    if pattern.starts_with('/') || pattern.starts_with("./") {
        return Err(
            "patterns are relative to the repository root; drop the leading `/` or `./`".to_owned(),
        );
    }
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .backslash_escape(true)
        .build()
        .map_err(|e| e.kind().to_string())
}

fn validate_waiver(
    location: &str,
    raw: RawWaiver,
    rule_ids: &BTreeSet<String>,
    issues: &mut Vec<PolicyIssue>,
) -> Option<Waiver> {
    let before = issues.len();
    if !rule_ids.contains(&raw.rule) {
        issues.push(issue(
            location,
            format!(
                "waives rule {:?}, which the policy does not declare",
                raw.rule
            ),
        ));
    }
    if raw.reason.trim().chars().count() < MIN_WHY_CHARS {
        issues.push(issue(
            location,
            format!("`reason` must justify the exemption (at least {MIN_WHY_CHARS} characters)"),
        ));
    }
    if raw.owner.trim().is_empty() {
        issues.push(issue(
            location,
            "`owner` names who answers for the exemption; it must not be empty".to_owned(),
        ));
    }
    let expires = Date::parse(&raw.expires);
    if expires.is_none() {
        issues.push(issue(
            location,
            format!("expires {:?} is not a YYYY-MM-DD date", raw.expires),
        ));
    }
    let paths = compile(location, "paths", &raw.paths, issues);
    match (paths, expires) {
        (Some(paths), Some(expires)) if issues.len() == before => Some(Waiver {
            rule: raw.rule,
            paths,
            reason: raw.reason.trim().to_owned(),
            owner: raw.owner.trim().to_owned(),
            expires,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(body: &str) -> String {
        format!("schema = \"forge-warden.policy.v0\"\n[[rule]]\n{body}\n")
    }

    fn issues_of(text: &str) -> Vec<String> {
        match parse_policy(text.as_bytes()) {
            Err(PolicyError::Invalid(issues)) => issues.iter().map(ToString::to_string).collect(),
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    const GOOD: &str = r#"id = "no-secrets"
why = "A credential committed once is compromised forever."
kind = "forbid_signal"
severity = "fail"
signals = ["secret_like"]"#;

    #[test]
    fn a_good_rule_loads() {
        let policy = parse_policy(rule(GOOD).as_bytes()).unwrap();
        assert_eq!(policy.rules.len(), 1);
        assert_eq!(policy.rules[0].check.kind(), "forbid_signal");
    }

    #[test]
    fn unknown_keys_are_refused_not_ignored() {
        let text = rule(&format!("{GOOD}\nsignal = [\"unsafe_added\"]"));
        assert!(
            matches!(parse_policy(text.as_bytes()), Err(PolicyError::Malformed(m)) if m.contains("signal"))
        );
    }

    #[test]
    fn empty_policy_is_refused() {
        let issues = issues_of("schema = \"forge-warden.policy.v0\"\n");
        assert!(issues.iter().any(|i| i.contains("no [[rule]]")));
    }

    #[test]
    fn typoed_signal_gets_a_suggestion() {
        let issues = issues_of(&rule(&GOOD.replace("secret_like", "secrets_like")));
        assert!(
            issues
                .iter()
                .any(|i| i.contains("did you mean \"secret_like\"")),
            "{issues:?}"
        );
    }

    #[test]
    fn every_issue_is_reported_at_once() {
        let text = format!(
            "schema = \"forge-warden.policy.v1\"\n[[rule]]\n{}\n[[rule]]\n{}\n",
            GOOD.replace("no-secrets", "No_Secrets").replace(
                "A credential committed once is compromised forever.",
                "because"
            ),
            GOOD,
        );
        let issues = issues_of(&text);
        assert!(issues.len() >= 3, "{issues:?}");
        assert!(issues.iter().any(|i| i.contains("schema")));
        assert!(issues.iter().any(|i| i.contains("kebab-case")));
        assert!(issues.iter().any(|i| i.contains("`why`")));
    }

    #[test]
    fn duplicate_ids_are_refused() {
        let text = format!("{}[[rule]]\n{GOOD}\n", rule(GOOD));
        assert!(issues_of(&text).iter().any(|i| i.contains("duplicate id")));
    }

    #[test]
    fn fields_must_apply_to_their_kind() {
        let issues = issues_of(&rule(&format!("{GOOD}\nlimit = 3")));
        assert!(
            issues.iter().any(|i| i.contains("`limit` does not apply")),
            "{issues:?}"
        );
    }

    #[test]
    fn globs_are_segment_aware_and_validated() {
        let text = rule(
            r#"id = "src"
why = "Source changes need a person to look at them."
kind = "protect_paths"
severity = "hold"
paths = ["src/*.rs"]"#,
        );
        let policy = parse_policy(text.as_bytes()).unwrap();
        let Check::ProtectPaths { scope } = &policy.rules[0].check else {
            panic!()
        };
        assert!(scope.contains("src/lib.rs"));
        assert!(!scope.contains("src/deep/lib.rs"), "`*` must not cross `/`");

        let bad = rule(&GOOD.replace(
            "signals = [\"secret_like\"]",
            "signals = [\"secret_like\"]\npaths = [\"src/[\"]",
        ));
        assert!(
            issues_of(&bad)
                .iter()
                .any(|i| i.contains("pattern \"src/[\""))
        );
        let rooted = rule(&GOOD.replace(
            "signals = [\"secret_like\"]",
            "signals = [\"secret_like\"]\npaths = [\"/src/**\"]",
        ));
        assert!(
            issues_of(&rooted)
                .iter()
                .any(|i| i.contains("relative to the repository root"))
        );
    }

    #[test]
    fn waivers_must_name_a_rule_an_owner_a_reason_and_a_date() {
        let text = format!(
            "{}[[waiver]]\nrule = \"no-such-rule\"\npaths = [\"a\"]\nreason = \"short\"\nowner = \" \"\nexpires = \"2026-13-01\"\n",
            rule(GOOD)
        );
        let issues = issues_of(&text);
        for needle in ["does not declare", "`reason`", "`owner`", "YYYY-MM-DD"] {
            assert!(
                issues.iter().any(|i| i.contains(needle)),
                "missing {needle:?} in {issues:?}"
            );
        }
    }

    #[test]
    fn max_score_needs_limit_and_per() {
        let base = r#"id = "budget"
why = "Large risky changes are reviewed in pieces."
kind = "max_score"
severity = "hold""#;
        assert!(
            issues_of(&rule(base))
                .iter()
                .any(|i| i.contains("requires `limit`"))
        );
        let no_per = format!("{base}\nlimit = 10");
        assert!(
            issues_of(&rule(&no_per))
                .iter()
                .any(|i| i.contains("requires `per`"))
        );
        let ok = format!("{base}\nlimit = 10\nper = \"change\"");
        assert!(parse_policy(rule(&ok).as_bytes()).is_ok());
    }

    #[test]
    fn non_utf8_is_malformed() {
        assert!(matches!(
            parse_policy(b"\xff"),
            Err(PolicyError::Malformed(_))
        ));
    }
}
