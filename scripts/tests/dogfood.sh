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
cp "$source_root/scripts/make_patch.sh" "$trusted/scripts/make_patch.sh"
cp "$source_root/scripts/judge_mode.py" "$trusted/scripts/judge_mode.py"
cp "$source_root/scripts/tool_versions.py" "$source_root/scripts/tool-versions.toml" "$trusted/scripts/"
cp "$source_root/rust-toolchain.toml" "$source_root/Cargo.toml" "$trusted/"
# The release archive the driver fetches is a fixture: one stub ranker under the release's own
# directory name. Its sha256 replaces the pinned one in the trusted checkout's tool-versions.toml,
# so the driver's own check runs against real bytes.
release="$(python3 -I "$trusted/scripts/tool_versions.py" ddf_release)"
ddf_name="deep-diff-forge-$release-x86_64-unknown-linux-gnu"
mkdir -p "$sandbox/archive/$ddf_name"
cat > "$sandbox/archive/$ddf_name/deep-diff-forge" <<'RANK'
#!/usr/bin/env bash
cat >/dev/null
printf '{}\n'
exit "$RANK_EXIT"
RANK
chmod +x "$sandbox/archive/$ddf_name/deep-diff-forge"
tar --create --gzip --file "$sandbox/release.tar.gz" --directory "$sandbox/archive" "$ddf_name"
printf 'not the release\n' > "$sandbox/other.tar.gz"
pinned="$(python3 -I "$trusted/scripts/tool_versions.py" ddf_archive_sha256)"
fixture="$(sha256sum < "$sandbox/release.tar.gz" | cut -d' ' -f1)"
sed -i "s/$pinned/$fixture/" "$trusted/scripts/tool-versions.toml"
test "$(python3 -I "$trusted/scripts/tool_versions.py" ddf_archive_sha256)" = "$fixture"
export EXPECTED_URL="https://github.com/Louranicas/deep-diff-forge/releases/download/$release/$ddf_name.tar.gz"
export FIXTURE_ARCHIVE="$sandbox/release.tar.gz" OTHER_ARCHIVE="$sandbox/other.tar.gz"
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
git -C "$trusted" add -- scripts/dogfood.sh scripts/make_patch.sh scripts/hold_ack.py scripts/judge_mode.py scripts/tool_versions.py scripts/tool-versions.toml rust-toolchain.toml Cargo.toml warden.toml build.rs
git -C "$trusted" -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm baseline -- scripts/dogfood.sh scripts/make_patch.sh scripts/hold_ack.py scripts/judge_mode.py scripts/tool_versions.py scripts/tool-versions.toml rust-toolchain.toml Cargo.toml warden.toml build.rs
base="$(git -C "$trusted" rev-parse HEAD)"
git clone -q --no-local "$trusted" "$sandbox/candidate"
candidate="$sandbox/candidate"
printf 'exit 0 # candidate-controlled driver\n' > "$candidate/scripts/dogfood.sh"
printf 'allow-all\n' > "$candidate/warden.toml"
printf 'malicious-build\n' > "$candidate/build.rs"
git -C "$candidate" -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm candidate -- scripts/dogfood.sh scripts/make_patch.sh scripts/hold_ack.py scripts/judge_mode.py scripts/tool_versions.py scripts/tool-versions.toml rust-toolchain.toml Cargo.toml warden.toml build.rs
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
 *' --ranker '*)
  [ "${EXPECT_MODE:-authority}" = authority ] || exit 98
  # As the real judge does without an owner-enrolled Git allowlist: exit 3, no verdict.
  if [ "${ALLOWLIST:-enrolled}" != enrolled ]; then echo 'Git admission: no Git allowlist enrolled' >&2; exit 3; fi
  [ "$RANK_EXIT" = 0 ] || exit 3 ;;
 *' --advisory '*)
  [ "${EXPECT_MODE:-advisory}" = advisory ] || exit 98
  [[ " $* " == *' --rank '* ]] || exit 97 ;;
 *) echo 'missing explicit judge mode' >&2; exit 97 ;;
esac
printf '{"fixture":true}\n'
exit "$JUDGE_EXIT"
JUDGE
  chmod +x "$CARGO_TARGET_DIR/release/forge-warden" ;;
 *) exit 96 ;; # the ranker is never built here: `cargo install` is refused
