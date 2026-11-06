//! The deep-diff-forge vocabulary forge-warden understands.
//!
//! A policy may name only signals listed here, so a typo (`secrets_like`) is refused when the
//! policy is loaded instead of silently never matching. A ranking may carry signals or statuses
//! that are not listed (deep-diff-forge adds vocabulary additively); those make a verdict stricter,
//! never looser (see [`crate::eval`]).
//!
//! Source of truth: the `label` functions of `RiskSignal` (`risk.rs`) and `EvidenceSignal`
//! (`evidence.rs`) in `crates/deep-diff-forge-graph/src`. `scripts/check_vocab.sh <commit>` compares
//! this list with those functions at a named deep-diff-forge commit (CI job `vocabulary`), and
//! `crates/forge-warden-cli/tests/corpus.rs` requires the signals a real deep-diff-forge binary
//! wrote into `fixtures/corpus/` to equal it.

use crate::defaults::NamedDefault as _;

/// Every risk signal deep-diff-forge 0.7.0 can put in a `rank.v0` file entry: path signals first,
/// then code-evidence signals.
pub const KNOWN_SIGNALS: [&str; 21] = [
    // Path and shape signals (`risk.rs`).
    "public_api_surface",
    "new_file",
    "deleted_file",
    "binary_change",
    "large_change",
    "many_hunks",
    "config_or_lockfile",
    "test_only",
    "generated_or_vendored",
    "hazardous_text",
    "symlink_change",
    "executable_bit",
    // Code-evidence signals (`evidence.rs`).
    "unsafe_added",
    "process_spawn",
    "network_fetch",
    "dynamic_eval",
    "ci_privilege",
    "build_hook",
    "secret_like",
    "check_suppressed",
    "test_removed",
];

/// Every file status deep-diff-forge 0.7.0 writes **that says what happened to the file**.
/// deep-diff-forge also writes `unknown` when it cannot tell; that is deliberately absent, so such
/// a file holds the verdict (ADR 0003) instead of passing silently.
pub const KNOWN_STATUSES: [&str; 8] = [
    "added",
    "modified",
    "deleted",
    "renamed",
    "copied",
    "type_changed",
    "mode_changed",
    "binary_changed",
];

/// Whether `name` is a signal this version of forge-warden understands.
#[must_use]
pub fn is_known_signal(name: &str) -> bool {
    KNOWN_SIGNALS.contains(&name)
}

/// Whether `name` is a file status this version of forge-warden understands.
#[must_use]
pub fn is_known_status(name: &str) -> bool {
    KNOWN_STATUSES.contains(&name)
}

/// The known signal closest to `name` by edit distance, offered as a "did you mean" when a policy
/// names an unknown one. `None` when nothing is close enough to be a plausible typo.
#[must_use]
pub fn suggest_signal(name: &str) -> Option<&'static str> {
    KNOWN_SIGNALS
        .iter()
        .map(|known| (levenshtein(name, known), *known))
        .filter(|(distance, _)| *distance <= 3)
        .min()
        .map(|(_, known)| known)
}

/// Levenshtein distance over chars, in O(len(a) × len(b)) time and O(len(b)) space.
fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = i;
        let mut left = i.saturating_add(1);
        let mut next = Vec::with_capacity(row.len());
        next.push(left);
        for (cb, above) in b.iter().zip(row.iter().skip(1)) {
            let substitute = diagonal.saturating_add(usize::from(ca != *cb));
            let cell = substitute
                .min(above.saturating_add(1))
                .min(left.saturating_add(1));
            next.push(cell);
            diagonal = *above;
            left = cell;
        }
        row = next;
    }
    // `row` always has `b.len() + 1 >= 1` entries. Were it ever empty, the distance would read as
    // "infinitely far" (no suggestion), never as 0 (an exact match).
    // An impossible empty row means no suggestion, not a match.
    row.last().copied().or_fail_safe(usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn vocabularies_have_no_duplicates() {
        assert_eq!(
            KNOWN_SIGNALS.iter().collect::<BTreeSet<_>>().len(),
            KNOWN_SIGNALS.len()
        );
        assert_eq!(
            KNOWN_STATUSES.iter().collect::<BTreeSet<_>>().len(),
            KNOWN_STATUSES.len()
        );
    }

    #[test]
    fn typos_get_a_suggestion_and_noise_does_not() {
        assert_eq!(suggest_signal("secrets_like"), Some("secret_like"));
        assert_eq!(suggest_signal("unsafe-added"), Some("unsafe_added"));
        assert_eq!(suggest_signal("completely_unrelated_words"), None);
    }

    #[test]
    fn levenshtein_basics() {
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("abc", ""), 3);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
    }
}
