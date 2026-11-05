# ADR 0001: A pure core, and effects only through named doors

**Status:** accepted, 2026-11-05

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

## Consequences

- Every verdict is reproducible from `(policy bytes, ranking bytes, date)`; the golden test and
  `evaluation_is_pure` property hold it to that.
- The date is an explicit input (`--today`), which also makes waiver expiry testable.
- Adding an effect costs a reviewed diff to one small file. That cost is the point.
