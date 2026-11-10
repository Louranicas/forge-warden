#!/usr/bin/env bash
# Everything CI runs that can run offline, in CI's order. `just ci` calls this.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
# Hook-exported Git state must not redirect scratch-repository commands.
while IFS= read -r name; do unset "$name"; done < <(compgen -v GIT_)
# Gate artifacts stay local even when cargo builds in an external target directory.
mkdir -p target
export RUSTDOCFLAGS="-D warnings"
step() { printf '\n== %s\n' "$*"; }
step tool-versions; python3 -I scripts/tool_versions.py --check
# G-18: a required tool that is missing or at another version is a refusal, not a skip.
step local-tools; python3 -I scripts/tool_versions.py --check-local-tools
step perimeter; python3 -I scripts/tests/perimeter.py
step runner-routing; python3 -I scripts/tests/runner_routing.py
step plant-runner; python3 -I plants/test_runner.py
step guardrails; python3 -I scripts/check_guardrails.py
step fmt;     cargo fmt --all --check
step clippy;  cargo clippy --workspace --all-targets --locked
step test;    cargo test --workspace --locked
step doc;     cargo doc --workspace --no-deps --locked --quiet
# Steps that cannot run here (scripts/tool-versions.toml [local_gate] hosted_only) are named at the end.
hosted_only_not_run=""
step deny; cargo deny check
if command -v zizmor >/dev/null; then step zizmor; zizmor --persona=pedantic .github/workflows; else hosted_only_not_run+=" zizmor"; fi
step mutation
base="$(git merge-base HEAD origin/main 2>/dev/null || git rev-list --max-parents=0 HEAD)"
git diff "$base" -- 'crates/*/src/**.rs' > target/pr.diff
if [ -s target/pr.diff ]; then env -u CARGO_TARGET_DIR cargo mutants --no-config --cap-lints true --in-diff target/pr.diff --workspace; else echo "no source changes to mutate"; fi
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
else
  hosted_only_not_run+=" deep-diff-forge"
fi
step plants; python3 -I scripts/check_plants.py
if [ -n "$hosted_only_not_run" ]; then
  printf '\nci_local: offline gates passed; hosted-only, not run here:%s\n' "$hosted_only_not_run"
else
  printf '\nci_local: all gates passed\n'
fi
