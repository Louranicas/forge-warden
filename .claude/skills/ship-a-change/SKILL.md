---
name: ship-a-change
description: Run forge-warden's definition of done on the current change and produce an evidence-backed summary. Use before saying any change is finished, ready, or done.
when_to_use: Before a final summary, a commit, or a pull request description.
---

# Ship a change

Run each step. Record the command and its result; a step you could not run is reported as
**not verified**, never as passed.

1. **Fast gate**: `scripts/ci_fast.sh`. Must be green.
2. **Docs build**: `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`.
3. **New behaviour has a failing-first test.** For each behaviour change, name the test and
   confirm it fails without the change: stash the source change
   (`git stash push -- crates/*/src`), run the test, see it fail, `git stash pop`.
4. **Mutation** (if `cargo-mutants` is installed):
   `git diff "$(git merge-base HEAD origin/main)" -- 'crates/*/src/**.rs' > target/pr.diff &&
   cargo mutants --no-config --in-diff target/pr.diff --workspace`.
   Every surviving mutant is a missing test: add it, or explain why the mutant is equivalent.
5. **Judge the change with its own policy** (if `deep-diff-forge` is installed): `just judge`.
   Fail means fix. Hold means the summary must explain each held rule to the human.
6. **Contracts**: if `schemas/`, `exit.rs`, `verdict.rs` or `vocab.rs` changed, confirm
   `CHANGELOG.md` has an entry and ask the human to run `scripts/bless.sh` if goldens moved.
7. **Independent review**: run the `reviewer` subagent and address every blocker.

## Summary format

```
What changed: <one paragraph>
Verified:
  - <command> -> <result>
Not verified: <what, and why>
Tests that fail without this change: <names>
Held rules / open questions for the human: <list or "none">
```
