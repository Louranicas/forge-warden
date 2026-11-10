# Hostile-input corpus

Inputs that once crashed, hung or were refused wrongly by a parser, kept so that they stay fixed.
The harness is `crates/forge-warden-core/tests/hostile_input.rs`; it replays every file under
`rank/` (through the ranking parser) and `policy/` (through the policy parser) on every run.

Budget: 256 generated cases per property per run, with proptest's default seeding, and a 2 s
bound on any one parse in a debug build. A finding is added here, with a ledger row, the same day
it is found; the fix is its own package.

Seeds: `empty.json` and `empty.toml`, the empty input.
