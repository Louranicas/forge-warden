//! REFUSAL-MAP (finish condition 5, ledger Q38): the CLI's door refusals, each mapped to the one
//! test that observes it. The core's rows are in `crates/forge-warden-core/src/refusal_rows.rs`;
//! both files are parsed and rendered together by `crates/forge-warden-cli/tests/refusal_map.rs`.
//!
//! Each function is an exhaustive `match` with no `_` arm, inside the crate that defines the enums
//! (they are `pub(crate)`), so a new variant does not compile until it has a row. Each arm returns
//! its own variant's name (checked against the pattern by the map test) and the observing test as
//! `path::function`. `MISSING` marks a row whose observing test is still to be written.

use crate::doors::DoorErrorKind;
use crate::doors::child::{ChildFailure, GitFactsFailure, RankerFailure};

/// A row: the variant's own name and the `path::function` of the test that observes it.
pub(crate) type Row = (&'static str, &'static str);

/// A row whose observing test is still to be written.
pub(crate) const MISSING: &str = "MISSING";

/// Rows for [`DoorErrorKind`], how a door refuses.
pub(crate) const fn door(e: DoorErrorKind) -> Row {
    match e {
        DoorErrorKind::Io => (
            "Io",
            "crates/forge-warden-cli/src/doors/file.rs::a_missing_file_is_an_io_failure_and_a_directory_is_refused",
        ),
        DoorErrorKind::Environment => (
            "Environment",
            "crates/forge-warden-cli/src/doors/mod.rs::the_private_base_is_admitted_only_if_safe_for_cwd_home_and_ceiling",
        ),
        DoorErrorKind::OverBudget => (
            "OverBudget",
            "crates/forge-warden-cli/src/doors/file.rs::the_budget_is_inclusive_and_one_byte_more_is_refused",
        ),
        DoorErrorKind::Refused => (
            "Refused",
            "crates/forge-warden-cli/src/doors/file.rs::a_missing_file_is_an_io_failure_and_a_directory_is_refused",
        ),
    }
}

/// Rows for [`ChildFailure`], how a child process ends or is refused.
pub(crate) const fn child(e: &ChildFailure) -> Row {
    const OUTPUT: &str = "crates/forge-warden-cli/src/doors/mod.rs::output_over_either_cap_is_a_failure_not_a_truncation";
    const ENDED: &str = "crates/forge-warden-cli/src/doors/mod.rs::a_nonzero_exit_or_a_signal_fails_even_with_valid_looking_output";
    match e {
        ChildFailure::Spawn(..) => (
            "Spawn",
            "crates/forge-warden-cli/src/doors/mod.rs::a_relative_program_is_refused",
        ),
        ChildFailure::Timeout => (
            "Timeout",
            "crates/forge-warden-cli/src/doors/mod.rs::a_timeout_kills_the_whole_group",
        ),
        ChildFailure::StdoutOverflow => ("StdoutOverflow", OUTPUT),
        ChildFailure::StderrOverflow => ("StderrOverflow", OUTPUT),
        ChildFailure::Signal(..) => ("Signal", ENDED),
        ChildFailure::Exit { .. } => ("Exit", ENDED),
        ChildFailure::Io(..) => (
            "Io",
            "crates/forge-warden-cli/src/doors/mod.rs::a_group_kill_that_cannot_run_or_fails_is_reported",
        ),
    }
}

/// Rows for [`RankerFailure`], why authority mode has no produced ranking.
pub(crate) const fn ranker(e: &RankerFailure) -> Row {
    match e {
        RankerFailure::NotAdmitted(..) => (
            "NotAdmitted",
            "crates/forge-warden-cli/tests/authority_mode.rs::a_ranker_the_policy_does_not_list_is_refused_without_a_verdict",
        ),
        RankerFailure::Door(..) => (
            "Door",
            "crates/forge-warden-cli/tests/authority_mode.rs::a_ranker_that_cannot_be_pinned_or_executed_is_refused",
        ),
        RankerFailure::Failed(..) => (
            "Failed",
            "crates/forge-warden-cli/tests/authority_mode.rs::a_ranker_failure_gives_exit_3_and_no_verdict",
        ),
    }
}

/// Rows for [`GitFactsFailure`], why Git produced no facts.
pub(crate) const fn git(e: &GitFactsFailure) -> Row {
    match e {
        GitFactsFailure::Door(..) => (
            "Door",
            "crates/forge-warden-cli/src/doors/mod.rs::git_facts_name_a_door_failure_and_an_unavailable_git",
        ),
        GitFactsFailure::Refused(..) => (
            "Refused",
            "crates/forge-warden-cli/src/doors/mod.rs::git_facts_reproduces_the_coverage_corpus_captures_byte_for_byte",
        ),
        GitFactsFailure::Unavailable(..) => (
            "Unavailable",
            "crates/forge-warden-cli/src/doors/mod.rs::git_facts_name_a_door_failure_and_an_unavailable_git",
        ),
    }
}

#[test]
fn every_row_function_answers_with_a_name_and_a_test() {
    // Uses each function once (the rows themselves are checked by tests/refusal_map.rs).
    assert_eq!(door(DoorErrorKind::Io).0, "Io");
    assert_eq!(child(&ChildFailure::Timeout).0, "Timeout");
    assert_eq!(ranker(&RankerFailure::Failed(String::new())).0, "Failed");
    assert_eq!(git(&GitFactsFailure::Refused(String::new())).0, "Refused");
}
