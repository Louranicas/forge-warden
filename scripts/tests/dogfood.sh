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
# The download URL is the driver's own; this test names only its path, never a real service host.
export EXPECTED_PATH="$release/$ddf_name.tar.gz"
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
[[ " $* " == *" --proto =https "* && " $* " == *" --proto-redir =https "* && " $* " == *" --fail "* ]] || exit 91
[[ "${!#}" == https://*/"$EXPECTED_PATH" ]] || exit 92
printf '%s\n' "${!#}" >> "$CURL_LOG"
output=
while [ "$#" -gt 0 ]; do if [ "$1" = --output ]; then output="$2"; fi; shift; done
[ -n "$output" ] || exit 90
[ "$CURL_EXIT" = 0 ] || exit "$CURL_EXIT"
cp -- "$ARCHIVE" "$output"
STUB
chmod +x "$sandbox/bin/cargo" "$sandbox/bin/curl"
export PATH="$sandbox/bin:$PATH" EXPECTED_TRUSTED="$trusted"
# No network even if a plant removes the curl stub: every proxy variable names a closed local port.
export https_proxy=http://127.0.0.1:9 HTTPS_PROXY=http://127.0.0.1:9 http_proxy=http://127.0.0.1:9 \
  ALL_PROXY=http://127.0.0.1:9 all_proxy=http://127.0.0.1:9 NO_PROXY= no_proxy=
# A local run names the archive (M7-DOGFOOD-LOCAL); only --hosted-advisory downloads it.
LOCAL=(--ranker-archive "$FIXTURE_ARCHIVE")
export CURL_LOG="$sandbox/curl.log"
: > "$CURL_LOG"
# Attempt an environment override; trusted script must retain its file-owned producer pin.
export DDF_REV=untrusted-override
export ARCHIVE="$FIXTURE_ARCHIVE" CURL_EXIT=0
for row in 'pass 0 0 0 0' 'fail 0 0 7 7' 'hold 0 0 8 8' 'rank-error 0 9 0 3' 'build-error 6 0 0 6'; do
 read -r name BUILD_EXIT RANK_EXIT JUDGE_EXIT expected <<< "$row"
 export BUILD_EXIT RANK_EXIT JUDGE_EXIT
 out="$sandbox/evidence-$name"
 set +e
 bash "$trusted/scripts/dogfood.sh" "${LOCAL[@]}" "$base" "$head" "$candidate" "$out" > "$sandbox/output" 2>&1
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
bash "$trusted/scripts/dogfood.sh" "${LOCAL[@]}" "$base" "$attr_head" "$attr" "$out" > "$sandbox/output" 2>&1
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
# Byte equality with sha256sum, which the gate's tool set has; cmp and diff are not in it.
same_bytes() { [ "$(sha256sum < "$1")" = "$(sha256sum < "$2")" ]; }
grep -q '^diff --git a/sub/k.rs b/sub/k.rs$' "$clean_patch" || { echo 'FAIL git environment: the clean patch has no a/ b/ prefixes'; exit 1; }
gitenv() {
 name="$1"; shift
 out="$sandbox/evidence-gitenv-$name"
 env "$@" bash "$trusted/scripts/dogfood.sh" "${LOCAL[@]}" "$base" "$attr_head" "$attr" "$out" > "$sandbox/output" 2>&1
 if ! same_bytes "$clean_patch" "$out/change.patch"; then
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
 bash "$trusted/scripts/dogfood.sh" "${LOCAL[@]}" "$base" "$attr_head" "$where" "$out" > "$sandbox/output" 2>&1
 git -C "$attr" config --unset "$key"
 if ! same_bytes "$clean_patch" "$out/change.patch"; then
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
printf '[diff]\n\tnoprefix = true\n\trelative = true\n\tcontext = 0\n\tmnemonicPrefix = true\n\talgorithm = histogram\n[color]\n\tui = always\n[core]\n\tquotePath = false\n\thooksPath = %s/no-hooks\n\tfsmonitor = %s/no-fsmonitor\n' "$sandbox" "$sandbox" > "$sandbox/user-home/.gitconfig"
made() {
 name="$1"; shift
 if ! "$@" > "$sandbox/made.patch" 2> "$sandbox/made.err" || ! same_bytes "$clean_patch" "$sandbox/made.patch"; then
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
# Q58 S1: no repository at the caller's current directory is read; a SHA-256 one there once
# made the empty-tree id the wrong length for the SHA-1 scratch repository.
git init -q --object-format=sha256 "$sandbox/cwd-sha256"
made from-a-sha256-directory bash -c 'cd "$1" && bash "$2" "$3" "$4" "$5"' _ "$sandbox/cwd-sha256" "$maker" "$base" "$attr_head" "$attr"
made hostile-git-dir env GIT_DIR="$sandbox/cwd-sha256/.git" GIT_OBJECT_DIRECTORY="$sandbox/cwd-sha256/.git/objects" GIT_WORK_TREE="$sandbox/cwd-sha256" bash "$maker" "$base" "$attr_head" "$attr"
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
# ADR 0012 D7: several merge bases are refused with bound mode's message, not resolved by `...`.
cc="$sandbox/criss-cross"
git init -q "$cc"
ccgit() { git -C "$cc" -c user.name=Fixture -c user.email=fixture@example.invalid "$@"; }
printf 'root\n' > "$cc/r.txt"; ccgit add r.txt; ccgit commit -qm root
ccgit checkout -qb x; printf 'x\n' > "$cc/x.txt"; ccgit add x.txt; ccgit commit -qm x
ccgit checkout -q master 2>/dev/null || ccgit checkout -q main
ccgit checkout -qb y; printf 'y\n' > "$cc/y.txt"; ccgit add y.txt; ccgit commit -qm y
x="$(ccgit rev-parse x)"; y="$(ccgit rev-parse y)"
ccgit checkout -qb m1 "$x"; ccgit merge -q --no-edit "$y"
ccgit checkout -qb m2 "$y"; ccgit merge -q --no-edit "$x"
if bash "$maker" "$(ccgit rev-parse m1)" "$(ccgit rev-parse m2)" "$cc" > "$sandbox/made.patch" 2> "$sandbox/made.err" \
  || [ -s "$sandbox/made.patch" ] || ! grep -q '2 merge bases' "$sandbox/made.err"; then
  echo "FAIL make_patch: several merge bases were not refused: $(cat "$sandbox/made.err")"
  exit 1
fi
echo 'PASS make_patch refuses several merge bases with the door message'
# Bastion LOW (1591): a failure of Git is not called "no common history". Unrelated histories are
# (merge-base exits 1, silent); a shallow clone lacking the commits between base and head is Git's
# error, shown with its first line and the remedy.
ccgit checkout -q --orphan unrelated; ccgit rm -rqf .; printf 'u\n' > "$cc/u.txt"; ccgit add u.txt; ccgit commit -qm unrelated
if bash "$maker" "$x" "$(ccgit rev-parse unrelated)" "$cc" > "$sandbox/made.patch" 2> "$sandbox/made.err" \
  || [ -s "$sandbox/made.patch" ] || ! grep -q 'no common history' "$sandbox/made.err"; then
  echo "FAIL make_patch: unrelated histories were not refused as such: $(cat "$sandbox/made.err")"
  exit 1
fi
# Only x and y, one commit deep: their common parent is not in the clone.
git clone -q --depth 1 --single-branch --branch x "file://$cc" "$sandbox/shallow" 2> /dev/null
git -C "$sandbox/shallow" fetch -q --depth 1 origin y:refs/remotes/origin/y 2> /dev/null
if bash "$maker" "$(git -C "$sandbox/shallow" rev-parse HEAD)" "$(git -C "$sandbox/shallow" rev-parse origin/y)" "$sandbox/shallow" \
  > "$sandbox/made.patch" 2> "$sandbox/made.err" || [ -s "$sandbox/made.patch" ] \
  || ! grep -q 'git merge-base failed' "$sandbox/made.err" || grep -q 'no common history' "$sandbox/made.err"; then
  echo "FAIL make_patch: a shallow clone's missing history was called something else: $(cat "$sandbox/made.err")"
  exit 1
fi
echo 'PASS make_patch tells unrelated histories from a Git failure (shallow clone)'
# SUBMODULE-PIN (outside review O1): a submodule change always reaches the patch as its own
# `diff --git` section. The candidate's work tree holds the change's own `.gitmodules` with
# `ignore = all`; with that file alone, a `git diff` without the two submodule options leaves the
# gitlink out and the rest of the change is judged without it. The script's diff runs in another
# repository, so its rows pass with or without the options; the by-hand rows are the ones that
# fail without them. They run the option line read from the script, and the README and the
# runbook must print that same line, so the three cannot drift apart.
# M7-WORKTREE-ATTACK W1: the change also commits `.gitattributes` with `*.rs -diff`. Read from the
# work tree, it makes Git write `build.rs` as a binary section the ranker cannot read. The
# documented command takes attributes from the empty tree, as the script does; a control row
# shows the same command without those two options gives other bytes.
git clone -q --no-local "$trusted" "$sandbox/sub-candidate"
sub="$sandbox/sub-candidate"
fixture_commit() { git -C "$sub" -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm "$1"; }
git -C "$sub" update-index --add --cacheinfo "160000,$base,scripts/sub"
fixture_commit link
sub_base="$(git -C "$sub" rev-parse HEAD)"
git -C "$sub" update-index --cacheinfo "160000,$sub_base,scripts/sub"
printf '[submodule "sub"]\n\tpath = scripts/sub\n\turl = ./sub\n\tignore = all\n' > "$sub/.gitmodules"
printf 'trusted-build\nfn beside_the_bump() {}\n' > "$sub/build.rs"
printf '*.rs -diff\n' > "$sub/.gitattributes"
git -C "$sub" add -- .gitmodules build.rs .gitattributes
fixture_commit bump
sub_head="$(git -C "$sub" rev-parse HEAD)"
sub_patch="$sandbox/sub-clean.patch"
bash "$maker" "$sub_base" "$sub_head" "$sub" > "$sub_patch"
if ! grep -q '^diff --git a/scripts/sub b/scripts/sub$' "$sub_patch" || ! grep -q "^-Subproject commit $base\$" "$sub_patch" || ! grep -q "^+Subproject commit $sub_base\$" "$sub_patch"; then
 echo 'FAIL submodule: make_patch left the submodule change out of the patch'
 exit 1
fi
echo 'PASS make_patch writes a submodule change as a 160000 section, with ignore = all in the change'
for setting in diff.ignoreSubmodules=all diff.submodule=log; do
 git -C "$sub" config "${setting%%=*}" "${setting#*=}"
 bash "$maker" "$sub_base" "$sub_head" "$sub" > "$sandbox/made.patch"
 git -C "$sub" config --unset "${setting%%=*}"
 same_bytes "$sub_patch" "$sandbox/made.patch" || { echo "FAIL submodule: candidate config $setting shaped make_patch's patch"; exit 1; }
 echo "PASS make_patch candidate config $setting does not hide the submodule change"
done
by_hand="$(sed -n 's/^  \(diff --no-ext-diff .*\) \\$/\1/p' "$maker")"
if [ "$(printf '%s\n' "$by_hand" | grep -c -- '--dst-prefix=b/')" != 1 ]; then
 echo "FAIL by hand: make_patch.sh has no single line of diff options: $by_hand"
 exit 1
fi
attr_options='--attr-source="$(git hash-object -t tree /dev/null)" -c core.attributesFile=/dev/null'
grep -qF "\`git $attr_options $by_hand <base>...<head>\`" "$source_root/README.md" || { echo "FAIL by hand: README.md does not print: git $attr_options $by_hand <base>...<head>"; exit 1; }
grep -qF "\`git $attr_options $by_hand main...HEAD\`" "$source_root/docs/RUNBOOK.md" || { echo "FAIL by hand: docs/RUNBOOK.md does not print: git $attr_options $by_hand main...HEAD"; exit 1; }
echo 'PASS the README and the runbook print the diff command make_patch.sh runs'
grep -q '^diff --git a/build.rs b/build.rs$' "$sub_patch" && grep -q '^+fn beside_the_bump() {}$' "$sub_patch" || { echo 'FAIL attributes: make_patch did not write build.rs as text'; exit 1; }
empty_tree="$(git -C "$sub" hash-object -t tree /dev/null)"
# shellcheck disable=SC2086
git -C "$sub" $by_hand "$sub_base...$sub_head" > "$sandbox/made.patch"
if same_bytes "$sub_patch" "$sandbox/made.patch" || grep -q '^+fn beside_the_bump() {}$' "$sandbox/made.patch"; then
 echo 'FAIL attributes control: without the attribute options the committed *.rs -diff should hide build.rs'
 exit 1
fi
echo 'PASS attributes control: without the attribute options build.rs becomes a binary section'
by_hand_row() {
 name="$1"; shift
 # The option line is split into words on purpose, as a shell would split the documented command.
 # shellcheck disable=SC2086
 git -C "$sub" --attr-source="$empty_tree" -c core.attributesFile=/dev/null "$@" $by_hand "$sub_base...$sub_head" > "$sandbox/made.patch"
 if ! same_bytes "$sub_patch" "$sandbox/made.patch"; then
  echo "FAIL by hand $name: the documented command does not give make_patch's patch ($(grep -c '^diff --git a/scripts/sub ' "$sandbox/made.patch" || true) submodule sections)"
  exit 1
 fi
 echo "PASS by hand $name: the documented command keeps the submodule change"
}
by_hand_row the-changes-own-gitmodules-ignore-all
by_hand_row ignore-submodules-all -c diff.ignoreSubmodules=all
by_hand_row submodule-log -c diff.submodule=log
by_hand_row submodule-diff -c diff.submodule=diff
# Every local run so far left the curl log empty: none of them downloaded the ranker.
test ! -s "$CURL_LOG" || { cat "$CURL_LOG"; echo 'FAIL a local run called curl'; exit 1; }
# The ranker is the pinned release archive or nothing: other bytes and a failed download both stop
# the run before any ranker exists and before the judge runs, even when the judge would pass.
export BUILD_EXIT=0 RANK_EXIT=0 JUDGE_EXIT=0
for row in "other-bytes $OTHER_ARCHIVE 0 3 local" "download-fails $FIXTURE_ARCHIVE 22 22 hosted"; do
 read -r name ARCHIVE CURL_EXIT expected how <<< "$row"
 export ARCHIVE CURL_EXIT
 out="$sandbox/archive-$name"
 if [ "$how" = local ]; then how=(--ranker-archive "$ARCHIVE"); else how=(--hosted-advisory); fi
 set +e
 bash "$trusted/scripts/dogfood.sh" "${how[@]}" "$base" "$head" "$candidate" "$out" > "$sandbox/output" 2>&1
 actual=$?
 set -e
 if [ "$actual" -ne "$expected" ]; then cat "$sandbox/output"; echo "FAIL archive $name: expected $expected got $actual"; exit 1; fi
 test ! -e "$out/ranker/bin/deep-diff-forge"
 test ! -e "$out/verdict.json"
 echo "PASS archive $name exit=$actual"
done
export ARCHIVE="$FIXTURE_ARCHIVE" CURL_EXIT=0
# M7-DOGFOOD-LOCAL (ledger Q67): a local run never downloads. Without --ranker-archive and without
# --hosted-advisory the driver refuses before any effect; a missing or empty archive argument too.
# A relative archive path works from any directory. (The local runs before the archive rows left
# the curl log empty; the hosted download row wrote it, so it starts empty again here.)
: > "$CURL_LOG"
set +e
bash "$trusted/scripts/dogfood.sh" --hosted-advisory --ranker-archive '' "$base" "$head" "$candidate" "$sandbox/hosted-empty" > "$sandbox/output" 2>&1
actual=$?
set -e
[ "$actual" = 2 ] && test ! -e "$sandbox/hosted-empty" && test ! -s "$CURL_LOG" \
  || { cat "$sandbox/output"; echo "FAIL --hosted-advisory with an empty --ranker-archive: got $actual"; exit 1; }
echo 'PASS an empty --ranker-archive is refused, also with --hosted-advisory (no silent download)'
export BUILD_EXIT=0 RANK_EXIT=0 JUDGE_EXIT=0
for row in 'no-archive' 'missing-file --ranker-archive /nonexistent/archive.tar.gz' 'no-value --ranker-archive' 'empty-value --ranker-archive ='; do
 read -r name flag value <<< "$row"
 args=(); if [ -n "${flag:-}" ]; then args=("$flag"); fi
 if [ "${value:-}" = '=' ]; then args+=(''); elif [ -n "${value:-}" ]; then args+=("$value"); fi
 out="$sandbox/local-$name"
 set +e
 bash "$trusted/scripts/dogfood.sh" "${args[@]}" "$base" "$head" "$candidate" "$out" > "$sandbox/output" 2>&1
 actual=$?
 set -e
 if [ "$actual" -ne 2 ]; then cat "$sandbox/output"; echo "FAIL local $name: expected 2 got $actual"; exit 1; fi
 if [ "$name" = no-archive ]; then grep -q 'a local run takes --ranker-archive' "$sandbox/output"; fi
 case "$name" in empty-value) grep -q "needs a file" "$sandbox/output" ;; no-value|missing-file) grep -q "is not a file" "$sandbox/output" ;; esac
 test ! -e "$out"
 test ! -s "$CURL_LOG"
 echo "PASS local $name refused before any effect"
done
out="$sandbox/local-relative"
( cd "$sandbox" && bash "$trusted/scripts/dogfood.sh" --ranker-archive release.tar.gz "$base" "$head" "$candidate" "$out" ) > "$sandbox/output" 2>&1 \
  || { cat "$sandbox/output"; echo 'FAIL local relative archive path'; exit 1; }
grep -qx 'ranker_archive=local' "$out/provenance.txt"
test ! -s "$CURL_LOG"
echo 'PASS a relative --ranker-archive is the local archive; curl never ran'
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
bash "$trusted/scripts/dogfood.sh" "${LOCAL[@]}" "$base" "$head" "$candidate" "$out" > "$sandbox/output" 2>&1
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
 env "$variable=advisory" "DOGFOOD_ARGS=--hosted-advisory" bash "$trusted/scripts/dogfood.sh" "${LOCAL[@]}" "$base" "$head" "$candidate" "$out" > "$sandbox/output" 2>&1
 actual=$?
 set -e
 [ "$actual" = 0 ] || { cat "$sandbox/output"; echo "FAIL env $variable: got $actual (98 = the judge was asked for advisory mode)"; exit 1; }
 grep -qx 'mode=authority' "$out/provenance.txt"
done
echo 'PASS no environment variable selects advisory mode'
# The argument counts only in first place; anywhere else it is a usage error before any effect.
set +e
bash "$trusted/scripts/dogfood.sh" "${LOCAL[@]}" "$base" "$head" "$candidate" "$sandbox/misplaced" --hosted-advisory > "$sandbox/output" 2>&1
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
 bash "$trusted/scripts/dogfood.sh" "${LOCAL[@]}" "$base" "$head" "$candidate" "$sandbox/ack-$name" \
   "$sandbox/receipt.json" "$sandbox/receipt.sig" > "$sandbox/output" 2>&1
 actual=$?
 set -e
 if [ "$actual" -ne "$expected" ]; then cat "$sandbox/output"; echo "FAIL ack $name"; exit 1; fi
 # Only an accepted acknowledgement leaves acknowledgement.json; a refused one leaves no file.
 if [ "$name" = accepted ]; then test -s "$sandbox/ack-$name/acknowledgement.json"
 elif [ -e "$sandbox/ack-$name/acknowledgement.json" ]; then echo "FAIL ack $name left acknowledgement.json"; exit 1; fi
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
 bash "$advice/scripts/dogfood.sh" "${LOCAL[@]}" "$advice_base" "$advice_head" "$sandbox/advisory-candidate" "$out" "$sandbox/receipt.json" "$sandbox/receipt.sig" > "$sandbox/output" 2>&1
 actual=$?
 set -e
 expected=8; if [ "$RANK_EXIT" != 0 ]; then expected=9; fi
 [ "$actual" = "$expected" ] || { cat "$sandbox/output"; exit 1; }
 test ! -e "$out/acknowledgement.json"
 if [ "$RANK_EXIT" = 0 ]; then grep -q 'mode=advisory' "$out/provenance.txt"; fi
 echo "PASS explicit advisory cannot acknowledge or mask ranker failure: $actual"
done
export EXPECTED_TRUSTED="$trusted"
# Q58 S3 (third read): the trusted checkout given as the candidate, by its path and through a
# symbolic link, and an ignored file in the trusted checkout, each refuse before any effect.
export BUILD_EXIT=0 RANK_EXIT=0 JUDGE_EXIT=0
ln -s "$trusted" "$sandbox/trusted-link"
for row in "same-path $trusted" "same-through-link $sandbox/trusted-link"; do
 read -r name given <<< "$row"
 set +e
 bash "$trusted/scripts/dogfood.sh" "${LOCAL[@]}" "$base" "$head" "$given" "$sandbox/s3-$name" > "$sandbox/output" 2>&1
 actual=$?
 set -e
 [ "$actual" = 2 ] && grep -q 'judge checkout must be the distinct trusted base' "$sandbox/output" && test ! -e "$sandbox/s3-$name" || { cat "$sandbox/output"; echo "FAIL trusted as candidate $name: got $actual"; exit 1; }
 echo "PASS trusted checkout as the candidate ($name) refused before effects"
done
printf 'ignored.txt\n' >> "$trusted/.git/info/exclude"
printf 'ignored\n' > "$trusted/ignored.txt"
test -z "$(git -C "$trusted" status --porcelain --untracked-files=all)"
set +e
bash "$trusted/scripts/dogfood.sh" "${LOCAL[@]}" "$base" "$head" "$candidate" "$sandbox/s3-ignored" > "$sandbox/output" 2>&1
actual=$?
set -e
rm -- "$trusted/ignored.txt"
[ "$actual" = 2 ] && grep -q 'trusted checkout must be clean, including ignored files' "$sandbox/output" && test ! -e "$sandbox/s3-ignored" || { cat "$sandbox/output"; echo "FAIL ignored file in the trusted checkout: got $actual"; exit 1; }
echo 'PASS an ignored file in the trusted checkout refused before effects'
# Diverged history: the trusted base moved on after the candidate forked. The judged change is
# from the merge base to head (base...head), so the patch is the clean one; base..head would also
# undo the base's own new commit.
git clone -q --no-local "$trusted" "$sandbox/moved-trusted"
moved="$sandbox/moved-trusted"
printf 'a later trusted commit\n' > "$moved/later.txt"
git -C "$moved" add -- later.txt
git -C "$moved" -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm later -- later.txt
moved_base="$(git -C "$moved" rev-parse HEAD)"
git -C "$attr" fetch -q "$moved" HEAD
export EXPECTED_TRUSTED="$moved"
out="$sandbox/s3-diverged"
set +e
bash "$moved/scripts/dogfood.sh" "${LOCAL[@]}" "$moved_base" "$attr_head" "$attr" "$out" > "$sandbox/output" 2>&1
set -e
export EXPECTED_TRUSTED="$trusted"
if ! same_bytes "$clean_patch" "$out/change.patch"; then
 echo "FAIL diverged history: the patch is not base...head ($(grep -c '^diff --git' "$out/change.patch" || true) sections)"
 exit 1
fi
echo 'PASS diverged history: the patch is the change from the merge base to head'
# Candidate cannot be its own trusted checkout; modified trusted files refuse before any build.
set +e
bash "$trusted/scripts/dogfood.sh" "${LOCAL[@]}" "$head" "$head" "$candidate" "$sandbox/wrong-base" >/dev/null 2>&1
wrong=$?
printf 'dirty\n' >> "$trusted/warden.toml"
bash "$trusted/scripts/dogfood.sh" "${LOCAL[@]}" "$base" "$head" "$candidate" "$sandbox/dirty" >/dev/null 2>&1
dirty=$?
set -e
[ "$wrong" = 2 ] && [ "$dirty" = 2 ]
test ! -e "$sandbox/wrong-base"
test ! -e "$sandbox/dirty"
echo 'PASS wrong-base and dirty trusted checkout refused before effects'
