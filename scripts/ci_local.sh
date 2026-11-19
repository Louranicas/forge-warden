#!/usr/bin/env bash
# The build, lint, test and audit part of CI that runs offline, in CI's order: a subset of the gate.
# Not run here, and run by scripts/ci_fast.sh or by CI itself: tool_versions.py --check-toolchain,
# the mutation budget and environment tests, the local-tools tests (scripts/tests/local_tools.py),
# the dogfood, hold-ack, hook-guard and managed-review-bundle tests, and the guardrails tests. (The
# local-tools check itself does run here, below: tool_versions.py --check-local-tools.) `just ci`
# calls this.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
# Hook-exported Git state must not redirect scratch-repository commands.
while IFS= read -r name; do unset "$name"; done < <(compgen -v GIT_)
export RUSTDOCFLAGS="-D warnings"
step() { printf '\n== %s\n' "$*"; }
step tool-versions; python3 -I scripts/tool_versions.py --check
# G-18: a required tool that is missing or at another version is a refusal, not a skip.
# G-16: the compiler running this gate is the one rust-toolchain.toml pins (no override, no download).
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
head="$(git rev-parse --verify 'HEAD^{commit}')"
comparison_ref=refs/remotes/origin/main
ref_path="$(git rev-parse --git-path "$comparison_ref")"
loose="$(python3 -I - "$ref_path" <<'PYREF'
import os
import stat
import sys

try:
    mode = os.lstat(sys.argv[1]).st_mode
except (FileNotFoundError, NotADirectoryError):
    print("absent")
except OSError as error:
    sys.exit(f"ci_local: cannot inspect comparison ref: {error}")
else:
    print("absent" if stat.S_ISDIR(mode) else "present")
PYREF
)"
if [ "$loose" = present ] || git show-ref --verify --quiet "$comparison_ref"; then
  tip="$(git show-ref --verify --hash "$comparison_ref")" || exit 1
elif [ "$?" -eq 1 ]; then
  tip=""
else
  exit 1
fi
comparison="$(bash scripts/make_patch.sh --resolve-comparison "$tip" "$head")"
read -r base mutation_base <<< "$comparison"
scratch="$(mktemp -d)"
trap 'rm -rf -- "$scratch"' EXIT
patch="$scratch/change.patch"
mutation_diff="$scratch/pr.diff"
bash scripts/make_patch.sh "$base" "$head" > "$patch"
step mutation
git --no-replace-objects diff "$mutation_base" -- 'crates/*/src/**.rs' > "$mutation_diff"
if [ -s "$mutation_diff" ]; then env -u CARGO_TARGET_DIR cargo mutants --no-config --cap-lints true --in-diff "$mutation_diff" --workspace; else echo "no source changes to mutate"; fi
step dogfood
repo_git="$(git rev-parse --path-format=absolute --git-common-dir)"
if [ -s "$patch" ]; then
  ranker="$(type -P deep-diff-forge)" || { echo 'ci_local: required tool missing: deep-diff-forge' >&2; exit 1; }
  cargo run --quiet --bin forge-warden -- check --policy warden.toml \
    --ranker "$ranker" --patch "$patch" \
    --repo "$repo_git" --base "$base" --head "$head"
else
  cargo run --quiet --bin forge-warden -- empty --repo "$repo_git" --base "$base" --head "$head"
fi
step plants; python3 -I scripts/check_plants.py
if [ -n "$hosted_only_not_run" ]; then
  printf '\nci_local: offline gates passed; hosted-only, not run here:%s\n' "$hosted_only_not_run"
else
  printf '\nci_local: all gates passed\n'
fi
