---
name: ship-a-change
description: Run forge-warden's definition of done on the current change and produce an evidence-backed summary. Use before saying any change is finished, ready, or done.
when_to_use: Before a final summary, a commit, or a pull request description.
---

# Ship a change

Start each Bash session at the root of the checkout under review and run `unset "${!GIT_@}"`
before the first step in that session. Record the command and its result; a step you could not run is reported as
**not verified**, never as passed.

1. **Fast gate**: `scripts/ci_fast.sh`. Must be green.
2. **Docs build**: `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`.
3. **New behaviour has a failing-first test.** For each behaviour change, name the test and
   confirm it fails without the change: stash the source change
   with `(unset "${!GIT_@}"; git stash push -- crates/*/src)`. If the push created a stash, run the
   test, record its failure, and restore that stash with `(unset "${!GIT_@}"; git stash pop)`.
4. **Mutation** (if `cargo-mutants` is installed): run `just mutants`.
   Every surviving mutant is a missing test: add it, or explain why the mutant is equivalent.
5. **Judge the change with its own policy** (if `deep-diff-forge` is installed): `just judge`.
   Fail means fix. Hold means the summary must explain each held rule to the human. `just judge` is
   advisory: it can never pass. `warden.toml` requires verified claims (`[claims] require_verified`),
   so only a bound authority run can pass; with the pinned deep-diff-forge release binary and an
   owner-enrolled Git allowlist ([GIT_RUNTIME](../../../docs/GIT_RUNTIME.md)), run it beside the advisory one:
   ```bash
   set -e
   unset "${!GIT_@}"
   base="$(git rev-parse --verify 'refs/remotes/origin/main^{commit}')"
   head="$(git rev-parse --verify 'HEAD^{commit}')"
   scratch="$(mktemp -d)"
   trap 'rm -rf -- "$scratch"' EXIT
   scripts/make_patch.sh "$base" "$head" > "$scratch/change.patch"
   cargo run --quiet --bin forge-warden -- check --policy warden.toml --patch "$scratch/change.patch" \
     --ranker "$(command -v deep-diff-forge)" \
     --repo "$(git rev-parse --path-format=absolute --git-common-dir)" --base "$base" --head "$head"
   ```
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
