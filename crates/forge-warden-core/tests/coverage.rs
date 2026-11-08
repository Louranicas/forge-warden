//! ADR 0005 §2.2/§2.9 over a permanent corpus: reviewed real patches (Mission 3 forgeries, Keel's and
//! Touchstone's held-out cases, renames, binaries, modes, symlinks, submodules, odd names, CRLF,
//! `--no-prefix`), each with Git's captured `apply --numstat -z` / `--summary` output and a ranking
//! written by the real deep-diff-forge 0.7.0 binary. `fixtures/coverage/cases.json` holds the
//! expected classification of each and the Git version that produced the captures.
#![expect(
    clippy::disallowed_methods,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test helpers read committed fixtures; a panic here is a failed test"
)]

#[path = "support/corpus_manifest.rs"]
mod corpus_manifest;

use forge_warden_core::facts::{Disagreement, PatchFacts, compare};
use forge_warden_core::rank::parse_rank;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::PathBuf;

fn fixture(name: &str) -> Vec<u8> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/coverage");
    std::fs::read(root.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn classify(case: &Value) -> String {
    let name = case["name"].as_str().unwrap();
    let Ok(rank) = parse_rank(&fixture(&format!("{name}.rank.json"))) else {
        return "rank_refused".to_owned();
    };
    if case["git_rc"] != serde_json::json!([0, 0]) {
        return "unverifiable".to_owned();
    }
    let facts = PatchFacts::from_git(
        &fixture(&format!("{name}.numstat")),
        &fixture(&format!("{name}.summary")),
        &fixture(&format!("{name}.patch")),
    );
    let Ok(facts) = facts else {
        return "unverifiable".to_owned();
    };
    let found = compare(&rank, &facts);
    let mut seen = BTreeSet::new();
    let duplicated = rank.files().iter().any(|f| !seen.insert(f.path()));
    let has = |pick: fn(&Disagreement) -> bool| found.iter().any(pick);
    let classes = [
        (
            "status_mismatch",
            has(|d| matches!(d, Disagreement::Status(_))),
        ),
        ("duplicate_paths", duplicated),
        (
            "rank_incomplete",
            has(|d| matches!(d, Disagreement::Missing(_))),
        ),
        (
            "rank_extraneous",
            has(|d| matches!(d, Disagreement::Extraneous(_))),
        ),
    ];
    let joined: Vec<&str> = classes
        .iter()
        .filter(|(_, on)| *on)
        .map(|(c, _)| *c)
        .collect();
    if joined.is_empty() {
        "covered".to_owned()
    } else {
        joined.join("+")
    }
}

#[test]
fn every_case_in_the_coverage_corpus_classifies_as_expected() {
    let index: Value = serde_json::from_slice(&fixture("cases.json")).unwrap();
    let cases = index["cases"].as_array().unwrap();
    corpus_manifest::check(
        "coverage-cases",
        cases.iter().map(|case| {
            (
                case["name"].as_str().unwrap().to_owned(),
                serde_json::to_vec(case).unwrap(),
            )
        }),
    )
    .unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/coverage");
    corpus_manifest::check(
        "coverage-files",
        std::fs::read_dir(root)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (entry.file_name().into_string().unwrap(), entry.path())
            })
            .filter(|(name, _)| name != "MANIFEST")
            .map(|(name, path)| (name, std::fs::read(path).unwrap())),
    )
    .unwrap();
    let wrong: Vec<String> = cases
        .iter()
        .filter_map(|case| {
            let got = classify(case);
            let want = case["expect"].as_str().unwrap();
            (got != want)
                .then(|| format!("{}: got {got}, want {want}", case["name"].as_str().unwrap()))
        })
        .collect();
    assert_eq!(wrong, Vec::<String>::new());
}
