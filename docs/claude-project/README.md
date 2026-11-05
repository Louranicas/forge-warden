# Claude Project pack

A Claude Project gives design conversations the same contract the code is held to.

1. Create a project named **forge-warden** in Claude.
2. Paste [`INSTRUCTIONS.md`](INSTRUCTIONS.md) (below its line) into the project instructions.
3. Run `scripts/claude_project_bundle.sh` and upload `target/claude-project/` (one file per
   source, each headed with its path and the commit it came from). Re-run and re-upload after
   contract changes so the project never argues from a stale schema.

The bundle contains: `CLAUDE.md`, `README.md`, `CHANGELOG.md`, `docs/POLICY_REFERENCE.md`,
`docs/adr/*`, `schemas/*`, `warden.toml`, `examples/starter.warden.toml`, and the core sources
(`crates/forge-warden-core/src/*.rs`).
