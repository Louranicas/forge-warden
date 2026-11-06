//! Every ranking in `fixtures/corpus/` was written by a real deep-diff-forge 0.7.0 binary: 39 from
//! deep-diff-forge's own patch and evaluation corpus, and `shape-many-hunks` from the patch beside
//! it (five hunks in one file). forge-warden must read every one, know its whole vocabulary, and
//! judge it deterministically; together they exercise every signal forge-warden knows.
#![expect(
    clippy::disallowed_methods,
    reason = "test door: reads committed fixtures"
)]

mod common;
use common::{files_under, run};
use forge_warden_core::rank::parse_rank;
use forge_warden_core::vocab::is_known_signal;

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
    for path in files_under("fixtures/corpus", ".rank.json") {
        let rank = std::fs::read(&path).unwrap();
        let args = [
            "check",
            "--policy",
            "examples/starter.warden.toml",
            "--today",
            "2026-11-05",
            "--json",
        ];
        let first = run(&args, &rank);
        let second = run(&args, &rank);
        assert!(
            matches!(first.code, 0 | 7 | 8),
            "{}: exit {} {}",
            path.display(),
            first.code,
            first.stderr
        );
        assert_eq!(first.stdout, second.stdout, "{}", path.display());
    }
}
