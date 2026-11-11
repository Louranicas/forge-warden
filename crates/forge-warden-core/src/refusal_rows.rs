//! REFUSAL-MAP (finish condition 5, ledger Q38): every refusal variant of this crate, and every
//! exit, mapped to the one test that observes it.
//!
//! Layer 1, completeness by construction: each function below is an exhaustive `match` with no
//! `_` arm, so a new variant does not compile until it has a row. The matches live here, inside
//! the defining crate, because `Unverifiable`, `PolicyError`, `RankError`, `SealError`,
//! `DateError`, `PathError` and `Exit` are `#[non_exhaustive]`: outside this crate a match on
//! them needs a `_` arm, which is exactly the hole this map exists to close.
//!
//! Each arm returns `(variant name, observing test)`. The variant name must equal the arm's own
//! pattern (checked by `crates/forge-warden-cli/tests/refusal_map.rs`, which parses this file),
//! so an arm cannot borrow another variant's row. That test also renders `docs/REFUSAL_MAP.md`
//! from these arms and compares it byte for byte. The test is named `path::function`; that a
//! test really observes its variant is proven per row by a plant (layer 3), not by this file.
//!
//! `MISSING` marks a row whose observing test is still to be written.

use crate::date::DateError;
use crate::exit::Exit;
use crate::facts::Unverifiable;
use crate::path::PathError;
use crate::policy::PolicyError;
use crate::rank::RankError;
use crate::seal::SealError;

/// A row: the variant's own name and the `path::function` of the test that observes it.
pub(crate) type Row = (&'static str, &'static str);

/// A row whose observing test is still to be written.
pub(crate) const MISSING: &str = "MISSING";

/// The facts test that observes the unreadable-answer variants.
const GIT_ANSWERS: &str = "crates/forge-warden-core/src/facts.rs::each_unreadable_answer_from_git_is_refused_with_its_own_variant";

/// Rows for [`Unverifiable`], the reasons patch facts cannot be verified.
pub(crate) const fn unverifiable(e: &Unverifiable) -> Row {
    match e {
        Unverifiable::GitRefused(..) => (
            "GitRefused",
            "crates/forge-warden-cli/tests/authority_mode.rs::git_still_checks_the_producer_and_refusals_hold",
        ),
        Unverifiable::Empty => ("Empty", GIT_ANSWERS),
        Unverifiable::NonUtf8Path => ("NonUtf8Path", GIT_ANSWERS),
        Unverifiable::NewlineInPath => ("NewlineInPath", GIT_ANSWERS),
        Unverifiable::NonCanonicalPath { .. } => (
            "NonCanonicalPath",
            "crates/forge-warden-core/src/facts.rs::a_non_canonical_git_path_keeps_its_reason",
        ),
        Unverifiable::RepeatedPath(..) => (
            "RepeatedPath",
            "crates/forge-warden-core/tests/authority.rs::a_no_prefix_patch_is_refused_as_a_repeated_path",
        ),
        Unverifiable::SummaryPathUnknown(..) => ("SummaryPathUnknown", GIT_ANSWERS),
        Unverifiable::TypeChange(..) => ("TypeChange", GIT_ANSWERS),
        Unverifiable::RenameOrCopy => (
            "RenameOrCopy",
            "crates/forge-warden-core/src/facts.rs::renames_and_copies_refuse_in_either_output",
        ),
        Unverifiable::UnsupportedMode(..) => (
            "UnsupportedMode",
            "crates/forge-warden-core/src/facts.rs::every_mode_bearing_header_is_admitted_by_one_total_grammar",
        ),
        Unverifiable::UnreadableModeHeader(..) => (
            "UnreadableModeHeader",
            "crates/forge-warden-core/src/facts.rs::every_mode_bearing_header_is_admitted_by_one_total_grammar",
        ),
        Unverifiable::SymlinkTargetEdit => (
            "SymlinkTargetEdit",
            "crates/forge-warden-core/src/facts.rs::an_existing_symlink_target_edit_is_refused_not_called_covered",
        ),
        Unverifiable::UnreadableGitOutput(..) => (
            "UnreadableGitOutput",
            "crates/forge-warden-core/src/facts.rs::renames_and_copies_refuse_in_either_output",
        ),
    }
}

/// Rows for [`PathError`].
pub(crate) const fn path(e: PathError) -> Row {
    const T: &str = "crates/forge-warden-core/src/path.rs::every_non_canonical_spelling_is_refused_with_its_reason";
    match e {
        PathError::Empty => ("Empty", T),
        PathError::Nul => ("Nul", T),
        PathError::Absolute => ("Absolute", T),
        PathError::NonCanonical => ("NonCanonical", T),
    }
}

