//! Behaviour of `evaluate`: one test per clause of its contract (see `src/eval.rs`), plus the
//! properties that must hold for every policy and ranking.
#![expect(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "test helpers outside #[test] fns: a panic here is a failed test, which is the point"
)]

use forge_warden_core::path::RepoPath;
use forge_warden_core::policy::parse_policy;
use forge_warden_core::rank::{Evidence, RankDoc, RankedFile};
use forge_warden_core::verdict::{RuleOutcome, Seals};
use forge_warden_core::{Date, Outcome, evaluate};
use proptest::prelude::*;

const SEAL: &str = "bbf580931c6739af491bb5e2258550631327ca07f1078135cb008797e22b836e";

fn file(path: &str, status: &str, score: u32, signals: &[&str]) -> RankedFile {
    file_with(path, status, score, signals, Vec::new(), false)
}

fn file_with(
    path: &str,
    status: &str,
    score: u32,
    signals: &[&str],
    evidence: Vec<Evidence>,
    evidence_truncated: bool,
) -> RankedFile {
    RankedFile::new(
        RepoPath::parse(path).unwrap(),
        status.to_owned(),
        score,
        signals.iter().map(|s| (*s).to_owned()).collect(),
        evidence,
        evidence_truncated,
    )
    .unwrap()
}

fn evidence(signal: &str, side: &str, line: u64) -> Evidence {
    Evidence::new(signal.to_owned(), SEAL.to_owned(), side.to_owned(), line).unwrap()
}

fn rank(files: Vec<RankedFile>) -> RankDoc {
    RankDoc::new(SEAL.to_owned(), "0.7.0".to_owned(), files).unwrap()
}

fn seals() -> Seals {
    Seals::new(
        SEAL.to_owned(),
        SEAL.to_owned(),
        SEAL.to_owned(),
        "0.7.0".to_owned(),
    )
}

fn today() -> Date {
    Date::new(2026, 11, 5).unwrap()
}

fn policy(rules: &str) -> forge_warden_core::Policy {
    let text = format!("schema = \"forge-warden.policy.v0\"\n{rules}");
    match parse_policy(text.as_bytes()) {
        Ok(policy) => policy,
        Err(e) => panic!("{e}"),
    }
}

const NO_UNSAFE: &str = r#"
[[rule]]
id = "no-unsafe"
why = "Unsafe code needs a dedicated safety review."
kind = "forbid_signal"
severity = "fail"
signals = ["unsafe_added"]
except = ["crates/ffi/**"]
"#;

const TESTS_WITH_SOURCE: &str = r#"
[[rule]]
id = "tests-with-source"
why = "Behaviour changes ship with the tests that pin them."
kind = "require_companion"
severity = "fail"
when = ["src/**"]
require = ["tests/**"]
"#;

#[test]
fn the_text_summary_shows_outcome_seals_evidence_and_notes() {
    // The core owns the human summary; these facts are pinned here, not only by CLI tests.
    let lib = file_with(
        "src/lib.rs",
        "modified",
        9,
        &["unsafe_added"],
        vec![evidence("unsafe_added", "old", 3)],
        false,
    );
    let odd = file("docs/x.md", "unknown", 0, &[]);
    let verdict = evaluate(&policy(NO_UNSAFE), &rank(vec![lib, odd]), seals(), today());
    let citation = &verdict.rules[0].violations[0].evidence[0];
    assert_eq!(
        (
            citation.address.as_str(),
            citation.side.as_str(),
            citation.line
        ),
        (SEAL, "old", 3)
    );
    let text = forge_warden_core::render::render_text(&verdict);
    let short = SEAL.get(..12).unwrap();
    assert!(text.starts_with("forge-warden "), "{text}");
    assert!(text.contains(": FAIL (exit 7 policy_fail"), "{text}");
    assert!(
        text.contains(&format!(
            "patch {short}, policy {short}, ranked by deep-diff-forge 0.7.0 (supplied, unverified), on 2026-11-05"
        )),
        "{text}"
    );
    assert!(
        text.contains(&format!("at old line 3 (hunk {short})")),
        "{text}"
    );
    assert!(text.contains("[NOTE] unknown_status docs/x.md:"), "{text}");
}

