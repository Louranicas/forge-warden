//! Every ranking in `fixtures/corpus/` was written by a real deep-diff-forge 0.7.0 binary: 39 from
//! deep-diff-forge's own patch and evaluation corpus, and `shape-many-hunks` from the patch beside
//! it (five hunks in one file). forge-warden must read every one, know its whole vocabulary, and
//! judge it deterministically; together they exercise every signal forge-warden knows.
#![expect(
    clippy::disallowed_methods,
    reason = "test door: reads committed fixtures"
)]

mod common;
use common::files_under;
use forge_warden_core::policy::parse_policy;
use forge_warden_core::rank::parse_rank;
use forge_warden_core::seal::sha256_hex;
use forge_warden_core::verdict::Seals;
use forge_warden_core::vocab::is_known_signal;
use forge_warden_core::{Date, evaluate};

#[test]
fn every_real_ranking_parses_with_known_vocabulary() {
    let files = files_under("fixtures/corpus", ".rank.json");
    assert!(files.len() >= 30, "the corpus shrank to {}", files.len());
    let mut signals = std::collections::BTreeSet::new();
    for path in &files {
        let bytes = std::fs::read(path).unwrap();
        let rank = parse_rank(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        for file in rank.files() {
            for signal in file.signals() {
                assert!(
                    is_known_signal(signal),
                    "{}: unknown signal {signal}",
                    path.display()
                );
                signals.insert(signal.clone());
            }
        }
    }
    let known: std::collections::BTreeSet<String> = forge_warden_core::vocab::KNOWN_SIGNALS
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    assert_eq!(
        signals, known,
        "the real-producer corpus must exercise exactly the known vocabulary (R-17)"
    );
}

#[test]
fn every_real_ranking_is_judged_deterministically() {
    // These rankings come without their patches, so they are judged by the engine itself (the
    // binary needs the patch: ADR 0005). Same bytes in, same bytes out.
    let policy_bytes = std::fs::read(common::root().join("examples/starter.warden.toml")).unwrap();
    let policy = parse_policy(&policy_bytes).unwrap();
    let today = Date::parse("2026-11-05").unwrap();
    for path in files_under("fixtures/corpus", ".rank.json") {
        let bytes = std::fs::read(&path).unwrap();
        let judge = || {
            let rank = parse_rank(&bytes).unwrap();
            let seals = Seals::new(
                sha256_hex(&policy_bytes),
                sha256_hex(&bytes),
                rank.input_sha256().to_owned(),
                rank.tool_version().to_owned(),
            );
            evaluate(&policy, &rank, seals, today).to_json()
        };
        let first = judge();
        assert_eq!(first, judge(), "{}", path.display());
        assert!(first.contains("\"verdict\""), "{}", path.display());
    }
}