/// Rows for [`PolicyError`].
pub(crate) const fn policy(e: &PolicyError) -> Row {
    match e {
        PolicyError::TooLarge(..) => (
            "TooLarge",
            "crates/forge-warden-core/src/policy/tests.rs::the_policy_budget_is_enforced_before_parsing",
        ),
        PolicyError::Malformed { .. } => (
            "Malformed",
            "crates/forge-warden-core/src/policy/tests.rs::non_utf8_is_malformed",
        ),
        PolicyError::Invalid(..) => (
            "Invalid",
            "crates/forge-warden-core/src/policy/tests.rs::empty_policy_is_refused",
        ),
    }
}

/// Rows for [`RankError`].
pub(crate) const fn rank(e: &RankError) -> Row {
    match e {
        RankError::TooLarge(..) => (
            "TooLarge",
            "crates/forge-warden-core/src/rank.rs::the_budget_is_64_mib_and_one_byte_more_is_refused_unparsed",
        ),
        RankError::Malformed(..) => (
            "Malformed",
            "crates/forge-warden-core/src/rank.rs::non_json_is_malformed_and_never_panics",
        ),
        RankError::Invalid(..) => (
            "Invalid",
            "crates/forge-warden-core/tests/adversarial.rs::invalid_rank_meaning_is_distinct_from_malformed_shape",
        ),
    }
}

/// Rows for [`SealError`].
pub(crate) const fn seal(e: &SealError) -> Row {
    const T: &str = "crates/forge-warden-core/src/seal.rs::uppercase_short_long_and_non_hex_seals_are_refused_with_their_reason";
    match e {
        SealError::Length { .. } => ("Length", T),
        SealError::NotLowercaseHex { .. } => ("NotLowercaseHex", T),
    }
}

/// Rows for [`DateError`].
pub(crate) const fn date(e: DateError) -> Row {
    const T: &str = "crates/forge-warden-core/src/date.rs::a_refused_date_names_its_reason";
    match e {
        DateError::BadShape => ("BadShape", T),
        DateError::OutOfRange => ("OutOfRange", T),
        DateError::NotACalendarDate => ("NotACalendarDate", T),
    }
}

/// Rows for [`Exit`]: the CLI test that runs the binary and observes the status number.
pub(crate) const fn exit(e: Exit) -> Row {
    match e {
        Exit::Pass => (
            "Pass",
            "crates/forge-warden-cli/tests/authority_mode.rs::a_produced_ranking_that_covers_the_patch_passes_and_says_who_produced_it",
        ),
        Exit::Usage => (
            "Usage",
            "crates/forge-warden-cli/tests/cli.rs::usage_errors_exit_2_and_io_errors_exit_3",
        ),
        Exit::Io => (
            "Io",
            "crates/forge-warden-cli/tests/authority_mode.rs::a_ranker_failure_gives_exit_3_and_no_verdict",
        ),
        Exit::InvalidInput => (
            "InvalidInput",
            "crates/forge-warden-cli/tests/adversarial.rs::a_hash_mismatch_refuses_without_emitting_a_verdict",
        ),
        Exit::PolicyFail => (
            "PolicyFail",
            "crates/forge-warden-cli/tests/adversarial.rs::protection_and_forbidden_signal_have_fail_controls",
        ),
        Exit::PolicyHold => (
            "PolicyHold",
            "crates/forge-warden-cli/tests/adversarial.rs::matching_patch_and_known_ranking_hold_in_advisory_mode",
        ),
        Exit::Internal => ("Internal", MISSING),
    }
}

#[test]
fn every_row_function_answers_with_a_name_and_a_test() {
    // Uses each function once (the rows themselves are checked by the CLI's refusal_map test).
    let rows = [
        unverifiable(&Unverifiable::Empty),
        path(PathError::Empty),
        date(DateError::BadShape),
        exit(Exit::Pass),
    ];
    assert!(
        rows.iter()
            .all(|(name, test)| !name.is_empty() && !test.is_empty())
    );
    assert_eq!(policy(&PolicyError::TooLarge(1)).0, "TooLarge");
    assert_eq!(rank(&RankError::TooLarge(1)).0, "TooLarge");
    assert_eq!(seal(&SealError::Length { len: 1 }).0, "Length");
}
