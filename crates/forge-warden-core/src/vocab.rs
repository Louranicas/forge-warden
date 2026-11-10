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

/// A deep-diff-forge risk signal, parsed once where it is read (Q-TYPES-2, Q1-02).
///
/// Exhaustive on purpose: a new known value must break every `match` on it, so vocabulary growth
/// forces a review of the logic that reads it. An upstream value this version does not know is
/// [`Signal::Unknown`], which can only make a verdict stricter (ADR 0003).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    /// `public_api_surface`.
    PublicApiSurface,
    /// `new_file`.
    NewFile,
    /// `deleted_file`.
    DeletedFile,
    /// `binary_change`.
    BinaryChange,
    /// `large_change`.
    LargeChange,
    /// `many_hunks`.
    ManyHunks,
    /// `config_or_lockfile`.
    ConfigOrLockfile,
    /// `test_only`.
    TestOnly,
    /// `generated_or_vendored`.
    GeneratedOrVendored,
    /// `hazardous_text`.
    HazardousText,
    /// `symlink_change`.
    SymlinkChange,
    /// `executable_bit`.
    ExecutableBit,
    /// `unsafe_added`.
    UnsafeAdded,
    /// `process_spawn`.
    ProcessSpawn,
    /// `network_fetch`.
    NetworkFetch,
    /// `dynamic_eval`.
    DynamicEval,
    /// `ci_privilege`.
    CiPrivilege,
    /// `build_hook`.
    BuildHook,
    /// `secret_like`.
    SecretLike,
    /// `check_suppressed`.
    CheckSuppressed,
    /// `test_removed`.
    TestRemoved,
    /// A signal this version does not know, carried by name.
    Unknown(String),
}

impl Signal {
    /// Every known value, in the order of the canonical list above.
    pub const KNOWN: [Self; 21] = [
        Self::PublicApiSurface,
        Self::NewFile,
        Self::DeletedFile,
        Self::BinaryChange,
        Self::LargeChange,
        Self::ManyHunks,
        Self::ConfigOrLockfile,
        Self::TestOnly,
        Self::GeneratedOrVendored,
        Self::HazardousText,
        Self::SymlinkChange,
        Self::ExecutableBit,
        Self::UnsafeAdded,
        Self::ProcessSpawn,
        Self::NetworkFetch,
        Self::DynamicEval,
        Self::CiPrivilege,
        Self::BuildHook,
        Self::SecretLike,
        Self::CheckSuppressed,
        Self::TestRemoved,
    ];

    /// `name` as a value: a known one, or [`Signal::Unknown`]. Never refuses: unknown values are
    /// carried so they can hold the verdict.
    #[must_use]
    pub fn parse(name: &str) -> Self {
        match Self::KNOWN.iter().find(|known| known.name() == name) {
            Some(known) => known.clone(),
            None => Self::Unknown(name.to_owned()),
        }
    }

    /// The wire name, exactly as deep-diff-forge writes it.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::PublicApiSurface => "public_api_surface",
            Self::NewFile => "new_file",
            Self::DeletedFile => "deleted_file",
            Self::BinaryChange => "binary_change",
            Self::LargeChange => "large_change",
            Self::ManyHunks => "many_hunks",
            Self::ConfigOrLockfile => "config_or_lockfile",
            Self::TestOnly => "test_only",
            Self::GeneratedOrVendored => "generated_or_vendored",
            Self::HazardousText => "hazardous_text",
            Self::SymlinkChange => "symlink_change",
            Self::ExecutableBit => "executable_bit",
            Self::UnsafeAdded => "unsafe_added",
            Self::ProcessSpawn => "process_spawn",
            Self::NetworkFetch => "network_fetch",
            Self::DynamicEval => "dynamic_eval",
            Self::CiPrivilege => "ci_privilege",
            Self::BuildHook => "build_hook",
            Self::SecretLike => "secret_like",
            Self::CheckSuppressed => "check_suppressed",
            Self::TestRemoved => "test_removed",
            Self::Unknown(name) => name,
        }
    }

    /// Whether this version of forge-warden understands it.
    #[must_use]
    pub const fn is_known(&self) -> bool {
        !matches!(self, Self::Unknown(_))
    }
}

// Ordered and compared by wire name, so sorted output is the same as when this was a string.
impl PartialOrd for Signal {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Signal {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.name().cmp(other.name())
    }
}

impl std::fmt::Display for Signal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl serde::Serialize for Signal {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

/// A deep-diff-forge file status, parsed once where it is read (Q-TYPES-2, Q1-02). `unknown` (which
/// deep-diff-forge writes when it cannot tell) is deliberately not a known status.
///
/// Exhaustive on purpose: a new known value must break every `match` on it, so vocabulary growth
/// forces a review of the logic that reads it. An upstream value this version does not know is
/// [`Status::Unknown`], which can only make a verdict stricter (ADR 0003).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// `added`.
    Added,
    /// `modified`.
    Modified,
    /// `deleted`.
    Deleted,
    /// `renamed`.
    Renamed,
    /// `copied`.
    Copied,
    /// `type_changed`.
    TypeChanged,
    /// `mode_changed`.
    ModeChanged,
    /// `binary_changed`.
    BinaryChanged,
    /// A status this version does not know (including `unknown`), carried by name.
    Unknown(String),
}

