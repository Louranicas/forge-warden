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
for script in plants/test_runner.py scripts/tests/perimeter.py scripts/tests/runner_routing.py; do
 : > "$TEST_ROOT/$script"
done
cat > "$TEST_ROOT/scripts/tool_versions.py" <<'STUB'
import os
import sys

statuses = {"--check": "VERSIONS_EXIT", "--check-local-tools": "LOCAL_TOOLS_EXIT"}
sys.exit(int(os.environ.get(statuses[sys.argv[1]], "0")))
STUB
printf 'import os, sys\nsys.exit(int(os.environ.get("PLANTS_EXIT", "0")))\n' > "$TEST_ROOT/scripts/check_plants.py"
printf 'import os, sys\nsys.exit(int(os.environ.get("GUARDRAIL_EXIT", "0")))\n' > "$TEST_ROOT/scripts/check_guardrails.py"
cp "$source_root/warden.toml" "$TEST_ROOT/warden.toml"
cat > "$sandbox/bin/cargo" <<'STUB'
#!/usr/bin/env python3
import argparse
import os
from pathlib import Path
import sys

root = Path(os.environ["TEST_ROOT"])
command, *args = sys.argv[1:]
with (root / "cargo-calls").open("a") as calls:
    print(command, *args, file=calls)

if command == "run":
    split = args.index("--")
    cargo = argparse.ArgumentParser(allow_abbrev=False)
    cargo.add_argument("--quiet", action="store_true")
    cargo.add_argument("--bin", required=True, choices=["forge-warden"])
    cargo.parse_args(args[:split])
    verb, *options = args[split + 1:]
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("--repo", required=True, type=Path)
    parser.add_argument("--base", required=True)
    parser.add_argument("--head", required=True)
    if verb == "check":
        for name in ("policy", "ranker", "patch"):
            parser.add_argument("--" + name, required=True, type=Path)
    elif verb != "empty":
        sys.exit(95)
    inputs = parser.parse_args(options)
    expected = [os.environ["BASE_ID"], os.environ["HEAD_ID"]]
    if (inputs.repo.resolve() != root / ".git" or [inputs.base, inputs.head] != expected
            or (root / "patch-inputs").read_text().splitlines() != expected):
        sys.exit(96)
    if verb == "empty":
        (root / "verified-empty").touch()
        sys.exit(int(os.environ.get("EMPTY_EXIT", "0")))
    if (inputs.policy.resolve() != root / "warden.toml"
            or inputs.ranker.resolve() != root.parent / "bin/deep-diff-forge"
            or inputs.patch.read_bytes() != (root / "expected.patch").read_bytes()):
        sys.exit(95)
    (root / "judged-patch").write_text(str(inputs.patch))
    sys.exit(int(os.environ.get("JUDGE_EXIT", "0")))
if command == "deny":
    sys.exit(int(os.environ.get("DENY_EXIT", "0")))
if command == "mutants":
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("--workspace", required=True, action="store_true")
    parser.add_argument("--no-config", required=True, action="store_true")
    parser.add_argument("--cap-lints", required=True, choices=["true"])
    parser.add_argument("--in-diff", required=True, type=Path)
    diff = parser.parse_args(args).in_diff
    if "+fixture" not in diff.read_text().splitlines():
        sys.exit(93)
    (root / "mutated-diff").write_text(str(diff))
    sys.exit(int(os.environ.get("MUTANTS_EXIT", "0")))
STUB
printf '#!/usr/bin/env bash\nexit 98\n' > "$sandbox/bin/deep-diff-forge"
printf '#!/usr/bin/env bash\nexit 0\n' > "$sandbox/bin/zizmor"
chmod +x "$sandbox/bin/"*
for tool in git bash env python3 mkdir mktemp rm head grep cat mv cp chmod; do
 ln -s "$(type -P "$tool")" "$sandbox/bin/$tool"
done
export PATH="$sandbox/bin"
while IFS= read -r name; do unset "$name"; done < <(compgen -v GIT_)
git() { GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 command git "$@"; }
export -f git
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
git pack-refs --all
bash "$source_root/scripts/make_patch.sh" "$BASE_ID" "$HEAD_ID" > "$TEST_ROOT/expected.patch"
git config --local diff.noprefix true

