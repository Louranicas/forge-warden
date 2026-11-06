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
printf 'trusted-policy\n' > "$trusted/warden.toml"
printf 'trusted-build\n' > "$trusted/build.rs"
git -C "$trusted" init -q
# Do not inherit signing, hooks or identities from the reviewer's environment.
git -C "$trusted" add -- scripts/dogfood.sh warden.toml build.rs
git -C "$trusted" -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm baseline -- scripts/dogfood.sh warden.toml build.rs
base="$(git -C "$trusted" rev-parse HEAD)"
git clone -q --no-local "$trusted" "$sandbox/candidate"
candidate="$sandbox/candidate"
printf 'exit 0 # candidate-controlled driver\n' > "$candidate/scripts/dogfood.sh"
printf 'allow-all\n' > "$candidate/warden.toml"
printf 'malicious-build\n' > "$candidate/build.rs"
git -C "$candidate" -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm candidate -- scripts/dogfood.sh warden.toml build.rs
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
for row in 'pass 0 0 0 0' 'fail 0 0 7 7' 'hold 0 0 8 8' 'rank-error 0 9 0 9' 'build-error 6 0 0 6'; do
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
 if [ "$RANK_EXIT" -ne 0 ] || [ "$BUILD_EXIT" -ne 0 ]; then test ! -e "$out/verdict.json"; fi
 echo "PASS $name exit=$actual"
done
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
