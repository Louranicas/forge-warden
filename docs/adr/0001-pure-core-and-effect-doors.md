# ADR 0001: A pure core, and effects only through named doors

Status: Accepted
Owner-deployment: Not-required

Historical status record: **Status:** accepted, 2026-11-05

## Context

A gate is trusted only if its answer depends on nothing but its inputs. Hidden inputs (the clock,
the environment, a file the engine decided to read, a hash seed) make verdicts irreproducible and
let a change pass on one machine and fail on another.

## Decision

1. `forge-warden-core` is a pure library: bytes and a `Date` in, a `Verdict` out. Its own
   `clippy.toml` disallows file, stdin/stdout/stderr, environment, process, network, clock and
   thread APIs, and `HashMap`/`HashSet`. Because clippy uses the nearest `clippy.toml`, the core
   has no exemptions at all.
2. `forge-warden-cli` reaches the outside world only through functions in `src/doors.rs`, each
   carrying `#[expect(clippy::disallowed_methods, reason = "door: <name>")]`. The workspace
   `clippy.toml` makes the same calls a compile error everywhere else.
3. `tests/repo_invariants.rs` fails if any production file other than `doors.rs` mentions
   `disallowed_methods` or `disallowed_types`, or if the core names `std::fs` or `std::process`.
4. `doors.rs` is owned (CODEOWNERS, `warden.toml` rule `quality-config-is-owned`, and the
   Claude Code PreToolUse hook), so a new effect is always a reviewed decision.

Amended 2026-11-10 (WP-Q-DOOR-SPLIT, Q2-09): `src/doors.rs` became the door modules under
`src/doors/`. Points 2-4 apply to that exact, closed list (`DOOR_FILES` in the CLI's
`tests/common`); a file under `src/doors/` that is not listed fails the invariants. Ownership
moved with the code to `crates/forge-warden-cli/src/doors/**`.

## Consequences

- Every verdict is reproducible from `(policy bytes, ranking bytes, date)`; the golden test
  holds it to that, and the core cannot reach an effect (`clippy.toml`, `repo_invariants.rs`).
- The date is an explicit input (`--today`), which also makes waiver expiry testable.
- Adding an effect costs a reviewed diff to one small file. That cost is the point.

## Amendment (2026-11-17, EFFECT-WALL step 2): the core is `no_std`

The core's purity no longer rests on a list. `forge-warden-core` is `#![no_std]` (with `alloc`): it cannot name
`std::fs`, `std::net`, `std::process`, `std::env`, `std::thread` or `std::time` at all, so an effect of any
spelling, listed in `clippy.toml` or not, is a compile error there. Its glob matcher is its own (`policy/glob.rs`, a
port of globset 0.4.21 with the options the policy used, pinned and differentially tested against globset), because
globset has no `no_std` mode; its other dependencies (serde, serde_json, sha2, thiserror, toml) are built without
`std` for it, and the binary turns `std` back on for its own use. What remains as text is exact
(`the_core_is_no_std_unconditionally`): the one unconditional `#![no_std]` line, no `extern crate std`, and no core
feature that enables anything. The core's `clippy.toml` still refuses hash-ordered collections, which `alloc` has.


## Amendment (2026-11-17, EFFECT-WALL step 3): the doors are a crate, and the binary forbids the effect lints

The door modules moved, unchanged, from `crates/forge-warden-cli/src/doors/` to the crate `forge-warden-doors`
(`crates/forge-warden-doors/src/`; `mod.rs` became `lib.rs`). The binary depends on it and its `main.rs` carries
`#![forbid(clippy::disallowed_methods, clippy::disallowed_types)]`: no `allow`, `expect`, `cfg_attr`, macro output,
`#[path]` or `include!` can lower those lints in the binary, so a door can only exist in the doors crate. The
binary builds its command line with clap's builder, because clap's derive emits `allow(clippy::style)`, which the
forbid refuses (measured, R2). The core feature `mint` is enabled by the doors crate alone (ruling R1). Ownership
moved with the code to `crates/forge-warden-doors/src/**`; its `Cargo.toml` is covered by `crates/*/Cargo.toml`.
Test seams the fixture CLI needs are compiled under `cfg(any(test, all(fw_fixture_cli, debug_assertions)))`:
`fw_fixture_cli` is refused in every operational build surface exactly as `--cfg test` is, and a `--release` build
keeps the production seams (the Git allowlist from /etc) even with it (`door_lints` pins the spelling).
