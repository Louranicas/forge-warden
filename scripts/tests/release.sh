#!/usr/bin/env bash
# Tests for scripts/release_prepare.sh and scripts/release_tag.sh (M7-RELEASE-SCRIPT). No network: the
# three services are local bare repositories, the keys are throwaway keys made here, the repository is a
# small synthetic one carrying the files a release states its version in. Commands: the gate image's.
set -uo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
S="$(mktemp -d)"; trap 'rm -rf -- "$S"' EXIT
fails=0
pass() { echo "PASS $1"; }
fail() { echo "FAIL $1"; fails=$((fails + 1)); }
export HOME="$S/home"; mkdir -p "$HOME"
export GIT_CONFIG_NOSYSTEM=1
v=9.8.7

g() { git -c user.name=t -c user.email=t@example.invalid -c commit.gpgsign=false -c init.defaultBranch=main "$@"; }
ssh-keygen -q -t ed25519 -N '' -C release-a -f "$S/key-a" </dev/null
ssh-keygen -q -t ed25519 -N '' -C release-b -f "$S/key-b" </dev/null

# A fresh synthetic repository in $W, its main pushed to three bare remotes. $1 = name.
fresh() {
  W="$S/$1"; R="$S/$1-remotes"
  mkdir -p "$W/scripts" "$W/docs" "$W/.github/release-signers" "$R"
  for r in gitlab gitea forgejo; do g init -q --bare "$R/$r.git"; done
  cp "$root/scripts/release_prepare.sh" "$root/scripts/release_tag.sh" "$W/scripts/"
  cp "$root/.github/release-signers/allowed_signers" "$W/.github/release-signers/"
  printf '[workspace]\nmembers = []\n\n[workspace.package]\nversion = "%s"\nedition = "2024"\n' "$v" > "$W/Cargo.toml"
  printf '[[package]]\nname = "forge-warden-cli"\nversion = "%s"\n\n[[package]]\nname = "forge-warden-core"\nversion = "%s"\n' "$v" "$v" > "$W/Cargo.lock"
  printf '# Changelog\n\n## [Unreleased]\n\n## [%s] - 2026-11-16\n\n- one change\n\n## [0.0.1] - 2026-01-01\n\n- old\n' "$v" > "$W/CHANGELOG.md"
  printf 'cargo install --git https://gitlab.com/x/forge-warden --tag v%s forge-warden-cli --locked\n' "$v" > "$W/README.md"
  printf '# Evidence\n\n## %s (2026-11-16)\n\nmeasured\n' "$v" > "$W/EVIDENCE.md"
  printf '# Release %s\n' "$v" > "$W/docs/RELEASE-$v.md"
  printf '/dist/\n' > "$W/.gitignore"
  g -C "$W" init -q
  g -C "$W" config user.name 'Release Owner'; g -C "$W" config user.email owner@example.invalid
  for r in gitlab gitea forgejo; do g -C "$W" remote add "$r" "$R/$r.git"; done
  g -C "$W" add -A; g -C "$W" commit -q -m base
  publish
}
publish() { for r in gitlab gitea forgejo; do g -C "$W" push -q "$r" HEAD:refs/heads/main; done; }
commit_all() { g -C "$W" add -A; g -C "$W" commit -q -m "$1"; publish; }
prepare() { (cd "$W" && bash scripts/release_prepare.sh "$@") > "$S/out" 2>&1; }
tagit() { (cd "$W" && FW_TAG_KEY="$1" bash scripts/release_tag.sh "$v" "$(git -C "$W" rev-parse HEAD)") > "$S/out" 2>&1; }
refused() { # name, expected message part
  if [ "$rc" -ne 0 ] && grep -q -- "$2" "$S/out"; then pass "$1"; else fail "$1: rc=$rc $(tail -1 "$S/out")"; fi; }
remote_has_tag() { git --git-dir="$R/$1.git" rev-parse -q --verify "refs/tags/v$v" >/dev/null; }

# prepare: the happy path, checksums, and a second run gives the same bytes
fresh ok; prepare "$v"; rc=$?
if [ "$rc" -eq 0 ] && (cd "$W/dist/$v" && sha256sum -c --quiet SHA256SUMS) \
  && grep -q "scripts/release_tag.sh $v $(git -C "$W" rev-parse HEAD)" "$S/out" \
  && grep -q '^- one change$' "$W/dist/$v/NOTES.md" && ! grep -q 'old' "$W/dist/$v/NOTES.md"; then
  pass "prepare writes archive, notes and checksums and prints the owner's command"
