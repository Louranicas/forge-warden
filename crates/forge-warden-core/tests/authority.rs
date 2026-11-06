//! ADR 0005 §2.1/§2.7/§2.8 over the real coverage corpus: provenance, coverage notes and the T2
//! signal union, each compared with `evaluate` on the raw ranking (the behaviour before C1c).
#![expect(
    clippy::disallowed_methods,
    clippy::unwrap_used,
    reason = "test helpers read committed fixtures; a panic is a failed test"
)]

use forge_warden_core::authority::{
    AdvisoryOutcome, Ranking, check, evaluate_advisory, evaluate_authority,
};
use forge_warden_core::facts::PatchFacts;
use forge_warden_core::policy::parse_policy;
use forge_warden_core::rank::{RankDoc, parse_rank};
use forge_warden_core::verdict::Seals;
use forge_warden_core::{Date, Outcome, Policy, evaluate};
use std::path::PathBuf;

const HASH: &str = "bbf580931c6739af491bb5e2258550631327ca07f1078135cb008797e22b836e";

fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/coverage")
            .join(name),
    )
    .unwrap()
}

fn rank(name: &str) -> Option<RankDoc> {
    parse_rank(&read(&format!("{name}.rank.json"))).ok()
}

fn facts(name: &str) -> Result<PatchFacts, forge_warden_core::facts::Unverifiable> {
    PatchFacts::from_git(
        &read(&format!("{name}.numstat")),
        &read(&format!("{name}.summary")),
        &read(&format!("{name}.patch")),
    )
}

fn policy(body: &str) -> Policy {
    parse_policy(format!("schema = \"forge-warden.policy.v0\"\n{body}").as_bytes()).unwrap()
}

fn seals() -> Seals {
    Seals {
        policy_sha256: HASH.to_owned(),
        rank_sha256: HASH.to_owned(),
        patch_sha256: HASH.to_owned(),
        patch_verified: true,
        ranked_by: "0.7.0".to_owned(),
    }
}

fn today() -> Date {
    Date::parse("2026-11-06").unwrap()
}

/// A rule nothing in the corpus trips, so the raw ranking passes and only C1c can hold it.
const PERMISSIVE: &str = r#"
[[rule]]
id = "no-secrets"
why = "Credentials require immediate remediation."
kind = "forbid_signal"
severity = "fail"
signals = ["secret_like"]
"#;

const NO_DELETIONS: &str = r#"
[[rule]]
id = "keep-assets"
why = "Deleting a shipped asset needs a release decision."
kind = "forbid_signal"
severity = "fail"
signals = ["deleted_file"]
"#;

fn codes(notes: &[forge_warden_core::verdict::Note]) -> Vec<&str> {
    notes.iter().map(|n| n.code).collect()
}

#[test]
fn a_supplied_ranking_never_passes_anywhere_in_the_corpus() {
    let index: serde_json::Value = serde_json::from_slice(&read("cases.json")).unwrap();
    let p = policy(PERMISSIVE);
    let mut judged = 0;
    for case in index["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let Some(doc) = rank(name) else { continue };
        let checked = check(&doc, facts(name));
        let advisory = evaluate_advisory(&p, &Ranking::supplied(doc), &checked, seals(), today());
        assert_ne!(advisory.verdict().verdict, Outcome::Pass, "{name}");
        assert!(matches!(
            advisory.outcome(),
            AdvisoryOutcome::Hold | AdvisoryOutcome::Fail
        ));
        assert!(
            codes(&advisory.verdict().notes).contains(&"rank_unverified"),
            "{name}"
        );
        judged += 1;
    }
    assert!(judged >= 45, "only {judged} cases had a parseable ranking");
}

#[test]
fn a_forged_ranking_with_the_real_patch_holds_instead_of_passing() {
    // Mission 3 R-01: only tests/it.rs listed against the real demo patch.
    let name = "m3-keel-only-test";
    let doc = rank(name).unwrap();
    let p = policy(PERMISSIVE);
    assert_eq!(
        evaluate(&p, &doc, seals(), today()).verdict,
        Outcome::Pass,
        "the parent passed it"
    );
    let checked = check(&doc, facts(name));
    assert!(!checked.is_covered());
    let verdict = evaluate_authority(
        &p,
        &Ranking::attest(doc, HASH.to_owned()),
        &checked,
        seals(),
        today(),
    );
    assert_eq!(verdict.verdict, Outcome::Hold);
    assert_eq!(verdict.exit.code, 8);
    assert!(codes(&verdict.notes).contains(&"rank_incomplete"));
}

#[test]
fn git_adds_the_deletion_the_producer_omitted() {
    // DDF 0.7.0 ranks a binary deletion as binary_changed with no deleted_file (MATRIX fact 3).
    let name = "ts-binary-deletion-1";
    let doc = rank(name).unwrap();
    let p = policy(NO_DELETIONS);
    assert_eq!(
        evaluate(&p, &doc, seals(), today()).verdict,
        Outcome::Pass,
        "the parent missed it"
    );
    let checked = check(&doc, facts(name));
    let verdict = evaluate_authority(
        &p,
        &Ranking::attest(doc, HASH.to_owned()),
        &checked,
        seals(),
        today(),
    );
    assert_eq!(verdict.verdict, Outcome::Fail);
    assert_eq!(verdict.rules[0].violations.len(), 1);
    assert!(codes(&verdict.notes).contains(&"status_mismatch"));
}

#[test]
fn a_covered_ranking_is_judged_exactly_as_before() {
    for name in [
        "new-mode",
        "new-oddname",
        "keel-c1b-v4-respaced-ordinary-mode",
    ] {
        let doc = rank(name).unwrap();
        let p = policy(PERMISSIVE);
        let before = evaluate(&p, &doc, seals(), today());
        let checked = check(&doc, facts(name));
        assert!(checked.is_covered(), "{name}");
        let after = evaluate_authority(
            &p,
            &Ranking::attest(doc, HASH.to_owned()),
            &checked,
            seals(),
            today(),
        );
        assert_eq!(after, before, "{name}");
    }
}

#[test]
fn an_unverifiable_patch_holds_with_the_reason() {
    let name = "keel-submodule-update";
    let doc = rank(name).unwrap();
    let checked = check(&doc, facts(name));
    let verdict = evaluate_authority(
        &policy(PERMISSIVE),
        &Ranking::attest(doc, HASH.to_owned()),
        &checked,
        seals(),
        today(),
    );
    assert_eq!(verdict.verdict, Outcome::Hold);
    let note = verdict
        .notes
        .iter()
        .find(|n| n.code == "patch_unverifiable")
        .unwrap();
    assert!(
        note.detail.contains("unsupported file mode 160000"),
        "{}",
        note.detail
    );
}

#[test]
fn only_a_produced_ranking_names_its_producer() {
    let doc = rank("new-mode").unwrap();
    assert_eq!(Ranking::supplied(doc.clone()).producer_sha256(), None);
    let produced = Ranking::attest(doc.clone(), HASH.to_owned());
    assert_eq!(produced.producer_sha256(), Some(HASH));
    assert_eq!(produced.doc(), &doc);
}
