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
use forge_warden_core::facts::{PatchFacts, Unverifiable};
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

fn facts(name: &str) -> Result<PatchFacts, Unverifiable> {
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

/// The file must be its own companion: a rule a file trips unless it can stand as one.
fn self_companion(path: &str) -> String {
    format!(
        r#"
[[rule]]
id = "asset-is-live"
why = "A changed asset must still exist after the change."
kind = "require_companion"
severity = "fail"
when = ["{path}"]
require = ["{path}"]
"#
    )
}

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

/// The violations of the policy's only rule, judged on the authority path, with the verdict.
fn companion_violations(name: &str, path: &str) -> (usize, forge_warden_core::Verdict) {
    let doc = rank(name).unwrap();
    assert_eq!(
        doc.files()
            .iter()
            .map(|f| (f.path().as_str(), f.status().name(), f.signals().len()))
            .collect::<Vec<_>>(),
        [(path, "binary_changed", 1)],
        "{name}: the producer ranked one binary file with `binary_change` only"
    );
    let p = policy(&self_companion(path));
    let ranking = produced(doc);
    let checked = check(&ranking, facts(name));
    let verdict = evaluate_authority(&p, &checked, SuppliedCopy::Absent, seals(), today());
    assert_eq!(verdict.rules().len(), 1, "{name}");
    let violations = verdict.rules().first().unwrap().violations.len();
    (violations, verdict)
}

#[test]
fn a_binary_addition_is_a_live_companion_once_git_adds_new_file() {
    // Real producer output: `binary_changed` + `binary_change`, no `new_file` (ADR 0005 fact 3).
    for (name, path) in [("m-binadd", "b.bin"), ("ts2-new-binary-add", "binary.bin")] {
        // The raw engine cannot know the file exists afterwards, so the file trips its own rule.
        let raw = evaluate(
            &policy(&self_companion(path)),
            &rank(name).unwrap(),
            seals(),
            today(),
        );
        assert_eq!(raw.rules()[0].violations.len(), 1, "{name}: raw engine");
        // On the judged path Git's `new_file` makes it a live companion: the rule is satisfied.
        let (violations, verdict) = companion_violations(name, path);
        assert_eq!(violations, 0, "{name}");
        // The producer still missed a fact, so the change holds for a person; it never passes.
        assert_eq!(verdict.outcome(), Outcome::Hold, "{name}");
        assert!(
            codes(verdict.notes()).contains(&"status_mismatch"),
            "{name}"
        );
    }
}

#[test]
fn a_binary_deletion_triggers_no_companion_rule_once_git_adds_deleted_file() {
    // Real producer output: `binary_changed` + `binary_change`, no `deleted_file`.
    for (name, path) in [
        ("new-bindel", "logo.png"),
        ("ts-binary-deletion-1", "plain.bin"),
    ] {
        // The raw engine takes the deletion for a change that needs a companion.
        let raw = evaluate(
            &policy(&self_companion(path)),
            &rank(name).unwrap(),
            seals(),
            today(),
        );
        assert_eq!(raw.rules()[0].violations.len(), 1, "{name}: raw engine");
        // On the judged path Git's `deleted_file` makes it a deletion: it triggers nothing.
        let (violations, verdict) = companion_violations(name, path);
        assert_eq!(violations, 0, "{name}");
        assert_eq!(verdict.outcome(), Outcome::Hold, "{name}");
        assert!(
            codes(verdict.notes()).contains(&"status_mismatch"),
            "{name}"
        );
    }
}

#[test]
fn a_binary_modification_is_never_a_companion() {
    // `m-binmod` is covered (the producer and Git agree), and Git derives neither `new_file` nor
    // `deleted_file`: the size of the change is unknowable, so the file cannot vouch for itself.
    let (violations, verdict) = companion_violations("m-binmod", "b.bin");
    assert_eq!(violations, 1);
    assert_eq!(verdict.outcome(), Outcome::Fail);
    assert!(!codes(verdict.notes()).contains(&"status_mismatch"));
}

#[test]
fn a_facts_error_never_passes_anywhere_in_the_corpus() {
    // With no facts from Git there is nothing to widen with, so the rules read the ranking as
    // written. `judge`'s refusal arm finishes through `Verdict::held_by`, which takes the
    // `patch_unverifiable` note as a required argument; this checks the result over the corpus.
    let index: serde_json::Value = serde_json::from_slice(&read("cases.json")).unwrap();
    let p = policy(PERMISSIVE);
    let mut raw_passes = 0_usize;
    let mut judged = 0_usize;
    for case in index["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let Some(doc) = rank(name) else { continue };
        let raw = evaluate(&p, &doc, seals(), today()).outcome();
        let ranking = produced(doc);
        let checked = check(&ranking, Err(Unverifiable::Empty));
        assert!(!checked.is_covered(), "{name}");
        let verdict = evaluate_authority(&p, &checked, SuppliedCopy::Absent, seals(), today());
        assert_ne!(verdict.outcome(), Outcome::Pass, "{name}");
        let notes = codes(verdict.notes());
        assert!(notes.contains(&"patch_unverifiable"), "{name}: {notes:?}");
        // No disagreement with Git is computed without Git's facts. A note the rule engine raises
        // from the ranking alone (`duplicate_paths`) may still be there.
        for from_facts in ["status_mismatch", "rank_incomplete", "rank_extraneous"] {
            assert!(!notes.contains(&from_facts), "{name}: {notes:?}");
        }
        if raw == Outcome::Pass {
            assert_eq!(verdict.outcome(), Outcome::Hold, "{name}");
            raw_passes += 1;
        }
        judged += 1;
    }
    // Not vacuous: the corpus has rankings the raw engine passes.
    assert!(
        raw_passes > 0 && judged >= raw_passes,
        "{raw_passes}/{judged}"
    );
}

#[test]
fn a_no_prefix_patch_is_refused_as_a_repeated_path() {
    // The patch was made with `--no-prefix` and changes `other src/a` and `src/a`. Git strips one
    // leading component from each, so its numstat lists the path `a` twice. The corpus pins only
    // the class (`unverifiable`) and the message; this pins the variant and the path it names, on
    // Git's real output (G8).
    assert_eq!(
        facts("ts-no-prefix").err(),
        Some(Unverifiable::RepeatedPath("a".to_owned()))
    );
}
