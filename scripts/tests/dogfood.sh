#!/usr/bin/env bash
# Isolated real Git subjects with stubbed compiler/producer: no builds or network.
set -euo pipefail
source_root="$(cd "$(dirname "$0")/../.." && pwd)"
sandbox="$(mktemp -d)"
trap 'rm -rf "$sandbox"' EXIT
export HOME="$sandbox/home" GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null
mkdir -p "$HOME" "$sandbox/trusted/scripts" "$sandbox/bin"
trusted="$sandbox/trusted"
cp "$source_root/scripts/dogfood.sh" "$trusted/scripts/dogfood.sh"
cp "$source_root/scripts/judge_mode.py" "$trusted/scripts/judge_mode.py"
cat > "$trusted/scripts/hold_ack.py" <<'ACK'
import os, sys
assert sys.argv[1:4] == ["verify", "--base", os.environ["ACK_BASE"]]
assert sys.argv[4:6] == ["--head", os.environ["ACK_HEAD"]]
print('{"fixture_ack":true}')
sys.exit(int(os.environ["ACK_EXIT"]))
ACK
printf 'schema = "forge-warden.policy.v0"\n[producer]\nsha256 = ["0000000000000000000000000000000000000000000000000000000000000000"]\n' > "$trusted/warden.toml"
printf 'trusted-build\n' > "$trusted/build.rs"
git -C "$trusted" init -q
# Do not inherit signing, hooks or identities from the reviewer's environment.
git -C "$trusted" add -- scripts/dogfood.sh scripts/hold_ack.py scripts/judge_mode.py warden.toml build.rs
git -C "$trusted" -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm baseline -- scripts/dogfood.sh scripts/hold_ack.py scripts/judge_mode.py warden.toml build.rs
base="$(git -C "$trusted" rev-parse HEAD)"
git clone -q --no-local "$trusted" "$sandbox/candidate"
candidate="$sandbox/candidate"
printf 'exit 0 # candidate-controlled driver\n' > "$candidate/scripts/dogfood.sh"
printf 'allow-all\n' > "$candidate/warden.toml"
printf 'malicious-build\n' > "$candidate/build.rs"
git -C "$candidate" -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm candidate -- scripts/dogfood.sh scripts/hold_ack.py scripts/judge_mode.py warden.toml build.rs
head="$(git -C "$candidate" rev-parse HEAD)"
cat > "$sandbox/bin/cargo" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
[ "$PWD" = "$EXPECTED_TRUSTED" ] || exit 93
[ "$(cat build.rs)" = trusted-build ] || exit 94
case "$1" in
 build)
  [ "$BUILD_EXIT" = 0 ] || exit "$BUILD_EXIT"
  mkdir -p "$CARGO_TARGET_DIR/release"
  cat > "$CARGO_TARGET_DIR/release/forge-warden" <<'JUDGE'
#!/usr/bin/env bash
case " $* " in
 *' --ranker '*) [ "$RANK_EXIT" = 0 ] || exit 3 ;;
 *' --advisory '*) [[ " $* " == *' --rank '* ]] || exit 97 ;;
 *) echo 'missing explicit judge mode' >&2; exit 97 ;;
esac
printf '{"fixture":true}\n'
exit "$JUDGE_EXIT"
JUDGE
  chmod +x "$CARGO_TARGET_DIR/release/forge-warden" ;;
 install)
  [[ "$*" == *'--rev 587a6afd559a4bd9f8c0aabcef4c1e5de85ffc54 '* ]] || exit 95
  while [ "$1" != --root ]; do shift; done
  mkdir -p "$2/bin"
  cat > "$2/bin/deep-diff-forge" <<'RANK'
#!/usr/bin/env bash
cat >/dev/null
printf '{}\n'
exit "$RANK_EXIT"
RANK
  chmod +x "$2/bin/deep-diff-forge" ;;
 *) exit 96 ;;
