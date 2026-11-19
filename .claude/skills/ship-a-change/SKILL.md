---
name: ship-a-change
description: Run forge-warden's definition of done on the current change and produce an evidence-backed summary. Use before saying any change is finished, ready, or done.
when_to_use: Before a final summary, a commit, or a pull request description.
---

# Ship a change

Start each Bash session at the root of the checkout under review and run `unset "${!GIT_@}"`
before the first step in that session. Record each command and its result. Report a step you
could not run as **not verified**.

1. **Fast gate**: `scripts/ci_fast.sh`. Must be green.
2. **Docs build**: `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`.
3. **New behaviour has a failing-first test.** For each behaviour change, name the test and
   confirm it fails without the change: stash the source change
   with `just stash-source`. If it lists untracked files, stage the listed source before retrying.
   Ignored Rust source also needs to be included. If a file or symlink replaces a directory, or a
   directory replaces either, the save refuses and leaves the changes in place. It also refuses when
   source edits are staged but the working source matches HEAD. Mark the failing-first check
   **not verified** until the save succeeds. When the save prints a stash object ID,
   record it, run the test, and record the failure. Run `just restore-source OBJECT_ID` with that exact ID to restore the
   source and its staged changes. The saved entry remains in the stash list for recovery.
4. **Mutation** (if `cargo-mutants` is installed): run `just mutants`.
   Every surviving mutant is a missing test: add it, or explain why the mutant is equivalent.
5. **Judge the change with its own policy** (if `deep-diff-forge` is installed): `just judge`.
   Fail means fix. Hold means the summary must explain each held rule to the human. `just judge` is
   advisory: it can never pass. `warden.toml` requires verified claims (`[claims] require_verified`),
   so only a bound authority run can pass; with the pinned deep-diff-forge release binary and an
   owner-enrolled Git allowlist ([GIT_RUNTIME](../../../docs/GIT_RUNTIME.md)), run `just judge-bound` beside the advisory check.
   A `claims_unverified` hold means the run was not bound: re-run with `--repo`; it cannot be acknowledged.
6. **Contracts**: if `schemas/`, `exit.rs`, `verdict.rs` or `vocab.rs` changed, confirm
   `CHANGELOG.md` has an entry and ask the human to run `scripts/bless.sh` if goldens moved.
7. **Independent review**: run the `reviewer` subagent and address every blocker.

## Summary format

```text
What changed: <one paragraph>
Verified:
  - <command> -> <result>
Not verified: <what, and why>
Tests that fail without this change: <names>
Held rules / open questions for the human: <list or "none">
```
