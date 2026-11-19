#!/usr/bin/env bash
# Exercise the gate and its pre-push entrypoint with controlled process failures.
set -euo pipefail
source_root="$(cd "$(dirname "$0")/../.." && pwd)"
sandbox="$(mktemp -d)"
trap 'rm -rf "$sandbox"' EXIT
export TEST_ROOT="$sandbox/repo" CARGO_TARGET_DIR="$sandbox/external-target"
mkdir -p "$sandbox/bin" "$TEST_ROOT/scripts/tests" "$TEST_ROOT/plants"
cp "$source_root/scripts/ci_local.sh" "$source_root/scripts/make_patch.sh" "$TEST_ROOT/scripts/"
cp "$source_root/.githooks/pre-push" "$sandbox/pre-push"
cat >> "$TEST_ROOT/scripts/make_patch.sh" <<'STUB'
printf '%s\n' "$1" "$2" > "$TEST_ROOT/patch-inputs"
if [ -n "${NEXT_HEAD:-}" ]; then
  git update-ref HEAD "$NEXT_HEAD"
  git update-ref refs/remotes/origin/main "$NEXT_HEAD"
fi
STUB
for script in scripts/tool_versions.py plants/test_runner.py scripts/tests/perimeter.py scripts/tests/runner_routing.py; do
 : > "$TEST_ROOT/$script"
done
printf 'import os, sys\nsys.exit(int(os.environ.get("PLANTS_EXIT", "0")))\n' > "$TEST_ROOT/scripts/check_plants.py"
printf 'import os, sys\nsys.exit(int(os.environ.get("GUARDRAIL_EXIT", "0")))\n' > "$TEST_ROOT/scripts/check_guardrails.py"
cp "$source_root/warden.toml" "$TEST_ROOT/warden.toml"
cat > "$sandbox/bin/cargo" <<'STUB'
#!/usr/bin/env bash
set -eu
printf '%s\n' "$*" >> "$TEST_ROOT/cargo-calls"
case "$1" in
 run)
  [[ " $* " == *" --ranker ${TEST_ROOT%/*}/bin/deep-diff-forge "* ]] || exit 97
  [[ " $* " == *" --repo $TEST_ROOT/.git --base $BASE_ID --head $HEAD_ID "* ]] || exit 96
  [[ " $* " == *' --patch target/change.patch '* && " $* " != *' --advisory '* ]] || exit 95
  [[ "$(cat "$TEST_ROOT/patch-inputs")" == "$(printf '%s\n' "$BASE_ID" "$HEAD_ID")" ]] || exit 94
  exit "${JUDGE_EXIT:-0}" ;;
 deny) exit "${DENY_EXIT:-0}" ;;
 mutants) exit "${MUTANTS_EXIT:-0}" ;;
esac
exit 0
STUB
printf '#!/usr/bin/env bash\nexit 98\n' > "$sandbox/bin/deep-diff-forge"
printf '#!/usr/bin/env bash\nexit 0\n' > "$sandbox/bin/zizmor"
chmod +x "$sandbox/bin/"*
for tool in git bash env python3 mkdir mktemp rm head grep cat mv cp; do
 ln -s "$(type -P "$tool")" "$sandbox/bin/$tool"
done
export PATH="$sandbox/bin"
while IFS= read -r name; do unset "$name"; done < <(compgen -v GIT_)
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME=Fixture GIT_AUTHOR_EMAIL=fixture@example.invalid
export GIT_COMMITTER_NAME="$GIT_AUTHOR_NAME" GIT_COMMITTER_EMAIL="$GIT_AUTHOR_EMAIL"
cd "$TEST_ROOT"
git init -q
git commit --allow-empty -qm root
root_id="$(git rev-parse HEAD)"
mkdir -p crates/sample/src
printf 'fixture\n' > crates/sample/src/lib.rs
git add crates/sample/src/lib.rs
git commit -qm head
HEAD_ID="$(git rev-parse HEAD)"
tree="$(git rev-parse 'HEAD^{tree}')"
BASE_ID="$(git commit-tree "$tree" -p "$root_id" -m upstream)"
export BASE_ID HEAD_ID
git update-ref refs/remotes/origin/main "$BASE_ID"