else fail "prepare happy path: rc=$rc $(tail -2 "$S/out")"; fi
first="$(cat "$W/dist/$v/SHA256SUMS")"; rm -r -- "$W/dist"; prepare "$v"; rc=$?
[ "$rc" -eq 0 ] && [ "$(cat "$W/dist/$v/SHA256SUMS")" = "$first" ] && pass "a second prepare gives the same archive bytes" || fail "archive not repeatable"
for r in gitlab gitea forgejo; do remote_has_tag "$r" && fail "prepare pushed a tag to $r"; done
git -C "$W" rev-parse -q --verify "refs/tags/v$v" >/dev/null && fail "prepare created a tag" || pass "prepare creates and pushes no tag"
prepare "$v"; rc=$?; refused "prepare refuses an existing dist directory" "already exists"
prepare 9.8; rc=$?; [ "$rc" -eq 2 ] && pass "prepare refuses a version that is not MAJOR.MINOR.PATCH" || fail "bad version rc=$rc"

# prepare refusals, each on a fresh repository with main published
fresh cargo; sed -i "s/^version = \"$v\"/version = \"9.8.6\"/" "$W/Cargo.toml"; commit_all c; prepare "$v"; rc=$?; refused "workspace version differs" "Cargo.toml workspace version"
fresh lock; sed -i '0,/^version = /s//version = "1.0.0"\n#/' "$W/Cargo.lock"; commit_all c; prepare "$v"; rc=$?; refused "Cargo.lock version differs" "Cargo.lock has"
fresh log1; sed -i "s/^## \[$v\]/## [9.8.6]/" "$W/CHANGELOG.md"; commit_all c; prepare "$v"; rc=$?; refused "CHANGELOG has no section for the version" "CHANGELOG.md"
fresh log2; sed -i 's/^## \[Unreleased\]$/## [Unreleased]\n\n- not released yet/' "$W/CHANGELOG.md"; commit_all c; prepare "$v"; rc=$?; refused "Unreleased is not empty" "CHANGELOG.md"
fresh log3; sed -i "s/^## \[$v\] - .*/## [$v] - soon/" "$W/CHANGELOG.md"; commit_all c; prepare "$v"; rc=$?; refused "the section has no date" "CHANGELOG.md"
fresh readme; sed -i "s/--tag v$v/--tag v9.8.6/" "$W/README.md"; commit_all c; prepare "$v"; rc=$?; refused "README install line pins another tag" "README.md"
fresh ev; sed -i "s/^## $v/## 9x8x7/" "$W/EVIDENCE.md"; commit_all c; prepare "$v"; rc=$?; refused "EVIDENCE has no section (a dot is not a wildcard)" "EVIDENCE.md"
fresh doc; git -C "$W" rm -q "docs/RELEASE-$v.md"; commit_all c; prepare "$v"; rc=$?; refused "release document missing" "docs/RELEASE-"
fresh ahead; echo x >> "$W/README.md"; g -C "$W" commit -q -am local; prepare "$v"; rc=$?; refused "HEAD is not gitlab main" "is not gitlab main"
fresh dirty; echo x >> "$W/README.md"; prepare "$v"; rc=$?; refused "a dirty working tree" "not clean"
fresh untracked; echo x > "$W/new.txt"; prepare "$v"; rc=$?; refused "an untracked file" "not clean"
fresh hub; git -C "$W" remote set-url gitea "$S/hub-remotes/GitHub.git"; prepare "$v"; rc=$?; refused "a GitHub remote (any case) is refused" "is GitHub"
fresh exist; g -C "$W" tag "v$v"; g -C "$W" push -q gitea "refs/tags/v$v"; g -C "$W" tag -d "v$v" >/dev/null; prepare "$v"; rc=$?; refused "a tag already on a remote" "already exists on gitea"
fresh noremote; git -C "$W" remote remove forgejo; prepare "$v"; rc=$?; refused "a missing service remote" "no remote 'forgejo'"