run_gate() {
 local entry="$1" expected="$2" actual=0
 : > "$TEST_ROOT/cargo-calls"
 bash "$entry" > "$sandbox/output" 2>&1 || actual=$?
 if [ "$actual" -ne "$expected" ] || { [ "$actual" -ne 0 ] && grep -qE 'ci_local: (all gates passed|offline gates passed)' "$sandbox/output"; }; then
  cat "$sandbox/output" >&2
  echo "FAIL ${entry##*/}: expected exit $expected, got $actual" >&2
  exit 1
 fi
}
gate="$TEST_ROOT/scripts/ci_local.sh"
for entry in "$gate" "$sandbox/pre-push"; do
 rm -rf "$TEST_ROOT/target"
 previous_patch=""
 previous_diff=""
 for JUDGE_EXIT in 0 3 4 7 8 9; do
  export JUDGE_EXIT
  rm -f "$TEST_ROOT/judged-patch" "$TEST_ROOT/mutated-diff"
  run_gate "$entry" "$JUDGE_EXIT"
  patch="$(cat "$TEST_ROOT/judged-patch")"
  mutation_diff="$(cat "$TEST_ROOT/mutated-diff")"
  [[ ! -e "$patch" && ! -e "$mutation_diff" && "$patch" != "$previous_patch" && "$mutation_diff" != "$previous_diff" ]]
  previous_patch="$patch"
  previous_diff="$mutation_diff"
  grep -q '^run ' "$TEST_ROOT/cargo-calls"
  if [ "$JUDGE_EXIT" -eq 0 ]; then grep -qxF 'ci_local: all gates passed' "$sandbox/output"; fi
 done
 unset JUDGE_EXIT
 grep -qx 'deny check' "$TEST_ROOT/cargo-calls"
 for row in 'VERSIONS_EXIT 10' 'LOCAL_TOOLS_EXIT 11' 'DENY_EXIT 5' 'MUTANTS_EXIT 7' 'GUARDRAIL_EXIT 1' 'PLANTS_EXIT 23'; do
  read -r name code <<< "$row"
  export "$name=$code"
  run_gate "$entry" "$code"
  if [ "$name" != PLANTS_EXIT ] && grep -q '^run ' "$TEST_ROOT/cargo-calls"; then exit 1; fi
  unset "$name"
 done
done

cp "$source_root/fixtures/adversarial/no-secrets.toml" "$TEST_ROOT/warden.toml"
for entry in "$gate" "$sandbox/pre-push"; do
 rm -f "$TEST_ROOT/judged-patch"
 JUDGE_EXIT=4 run_gate "$entry" 4
 [[ -s "$TEST_ROOT/judged-patch" ]]
done
cp "$source_root/warden.toml" "$TEST_ROOT/warden.toml"

NEXT_HEAD="$(git commit-tree "$tree" -p "$HEAD_ID" -m later)"
export NEXT_HEAD
run_gate "$gate" 0
unset NEXT_HEAD
git update-ref HEAD "$HEAD_ID"
git update-ref refs/remotes/origin/main "$BASE_ID"

mkdir -p .git/refs/remotes/origin
(
 trap 'chmod 700 .git/refs/remotes/origin' EXIT
 chmod 600 .git/refs/remotes/origin
 if [ ! -x .git/refs/remotes/origin ]; then
  run_gate "$gate" 1
  grep -qF 'ci_local: cannot inspect comparison ref' "$sandbox/output"
 fi
)

mkdir -p .git/info
printf '%s\n' "$HEAD_ID" > .git/info/grafts
git replace "$root_id" "$(git commit-tree "$tree" -m replacement)"
run_gate "$gate" 0
grep -q '^mutants ' "$TEST_ROOT/cargo-calls"
grep -q '^run ' "$TEST_ROOT/cargo-calls"
git update-ref -d refs/remotes/origin/main
BASE_ID="$root_id"
run_gate "$gate" 0

grep -q '^mutants ' "$TEST_ROOT/cargo-calls"
grep -q '^run ' "$TEST_ROOT/cargo-calls"
rm .git/info/grafts
git replace -d "$root_id"

printf '%s\n' "$HEAD_ID" > .git/shallow
run_gate "$gate" 1
grep -qF 'root comparison requires complete history' "$sandbox/output"
rm .git/shallow

mkdir -p .git/refs/remotes/origin
printf 'broken\n' > .git/refs/remotes/origin/main
run_gate "$gate" 1
rm .git/refs/remotes/origin/main
git update-ref refs/remotes/origin/main/other "$HEAD_ID"
run_gate "$gate" 0
git update-ref -d refs/remotes/origin/main/other

mv "$sandbox/bin/deep-diff-forge" "$sandbox/ranker"
run_gate "$gate" 1
grep -qF 'required tool missing: deep-diff-forge' "$sandbox/output"
git update-ref refs/remotes/origin/main "$HEAD_ID"
rm "$sandbox/bin/zizmor"
BASE_ID="$HEAD_ID"
for entry in "$gate" "$sandbox/pre-push"; do
 rm -f "$TEST_ROOT/judged-patch" "$TEST_ROOT/verified-empty"
 EMPTY_EXIT=3 run_gate "$entry" 3
 run_gate "$entry" 0
 [[ -e "$TEST_ROOT/verified-empty" && ! -e "$TEST_ROOT/judged-patch" ]]
 grep -qxF 'ci_local: offline gates passed; hosted-only, not run here: zizmor' "$sandbox/output"
done
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
