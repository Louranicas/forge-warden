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
git -C "$trusted" add -- scripts/dogfood.sh scripts/hold_ack.py scripts/judge_mode.py scripts/tool_versions.py scripts/tool-versions.toml rust-toolchain.toml Cargo.toml warden.toml build.rs
git -C "$trusted" -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm baseline -- scripts/dogfood.sh scripts/hold_ack.py scripts/judge_mode.py scripts/tool_versions.py scripts/tool-versions.toml rust-toolchain.toml Cargo.toml warden.toml build.rs
base="$(git -C "$trusted" rev-parse HEAD)"
git clone -q --no-local "$trusted" "$sandbox/candidate"
candidate="$sandbox/candidate"
printf 'exit 0 # candidate-controlled driver\n' > "$candidate/scripts/dogfood.sh"
printf 'allow-all\n' > "$candidate/warden.toml"
printf 'malicious-build\n' > "$candidate/build.rs"
git -C "$candidate" -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm candidate -- scripts/dogfood.sh scripts/hold_ack.py scripts/judge_mode.py scripts/tool_versions.py scripts/tool-versions.toml rust-toolchain.toml Cargo.toml warden.toml build.rs
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
