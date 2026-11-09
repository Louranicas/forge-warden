//! Parsing a policy and validating every rule and waiver.

use super::error::PolicyError;
use super::issues::{Issues, Rejected};
use super::kind::{Field, Kind};
use super::model::{Check, Policy, Rule, ScorePer, Severity, Waiver};
use super::paths::{PathSet, Scope, compile};
use super::raw::{RawPolicy, RawProducer, RawRule, RawWaiver};
use crate::date::Date;
use crate::defaults::NamedDefault as _;
use crate::nonempty::NonEmpty;
use crate::seal::Sha256Hex;
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
    let mut issues = Issues::new();
    if raw.schema != POLICY_SCHEMA {
        issues.push(
            "policy",
            format!("schema is {:?}; expected {POLICY_SCHEMA:?}", raw.schema),
        );
    }
    if raw.rules.is_empty() {
        issues.push(
            "policy",
            "declares no [[rule]]; an empty policy would pass every change".to_owned(),
        );
    }
    let mut seen = BTreeSet::new();
    let mut rules = Vec::with_capacity(raw.rules.len());
    for (index, raw_rule) in raw.rules.into_iter().enumerate() {
        let location = format!("rule[{index}] {:?}", raw_rule.id);
        if !seen.insert(raw_rule.id.clone()) {
            issues.push(&location, "duplicate id; ids must be unique".to_owned());
        }
        // `Err` carries proof that an issue was recorded, so the policy is refused below.
        if let Ok(rule) = validate_rule(&location, raw_rule, &mut issues) {
            rules.push(rule);
        }
    }
    let mut waivers = Vec::with_capacity(raw.waivers.len());
    for (index, raw_waiver) in raw.waivers.into_iter().enumerate() {
        let location = format!("waiver[{index}]");
        if let Ok(waiver) = validate_waiver(&location, raw_waiver, &seen, &mut issues) {
            waivers.push(waiver);
        }
    }
    let producers = match raw.producer {
        Some(producer) => validate_producers(&producer, &mut issues),
        None => Vec::new(),
    };
    // An empty rule list always comes with an issue ("declares no [[rule]]", or the rule's own
    // refusal), so the `None` arm returns those issues; the policy is refused either way.
    let Some(rules) = NonEmpty::from_vec(rules) else {
        return Err(PolicyError::Invalid(issues.into_vec()));
    };
    issues
        .finish(Policy::new(rules, waivers, producers))
        .map_err(PolicyError::Invalid)
}