esac
STUB
chmod +x "$sandbox/bin/cargo"
export PATH="$sandbox/bin:$PATH" EXPECTED_TRUSTED="$trusted"
# Attempt an environment override; trusted script must retain its literal producer pin.
export DDF_REV=untrusted-override
for row in 'pass 0 0 0 0' 'fail 0 0 7 7' 'hold 0 0 8 8' 'rank-error 0 9 0 3' 'build-error 6 0 0 6'; do
 read -r name BUILD_EXIT RANK_EXIT JUDGE_EXIT expected <<< "$row"
 export BUILD_EXIT RANK_EXIT JUDGE_EXIT
 out="$sandbox/evidence-$name"
 set +e
 bash "$trusted/scripts/dogfood.sh" "$base" "$head" "$candidate" "$out" > "$sandbox/output" 2>&1
 actual=$?
 set -e
 if [ "$actual" -ne "$expected" ]; then cat "$sandbox/output"; echo "FAIL $name: expected $expected got $actual"; exit 1; fi
 if [ "$name" = pass ]; then
  test "$(sha256sum < "$out/policy.toml")" = "$(sha256sum < "$trusted/warden.toml")"
  grep -q malicious-build "$out/change.patch"
  grep -q "head=$head" "$out/provenance.txt"
 fi
 if [ "$RANK_EXIT" -ne 0 ] || [ "$BUILD_EXIT" -ne 0 ]; then test ! -s "$out/verdict.json"; fi
 echo "PASS $name exit=$actual"
done
# Receipt verification is independently tested with real signatures in hold_ack.py.
# Here a stub proves the driver admits only successful verification of a HOLD.
export ACK_BASE="$base" ACK_HEAD="$head" BUILD_EXIT=0 RANK_EXIT=0
for row in 'accepted 8 0 0' 'stale 8 8 8' 'broken-verifier 8 3 8' 'fail-with-ack 7 0 7'; do
 read -r name JUDGE_EXIT ACK_EXIT expected <<< "$row"
 export JUDGE_EXIT ACK_EXIT
 set +e
 bash "$trusted/scripts/dogfood.sh" "$base" "$head" "$candidate" "$sandbox/ack-$name" \
   "$sandbox/receipt.json" "$sandbox/receipt.sig" > "$sandbox/output" 2>&1
 actual=$?
 set -e
 if [ "$actual" -ne "$expected" ]; then cat "$sandbox/output"; echo "FAIL ack $name"; exit 1; fi
 echo "PASS ack $name exit=$actual"
done
# A source revision is not a binary hash enrollment. Without a producer section,
# advisory remains HOLD and even a supplied signed receipt cannot turn it into PASS.
git clone -q --no-local "$trusted" "$sandbox/advisory-trusted"
advice="$sandbox/advisory-trusted"
printf 'schema = "forge-warden.policy.v0"\n' > "$advice/warden.toml"
git -C "$advice" -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm advisory -- warden.toml
advice_base="$(git -C "$advice" rev-parse HEAD)"
git clone -q --no-local "$advice" "$sandbox/advisory-candidate"
printf 'changed\n' > "$sandbox/advisory-candidate/build.rs"
git -C "$sandbox/advisory-candidate" -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm change -- build.rs
advice_head="$(git -C "$sandbox/advisory-candidate" rev-parse HEAD)"
export EXPECTED_TRUSTED="$advice" BUILD_EXIT=0 JUDGE_EXIT=8 ACK_EXIT=0
for RANK_EXIT in 0 9; do
 export RANK_EXIT
 out="$sandbox/advisory-$RANK_EXIT"
 set +e
 bash "$advice/scripts/dogfood.sh" "$advice_base" "$advice_head" "$sandbox/advisory-candidate" "$out" "$sandbox/receipt.json" "$sandbox/receipt.sig" > "$sandbox/output" 2>&1
 actual=$?
 set -e
 expected=8; if [ "$RANK_EXIT" != 0 ]; then expected=9; fi
 [ "$actual" = "$expected" ] || { cat "$sandbox/output"; exit 1; }
 test ! -e "$out/acknowledgement.json"
 if [ "$RANK_EXIT" = 0 ]; then grep -q 'mode=advisory' "$out/provenance.txt"; fi
 echo "PASS explicit advisory cannot acknowledge or mask ranker failure: $actual"
done
export EXPECTED_TRUSTED="$trusted"
# Candidate cannot be its own trusted checkout; modified trusted files refuse before any build.
set +e
bash "$trusted/scripts/dogfood.sh" "$head" "$head" "$candidate" "$sandbox/wrong-base" >/dev/null 2>&1
wrong=$?
printf 'dirty\n' >> "$trusted/warden.toml"
bash "$trusted/scripts/dogfood.sh" "$base" "$head" "$candidate" "$sandbox/dirty" >/dev/null 2>&1
dirty=$?
set -e
[ "$wrong" = 2 ] && [ "$dirty" = 2 ]
test ! -e "$sandbox/wrong-base"
test ! -e "$sandbox/dirty"
echo 'PASS wrong-base and dirty trusted checkout refused before effects'
