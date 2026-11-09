//! ADR 0005 §2.1/§2.7/§2.8 over the real coverage corpus: provenance, coverage notes and the T2
//! signal union, each compared with `evaluate` on the raw ranking (the behaviour before C1c).
#![expect(
    clippy::disallowed_methods,
    clippy::unwrap_used,
    reason = "test helpers read committed fixtures; a panic is a failed test"
)]

#[path = "support/corpus_manifest.rs"]
mod corpus_manifest;

use forge_warden_core::authority::{
    AdvisoryOutcome, Produced, Ranking, SuppliedCopy, check, evaluate_advisory, evaluate_authority,
};
use forge_warden_core::facts::PatchFacts;
use forge_warden_core::policy::parse_policy;
use forge_warden_core::rank::{RankDoc, parse_rank};
use forge_warden_core::testing::evaluate;
use forge_warden_core::verdict::{Provenance, RankedBy, Seals};
use forge_warden_core::{Date, Outcome, Policy};
use std::path::PathBuf;

/// A test seal from a known-good constant.
fn seal(text: &str) -> forge_warden_core::seal::Sha256Hex {
    forge_warden_core::seal::Sha256Hex::parse(text).unwrap()
}

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
    Seals::new(seal(HASH), seal(HASH), seal(HASH), "0.7.0".to_owned())
}

/// The seals an authority verdict carries for a ranking attested with `HASH`.
fn produced_seals() -> Seals {
    let mut seals = seals();
    seals.provenance = Provenance::Produced;
    seals.ranked_by.sha256 = Some(seal(HASH));
    seals
}

/// `doc` attested as produced by `HASH`, admitted by a policy whose allowlist lists it: the only
/// way to obtain the token (Q-TYPES-1).
fn produced(doc: RankDoc) -> Ranking<Produced> {
    let admitting = policy(&format!("[producer]\nsha256 = [\"{HASH}\"]\n{PERMISSIVE}"));
    Ranking::attest(doc, admitting.admit_producer(&seal(HASH)).unwrap())
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
    notes.iter().map(|n| n.code.name()).collect()
}

#[test]
fn a_supplied_ranking_never_passes_anywhere_in_the_corpus() {
    let index: serde_json::Value = serde_json::from_slice(&read("cases.json")).unwrap();
    let p = policy(PERMISSIVE);
    let mut judged = Vec::new();
    for case in index["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let Some(doc) = rank(name) else { continue };
        let supplied = Ranking::supplied(doc);
        let advisory = evaluate_advisory(&p, &check(&supplied, facts(name)), seals(), today());
        assert_ne!(advisory.verdict().outcome(), Outcome::Pass, "{name}");
        assert!(matches!(
            advisory.outcome(),
            AdvisoryOutcome::Hold | AdvisoryOutcome::Fail
        ));
        assert!(
            codes(advisory.verdict().notes()).contains(&"rank_unverified"),
            "{name}"
        );
        judged.push((name.to_owned(), read(&format!("{name}.rank.json"))));
    }
    corpus_manifest::check("parseable-rankings", judged).unwrap();
}

#[test]
fn a_forged_ranking_with_the_real_patch_holds_instead_of_passing() {
    // Mission 3 R-01: only tests/it.rs listed against the real demo patch.
    let name = "m3-keel-only-test";
    let doc = rank(name).unwrap();
    let p = policy(PERMISSIVE);
    assert_eq!(
        evaluate(&p, &doc, seals(), today()).outcome(),
        Outcome::Pass,
        "the parent passed it"
    );
    let ranking = produced(doc);
    let checked = check(&ranking, facts(name));
    assert!(!checked.is_covered());
    let verdict = evaluate_authority(&p, &checked, SuppliedCopy::Absent, seals(), today());
    assert_eq!(verdict.outcome(), Outcome::Hold);
    assert_eq!(verdict.exit().code, 8);
    assert!(codes(verdict.notes()).contains(&"rank_incomplete"));
}

#[test]
fn git_adds_the_deletion_the_producer_omitted() {
    // DDF 0.7.0 ranks a binary deletion as binary_changed with no deleted_file (MATRIX fact 3).
    let name = "ts-binary-deletion-1";
    let doc = rank(name).unwrap();
    let p = policy(NO_DELETIONS);
    assert_eq!(
        evaluate(&p, &doc, seals(), today()).outcome(),
        Outcome::Pass,
        "the parent missed it"
    );
    let ranking = produced(doc);
    let checked = check(&ranking, facts(name));
    let verdict = evaluate_authority(&p, &checked, SuppliedCopy::Absent, seals(), today());
    assert_eq!(verdict.outcome(), Outcome::Fail);
    assert_eq!(verdict.rules()[0].violations.len(), 1);
    assert!(codes(verdict.notes()).contains(&"status_mismatch"));
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
        let before = evaluate(&p, &doc, produced_seals(), today());
        let ranking = produced(doc);
        let checked = check(&ranking, facts(name));
        assert!(checked.is_covered(), "{name}");
        let after = evaluate_authority(&p, &checked, SuppliedCopy::Absent, seals(), today());
        assert_eq!(after, before, "{name}");
    }
}

#[test]
fn an_unverifiable_patch_holds_with_the_reason() {
    let name = "keel-submodule-update";
    let doc = rank(name).unwrap();
    let ranking = produced(doc);
    let checked = check(&ranking, facts(name));
    let verdict = evaluate_authority(
        &policy(PERMISSIVE),
        &checked,
        SuppliedCopy::Absent,
        seals(),
        today(),
    );
    assert_eq!(verdict.outcome(), Outcome::Hold);
    let note = verdict
        .notes()
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
    let ranking = produced(doc.clone());
    assert_eq!(
        ranking
            .producer_sha256()
            .map(forge_warden_core::seal::Sha256Hex::as_str),
        Some(HASH)
    );
    assert_eq!(ranking.doc(), &doc);
}