#[test]
fn forbid_signal_cites_evidence_and_respects_except() {
    let lib = file_with(
        "src/lib.rs",
        "modified",
        9,
        &["public_api_surface", "unsafe_added"],
        vec![evidence("unsafe_added", "new", 3)],
        false,
    );
    let ffi = file("crates/ffi/src/lib.rs", "modified", 9, &["unsafe_added"]);
    let verdict = evaluate(&policy(NO_UNSAFE), &rank(vec![lib, ffi]), seals(), today());

    assert_eq!(verdict.verdict, Outcome::Fail);
    assert_eq!(verdict.exit.code, 7);
    assert_eq!(verdict.rules[0].severity, "fail");
    let rule = &verdict.rules[0];
    assert_eq!(
        rule.violations.len(),
        1,
        "the excepted ffi crate is out of scope"
    );
    assert_eq!(rule.violations[0].path.as_deref(), Some("src/lib.rs"));
    assert_eq!(rule.violations[0].evidence[0].line, 3);
}

#[test]
fn require_companion_is_satisfied_by_a_live_companion_only() {
    let p = policy(TESTS_WITH_SOURCE);
    let with_tests = rank(vec![
        file("src/a.rs", "modified", 2, &[]),
        file("tests/a.rs", "modified", 1, &["test_only"]),
    ]);
    assert_eq!(
        evaluate(&p, &with_tests, seals(), today()).verdict,
        Outcome::Pass
    );

    let deleting_tests = rank(vec![
        file("src/a.rs", "modified", 2, &[]),
        file("tests/a.rs", "deleted", 4, &["deleted_file"]),
    ]);
    let verdict = evaluate(&p, &deleting_tests, seals(), today());
    assert_eq!(
        verdict.verdict,
        Outcome::Fail,
        "deleting tests is not a companion change"
    );
    // The violation tells the author which companion the rule wanted (PathSet::patterns).
    assert_eq!(
        verdict.rules[0].violations[0].message,
        "changed without a companion change matching [tests/**]"
    );

    let deleting_source = rank(vec![file("src/a.rs", "deleted", 4, &["deleted_file"])]);
    assert_eq!(
        evaluate(&p, &deleting_source, seals(), today()).verdict,
        Outcome::Pass,
        "removing code asks for no new tests"
    );
}

#[test]
fn protect_paths_holds_rather_than_fails_when_so_declared() {
    let p = policy(
        r#"
[[rule]]
id = "ci-is-owned"
why = "CI definitions decide what every other check means."
kind = "protect_paths"
severity = "hold"
paths = [".github/workflows/**"]
"#,
    );
    let verdict = evaluate(
        &p,
        &rank(vec![file(
            ".github/workflows/ci.yml",
            "modified",
            6,
            &["ci_privilege"],
        )]),
        seals(),
        today(),
    );
    assert_eq!(verdict.verdict, Outcome::Hold);
    assert_eq!(verdict.exit.code, 8);
    assert_eq!(verdict.exit.action, "review_evidence");
    assert_eq!(verdict.rules[0].severity, "hold");
    assert_eq!(verdict.rules[0].kind, "protect_paths");
}

#[test]
fn score_and_file_budgets() {
    let p = policy(
        r#"
[[rule]]
id = "file-budget"
why = "A single file this risky deserves its own change."
kind = "max_score"
severity = "hold"
limit = 10
per = "file"

[[rule]]
id = "change-budget"
why = "Large risky changes are reviewed in pieces."
kind = "max_score"
severity = "fail"
limit = 15
per = "change"

[[rule]]
id = "small-changes"
why = "Small changes get real review."
kind = "max_files"
severity = "hold"
limit = 2
paths = ["src/**"]
"#,
    );
    let r = rank(vec![
        file("src/a.rs", "modified", 11, &[]),
        file("src/b.rs", "modified", 3, &[]),
        file("src/c.rs", "modified", 2, &[]),
        file("docs/x.md", "modified", 2, &[]),
    ]);
    let verdict = evaluate(&p, &r, seals(), today());
    let outcomes: Vec<_> = verdict.rules.iter().map(|rule| rule.outcome).collect();
    assert_eq!(
        outcomes,
        [RuleOutcome::Hold, RuleOutcome::Fail, RuleOutcome::Hold]
    );
    assert_eq!(verdict.verdict, Outcome::Fail);
    assert!(
        verdict.rules[1].violations[0].path.is_none(),
        "a change-wide budget has no path"
    );
}

