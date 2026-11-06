#!/usr/bin/env bash
# Everything CI runs that can run offline, in CI's order. `just ci` calls this.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
# Gate artifacts stay local even when cargo builds in an external target directory.
mkdir -p target
export RUSTDOCFLAGS="-D warnings"
step() { printf '\n== %s\n' "$*"; }
step guardrails; python3 -I scripts/check_guardrails.py
step fmt;     cargo fmt --all --check
step clippy;  cargo clippy --workspace --all-targets --locked
step test;    cargo test --workspace --locked
step doc;     cargo doc --workspace --no-deps --locked --quiet
if command -v cargo-deny >/dev/null; then step deny; cargo deny check; else echo "skip deny: cargo install cargo-deny --locked"; fi
if command -v zizmor >/dev/null; then step zizmor; zizmor --persona=pedantic .github/workflows; else echo "skip zizmor: pipx install zizmor"; fi
if command -v cargo-mutants >/dev/null; then
  step mutation
  base="$(git merge-base HEAD origin/main 2>/dev/null || git rev-list --max-parents=0 HEAD)"
  git diff "$base" -- 'crates/*/src/**.rs' > target/pr.diff
  if [ -s target/pr.diff ]; then env -u CARGO_TARGET_DIR cargo mutants --no-config --cap-lints true --in-diff target/pr.diff --workspace; else echo "no source changes to mutate"; fi
else
  echo "skip mutation: cargo install cargo-mutants --locked"
fi
if command -v deep-diff-forge >/dev/null; then
  step dogfood
  base="$(git merge-base HEAD origin/main 2>/dev/null || git rev-list --max-parents=0 HEAD)"
  git diff --binary --no-renames "$base" > target/change.patch
  if [ -s target/change.patch ]; then
    mode="$(python3 -I scripts/judge_mode.py warden.toml)"
    if [ "$mode" = authority ]; then
      ranker="$(command -v deep-diff-forge)"
      # The judge checks the binary's hash against the existing policy; never auto-enroll it.
      cargo run --quiet --bin forge-warden -- check --policy warden.toml \
        --ranker "$ranker" --patch target/change.patch
    else
      echo 'ci_local: explicit advisory mode (no producer pin); this result cannot PASS' >&2
      deep-diff-forge --stdin-patch --rank --json < target/change.patch > target/rank.json
      cargo run --quiet --bin forge-warden -- check --policy warden.toml \
        --advisory --rank target/rank.json --patch target/change.patch
    fi
  fi
fi
printf '\nci_local: all available gates passed\n'