/// `[producer] sha256`: at least one digest, each a well-formed seal, none repeated. A table that
/// admits nothing is refused rather than read as "no producer": it is a mistake, not a mode.
fn validate_producers(raw: &RawProducer, issues: &mut Issues) -> Vec<Sha256Hex> {
    let location = "producer";
    if raw.sha256.is_empty() {
        issues.push(
            location,
            "`sha256` lists no ranker; remove [producer] for advisory-only use".to_owned(),
        );
    }
    let mut seen = BTreeSet::new();
    let mut admitted = Vec::new();
    for (index, digest) in raw.sha256.iter().enumerate() {
        match Sha256Hex::parse(digest) {
            Ok(seal) => admitted.push(seal),
            Err(_) => issues.push(
                location,
                format!("sha256[{index}] {digest:?} is not 64 lowercase hex digits"),
            ),
        }
        if !seen.insert(digest.as_str()) {
            issues.push(location, format!("sha256[{index}] {digest:?} is repeated"));
        }
    }
    admitted
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

fn validate_rule(location: &str, raw: RawRule, issues: &mut Issues) -> Result<Rule, Rejected> {
    let start = issues.mark();
    if !is_kebab_id(&raw.id) {
        issues.push(
            location,
            format!("id must be kebab-case (a-z, 0-9, single hyphens, starting with a letter), at most {MAX_ID_CHARS} chars"),
        );
    }
    if raw.why.trim().chars().count() < MIN_WHY_CHARS {
        issues.push(
            location,
            format!("`why` must explain the rule to the person it stops (at least {MIN_WHY_CHARS} characters)"),
        );
    }
    let severity = match raw.severity.as_str() {
        "fail" => Ok(Severity::Fail),
        "hold" => Ok(Severity::Hold),
        other => Err(issues.reject(
            location,
            format!("severity {other:?} is not \"fail\" or \"hold\""),
        )),
    };
    let Some(kind) = Kind::from_name(&raw.kind) else {
        let names: Vec<&str> = Kind::ALL.iter().map(|k| k.name()).collect();
        return Err(issues.reject(
            location,
            format!("kind {:?} is not one of: {}", raw.kind, names.join(", ")),
        ));
    };
    check_fields_apply(location, &raw, kind, issues);
    let check = build_check(location, &raw, kind, issues)?;
    let severity = severity?;
    issues.since(start)?;
    Ok(Rule {
        id: raw.id,
        why: raw.why.trim().to_owned(),
        severity,
        check,
    })
}

fn check_fields_apply(location: &str, raw: &RawRule, kind: Kind, issues: &mut Issues) {
    for field in Field::ALL {
        if raw.has(field) && !kind.fields().contains(&field) {
            issues.push(
                location,
                format!(
                    "field `{}` does not apply to kind {:?}",
                    field.name(),
                    kind.name()
                ),
            );
        }
    }
}

fn build_check(
    location: &str,
    raw: &RawRule,
    kind: Kind,
    issues: &mut Issues,
) -> Result<Check, Rejected> {
    // A bad `except` is recorded and validation carries on, so the author sees every issue at once;
    // `validate_rule` then refuses the rule because an issue was recorded after its mark.
    let except = raw
        .except
        .as_ref()
        .and_then(|globs| compile(location, "except", globs, issues).ok());
    let scope = |paths: PathSet| Scope {
        paths,
        except: except.clone(),
    };
    // `paths` defaults to everything only where the kind makes that meaningful; `protect_paths`
    // must say what it protects.
    let optional_paths = |sink: &mut Issues| match &raw.paths {
        Some(globs) => compile(location, "paths", globs, sink),
        None => compile(location, "paths", &["**".to_owned()], sink),
    };
    match kind {
        Kind::ForbidSignal => {
            let signals = required(location, "signals", raw.signals.as_ref(), issues)?;
            for signal in signals {
                if !is_known_signal(signal) {
                    // No hint when nothing is close; the signal is refused either way.
                    let hint = suggest_signal(signal)
                        .map(|s| format!(" (did you mean {s:?}?)"))
                        .or_display(String::new());
                    issues.push(location, format!("unknown signal {signal:?}{hint}"));
                }
            }
            Ok(Check::ForbidSignal {
                signals: signals.clone(),
                scope: scope(optional_paths(issues)?),
            })
        }
        Kind::RequireCompanion => {
            let when_globs = required(location, "when", raw.when.as_ref(), issues)?;
            let require_globs = required(location, "require", raw.require.as_ref(), issues)?;
            Ok(Check::RequireCompanion {
                when: scope(compile(location, "when", when_globs, issues)?),
                require: compile(location, "require", require_globs, issues)?,
            })
        }
        Kind::ProtectPaths => {
            let globs = required(location, "paths", raw.paths.as_ref(), issues)?;
            Ok(Check::ProtectPaths {
                scope: scope(compile(location, "paths", globs, issues)?),
            })
        }
        Kind::MaxScore => {
            let limit = required_limit(location, kind, raw.limit, issues)?;
            let per = match raw.per.as_deref() {
                Some("file") => ScorePer::File,
                Some("change") => ScorePer::Change,
                Some(other) => {
                    return Err(issues.reject(
                        location,
                        format!("per {other:?} is not \"file\" or \"change\""),
                    ));
                }
                None => {
                    return Err(issues.reject(
                        location,
                        "kind \"max_score\" requires `per` (\"file\" or \"change\")".to_owned(),
                    ));
                }
            };
            Ok(Check::MaxScore {
                limit,
                per,
                scope: scope(optional_paths(issues)?),
            })
        }
        Kind::MaxFiles => {
            let limit = required_limit(location, kind, raw.limit, issues)?;
            if limit == 0 {
                return Err(issues.reject(
                    location,
                    "limit 0 forbids every change; use protect_paths to say that".to_owned(),
                ));
            }
            Ok(Check::MaxFiles {
                limit,
                scope: scope(optional_paths(issues)?),
            })
        }
    }
}

fn required_limit(
    location: &str,
    kind: Kind,
    limit: Option<u32>,
    issues: &mut Issues,
) -> Result<u32, Rejected> {
    limit.ok_or_else(|| issues.reject(location, format!("kind {:?} requires `limit`", kind.name())))
}

fn required<'a>(
    location: &str,
    field: &str,
    value: Option<&'a Vec<String>>,
    issues: &mut Issues,
) -> Result<&'a Vec<String>, Rejected> {
    match value {
        Some(list) if !list.is_empty() => Ok(list),
        Some(_) => Err(issues.reject(location, format!("`{field}` must not be empty"))),
        None => Err(issues.reject(location, format!("this kind requires `{field}`"))),
    }
}

fn validate_waiver(
    location: &str,
    raw: RawWaiver,
    rule_ids: &BTreeSet<String>,
    issues: &mut Issues,
) -> Result<Waiver, Rejected> {
    let start = issues.mark();
    if !rule_ids.contains(&raw.rule) {
        issues.push(
            location,
            format!(
                "waives rule {:?}, which the policy does not declare",
                raw.rule
            ),
        );
    }
    if raw.reason.trim().chars().count() < MIN_WHY_CHARS {
        issues.push(
            location,
            format!("`reason` must justify the exemption (at least {MIN_WHY_CHARS} characters)"),
        );
    }
    if raw.owner.trim().is_empty() {
        issues.push(
            location,
            "`owner` names who answers for the exemption; it must not be empty".to_owned(),
        );
    }
    let expires = Date::parse(&raw.expires).ok_or_else(|| {
        issues.reject(
            location,
            format!("expires {:?} is not a YYYY-MM-DD date", raw.expires),
        )
    });
    let paths = compile(location, "paths", &raw.paths, issues);
    let (paths, expires) = (paths?, expires?);
    issues.since(start)?;
    Ok(Waiver {
        rule: raw.rule,
        paths,
        reason: raw.reason.trim().to_owned(),
        owner: raw.owner.trim().to_owned(),
        expires,
    })
}
