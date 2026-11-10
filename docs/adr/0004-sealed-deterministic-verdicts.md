# ADR 0004: Sealed, deterministic verdicts

Status: Accepted
Owner-deployment: Not-required

Historical status record: **Status:** accepted, 2026-11-05

## Decision

Every verdict carries `policy_sha256`, `rank_sha256` and deep-diff-forge's `patch_sha256`. With
`--patch`, forge-warden hashes the patch itself and refuses (exit 4) unless it equals the
ranking's seal, so a verdict cannot be produced for one patch and attached to another.

Output is a pure function of `(policy bytes, ranking bytes, date)`: rules in policy order,
violations in deep-diff-forge's risk order, evidence sorted, no timestamps beyond the date, no
hash-ordered collections. `fixtures/demo.verdict.json` is a golden file; the corpus test runs
every real ranking twice and compares bytes.

## Consequences

A verdict is evidence: anyone with the three inputs can reproduce it byte for byte, and
`sha256sum` checks each seal. Golden files change only through `scripts/bless.sh`, which agents
may not run, so a changed contract is always seen by a person.
