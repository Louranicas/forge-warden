# Evidence

Claims in this repository's README and ADRs, each with the command that constructs it and the
result observed. Re-run them; do not take this file's word for it.

## 0.1.0 (2026-11-05)

Environment: Linux x86_64, Rust 1.97.0 stable, deep-diff-forge 0.7.0 at
`587a6afd559a4bd9f8c0aabcef4c1e5de85ffc54`.

| Claim | Command | Result |
| --- | --- | --- |
| The lint wall, tests and docs are green | `scripts/ci_fast.sh`; `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | green |
| Every test passes | `cargo test --workspace` | 85 passed, 0 failed |
| The tests catch every viable mutant | `cargo mutants -p forge-warden-core`; `cargo mutants -p forge-warden-cli --file 'crates/forge-warden-cli/src/doors/**'` | 0 missed in both (first runs: 32 missed in the core, 7 in the doors; each closed by a test or by deleting dead code) |
| Supply chain is clean | `cargo deny check` | advisories, bans, licences, sources ok |
| Workflows have no audit findings | `zizmor --persona=pedantic .github/workflows` | no findings (first run flagged a superfluous toolchain action, replaced with `rustup`) |
| The vocabulary equals deep-diff-forge's | `scripts/check_vocab.sh 587a6af… <checkout>` | 21 signals match; renaming one fails with a diff |
| A real deep-diff-forge corpus is read | `cargo test --test corpus` | 39 rankings from deep-diff-forge's fixtures parse; 20 of 21 signals exercised |
| Bad code is refused | planted, one at a time, in a scratch clone: `#[allow]`, `unwrap`, `HashMap` in core, `std::fs` in core, clock in CLI, `println!`, `unsafe`, indexing, a missing doc, an `as` cast | each fails `cargo clippy` |
| Repository invariants bite | removed `[lints] workspace = true` from a crate; added `#[allow]` to a test | each fails `repo_invariants` |
| Agent guardrails bite | `.claude/hooks/guard.py pre` fed edits to `clippy.toml`, `#[allow]` (also inside `cfg_attr`), `unsafe`, a reasonless `#[expect]`, a loosened lint, `--no-verify`, `commit -n`, and shell commands that write protected files (`sed -i`, `git mv`, `>`, `rm`, `python -c open(…,'w')`) | each exits 2 with a reason; a dependency bump, `cargo test` and reading protected files pass. It errs towards refusing: `… lint warden.toml > target/x` is refused too. |

## Independent review (2026-11-05)

A reviewer agent that had not seen the design reasoning attacked the repository and found three
fail-open bugs and one guardrail gap, each verified by reproduction. All are fixed, each with a
regression test that failed before the fix:

| Finding | Before | After | Test |
| --- | --- | --- | --- |
| Renaming `warden.toml`, `clippy.toml` and `repo_invariants.rs` away | exit 0 pass | exit 8 hold (`source_path_unknown`); with `--no-renames`, `quality-config-is-owned` holds | `renames_and_copies_hold_because_their_source_is_invisible` |
| Deleting a binary test file satisfied `require_companion` | exit 0 pass | exit 7 fail | `binary_deletions_and_unknowable_binary_changes_are_not_companions` |
| deep-diff-forge status `unknown` passed silently | exit 0 pass | exit 8 hold | `status_unknown_holds` |
| Shell commands could edit protected files under the Claude Code hook | allowed | refused | guard checks above |

Also from the review: `evidence_truncated` now reaches the verdict, and every pipeline in the
repository ranks with `git diff --no-renames`. The dogfood job refuses a base commit without a
`warden.toml` instead of falling back to the pull request's own policy.

## Not verified here

- The MSRV job (Rust 1.88): the session could not install a second toolchain. CI runs it.
- The GitHub Actions workflow end to end: it has not run on GitHub yet.
- The Managed Agents calls in `agents/managed/`: written from Anthropic's documentation, not
  executed.
