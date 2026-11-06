//! Policy reader tests.

use super::validate::is_kebab_id;
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
    assert_eq!(policy.rules().len(), 1);
    assert_eq!(policy.rules().first().check.kind(), Kind::ForbidSignal);
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
    let Check::ProtectPaths { scope } = &policy.rules().first().check else {
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
fn empty_lists_are_refused_because_they_would_never_fire() {
    let issues = issues_of(&rule(&GOOD.replace("[\"secret_like\"]", "[]")));
    assert!(
        issues
            .iter()
            .any(|i| i.contains("`signals` must not be empty")),
        "{issues:?}"
    );
    let companion = r#"id = "c"
why = "Behaviour changes ship with their tests."
kind = "require_companion"
severity = "fail"
when = []
require = ["tests/**"]"#;
    assert!(
        issues_of(&rule(companion))
            .iter()
            .any(|i| i.contains("`when` must not be empty"))
    );
}

#[test]
fn rationale_minimums_are_inclusive() {
    let why = |text: &str| {
        rule(&GOOD.replace("A credential committed once is compromised forever.", text))
    };
    assert!(parse_policy(why(&"x".repeat(MIN_WHY_CHARS)).as_bytes()).is_ok());
    assert!(parse_policy(why(&"x".repeat(MIN_WHY_CHARS - 1)).as_bytes()).is_err());

    let waiver = |reason: &str| {
        format!(
            "{}[[waiver]]\nrule = \"no-secrets\"\npaths = [\"a\"]\nreason = \"{reason}\"\nowner = \"@o\"\nexpires = \"2030-01-01\"\n",
            rule(GOOD)
        )
    };
    assert!(parse_policy(waiver(&"y".repeat(MIN_WHY_CHARS)).as_bytes()).is_ok());
    assert!(parse_policy(waiver(&"y".repeat(MIN_WHY_CHARS - 1)).as_bytes()).is_err());
}

#[test]
fn kebab_ids_are_exactly_specified() {
    for good in [
        "a",
        "no-secrets",
        "rule-2",
        "a1-b2-c3",
        &"a".repeat(MAX_ID_CHARS),
    ] {
        assert!(is_kebab_id(good), "{good:?} should be accepted");
    }
    let too_long = "a".repeat(MAX_ID_CHARS + 1);
    for bad in [
        "",
        "-a",
        "a-",
        "a--b",
        "1rule",
        "No-Secrets",
        "no_secrets",
        "no secrets",
        "é",
        too_long.as_str(),
    ] {
        assert!(!is_kebab_id(bad), "{bad:?} should be refused");
    }
}

#[test]
fn the_policy_budget_is_enforced_before_parsing() {
    let over = vec![b' '; crate::MAX_INPUT_BYTES + 1];
    assert_eq!(
        parse_policy(&over).unwrap_err(),
        PolicyError::TooLarge(crate::MAX_INPUT_BYTES + 1)
    );
    let at = vec![b' '; crate::MAX_INPUT_BYTES];
    assert!(
        matches!(parse_policy(&at), Err(PolicyError::Malformed(_))),
        "at the budget it is parsed"
    );
}

#[test]
fn invalid_policies_print_one_issue_per_line() {
    let error = PolicyError::Invalid(vec![
        PolicyIssue {
            location: "policy".to_owned(),
            message: "first".to_owned(),
        },
        PolicyIssue {
            location: "rule[0] \"x\"".to_owned(),
            message: "second".to_owned(),
        },
    ]);
    assert_eq!(
        error.to_string(),
        "policy refused with 2 issue(s):\n  - policy: first\n  - rule[0] \"x\": second"
    );
}

#[test]
fn non_utf8_is_malformed() {
    assert!(matches!(
        parse_policy(b"\xff"),
        Err(PolicyError::Malformed(_))
    ));
}
