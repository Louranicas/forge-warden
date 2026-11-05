//! The exit-code registry: one table that the binary, `forge-warden explain`, the README and the
//! verdict document all read, so they cannot disagree. `tests/readme_tracks_registry.rs` fails if
//! the README's table drifts from this one.
//!
//! The `action` vocabulary is deep-diff-forge's, so an agent driving both tools needs one table
//! of next steps.

use serde::Serialize;

/// One documented exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ExitEntry {
    /// The process exit status.
    pub code: u8,
    /// A stable snake-case name.
    pub name: &'static str,
    /// What it means.
    pub meaning: &'static str,
    /// What the caller, person or agent, should do next.
    pub action: &'static str,
    /// Whether the same command can succeed later without any change to its input.
    pub may_be_transient: bool,
}

/// Every exit code forge-warden documents, in ascending order.
pub const EXIT_REGISTRY: [ExitEntry; 7] = [
    ExitEntry {
        code: 0,
        name: "pass",
        meaning: "Every rule passed or was waived by an active waiver.",
        action: "proceed",
        may_be_transient: false,
    },
    ExitEntry {
        code: 2,
        name: "usage",
        meaning: "Command-line usage error.",
        action: "fix_invocation",
        may_be_transient: false,
    },
    ExitEntry {
        code: 3,
        name: "io",
        meaning: "A file or stream could not be read, or output could not be written.",
        action: "check_environment",
        may_be_transient: true,
    },
    ExitEntry {
        code: 4,
        name: "invalid_input",
        meaning: "The policy or the ranking failed to parse or validate, exceeded the byte budget, or the ranking's seal does not match the patch given with --patch.",
        action: "change_input",
        may_be_transient: false,
    },
    ExitEntry {
        code: 7,
        name: "policy_fail",
        meaning: "At least one rule of severity `fail` was violated, or a waiver has expired.",
        action: "change_input",
        may_be_transient: false,
    },
    ExitEntry {
        code: 8,
        name: "policy_hold",
        meaning: "No rule failed, but a `hold` rule was violated or the ranking holds something the policy cannot judge (no files, unknown vocabulary): a person must decide.",
        action: "review_evidence",
        may_be_transient: false,
    },
    ExitEntry {
        code: 101,
        name: "internal",
        meaning: "Internal error (a bug). Report it with the command and its input.",
        action: "report_bug",
        may_be_transient: false,
    },
];

/// The overall outcome of an evaluation, strictest wins: `Fail` > `Hold` > `Pass`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Nothing to object to.
    Pass,
    /// A person must decide.
    Hold,
    /// The change does not meet the policy.
    Fail,
}

impl Outcome {
    /// The registry entry this outcome exits with.
    #[must_use]
    pub const fn exit_entry(self) -> &'static ExitEntry {
        match self {
            Self::Pass => &EXIT_REGISTRY[0],
            Self::Fail => &EXIT_REGISTRY[4],
            Self::Hold => &EXIT_REGISTRY[5],
        }
    }

    /// Lowercase name, as it appears in documents.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Hold => "hold",
            Self::Fail => "fail",
        }
    }
}

/// The registry entry named `name`, if any.
#[must_use]
pub fn entry_by_name(name: &str) -> Option<&'static ExitEntry> {
    EXIT_REGISTRY.iter().find(|entry| entry.name == name)
}

/// The registry entry for exit status `code`, if documented.
#[must_use]
pub fn entry_by_code(code: u8) -> Option<&'static ExitEntry> {
    EXIT_REGISTRY.iter().find(|entry| entry.code == code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_sorted_and_unique() {
        assert!(EXIT_REGISTRY.windows(2).all(|w| w[0].code < w[1].code));
        let mut names: Vec<_> = EXIT_REGISTRY.iter().map(|e| e.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), EXIT_REGISTRY.len());
    }

    #[test]
    fn outcomes_point_at_their_named_entries() {
        assert_eq!(Outcome::Pass.exit_entry().name, "pass");
        assert_eq!(Outcome::Hold.exit_entry().name, "policy_hold");
        assert_eq!(Outcome::Fail.exit_entry().name, "policy_fail");
    }

    #[test]
    fn lookups_find_every_entry_and_nothing_else() {
        for entry in &EXIT_REGISTRY {
            assert_eq!(entry_by_code(entry.code), Some(entry));
            assert_eq!(entry_by_name(entry.name), Some(entry));
        }
        assert_eq!(entry_by_code(1), None);
        assert_eq!(entry_by_name("no_such_name"), None);
    }

    #[test]
    fn labels_are_the_document_spellings() {
        assert_eq!(Outcome::Pass.label(), "pass");
        assert_eq!(Outcome::Hold.label(), "hold");
        assert_eq!(Outcome::Fail.label(), "fail");
    }

    #[test]
    fn strictest_outcome_wins() {
        assert_eq!(
            [Outcome::Pass, Outcome::Fail, Outcome::Hold]
                .into_iter()
                .max(),
            Some(Outcome::Fail)
        );
        assert_eq!(
            [Outcome::Pass, Outcome::Hold].into_iter().max(),
            Some(Outcome::Hold)
        );
    }

    #[test]
    fn only_environment_failures_are_transient() {
        for entry in EXIT_REGISTRY {
            assert_eq!(
                entry.may_be_transient,
                entry.action == "check_environment",
                "{}",
                entry.name
            );
        }
    }
}
