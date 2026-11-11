# Refusal map

Every refusal variant and every exit, with the one test that observes it. Generated from
`crates/forge-warden-core/src/refusal_rows.rs` and checked byte for byte by
`crates/forge-warden-cli/tests/refusal_map.rs`; edit the rows, not this file.

A row names its test by `path::function`. That the test exists is checked by name only; that
it observes its variant is proven by one plant per row in `plants/catalog.json`.

| Refusal | Observing test |
| --- | --- |
| `Unverifiable::GitRefused` | MISSING |
| `Unverifiable::Empty` | MISSING |
| `Unverifiable::NonUtf8Path` | MISSING |
| `Unverifiable::NewlineInPath` | MISSING |
| `Unverifiable::NonCanonicalPath` | `crates/forge-warden-core/src/facts.rs::a_non_canonical_git_path_keeps_its_reason` |
| `Unverifiable::RepeatedPath` | `crates/forge-warden-core/tests/authority.rs::a_no_prefix_patch_is_refused_as_a_repeated_path` |
| `Unverifiable::SummaryPathUnknown` | MISSING |
| `Unverifiable::TypeChange` | MISSING |
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
| `PolicyError::Invalid` | MISSING |
| `RankError::TooLarge` | `crates/forge-warden-core/src/rank.rs::the_budget_is_64_mib_and_one_byte_more_is_refused_unparsed` |
| `RankError::Malformed` | `crates/forge-warden-core/src/rank.rs::non_json_is_malformed_and_never_panics` |
| `RankError::Invalid` | `crates/forge-warden-core/tests/adversarial.rs::invalid_rank_meaning_is_distinct_from_malformed_shape` |
| `SealError::Length` | `crates/forge-warden-core/src/seal.rs::uppercase_short_long_and_non_hex_seals_are_refused_with_their_reason` |
| `SealError::NotLowercaseHex` | `crates/forge-warden-core/src/seal.rs::uppercase_short_long_and_non_hex_seals_are_refused_with_their_reason` |
| `DateError::BadShape` | `crates/forge-warden-core/src/date.rs::a_refused_date_names_its_reason` |
| `DateError::OutOfRange` | `crates/forge-warden-core/src/date.rs::a_refused_date_names_its_reason` |
| `DateError::NotACalendarDate` | `crates/forge-warden-core/src/date.rs::a_refused_date_names_its_reason` |
| `Exit::Pass` | MISSING |
| `Exit::Usage` | MISSING |
| `Exit::Io` | MISSING |
| `Exit::InvalidInput` | MISSING |
| `Exit::PolicyFail` | MISSING |
| `Exit::PolicyHold` | MISSING |
| `Exit::Internal` | MISSING |