esac
STUB
# curl: only the pinned release URL over https, written where the driver asked; never the network.
cat > "$sandbox/bin/curl" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
[[ " $* " == *" --proto =https "* && " $* " == *" --fail "* ]] || exit 91
[ "${!#}" = "$EXPECTED_URL" ] || exit 92
output=
while [ "$#" -gt 0 ]; do if [ "$1" = --output ]; then output="$2"; fi; shift; done
[ -n "$output" ] || exit 90
[ "$CURL_EXIT" = 0 ] || exit "$CURL_EXIT"
cp -- "$ARCHIVE" "$output"
STUB
chmod +x "$sandbox/bin/cargo" "$sandbox/bin/curl"
export PATH="$sandbox/bin:$PATH" EXPECTED_TRUSTED="$trusted"
# Attempt an environment override; trusted script must retain its file-owned producer pin.
export DDF_REV=untrusted-override
export ARCHIVE="$FIXTURE_ARCHIVE" CURL_EXIT=0
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
# DOGFOOD-ATTR (C2): the candidate's own attributes do not shape the patch. A candidate that adds
# `* -diff` at the root and `binary` in a subdirectory, with a secret-shaped added line, must still
# reach the judge as readable added text, not as `GIT binary patch` hunks.
git clone -q --no-local "$trusted" "$sandbox/attr-candidate"
attr="$sandbox/attr-candidate"
mkdir -p "$attr/sub"
printf '* -diff\n' > "$attr/.gitattributes"
printf '* binary\n' > "$attr/sub/.gitattributes"
key="AKIA$(printf 'ABCDEFGHIJKLMNOP')"
printf 'trusted-build\nfn planted() { let k = "%s"; }\n' "$key" > "$attr/build.rs"
printf 'fn sub() { let k = "%s"; }\n' "$key" > "$attr/sub/k.rs"
# A name Git quotes by default, for the core.quotePath row below.
printf 'fn accent() {}\n' > "$attr/sub/é.rs"
git -C "$attr" add -- .gitattributes sub/.gitattributes sub/k.rs sub/é.rs build.rs
git -C "$attr" -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm attributes -- .gitattributes sub/.gitattributes sub/k.rs sub/é.rs build.rs
attr_head="$(git -C "$attr" rev-parse HEAD)"
export BUILD_EXIT=0 RANK_EXIT=0 JUDGE_EXIT=0
out="$sandbox/evidence-attributes"
bash "$trusted/scripts/dogfood.sh" "$base" "$attr_head" "$attr" "$out" > "$sandbox/output" 2>&1
visible="$(grep -c "^+.*$key" "$out/change.patch" || true)"
binary="$(grep -c '^GIT binary patch' "$out/change.patch" || true)"
if [ "$visible" != 2 ] || [ "$binary" != 0 ]; then
 echo "FAIL attributes: the candidate's .gitattributes shaped the patch ($visible of 2 added lines readable, $binary binary patches)"
 exit 1