run_gate() {
 local entry="$1" expected="$2" actual=0
 : > "$TEST_ROOT/cargo-calls"
 bash "$entry" > "$sandbox/output" 2>&1 || actual=$?
 if [ "$actual" -ne "$expected" ] || { [ "$actual" -ne 0 ] && grep -qE 'ci_local: (all gates passed|offline gates passed)' "$sandbox/output"; }; then
  cat "$sandbox/output" >&2
  echo "FAIL $entry: expected exit $expected, got $actual" >&2
  exit 1
 fi
}
gate="$TEST_ROOT/scripts/ci_local.sh"
for entry in "$gate" "$sandbox/pre-push"; do
 rm -rf "$TEST_ROOT/target"
 for JUDGE_EXIT in 0 3 4 7 8 9; do
  export JUDGE_EXIT
  run_gate "$entry" "$JUDGE_EXIT"
  grep -q '^run ' "$TEST_ROOT/cargo-calls"
 done
 unset JUDGE_EXIT
 grep -qx 'deny check' "$TEST_ROOT/cargo-calls"
 grep -qx 'mutants --no-config --cap-lints true --in-diff target/pr.diff --workspace' "$TEST_ROOT/cargo-calls"
 for row in 'DENY_EXIT 5' 'MUTANTS_EXIT 7' 'GUARDRAIL_EXIT 1' 'PLANTS_EXIT 23'; do
  read -r name code <<< "$row"
  export "$name=$code"
  run_gate "$entry" "$code"
  if [ "$name" != PLANTS_EXIT ] && grep -q '^run ' "$TEST_ROOT/cargo-calls"; then exit 1; fi
  unset "$name"
 done
done

printf 'schema = "forge-warden.policy.v0"\n' > "$TEST_ROOT/warden.toml"
JUDGE_EXIT=4 run_gate "$gate" 4
grep -q '^run ' "$TEST_ROOT/cargo-calls"
cp "$source_root/warden.toml" "$TEST_ROOT/warden.toml"

NEXT_HEAD="$(git commit-tree "$tree" -p "$HEAD_ID" -m later)"
export NEXT_HEAD
run_gate "$gate" 0
unset NEXT_HEAD
git update-ref HEAD "$HEAD_ID"

git update-ref -d refs/remotes/origin/main
BASE_ID="$root_id"
run_gate "$gate" 0

mv "$sandbox/bin/deep-diff-forge" "$sandbox/ranker"
run_gate "$gate" 1
grep -qF 'required tool missing: deep-diff-forge' "$sandbox/output"
git update-ref refs/remotes/origin/main "$HEAD_ID"
rm "$sandbox/bin/zizmor"
run_gate "$gate" 0
if grep -q '^run ' "$TEST_ROOT/cargo-calls"; then exit 1; fi
grep -qxF 'ci_local: offline gates passed; hosted-only, not run here: zizmor' "$sandbox/output"
mv "$sandbox/ranker" "$sandbox/bin/deep-diff-forge"

unrelated="$(git commit-tree "$tree" -m unrelated)"
git update-ref refs/remotes/origin/main "$unrelated"
run_gate "$gate" 2
grep -qF 'no common history' "$sandbox/output"

sibling="$(git commit-tree "$tree" -p "$root_id" -m sibling)"
left="$(git commit-tree "$tree" -p "$HEAD_ID" -p "$sibling" -m left)"
right="$(git commit-tree "$tree" -p "$sibling" -p "$HEAD_ID" -m right)"
git update-ref HEAD "$left"
git update-ref refs/remotes/origin/main "$right"
run_gate "$gate" 2
grep -qF '2 merge bases' "$sandbox/output"
echo 'PASS local gate and pre-push bound judgments and refusals'
