# Refusal map

Every variant of the 11 enums listed below (10 refusal enums and `Exit`), with the one test that
observes it. Generated from `crates/forge-warden-core/src/refusal_rows.rs` and
`crates/forge-warden-cli/src/refusal_rows.rs`, and checked byte for byte by
`crates/forge-warden-cli/tests/refusal_map.rs`; edit the rows, not this file.

Scope: exactly these 11 enums (`Unverifiable`, `PathError`, `PolicyError`, `RankError`, `SealError`,
`DateError`, `Exit` in the core; `DoorErrorKind`, `ChildFailure`, `RankerFailure`, `GitFactsFailure` in
the CLI). Outside it: the reasons inside `RankError::Invalid` (one row for all of them), `Disagreement`
(a disagreement with Git becomes a note, not a refusal), the Git allowlist's admission refusals and the
CLI's named error messages (each reaches the user as one of the exits below), and the refusals of the
repository scripts (`scripts/hold_ack.py`, `scripts/dogfood.sh`), which have their own tests.

A row names its test by `path::function`; the test is checked to exist and to run (not ignored,
not behind another cfg). Three rows are proven by a plant that swaps the variant for a sibling
(guardrail G-53 in `plants/catalog.json`); for every other row the test's assertion was read, not
proven by a plant.