impl Status {
    /// Every known value, in the order of the canonical list above.
    pub const KNOWN: [Self; 8] = [
        Self::Added,
        Self::Modified,
        Self::Deleted,
        Self::Renamed,
        Self::Copied,
        Self::TypeChanged,
        Self::ModeChanged,
        Self::BinaryChanged,
    ];

    /// `name` as a value: a known one, or [`Status::Unknown`]. Never refuses: unknown values are
    /// carried so they can hold the verdict.
    #[must_use]
    pub fn parse(name: &str) -> Self {
        match Self::KNOWN.iter().find(|known| known.name() == name) {
            Some(known) => known.clone(),
            None => Self::Unknown(name.to_owned()),
        }
    }

    /// The wire name, exactly as deep-diff-forge writes it.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Added => "added",
            Self::Modified => "modified",
            Self::Deleted => "deleted",
            Self::Renamed => "renamed",
            Self::Copied => "copied",
            Self::TypeChanged => "type_changed",
            Self::ModeChanged => "mode_changed",
            Self::BinaryChanged => "binary_changed",
            Self::Unknown(name) => name,
        }
    }

    /// Whether this version of forge-warden understands it.
    #[must_use]
    pub const fn is_known(&self) -> bool {
        !matches!(self, Self::Unknown(_))
    }
}

// Ordered and compared by wire name, so sorted output is the same as when this was a string.
impl PartialOrd for Status {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Status {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.name().cmp(other.name())
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl serde::Serialize for Status {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

/// Which side of a hunk a piece of evidence points at. Closed: a ranking with any other side is
/// refused when it is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Side {
    /// The new (post-image) side, `new`.
    New,
    /// The old (pre-image) side, `old`.
    Old,
}

impl Side {
    /// `new` or `old`; anything else is `None`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "new" => Some(Self::New),
            "old" => Some(Self::Old),
            _ => None,
        }
    }

    /// The wire name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Old => "old",
        }
    }
}

impl std::fmt::Display for Side {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl serde::Serialize for Side {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

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
    fn the_enums_name_exactly_the_canonical_lists_in_order() {
        assert_eq!(
            Signal::KNOWN.map(|s| s.name().to_owned()),
            KNOWN_SIGNALS.map(str::to_owned)
        );
        assert_eq!(
            Status::KNOWN.map(|s| s.name().to_owned()),
            KNOWN_STATUSES.map(str::to_owned)
        );
        for name in KNOWN_SIGNALS {
            assert!(Signal::parse(name).is_known() && Signal::parse(name).name() == name);
        }
        for name in KNOWN_STATUSES {
            assert!(Status::parse(name).is_known() && Status::parse(name).name() == name);
        }
        assert_eq!(
            Signal::parse("brand_new"),
            Signal::Unknown("brand_new".to_owned())
        );
        assert_eq!(
            Status::parse("unknown"),
            Status::Unknown("unknown".to_owned())
        );
        assert_eq!(Side::parse("new"), Some(Side::New));
        assert_eq!(Side::parse("both"), None);
    }

    #[test]
    fn values_order_and_serialize_by_wire_name() {
        let mut signals: Vec<Signal> = ["secret_like", "zz_unknown", "binary_change", "new_file"]
            .map(Signal::parse)
            .to_vec();
        signals.sort();
        assert_eq!(
            signals.iter().map(Signal::name).collect::<Vec<_>>(),
            ["binary_change", "new_file", "secret_like", "zz_unknown"]
        );
        assert_eq!(
            serde_json::to_string(&Signal::SecretLike).unwrap(),
            "\"secret_like\""
        );
        assert_eq!(serde_json::to_string(&Side::Old).unwrap(), "\"old\"");
    }

    #[test]
    fn values_display_as_their_wire_name_and_compare_by_it() {
        assert_eq!(Signal::SecretLike.to_string(), "secret_like");
        assert_eq!(Signal::parse("zz_unknown").to_string(), "zz_unknown");
        assert_eq!(Status::parse("added").to_string(), "added");
        assert_eq!(Side::New.to_string(), "new");
        assert_eq!(Side::Old.to_string(), "old");
        let (added, deleted) = (Status::parse("added"), Status::parse("deleted"));
        assert_eq!(added.partial_cmp(&deleted), Some(std::cmp::Ordering::Less));
        assert!(added < deleted);
    }

    #[test]
    fn levenshtein_basics() {
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("abc", ""), 3);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
    }
}