fi
echo 'PASS candidate .gitattributes (root -diff, subdirectory binary) do not hide added text'
# DOGFOOD-GITENV (freeze review N3): the caller's Git environment and the candidate repository's
# own configuration do not shape the patch. Each case must give the bytes of the clean run above.
# Without the `a/` `b/` prefixes the judge reads `sub/k.rs` as `k.rs`, and no path rule matches.
clean_patch="$out/change.patch"
grep -q '^diff --git a/sub/k.rs b/sub/k.rs$' "$clean_patch" || { echo 'FAIL git environment: the clean patch has no a/ b/ prefixes'; exit 1; }
gitenv() {
 name="$1"; shift
 out="$sandbox/evidence-gitenv-$name"
 env "$@" bash "$trusted/scripts/dogfood.sh" "$base" "$attr_head" "$attr" "$out" > "$sandbox/output" 2>&1
 if ! cmp -s "$clean_patch" "$out/change.patch"; then
  echo "FAIL git environment $name: the patch differs from the clean one: $(sed -n 1p "$out/change.patch")"
  exit 1
 fi
 echo "PASS git environment $name does not shape the patch"
}
gitenv config-count-noprefix GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=diff.noprefix GIT_CONFIG_VALUE_0=true
gitenv config-parameters-noprefix "GIT_CONFIG_PARAMETERS='diff.noprefix=true'"
gitenv config-count-prefixes GIT_CONFIG_COUNT=2 GIT_CONFIG_KEY_0=diff.srcPrefix GIT_CONFIG_VALUE_0=a/x/ GIT_CONFIG_KEY_1=diff.dstPrefix GIT_CONFIG_VALUE_1=b/x/
gitenv config-count-context GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=diff.context GIT_CONFIG_VALUE_0=0
gitenv diff-opts GIT_DIFF_OPTS=--unified=0
# The candidate checkout's own .git/config, one setting at a time (Bastion R1; Touchstone's
# diff.relative finding). The last row also hands the driver a subdirectory of the candidate.
grep -q '^diff --git "a/sub/\\303\\251.rs" "b/sub/\\303\\251.rs"$' "$clean_patch" || { echo 'FAIL git environment: the clean patch does not quote the non-ASCII name'; exit 1; }
candidate_config() {
 name="$1"; key="$2"; value="$3"; where="${4:-$attr}"
 git -C "$attr" config "$key" "$value"
 out="$sandbox/evidence-gitenv-$name"
 bash "$trusted/scripts/dogfood.sh" "$base" "$attr_head" "$where" "$out" > "$sandbox/output" 2>&1
 git -C "$attr" config --unset "$key"
 if ! cmp -s "$clean_patch" "$out/change.patch"; then
  echo "FAIL candidate config $name: the patch differs from the clean one: $(sed -n 1p "$out/change.patch" | cat -v)"
  exit 1
 fi
 echo "PASS candidate config $name does not shape the patch"
}
candidate_config noprefix diff.noprefix true
candidate_config colour color.ui always
candidate_config quotepath core.quotePath false
candidate_config relative diff.relative true
candidate_config context diff.context 0
candidate_config blank-context diff.suppressBlankEmpty true
candidate_config algorithm diff.algorithm histogram
candidate_config order-file diff.orderFile "$sandbox/output"
candidate_config relative-from-a-subdirectory diff.relative true "$attr/sub"
# MAKE-PATCH: the patch-maker used by hand, as the runbook documents it. The same bytes as the
# driver's clean patch, whatever the user's own Git configuration, environment and directory are.
maker="$trusted/scripts/make_patch.sh"
mkdir -p "$sandbox/user-home" "$sandbox/maker-tmp"
printf '[diff]\n\tnoprefix = true\n\trelative = true\n\tcontext = 0\n\tmnemonicPrefix = true\n\talgorithm = histogram\n[color]\n\tui = always\n[core]\n\tquotePath = false\n' > "$sandbox/user-home/.gitconfig"
made() {
 name="$1"; shift
 if ! "$@" > "$sandbox/made.patch" 2> "$sandbox/made.err" || ! cmp -s "$clean_patch" "$sandbox/made.patch"; then
  echo "FAIL make_patch $name: not the clean patch: $(sed -n 1p "$sandbox/made.patch" | cat -v) $(sed -n 1p "$sandbox/made.err")"
  exit 1
 fi
 echo "PASS make_patch $name gives the clean patch"
}
made with-ids bash "$maker" "$base" "$attr_head" "$attr"
made hostile-user-config env -u GIT_CONFIG_GLOBAL -u GIT_CONFIG_NOSYSTEM HOME="$sandbox/user-home" bash "$maker" "$base" "$attr_head" "$attr"
made hostile-environment env GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=diff.noprefix GIT_CONFIG_VALUE_0=true GIT_DIFF_OPTS=--unified=0 GIT_EXTERNAL_DIFF=false bash "$maker" "$base" "$attr_head" "$attr"
git -C "$attr" branch maker-base "$base"
git -C "$attr" branch maker-head "$attr_head"
made with-branch-names bash "$maker" maker-base maker-head "$attr"
made from-a-subdirectory-with-relative bash -c 'cd "$1/sub" && env -u GIT_CONFIG_GLOBAL -u GIT_CONFIG_NOSYSTEM HOME="$2" bash "$3" "$4" "$5"' _ "$attr" "$sandbox/user-home" "$maker" "$base" "$attr_head"
made leaves-no-scratch env TMPDIR="$sandbox/maker-tmp" bash "$maker" "$base" "$attr_head" "$attr"
if [ -n "$(ls -A "$sandbox/maker-tmp")" ]; then echo 'FAIL make_patch: scratch repository left behind'; exit 1; fi
for bad in "nosuchrev $attr_head" "$base nosuchrev" "$base" "$base $attr_head $attr extra" "--output=$sandbox/x $attr_head"; do
 # shellcheck disable=SC2086
 if bash "$maker" $bad > "$sandbox/made.patch" 2> "$sandbox/made.err" || [ -s "$sandbox/made.patch" ]; then
  echo "FAIL make_patch refuses: '$bad' gave a patch or exit 0"
  exit 1
 fi
