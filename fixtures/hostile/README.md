# Hostile-input corpus

Inputs that once crashed, hung or were refused wrongly by a parser, kept so that they stay fixed.
The harness is `crates/forge-warden-core/tests/hostile_input.rs`; it replays every file under
`rank/` (through the ranking parser) and `policy/` (through the policy parser) on every run.

Budget: 256 generated cases per property per run (HOSTILE_CASES=<n> raises it for a campaign; a lower value is ignored), with proptest's default seeding, a 2 s
bound on any one parse in a debug build, enforced from a watchdog thread so a hang fails rather than stalls. A finding is added here, with a ledger row, the same day
it is found; the fix is its own package.

Generators: arbitrary bytes; real rankings, policies and Git output mutated byte by byte; and rankings that are
valid JSON with the right schema and tool name but hostile field values (non-ASCII, very long and malformed paths,
numbers at and past the integer edges, repeated entries), so field validation is reached and not only the JSON reader.

Seeds: `empty.json` and `empty.toml`, the empty input.