# tag: refused while no signer is enrolled
fresh none; tagit "$S/key-a"; rc=$?; refused "no signer enrolled: refuses to tag" "no release signer is enrolled"
git -C "$W" rev-parse -q --verify "refs/tags/v$v" >/dev/null && fail "a tag was left with no signer enrolled"
# tag: an enrolled key signs, verifies and pushes to the three
fresh good; printf 'owner namespaces="git" %s\n' "$(cut -d' ' -f1,2 "$S/key-a.pub")" >> "$W/.github/release-signers/allowed_signers"; commit_all enrol
tagit "$S/key-a"; rc=$?; c="$(git -C "$W" rev-parse HEAD)"; ok=1
for r in gitlab gitea forgejo; do
  remote_has_tag "$r" && [ "$(git --git-dir="$R/$r.git" cat-file -t "refs/tags/v$v")" = tag ] \
    && [ "$(git --git-dir="$R/$r.git" rev-parse "v$v^{commit}")" = "$c" ] || ok=0
done
[ "$rc" -eq 0 ] && [ "$ok" = 1 ] && pass "an enrolled key's signed tag goes to gitlab, gitea and forgejo, naming the commit" || fail "good tag: rc=$rc $(tail -2 "$S/out")"
tagit "$S/key-a"; rc=$?; refused "the same tag twice is refused" "already exists"
# tag: a key that is not enrolled -> local tag deleted, nothing pushed
fresh stranger; printf 'owner namespaces="git" %s\n' "$(cut -d' ' -f1,2 "$S/key-a.pub")" >> "$W/.github/release-signers/allowed_signers"; commit_all enrol
tagit "$S/key-b"; rc=$?; refused "a key that is not enrolled is refused" "deleted the local tag"
pushed=0; for r in gitlab gitea forgejo; do remote_has_tag "$r" && pushed=1; done
[ "$pushed" = 0 ] && ! git -C "$W" rev-parse -q --verify "refs/tags/v$v" >/dev/null && pass "nothing pushed and no local tag left for that key" || fail "stranger's tag left or pushed"
# tag: the trust root is read from the tagged commit, not the working tree
fresh worktree; printf 'owner namespaces="git" %s\n' "$(cut -d' ' -f1,2 "$S/key-b.pub")" >> "$W/.github/release-signers/allowed_signers"
g -C "$W" update-index --assume-unchanged .github/release-signers/allowed_signers
tagit "$S/key-b"; rc=$?; refused "a signer added only in the working tree does not count" "no release signer is enrolled"
# tag: GitHub remote refused before a tag exists
fresh hubtag; printf 'owner namespaces="git" %s\n' "$(cut -d' ' -f1,2 "$S/key-a.pub")" >> "$W/.github/release-signers/allowed_signers"; commit_all enrol
# A local path that names GitHub: refused by name, and never a network URL, so a test with the check
# removed (a plant) fails here instead of reaching any service.
git -C "$W" remote set-url --push forgejo "$S/remotes-github.com/x/forge-warden.git"
tagit "$S/key-a"; rc=$?; refused "a GitHub push URL is refused before tagging" "is GitHub"
git -C "$W" rev-parse -q --verify "refs/tags/v$v" >/dev/null && fail "a tag was made before the GitHub refusal" || pass "no tag made when a remote is GitHub"
# tag: a commit that is not gitlab main
fresh offmain; printf 'owner namespaces="git" %s\n' "$(cut -d' ' -f1,2 "$S/key-a.pub")" >> "$W/.github/release-signers/allowed_signers"; commit_all enrol
old="$(git -C "$W" rev-parse HEAD~1)"
(cd "$W" && FW_TAG_KEY="$S/key-a" bash scripts/release_tag.sh "$v" "$old") > "$S/out" 2>&1; rc=$?; refused "a commit that is not gitlab main is refused" "not $old"
# tag: one service refuses the push -> rc 1, the others are still pushed
fresh partial; printf 'owner namespaces="git" %s\n' "$(cut -d' ' -f1,2 "$S/key-a.pub")" >> "$W/.github/release-signers/allowed_signers"; commit_all enrol
printf '#!/bin/sh\nexit 1\n' > "$R/gitea.git/hooks/pre-receive"; chmod +x "$R/gitea.git/hooks/pre-receive"
tagit "$S/key-a"; rc=$?
[ "$rc" -eq 1 ] && remote_has_tag gitlab && remote_has_tag forgejo && ! remote_has_tag gitea && grep -q 'push gitea rc=1' "$S/out" \
  && pass "a refused push is reported (rc 1) and the other services still get the tag" || fail "partial push: rc=$rc $(tail -3 "$S/out")"

[ "$fails" -eq 0 ] && echo "release tests: all passed" || { echo "release tests: $fails failed"; exit 1; }
