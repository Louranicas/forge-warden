# Evidence

Claims in this repository's README and ADRs, each with the command that constructs it and the
result observed. Re-run them; do not take this file's word for it.

Each section below is a dated record of what was measured then, on the toolchain and commit it
names; its figures are not updated as the code moves. For example, the 0.1.0 section ran on Rust
1.97.0 and counted 85 tests and 39 corpus rankings; the toolchain is now pinned to 1.99.1
(`rust-toolchain.toml`) and `fixtures/corpus/` holds 40 rankings. Current figures come from
re-running the commands. A section for the next release is measured on its release commit.

## 0.2.0 (2026-11-13)

Measured on the release commit `7f36ba2a88e098bae1a936cb7f36523160053d4b`, from a clean clone, in a
Fedora 44 toolbox on Linux x86_64: Rust 1.99.1 (`rust-toolchain.toml`), MSRV 1.88, Git 2.56.0,
deep-diff-forge v0.8.2 (release binary SHA-256 `298e07511310b18051a4c8ed56c18fafa03e40dc8fc88bedf74edba0e2ba41ad`).

| Claim | Command | Result |
| --- | --- | --- |
| The lint wall, tests and docs are green | `bash scripts/ci_fast.sh`; `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --offline` | green |
| Every test passes | `cargo test --workspace --locked --offline` | 375 passed, 0 failed, 0 ignored, in 27 test binaries |
| The tests catch the viable mutants of the whole workspace | `cargo mutants --workspace --no-shuffle --cap-lints true --cargo-arg=--locked` | 737 mutants: 518 caught, 2 missed, 217 unviable, 0 timed out. Both missed mutants replace the body of `vocab::is_known_status`, a public function left with no caller when its one call in the evaluator was replaced by the typed `Status::is_known()`; no verdict depends on it |
| The tests catch the viable mutants of the release fix | `cargo mutants --in-diff` on the change from the frozen commit, in the merge gate | 19 caught, 0 missed, 1 unviable |
| The minimum Rust version builds and passes | `cargo +1.88.0 test --workspace --locked --offline`, in the merge gate | green |
| Supply chain is clean | `cargo deny check` | advisories, bans, licences, sources ok (advisory database as cached on the machine) |
| Bad code is refused | `python3 -I plants/run.py --source <repository> --revision <commit> --output <new dir>` | 114 of 114 planted defects caught over 51 guardrail rows; 2 rows (G-38, G-47) are pending, each waiting on a named owner input ([plants/README.md](plants/README.md)) |
| Every refusal and exit has a test that observes it | `cargo test -p forge-warden-cli --locked --offline --test refusal_map` | [docs/REFUSAL_MAP.md](docs/REFUSAL_MAP.md) is the rendering of the rows: 52 rows, 1 stated unreachable (`Exit::Internal`), 0 without a test; 3 rows also proven by a plant |
| A real deep-diff-forge corpus is read | `cargo test -p forge-warden-cli --locked --offline --test corpus`; `ls fixtures/corpus/*.rank.json \| wc -l` | passes; 40 rankings |
| The 0.1.0 review findings stay fixed | `cargo test --workspace --locked --offline -- renames_and_copies_hold_because_their_source_is_invisible binary_deletions_and_unknowable_binary_changes_are_not_companions status_unknown_holds` | pass |
| Agent guardrails bite | `python3 -I .claude/hooks/test_guard.py` | pass |
| The demo verdict is the reviewed golden | `python3 -I scripts/golden_output.py --binary <fixture CLI> --receipt <its receipt> --subject <its subject>` | `fixtures/demo.verdict.json`, SHA-256 `8068135b4bf40eea31fbf8f946b44fdba3ac6294f6a1b943ce1d86894d2b7e32` |
| The contributing walkthrough runs as written | the fourth `bash` block of [CONTRIBUTING.md](CONTRIBUTING.md) | exits 0 (advisory exits 8, the tightened control 7, as scripted) |
| The documented commands do what the documents say | README quick start (advisory), `init`, `lint`, `explain` for every exit, `schema exit-codes`, `schema verdict`, the 0.1.0 command forms, `--patch -`, `--json` failures | as documented: the demo exits 7 and prints the sample's lines; `schema verdict` equals `schemas/verdict.v2.schema.json`; the 0.1.0 forms exit 2; `--patch -` is refused; each `--json` failure is one JSON object |
| The pinned producer is the one the documents name | `sha256sum` of the release binary; `[producer]` in `warden.toml`; [docs/RUNBOOK.md](docs/RUNBOOK.md) step 3 | the same digest in all three |
| A patch is judged on the paths Git means | Git 2.56 writes 135 patches of a protected change under 17 documented settings (`diff.noprefix`, other prefixes, `diff.relative` in a subdirectory, `diff.external`, colour, `format.noprefix`, ...), each with four commands, judged in authority mode | every patch made with `scripts/make_patch.sh` or the documented command FAILs or HOLDs; the only passes come from a `git diff` without `--no-relative` run in a subdirectory, the limit stated in [docs/THREAT_MODEL.md](docs/THREAT_MODEL.md); a docs-only change still passes |
| Authority mode works end to end | a release build, the pinned ranker and an enrolled Git allowlist (installed by an agent, on the owner's instruction, in a container: the user's own uid) | on the frozen commit: a docs-only change passed (exit 0), a protected change held (8), a wrong producer digest was refused (4), an unowned or unenrolled Git was refused (3) |

Not verified here, for 0.2.0:
- An owner-installed Git allowlist on a host (runbook step 4) has not been run; the authority runs above used one an agent installed in a container.
- The workflow audit (`zizmor`) has no offline backend here; it runs in hosted CI only (G-47).
- The hosted CI run on this commit is recorded by the pull request that publishes it, not here.
- `cargo install --git … --tag v0.2.0` works only once the `v0.2.0` tag exists.
- The Managed Agents calls in `agents/managed/` are still not executed.

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
