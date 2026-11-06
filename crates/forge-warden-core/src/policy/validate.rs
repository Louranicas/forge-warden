//! Parsing a policy and validating every rule and waiver.

use super::error::{PolicyError, PolicyIssue, issue};
use super::model::{Check, Policy, Rule, ScorePer, Severity, Waiver};
use super::paths::{PathSet, Scope, compile};
use super::raw::{KINDS, RULE_FIELD_KINDS, RawPolicy, RawRule, RawWaiver};
use crate::date::Date;
use crate::vocab::{is_known_signal, suggest_signal};
use std::collections::BTreeSet;

/// The only policy schema this version reads.
pub const POLICY_SCHEMA: &str = "forge-warden.policy.v0";

/// A rationale shorter than this is refused: "because" is not a reason.
pub const MIN_WHY_CHARS: usize = 12;

/// The longest rule id accepted.
pub const MAX_ID_CHARS: usize = 64;

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

pub(super) fn is_kebab_id(id: &str) -> bool {
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