/// Boundaries found by cargo-mutants: `>` vs `>=`, and scope vs score.
#[test]
fn budgets_are_strict_at_the_boundary_and_respect_scope() {
    let p = policy(
        r#"
[[rule]]
id = "file-budget"
why = "A single file this risky deserves its own change."
kind = "max_score"
severity = "fail"
limit = 10
per = "file"
paths = ["src/**"]

[[rule]]
id = "change-budget"
why = "Large risky changes are reviewed in pieces."
kind = "max_score"
severity = "fail"
limit = 20
per = "change"
paths = ["src/**"]

[[rule]]
id = "small-changes"
why = "Small changes get real review."
kind = "max_files"
severity = "fail"
limit = 2
paths = ["src/**"]
"#,
    );
    let at_limit = rank(vec![
        file("src/a.rs", "modified", 10, &[]),
        file("src/b.rs", "modified", 10, &[]),
        file("docs/huge.md", "modified", 99, &[]),
    ]);
    let verdict = evaluate(&p, &at_limit, seals(), today());
    assert_eq!(
        verdict.verdict,
        Outcome::Pass,
        "a score equal to its limit is within it, and docs/ is out of scope: {verdict:#?}"
    );

    let over = rank(vec![
        file("src/a.rs", "modified", 11, &[]),
        file("src/b.rs", "modified", 10, &[]),
        file("src/c.rs", "modified", 0, &[]),
    ]);
    let over_verdict = evaluate(&p, &over, seals(), today());
    let outcomes: Vec<_> = over_verdict.rules.iter().map(|r| r.outcome).collect();
    assert_eq!(
        outcomes,
        [RuleOutcome::Fail, RuleOutcome::Fail, RuleOutcome::Fail]
    );
    let over_file_budget: Vec<_> = over_verdict.rules[0]
        .violations
        .iter()
        .map(|v| v.path.as_deref())
        .collect();
    assert_eq!(
        over_file_budget,
        [Some("src/a.rs")],
        "only the file above its limit"
    );
}