#[test]
fn a_differing_supplied_copy_holds_and_an_identical_one_changes_nothing() {
    assert_eq!(SuppliedCopy::compare(None, b"x"), SuppliedCopy::Absent);
    assert_eq!(
        SuppliedCopy::compare(Some(b"x"), b"x"),
        SuppliedCopy::Identical
    );
    assert_eq!(
        SuppliedCopy::compare(Some(b"x "), b"x"),
        SuppliedCopy::Different
    );
    assert_eq!(
        SuppliedCopy::compare(Some(b""), b"x"),
        SuppliedCopy::Different
    );

    let name = "new-mode";
    let doc = rank(name).unwrap();
    let p = policy(PERMISSIVE);
    let ranking = produced(doc);
    let checked = check(&ranking, facts(name));
    let judge = |copy| evaluate_authority(&p, &checked, copy, seals(), today());
    let absent = judge(SuppliedCopy::Absent);
    assert_eq!(absent.outcome(), Outcome::Pass);
    assert_eq!(judge(SuppliedCopy::Identical), absent);
    let different = judge(SuppliedCopy::Different);
    assert_eq!(different.outcome(), Outcome::Hold);
    assert_eq!(different.exit().code, 8);
    assert_eq!(codes(different.notes()), ["rank_mismatch"]);
    assert_eq!(
        different.rules(),
        absent.rules(),
        "the produced ranking is still the one judged"
    );
}

#[test]
fn provenance_and_the_ranker_are_sealed_from_the_ranking_not_the_caller() {
    let name = "new-mode";
    let doc = rank(name).unwrap();
    let p = policy(PERMISSIVE);
    let ranking = produced(doc.clone());
    let supplied = Ranking::supplied(doc.clone());
    let mut lying = seals();
    lying.provenance = Provenance::Produced;
    lying.ranked_by = RankedBy::new("9.9.9".to_owned(), Some(seal(&"f".repeat(64))));
    let authority = evaluate_authority(
        &p,
        &check(&ranking, facts(name)),
        SuppliedCopy::Absent,
        lying.clone(),
        today(),
    );
    assert_eq!(authority.seals().provenance, Provenance::Produced);
    assert_eq!(
        authority.seals().ranked_by,
        RankedBy::new(doc.tool_version().to_owned(), Some(seal(HASH)))
    );
    let advisory = evaluate_advisory(&p, &check(&supplied, facts(name)), lying, today());
    assert_eq!(advisory.verdict().seals().provenance, Provenance::Supplied);
    assert_eq!(
        advisory.verdict().seals().ranked_by,
        RankedBy::new(doc.tool_version().to_owned(), None)
    );
}

#[test]
fn a_recorded_git_and_the_verdict_report_exactly_what_was_recorded() {
    use forge_warden_core::seal::sha256_hex;
    use forge_warden_core::verdict::{Admission, GitOracle, VERDICT_SCHEMA};
    let (binary, allowlist) = (sha256_hex(b"git binary"), sha256_hex(b"owner policy"));
    let approved = GitOracle::approved(
        "/usr/bin/git".to_owned(),
        "git version 2.51.0".to_owned(),
        binary.clone(),
        allowlist.clone(),
    );
    assert_eq!(approved.path(), "/usr/bin/git");
    assert_eq!(approved.version(), "git version 2.51.0");
    assert_eq!(approved.sha256(), &binary);
    assert_eq!(approved.admission(), Admission::Approved);
    assert_eq!(approved.allowlist_sha256(), Some(&allowlist));
    let unpinned = GitOracle::unpinned("/bin/git".to_owned(), "v".to_owned(), binary);
    assert_eq!(unpinned.admission(), Admission::Unpinned);
    assert_eq!(unpinned.allowlist_sha256(), None);

    let supplied = Ranking::supplied(rank("m3-demo-real").unwrap());
    let checked = check(&supplied, facts("m3-demo-real"));
    let judged = evaluate_advisory(&policy(PERMISSIVE), &checked, seals(), today());
    assert_eq!(judged.verdict().schema(), VERDICT_SCHEMA);
    assert_eq!(VERDICT_SCHEMA, "forge-warden.verdict.v2");
}

#[test]
fn advisory_git_admission_note_is_composed_by_core() {
    use forge_warden_core::verdict::GitOracle;
    let supplied = Ranking::supplied(rank("m3-demo-real").unwrap());
    let checked = check(&supplied, facts("m3-demo-real"));
    let p = policy(PERMISSIVE);
    let original =
        "the ranking was supplied, not produced by forge-warden; it can inform, never approve";
    for (git, suffix) in [
        (None, ""),
        (
            Some(GitOracle::approved(
                "/usr/bin/git".to_owned(),
                "git fixture".to_owned(),
                seal(HASH),
                seal(HASH),
            )),
            "",
        ),
        (
            Some(GitOracle::unpinned(
                "/usr/bin/git".to_owned(),
                "git fixture".to_owned(),
                seal(HASH),
            )),
            "; Git is unpinned: no owner allowlist enrolled; its recorded executable digest is not approved",
        ),
    ] {
        let mut evidence = seals();
        evidence.git = git;
        let judged = evaluate_advisory(&p, &checked, evidence, today());
        let note = judged
            .verdict()
            .notes()
            .iter()
            .find(|note| note.code.name() == "rank_unverified")
            .unwrap();
        assert_eq!(note.detail, format!("{original}{suffix}"));
        assert_ne!(judged.verdict().outcome(), Outcome::Pass);
    }
}
