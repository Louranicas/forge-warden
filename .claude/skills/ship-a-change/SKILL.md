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
3. **New behaviour has a failing-first test.** Identify the full commit ID of the intended
   pre-change baseline. Create a fresh disposable worktree at that commit with
   `git --no-replace-objects worktree add --detach "$isolated" "$baseline"`.
   Use an authorized scratch location. In that worktree, bring over only the named regression
   test and its required test fixtures. For an inline test, apply only the test hunks.
   Verify the checkout and test inputs represent that baseline plus the test, including the
   bytes of any embedded files. Run the named test there with separate build output and
   confirm it fails at the expected assertion. Confirm the same test passes with the change.
   Record the baseline ID, test command, and both outcomes. Remove the disposable worktree
   and its test artifacts after use. If the baseline, isolation, or expected failure and pass
   cannot be established, mark the check **not verified**. Keep the developer’s working files
   and index untouched throughout.
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
