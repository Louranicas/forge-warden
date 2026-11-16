#!/usr/bin/env bash
# forge-warden release preparation v1 (M7-RELEASE-SCRIPT, ledger Q42).
# Usage: scripts/release_prepare.sh <version>      e.g. 0.2.1
#
# Run in a clean clone whose remotes `gitlab`, `gitea` and `forgejo` are the three services the
# release goes to (never GitHub). It creates no tag and pushes nothing. It checks that HEAD is the
# main branch on GitLab and that the version is the same everywhere a release states it, writes the
# source archive, the release notes and their checksums under dist/<version>/, and prints the one
# command the repository's owner runs to sign and push the tag (scripts/release_tag.sh).
# The GitHub check reads every remote URL, not where ssh or proxy settings then send it.
set -euo pipefail
version="${1:-}"
die() { echo "release_prepare: $*" >&2; exit 1; }
[[ "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] \
  || { echo "usage: scripts/release_prepare.sh <version>   (MAJOR.MINOR.PATCH)" >&2; exit 2; }
tag="v$version"
# Nothing about Git comes from the caller's environment, not even which repository this is.
while IFS= read -r name; do unset "$name"; done < <(compgen -v GIT_)
cd "$(git rev-parse --show-toplevel)"

for remote in gitlab gitea forgejo; do
  # Every URL Git may use for the remote, fetch and push, insteadOf expanded (Bastion 1072): one naming
  # GitHub refuses the release.
  urls="$(git remote get-url --all "$remote" 2>/dev/null)" || die "no remote '$remote' (the release goes to GitLab, Gitea and Forgejo)"
  urls+=$'\n'"$(git remote get-url --push --all "$remote")"
  while IFS= read -r url; do
    case "${url,,}" in *github*) die "remote '$remote' has a GitHub URL ($url): never" ;; esac
  done <<< "$urls"
done
[ -z "$(git status --porcelain --untracked-files=all)" ] || die "the working tree is not clean"
head="$(git rev-parse HEAD)"
main="$(git ls-remote --exit-code gitlab refs/heads/main | cut -f1)" || die "cannot read main from gitlab"
[ "$head" = "$main" ] || die "HEAD $head is not gitlab main $main"
git rev-parse -q --verify "refs/tags/$tag" >/dev/null && die "tag $tag already exists here"
for remote in gitlab gitea forgejo; do
  if git ls-remote --exit-code --tags "$remote" "refs/tags/$tag" >/dev/null 2>&1; then s=0; else s=$?; fi
  case "$s" in
    0) die "tag $tag already exists on $remote" ;;
    2) ;;
    *) die "cannot read the tags of $remote (git ls-remote exit $s)" ;;
  esac
done

# The version, wherever a release states it.
workspace="$(awk '/^\[workspace\.package\]/{p=1;next} /^\[/{p=0} p && /^version = /{gsub(/"/,"",$3); print $3}' Cargo.toml)"
[ "$workspace" = "$version" ] || die "Cargo.toml workspace version is '$workspace', not $version"
for crate in forge-warden-core forge-warden-cli; do
  locked="$(awk -v n="$crate" '$0=="name = \""n"\"" {getline; gsub(/"/,"",$3); print $3}' Cargo.lock)"
  [ "$locked" = "$version" ] || die "Cargo.lock has $crate '$locked', not $version"
done
awk -v h="## [$version] - " '
  /^## \[Unreleased\]$/ { u = NR; next }
  u && /^## \[/ { rest = substr($0, length(h) + 1)
    if (substr($0, 1, length(h)) == h && rest ~ /^[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]$/) ok = 1; exit }
  u && NF { exit }
  END { exit !ok }' CHANGELOG.md || die "CHANGELOG.md: '## [$version] - YYYY-MM-DD' must follow an empty '## [Unreleased]'"
grep -qF -- "--tag $tag " README.md || die "README.md: the install line does not pin --tag $tag"
grep -qE "^## ${version//./\\.}( |\$)" EVIDENCE.md || die "EVIDENCE.md has no '## $version' section"
[ -f "docs/RELEASE-$version.md" ] || die "docs/RELEASE-$version.md does not exist"

# The artefacts: a source archive (gzip -n: no name or time in the header, so the bytes repeat),
# the release notes (the CHANGELOG section verbatim) and their checksums.
out="dist/$version"
[ ! -e "$out" ] || die "$out already exists"
mkdir -p "$out"
archive="forge-warden-$version.tar.gz"
git archive --format=tar --prefix="forge-warden-$version/" "$head" | gzip -n -9 > "$out/$archive"
awk -v h="## [$version] - " '
  substr($0, 1, length(h)) == h { p = 1; print; next }
  p && /^## \[/ { exit }
  p { print }' CHANGELOG.md > "$out/NOTES.md"
(cd "$out" && sha256sum "$archive" NOTES.md > SHA256SUMS)
echo "release_prepare: $tag ready at $head"
cat "$out/SHA256SUMS"
echo "The owner signs and pushes the tag (to GitLab, Gitea and Forgejo only):"
echo "  scripts/release_tag.sh $version $head"