| Refusal | Observing test |
| --- | --- |
| `Unverifiable::GitRefused` | `crates/forge-warden-cli/tests/authority_mode.rs::git_still_checks_the_producer_and_refusals_hold` |
| `Unverifiable::Empty` | `crates/forge-warden-core/src/facts.rs::each_unreadable_answer_from_git_is_refused_with_its_own_variant` |
| `Unverifiable::NonUtf8Path` | `crates/forge-warden-core/src/facts.rs::each_unreadable_answer_from_git_is_refused_with_its_own_variant` |
| `Unverifiable::NewlineInPath` | `crates/forge-warden-core/src/facts.rs::each_unreadable_answer_from_git_is_refused_with_its_own_variant` |
| `Unverifiable::NonCanonicalPath` | `crates/forge-warden-core/src/facts.rs::a_non_canonical_git_path_keeps_its_reason` |
| `Unverifiable::RepeatedPath` | `crates/forge-warden-core/tests/authority.rs::a_no_prefix_patch_is_refused_as_a_repeated_path` |
| `Unverifiable::SummaryPathUnknown` | `crates/forge-warden-core/src/facts.rs::each_unreadable_answer_from_git_is_refused_with_its_own_variant` |
| `Unverifiable::TypeChange` | `crates/forge-warden-core/src/facts.rs::each_unreadable_answer_from_git_is_refused_with_its_own_variant` |
| `Unverifiable::RenameOrCopy` | `crates/forge-warden-core/src/facts.rs::renames_and_copies_refuse_in_either_output` |
| `Unverifiable::UnsupportedMode` | `crates/forge-warden-core/src/facts.rs::every_mode_bearing_header_is_admitted_by_one_total_grammar` |
| `Unverifiable::UnreadableModeHeader` | `crates/forge-warden-core/src/facts.rs::every_mode_bearing_header_is_admitted_by_one_total_grammar` |
| `Unverifiable::SymlinkTargetEdit` | `crates/forge-warden-core/src/facts.rs::an_existing_symlink_target_edit_is_refused_not_called_covered` |
| `Unverifiable::UnreadableGitOutput` | `crates/forge-warden-core/src/facts.rs::renames_and_copies_refuse_in_either_output` |
| `PathError::Empty` | `crates/forge-warden-core/src/path.rs::every_non_canonical_spelling_is_refused_with_its_reason` |
| `PathError::Nul` | `crates/forge-warden-core/src/path.rs::every_non_canonical_spelling_is_refused_with_its_reason` |
| `PathError::Absolute` | `crates/forge-warden-core/src/path.rs::every_non_canonical_spelling_is_refused_with_its_reason` |
| `PathError::NonCanonical` | `crates/forge-warden-core/src/path.rs::every_non_canonical_spelling_is_refused_with_its_reason` |
| `PolicyError::TooLarge` | `crates/forge-warden-core/src/policy/tests.rs::the_policy_budget_is_enforced_before_parsing` |
| `PolicyError::Malformed` | `crates/forge-warden-core/src/policy/tests.rs::non_utf8_is_malformed` |
| `PolicyError::Invalid` | `crates/forge-warden-core/src/policy/tests.rs::empty_policy_is_refused` |
| `RankError::TooLarge` | `crates/forge-warden-core/src/rank.rs::the_budget_is_64_mib_and_one_byte_more_is_refused_unparsed` |
| `RankError::Malformed` | `crates/forge-warden-core/src/rank.rs::non_json_is_malformed_and_never_panics` |
| `RankError::Invalid` | `crates/forge-warden-core/tests/adversarial.rs::invalid_rank_meaning_is_distinct_from_malformed_shape` |
| `SealError::Length` | `crates/forge-warden-core/src/seal.rs::uppercase_short_long_and_non_hex_seals_are_refused_with_their_reason` |
| `SealError::NotLowercaseHex` | `crates/forge-warden-core/src/seal.rs::uppercase_short_long_and_non_hex_seals_are_refused_with_their_reason` |
| `DateError::BadShape` | `crates/forge-warden-core/src/date.rs::a_refused_date_names_its_reason` |
| `DateError::OutOfRange` | `crates/forge-warden-core/src/date.rs::a_refused_date_names_its_reason` |
| `DateError::NotACalendarDate` | `crates/forge-warden-core/src/date.rs::a_refused_date_names_its_reason` |
| `Exit::Pass` | `crates/forge-warden-cli/tests/authority_mode.rs::a_produced_ranking_that_covers_the_patch_passes_and_says_who_produced_it` |
| `Exit::Usage` | `crates/forge-warden-cli/tests/cli.rs::usage_errors_exit_2_and_io_errors_exit_3` |
| `Exit::Io` | `crates/forge-warden-cli/tests/authority_mode.rs::a_ranker_failure_gives_exit_3_and_no_verdict` |
| `Exit::InvalidInput` | `crates/forge-warden-cli/tests/adversarial.rs::a_hash_mismatch_refuses_without_emitting_a_verdict` |
| `Exit::PolicyFail` | `crates/forge-warden-cli/tests/adversarial.rs::protection_and_forbidden_signal_have_fail_controls` |
| `Exit::PolicyHold` | `crates/forge-warden-cli/tests/adversarial.rs::matching_patch_and_known_ranking_hold_in_advisory_mode` |
| `Exit::Internal` | UNREACHABLE: only a registry serialisation failure (main.rs, schema) or a panic; no fault hook |
| `DoorErrorKind::Io` | `crates/forge-warden-cli/src/doors/file.rs::a_missing_file_is_an_io_failure_and_a_directory_is_refused` |
| `DoorErrorKind::Environment` | `crates/forge-warden-cli/src/doors/mod.rs::the_private_base_is_admitted_only_if_safe_for_cwd_home_and_ceiling` |
| `DoorErrorKind::OverBudget` | `crates/forge-warden-cli/src/doors/file.rs::the_budget_is_inclusive_and_one_byte_more_is_refused` |
| `DoorErrorKind::Refused` | `crates/forge-warden-cli/src/doors/file.rs::a_missing_file_is_an_io_failure_and_a_directory_is_refused` |
| `ChildFailure::Spawn` | `crates/forge-warden-cli/src/doors/mod.rs::a_relative_program_is_refused` |
| `ChildFailure::Timeout` | `crates/forge-warden-cli/src/doors/mod.rs::a_timeout_kills_the_whole_group` |
| `ChildFailure::StdoutOverflow` | `crates/forge-warden-cli/src/doors/mod.rs::output_over_either_cap_is_a_failure_not_a_truncation` |
| `ChildFailure::StderrOverflow` | `crates/forge-warden-cli/src/doors/mod.rs::output_over_either_cap_is_a_failure_not_a_truncation` |
| `ChildFailure::Signal` | `crates/forge-warden-cli/src/doors/mod.rs::a_nonzero_exit_or_a_signal_fails_even_with_valid_looking_output` |
| `ChildFailure::Exit` | `crates/forge-warden-cli/src/doors/mod.rs::a_nonzero_exit_or_a_signal_fails_even_with_valid_looking_output` |
| `ChildFailure::Io` | `crates/forge-warden-cli/src/doors/mod.rs::a_group_kill_that_cannot_run_or_fails_is_reported` |
| `RankerFailure::NotAdmitted` | `crates/forge-warden-cli/tests/authority_mode.rs::a_ranker_the_policy_does_not_list_is_refused_without_a_verdict` |
| `RankerFailure::Door` | `crates/forge-warden-cli/tests/authority_mode.rs::a_ranker_that_cannot_be_pinned_or_executed_is_refused` |
| `RankerFailure::Failed` | `crates/forge-warden-cli/tests/authority_mode.rs::a_ranker_failure_gives_exit_3_and_no_verdict` |
| `GitFactsFailure::Door` | `crates/forge-warden-cli/src/doors/mod.rs::git_facts_name_a_door_failure_and_an_unavailable_git` |
| `GitFactsFailure::Refused` | `crates/forge-warden-cli/src/doors/mod.rs::git_facts_reproduces_the_coverage_corpus_captures_byte_for_byte` |
| `GitFactsFailure::Unavailable` | `crates/forge-warden-cli/src/doors/mod.rs::git_facts_name_a_door_failure_and_an_unavailable_git` |
