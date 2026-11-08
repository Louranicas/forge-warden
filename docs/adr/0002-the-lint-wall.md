# ADR 0002: The lint wall

Status: Accepted

Historical status record: **Status:** accepted, 2026-11-05

## Context

Review attention is the scarcest resource in a codebase increasingly written by agents. Anything
a reviewer must *remember* to check will eventually be missed.

## Decision

`[workspace.lints]` in `Cargo.toml` turns every check we can express as a lint into an error:

- `rustc`: `warnings` and `rust_2018_idioms` denied; `unsafe_code` and `non_ascii_idents`
  forbidden; `missing_docs`, `unreachable_pub`, `unused_qualifications` denied.
- `clippy`: the `all`, `pedantic` and `cargo` groups denied, plus restriction lints for panic
  discipline (`unwrap_used`, `expect_used`, `panic`, `indexing_slicing`, `string_slice`,
  `arithmetic_side_effects`, `as_conversions`, …), output discipline (`print_stdout`,
  `print_stderr`, `dbg_macro`, `exit`), and honesty (`wildcard_enum_match_arm`,
  `shadow_unrelated`, `allow_attributes`, `allow_attributes_without_reason`).

A lint is silenced only by `#[expect(lint, reason = "...")]`. An expectation that stops firing is
itself an error, so suppressions cannot outlive their cause. Tests may unwrap, index and panic
(`clippy.toml` `allow-*-in-tests`); integration-test helpers opt in per file with a reason.

Every crate must opt in with `[lints] workspace = true`; `repo_invariants.rs` checks it. The
Claude Code hook refuses an edit that removes or loosens a lint line.

## Consequences

- Code is longer in places (`checked_add` chains, `.get()` instead of `[i]`). That cost is paid
  once by the author instead of every time by a reviewer.
- New toolchains add lints; CI's stable job will surface them. The MSRV job checks with
  `RUSTFLAGS=""` so an older clippy's missing lints do not block it.
