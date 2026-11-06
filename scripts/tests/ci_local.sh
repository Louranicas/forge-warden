#!/usr/bin/env bash
# Exercise the gate and its pre-push entrypoint with controlled process failures.
set -euo pipefail
source_root="$(cd "$(dirname "$0")/../.." && pwd)"
sandbox="$(mktemp -d)"
trap 'rm -rf "$sandbox"' EXIT
mkdir -p "$sandbox/bin" "$sandbox/repo/scripts" "$sandbox/repo/target"
cp "$source_root/scripts/ci_local.sh" "$sandbox/repo/scripts/ci_local.sh"
cp "$source_root/.githooks/pre-push" "$sandbox/pre-push"
printf 'import os, sys\nsys.exit(int(os.environ.get("GUARDRAIL_EXIT", "0")))\n' > "$sandbox/repo/scripts/check_guardrails.py"
cat > "$sandbox/bin/git" <<'STUB'
#!/usr/bin/env bash
case "$1" in
 rev-parse) echo "$TEST_ROOT" ;;
 merge-base) echo base ;;
 diff) if [[ "$*" == *--binary* ]]; then printf 'fixture patch\n'; fi ;;
 *) exit 90 ;;
esac
STUB
cat > "$sandbox/bin/cargo" <<'STUB'
#!/usr/bin/env bash
if [ "$1" = run ]; then
 echo judge >> "$TEST_ROOT/calls"
 # Emulate a judge which succeeds if fed an empty ranking: the ranker must stop it first.
 if [[ "$*" == *--rank* ]]; then
   test -s "$TEST_ROOT/target/rank.json" || exit 0
 else
   cat >/dev/null
 fi
 exit "$JUDGE_EXIT"
fi
exit "$GATE_EXIT"
STUB
cat > "$sandbox/bin/deep-diff-forge" <<'STUB'
#!/usr/bin/env bash
cat >/dev/null
if [ "$RANK_EXIT" -ne 0 ]; then exit "$RANK_EXIT"; fi
printf '{}\n'
STUB
for tool in cargo-deny cargo-mutants zizmor; do
 printf '#!/usr/bin/env bash\nexit 0\n' > "$sandbox/bin/$tool"
done
chmod +x "$sandbox/bin/"* "$sandbox/repo/scripts/ci_local.sh"
export PATH="$sandbox/bin:$PATH" TEST_ROOT="$sandbox/repo"
failures=0
for entry in "$sandbox/repo/scripts/ci_local.sh" "$sandbox/pre-push"; do
 for row in 'pass 0 0 0 0' 'fail 0 0 7 7' 'hold 0 0 8 8' 'rank-error 0 9 0 9' 'gate-error 6 0 0 6'; do
  read -r name GATE_EXIT RANK_EXIT JUDGE_EXIT expected <<< "$row"
  export GATE_EXIT RANK_EXIT JUDGE_EXIT
  : > "$TEST_ROOT/calls"
  set +e
  bash "$entry" > "$sandbox/output" 2>&1
  actual=$?
  set -e
  bad=0
  if [ "$actual" -ne "$expected" ]; then bad=1; fi
  if [ "$expected" -ne 0 ] && grep -q 'all available gates passed' "$sandbox/output"; then bad=1; fi
  if [ "$RANK_EXIT" -ne 0 ] && [ -s "$TEST_ROOT/calls" ]; then bad=1; fi
  if [ "$bad" -eq 1 ]; then
   printf 'FAIL %s %s: expected %s, got %s\n' "$(basename "$entry")" "$name" "$expected" "$actual"
   cat "$sandbox/output"
   failures=$((failures + 1))
  else
   printf 'PASS %s %s exit=%s\n' "$(basename "$entry")" "$name" "$actual"
  fi
 done
done
[ "$failures" -eq 0 ]

# Failure of the configuration admission checker must stop the gate before judge execution.
export GATE_EXIT=0 RANK_EXIT=0 JUDGE_EXIT=0 GUARDRAIL_EXIT=1
for entry in "$sandbox/repo/scripts/ci_local.sh" "$sandbox/pre-push"; do
 : > "$TEST_ROOT/calls"
 set +e
 bash "$entry" > "$sandbox/output" 2>&1
 actual=$?
 set -e
 [ "$actual" -eq 1 ] && [ ! -s "$TEST_ROOT/calls" ]
 echo "PASS guardrail admission failure propagates exit=$actual"
done
