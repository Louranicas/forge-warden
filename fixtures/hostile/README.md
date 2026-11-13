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

Regression inputs:
- `rank/long-path.rank.json` (EVIDENCE-0.2.0, ledger Q51): the demo ranking with one path of 10 006 bytes. HOSTILE-INPUT.1 found
  that a hang planted in `RepoPath::parse` for paths over 10 000 bytes escaped the 256-case budget when every field was hostile at
  once; the one-field-at-a-time properties now reach it, and this file replays that shape on every run.

Patch facts (`PatchFacts::from_git`) have generated properties but no replay directory here: a facts finding becomes a named test
in the harness instead.

Header shapes are not generated here. The properties put hostile values inside headers of the shape Git writes by
default; a header Git writes under another setting (`diff.noprefix`, other `diff.srcPrefix`/`dstPrefix`, external diff
output) was never produced, and that is how a prefix-less patch reached an authority pass on the frozen commit. Those shapes
are covered by PATCH-PREFIX's own tests and regression patches, and by an attack that lets Git write the patch under each
documented setting and judges it in authority mode (recorded in docs/THREAT_MODEL.md, scope section).
