# Evidence

Claims in this repository's README and ADRs, each with the command that constructs it and the
result observed. Re-run them; do not take this file's word for it.

## 0.1.0 (2026-11-05)

Environment: Linux x86_64, Rust 1.97.0 stable, deep-diff-forge 0.7.0 at
`587a6afd559a4bd9f8c0aabcef4c1e5de85ffc54`.

| Claim | Command | Result |
| --- | --- | --- |
| The lint wall, tests and docs are green | `scripts/ci_fast.sh`; `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | green |
| Every test passes | `cargo test --workspace` | 84 passed, 0 failed |
| The engine's tests catch every viable mutant | `cargo mutants -p forge-warden-core` | 0 missed (first run: 32 missed; each closed by a test, see commit `tests: close the gaps cargo-mutants found`) |
| Supply chain is clean | `cargo deny check` | advisories, bans, licences, sources ok |
| Workflows have no audit findings | `zizmor --persona=pedantic .github/workflows` | no findings (first run flagged a superfluous toolchain action, replaced with `rustup`) |
| The vocabulary equals deep-diff-forge's | `scripts/check_vocab.sh 587a6af… <checkout>` | 21 signals match; renaming one fails with a diff |
| A real deep-diff-forge corpus is read | `cargo test --test corpus` | 39 rankings from deep-diff-forge's fixtures parse; 20 of 21 signals exercised |
| Bad code is refused | planted, one at a time, in a scratch clone: `#[allow]`, `unwrap`, `HashMap` in core, `std::fs` in core, clock in CLI, `println!`, `unsafe`, indexing, a missing doc, an `as` cast | each fails `cargo clippy` |
| Repository invariants bite | removed `[lints] workspace = true` from a crate; added `#[allow]` to a test | each fails `repo_invariants` |
| Agent guardrails bite | `.claude/hooks/guard.py pre` fed edits to `clippy.toml`, `#[allow]`, `unsafe`, a reasonless `#[expect]`, a loosened lint, `--no-verify`, `commit -n` | each exits 2 with a reason; a dependency bump and `cargo test` pass |

## Not verified here

- The MSRV job (Rust 1.88): the session could not install a second toolchain. CI runs it.
- The GitHub Actions workflow end to end: it has not run on GitHub yet.
- The Managed Agents calls in `agents/managed/`: written from Anthropic's documentation, not
  executed.