#[test]
fn waivers_cover_only_their_own_rule_and_paths() {
    let text = format!(
        "{NO_UNSAFE}{TESTS_WITH_SOURCE}
[[waiver]]
rule = \"tests-with-source\"
paths = [\"src/**\"]
reason = \"Covers the other rule, never no-unsafe.\"
owner = \"@louranicas\"
expires = \"2099-01-01\"

[[waiver]]
rule = \"no-unsafe\"
paths = [\"src/simd/**\"]
reason = \"Covers a different directory only.\"
owner = \"@louranicas\"
expires = \"2099-01-01\"
"
    );
    let r = rank(vec![
        file("src/lib.rs", "modified", 9, &["unsafe_added"]),
        file("tests/it.rs", "modified", 1, &["test_only"]),
    ]);
    let verdict = evaluate(&policy(&text), &r, seals(), today());
    assert_eq!(verdict.rules[0].outcome, RuleOutcome::Fail);
    assert_eq!(verdict.rules[0].violations[0].waived_by, None);
    assert_eq!(
        verdict.rules[0].violations[0].signal.as_deref(),
        Some("unsafe_added")
    );
    assert_eq!(verdict.verdict, Outcome::Fail);
}

#[test]
fn waivers_cover_matching_paths_until_they_expire() {
    let waived = format!(
        "{NO_UNSAFE}
[[waiver]]
rule = \"no-unsafe\"
paths = [\"src/simd/**\"]
reason = \"Audited SIMD kernel; see docs/adr/0007.\"
owner = \"@louranicas\"
expires = \"2026-11-05\"
"
    );
    let p = policy(&waived);
    let r = rank(vec![file(
        "src/simd/k.rs",
        "modified",
        9,
        &["unsafe_added"],
    )]);

    let on_last_day = evaluate(&p, &r, seals(), today());
    assert_eq!(on_last_day.verdict, Outcome::Pass);
    assert_eq!(on_last_day.rules[0].outcome, RuleOutcome::Waived);
    assert_eq!(on_last_day.rules[0].violations[0].waived_by, Some(0));
    assert!(
        on_last_day.unused_waivers.is_empty(),
        "a waiver that covered something is used"
    );

    let next_day = evaluate(&p, &r, seals(), Date::new(2026, 11, 6).unwrap());
    assert_eq!(next_day.verdict, Outcome::Fail);
    assert_eq!(next_day.expired_waivers.len(), 1);
    assert!(
        next_day.unused_waivers.is_empty(),
        "an expired waiver is reported as expired, not unused"
    );

    let unrelated = evaluate(
        &p,
        &rank(vec![file("README.md", "modified", 2, &[])]),
        seals(),
        today(),
    );
    assert_eq!(unrelated.verdict, Outcome::Pass);
    assert_eq!(
        unrelated.unused_waivers.len(),
        1,
        "an unused waiver is reported"
    );

    let expired_and_unrelated = evaluate(
        &p,
        &rank(vec![file("README.md", "modified", 2, &[])]),
        seals(),
        Date::new(2027, 1, 1).unwrap(),
    );
    assert_eq!(
        expired_and_unrelated.verdict,
        Outcome::Fail,
        "an expired waiver fails even when unused"
    );
}

#[test]
fn change_wide_budgets_cannot_be_waived() {
    let text = r#"
[[rule]]
id = "small-changes"
why = "Small changes get real review."
kind = "max_files"
severity = "fail"
limit = 1

[[waiver]]
rule = "small-changes"
paths = ["**"]
reason = "Attempting to waive a whole-change budget."
owner = "@someone"
expires = "2099-01-01"
"#;
    let r = rank(vec![file("a", "added", 3, &[]), file("b", "added", 3, &[])]);
    assert_eq!(
        evaluate(&policy(text), &r, seals(), today()).verdict,
        Outcome::Fail
    );
}

#[test]
fn what_the_policy_cannot_judge_holds() {
    let p = policy(NO_UNSAFE);
    let empty = evaluate(&p, &rank(Vec::new()), seals(), today());
    assert_eq!(empty.verdict, Outcome::Hold);
    assert_eq!(empty.notes[0].code, "empty_ranking");

    let future = evaluate(
        &p,
        &rank(vec![file("a.rs", "teleported", 2, &["quantum_entangled"])]),
        seals(),
        today(),
    );
    assert_eq!(future.verdict, Outcome::Hold);
    let codes: Vec<_> = future.notes.iter().map(|n| n.code).collect();
    assert_eq!(codes, ["unknown_signal", "unknown_status"]);
}

// Regressions from the independent review of 2026-11-05: each of these passed (exit 0) before.

/// rank.v0 reports only a renamed file's new path, so renaming a protected file away would pass
/// every path rule. A rename or copy now holds until it is ranked with `--no-renames`.
#[test]
fn renames_and_copies_hold_because_their_source_is_invisible() {
    let p = policy(
        r#"
[[rule]]
id = "policy-is-owned"
why = "The policy decides what every other check means."
kind = "protect_paths"
severity = "hold"
paths = ["warden.toml"]
"#,
    );
    for status in ["renamed", "copied"] {
        let r = rank(vec![file("docs/old-policy.toml", status, 2, &[])]);
        let verdict = evaluate(&p, &r, seals(), today());
        assert_eq!(verdict.verdict, Outcome::Hold, "{status}");
        assert_eq!(verdict.notes[0].code, "source_path_unknown");
    }
    // Ranked with --no-renames, the same change is a deletion plus an addition: fully judged.
    let split = rank(vec![
        file("warden.toml", "deleted", 4, &["deleted_file"]),
        file("docs/old-policy.toml", "added", 3, &["new_file"]),
    ]);
    let verdict = evaluate(&p, &split, seals(), today());
    assert_eq!(verdict.rules[0].outcome, RuleOutcome::Hold);
    assert_eq!(
        verdict.notes,
        Vec::new(),
        "a --no-renames split leaves nothing unjudged"
    );
}

/// deep-diff-forge labels a deleted binary file `binary_changed` (with the `deleted_file`
/// signal), so status alone let a binary test deletion stand as the companion change.
#[test]
fn binary_deletions_and_unknowable_binary_changes_are_not_companions() {
    let p = policy(TESTS_WITH_SOURCE);
    let source = || file("src/a.rs", "modified", 2, &[]);
    let deleted_blob = rank(vec![
        source(),
        file(
            "tests/blob.bin",
            "binary_changed",
            4,
            &["deleted_file", "binary_change"],
        ),
    ]);
    assert_eq!(
        evaluate(&p, &deleted_blob, seals(), today()).verdict,
        Outcome::Fail
    );
    let changed_blob = rank(vec![
        source(),
        file("tests/blob.bin", "binary_changed", 2, &["binary_change"]),
    ]);
    assert_eq!(
        evaluate(&p, &changed_blob, seals(), today()).verdict,
        Outcome::Fail
    );
    let new_blob = rank(vec![
        source(),
        file(
            "tests/blob.bin",
            "binary_changed",
            4,
            &["new_file", "binary_change"],
        ),
    ]);
    assert_eq!(
        evaluate(&p, &new_blob, seals(), today()).verdict,
        Outcome::Pass
    );
    let deleted_source = rank(vec![file(
        "src/a.rs",
        "binary_changed",
        4,
        &["deleted_file"],
    )]);
    assert_eq!(
        evaluate(&p, &deleted_source, seals(), today()).verdict,
        Outcome::Pass,
        "a deletion triggers nothing"
    );
}

/// deep-diff-forge writes status `unknown` when it cannot tell what happened to a file.
#[test]
fn status_unknown_holds() {
    let r = rank(vec![file("src/x.rs", "unknown", 0, &[])]);
    let verdict = evaluate(&policy(NO_UNSAFE), &r, seals(), today());
    assert_eq!(verdict.verdict, Outcome::Hold);
    assert_eq!(verdict.notes[0].code, "unknown_status");
}

#[test]
fn truncated_evidence_is_flagged_on_the_violation() {
    let lib = file_with(
        "src/lib.rs",
        "modified",
        9,
        &["unsafe_added"],
        Vec::new(),
        true,
    );
    let verdict = evaluate(&policy(NO_UNSAFE), &rank(vec![lib]), seals(), today());
    assert!(verdict.rules[0].violations[0].evidence_truncated);
    let json = verdict.to_json();
    assert!(json.contains("\"evidence_truncated\": true"));
    let quiet = evaluate(
        &policy(NO_UNSAFE),
        &rank(vec![file("src/lib.rs", "modified", 9, &["unsafe_added"])]),
        seals(),
        today(),
    );
    assert!(
        !quiet.to_json().contains("evidence_truncated"),
        "absent when false"
    );
}

#[test]
fn serialization_is_deterministic_and_round_trips_as_json() {
    let r = rank(vec![file("src/lib.rs", "modified", 9, &["unsafe_added"])]);
    let p = policy(&format!("{NO_UNSAFE}{TESTS_WITH_SOURCE}"));
    let a = evaluate(&p, &r, seals(), today()).to_json();
    let b = evaluate(&p, &r, seals(), today()).to_json();
    assert_eq!(a, b);
    let value: serde_json::Value = serde_json::from_str(&a).unwrap();
    assert_eq!(value["schema"], "forge-warden.verdict.v1");
    assert_eq!(value["rules"][1]["violations"][0]["path"], "src/lib.rs");
}

// ---------------------------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------------------------

const PATHS: [&str; 6] = [
    "src/lib.rs",
    "src/a/b.rs",
    "tests/it.rs",
    "README.md",
    ".github/workflows/ci.yml",
    "crates/ffi/src/lib.rs",
];
const SIGNALS: [&str; 5] = [
    "unsafe_added",
    "secret_like",
    "test_only",
    "public_api_surface",
    "process_spawn",
];
const STATUSES: [&str; 3] = ["modified", "added", "deleted"];

fn arb_file() -> impl Strategy<Value = RankedFile> {
    (
        0..PATHS.len(),
        0..STATUSES.len(),
        0_u32..30,
        proptest::sample::subsequence(SIGNALS.to_vec(), 0..=3),
    )
        .prop_map(|(p, s, score, signals)| file(PATHS[p], STATUSES[s], score, &signals))
}

const RULES: [&str; 5] = [
    NO_UNSAFE,
    TESTS_WITH_SOURCE,
    "[[rule]]\nid = \"budget\"\nwhy = \"Large risky changes are reviewed in pieces.\"\nkind = \"max_score\"\nseverity = \"hold\"\nlimit = 20\nper = \"change\"\n",
    "[[rule]]\nid = \"ci\"\nwhy = \"CI definitions decide what every other check means.\"\nkind = \"protect_paths\"\nseverity = \"hold\"\npaths = [\".github/**\"]\n",
    "[[rule]]\nid = \"secrets\"\nwhy = \"A credential committed once is compromised forever.\"\nkind = \"forbid_signal\"\nseverity = \"fail\"\nsignals = [\"secret_like\"]\n",
];

// Two complementary checks in tests/laws.rs (WP-B5) replace the old properties here:
// `adding_a_rule_cannot_loosen_the_verdict` compares the evaluator's verdict before and after one
// added rule (monotonicity), and `all_rule_kinds_match_the_independent_reference` separately compares
// the evaluator with an independent reference model over all rule kinds and waivers. Determinism
// holds by construction: the core has no effects (clippy.toml, repo_invariants) and serialization is
// pinned above.
proptest! {
    /// The summary always accounts for every rule, and the exit code always matches the verdict.
    #[test]
    fn summary_and_exit_are_consistent(files in proptest::collection::vec(arb_file(), 0..6)) {
        let all: String = RULES.concat();
        let v = evaluate(&policy(&all), &rank(files), seals(), today());
        let s = v.summary;
        prop_assert_eq!(s.passed + s.waived + s.held + s.failed, s.rules);
        prop_assert_eq!(v.exit.code, v.verdict.exit_entry().code);
        if s.failed > 0 { prop_assert_eq!(v.verdict, Outcome::Fail); }
    }
}