done
[ ! -e "$sandbox/x" ] || { echo 'FAIL make_patch: an option was taken as a revision'; exit 1; }
echo 'PASS make_patch refuses a missing revision, a wrong argument count and an option in place of a revision'
# The ranker is the pinned release archive or nothing: other bytes and a failed download both stop
# the run before any ranker exists and before the judge runs, even when the judge would pass.
export BUILD_EXIT=0 RANK_EXIT=0 JUDGE_EXIT=0
for row in "other-bytes $OTHER_ARCHIVE 0 3" "download-fails $FIXTURE_ARCHIVE 22 22"; do
 read -r name ARCHIVE CURL_EXIT expected <<< "$row"
 export ARCHIVE CURL_EXIT
 out="$sandbox/archive-$name"
 set +e
 bash "$trusted/scripts/dogfood.sh" "$base" "$head" "$candidate" "$out" > "$sandbox/output" 2>&1
 actual=$?
 set -e
 if [ "$actual" -ne "$expected" ]; then cat "$sandbox/output"; echo "FAIL archive $name: expected $expected got $actual"; exit 1; fi
 test ! -e "$out/ranker/bin/deep-diff-forge"
 test ! -e "$out/verdict.json"
 echo "PASS archive $name exit=$actual"
done
export ARCHIVE="$FIXTURE_ARCHIVE" CURL_EXIT=0
# DOGFOOD-HOSTED-MODE. The trusted policy here pins a producer. (1) Asked with --hosted-advisory,
# the driver judges in advisory mode (the stub judge exits 98 if it is given --ranker), says why,
# still fetches the pinned ranker, and cannot be acknowledged into a pass. (2) Without the
# argument a pinned policy means authority mode, and a missing allowlist is the judge's exit 3 with
# no verdict: no silent downgrade. (3) Nothing in the environment selects advisory mode.
export BUILD_EXIT=0 RANK_EXIT=0 ACK_BASE="$base" ACK_HEAD="$head" ACK_EXIT=0
for row in 'hold 8 8' 'judge-zero 0 0'; do
 read -r name JUDGE_EXIT expected <<< "$row"
 export JUDGE_EXIT EXPECT_MODE=advisory
 out="$sandbox/hosted-$name"
 set +e
 bash "$trusted/scripts/dogfood.sh" --hosted-advisory "$base" "$head" "$candidate" "$out" \
   "$sandbox/receipt.json" "$sandbox/receipt.sig" > "$sandbox/output" 2>&1
 actual=$?
 set -e
 if [ "$actual" -ne "$expected" ]; then cat "$sandbox/output"; echo "FAIL hosted $name: expected $expected got $actual"; exit 1; fi
 grep -q 'explicit advisory mode (--hosted-advisory: a hosted runner has no owner-enrolled Git allowlist); this result cannot PASS' "$sandbox/output"
 grep -qx 'mode=advisory' "$out/provenance.txt"
 grep -q '^advisory_reason=--hosted-advisory' "$out/provenance.txt"
 test -x "$out/ranker/bin/deep-diff-forge"
 test -s "$out/rank.json"
 test ! -e "$out/acknowledgement.json"
 echo "PASS hosted advisory $name exit=$actual"
done
export JUDGE_EXIT=0 EXPECT_MODE=authority ALLOWLIST=absent
out="$sandbox/no-allowlist"
set +e
bash "$trusted/scripts/dogfood.sh" "$base" "$head" "$candidate" "$out" > "$sandbox/output" 2>&1
actual=$?
set -e
[ "$actual" = 3 ] || { cat "$sandbox/output"; echo "FAIL no-allowlist: got $actual"; exit 1; }
grep -qx 'mode=authority' "$out/provenance.txt"
test ! -s "$out/verdict.json"
echo 'PASS a pinned policy without the argument is authority mode: a missing allowlist is exit 3'
export ALLOWLIST=enrolled
for variable in HOSTED_ADVISORY DOGFOOD_MODE DOGFOOD_HOSTED_ADVISORY FORGE_WARDEN_MODE hosted_advisory mode; do
 out="$sandbox/env-$variable"
 set +e
 env "$variable=advisory" "DOGFOOD_ARGS=--hosted-advisory" bash "$trusted/scripts/dogfood.sh" "$base" "$head" "$candidate" "$out" > "$sandbox/output" 2>&1
 actual=$?
 set -e
 [ "$actual" = 0 ] || { cat "$sandbox/output"; echo "FAIL env $variable: got $actual (98 = the judge was asked for advisory mode)"; exit 1; }
 grep -qx 'mode=authority' "$out/provenance.txt"
done
echo 'PASS no environment variable selects advisory mode'
# The argument counts only in first place; anywhere else it is a usage error before any effect.
set +e
bash "$trusted/scripts/dogfood.sh" "$base" "$head" "$candidate" "$sandbox/misplaced" --hosted-advisory > "$sandbox/output" 2>&1
misplaced=$?
set -e
[ "$misplaced" = 2 ] && test ! -e "$sandbox/misplaced"
echo 'PASS a misplaced --hosted-advisory is a usage error'
unset EXPECT_MODE
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
