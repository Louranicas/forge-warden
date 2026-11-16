#!/usr/bin/env bash
# forge-warden release tag v1 (M7-RELEASE-SCRIPT, ledger Q42). The repository's OWNER runs it: the tag is
# the owner's act, and no agent or CI job runs this script.
# Usage: scripts/release_tag.sh <version> <full commit>     (the command scripts/release_prepare.sh prints)
#
# Signs the annotated tag v<version> on <commit> with the owner's release key, keeps it only if it
# verifies against .github/release-signers/allowed_signers AS COMMITTED IN <commit> (the reviewed
# trust root, not the working tree and not the key being used), and pushes the tag, and only the tag,
# to the remotes gitlab, gitea and forgejo. A remote whose URL names GitHub is refused before anything
# is created. A tag signed by a key that is not enrolled is deleted before any push.
# The key: FW_TAG_KEY (default ~/.ssh/id_ed25519_forge_warden_release_signing); only its .pub path is
# given to git, so the private key never leaves the owner's ~/.ssh. The tagger is the owner's own
# git config (user.name, user.email).
set -euo pipefail
version="${1:-}"; commit="${2:-}"
die() { echo "release_tag: $*" >&2; exit 1; }
[[ "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] && [[ "$commit" =~ ^[0-9a-f]{40}$ ]] \
  || { echo "usage: scripts/release_tag.sh <version> <full 40-hex commit>" >&2; exit 2; }
tag="v$version"
while IFS= read -r name; do unset "$name"; done < <(compgen -v GIT_)
cd "$(git rev-parse --show-toplevel)"
key="${FW_TAG_KEY:-$HOME/.ssh/id_ed25519_forge_warden_release_signing}"
[ -f "$key.pub" ] || die "no public key $key.pub (make the release key with: ssh-keygen -t ed25519 -f $key)"

for remote in gitlab gitea forgejo; do
  # Every URL Git may use for the remote, fetch and push, insteadOf expanded (Bastion 1072): one naming
  # GitHub refuses the release.
  urls="$(git remote get-url --all "$remote" 2>/dev/null)" || die "no remote '$remote' (the release goes to GitLab, Gitea and Forgejo)"
  urls+=$'\n'"$(git remote get-url --push --all "$remote")"
  while IFS= read -r url; do
    case "${url,,}" in *github*) die "remote '$remote' has a GitHub URL ($url): never" ;; esac
  done <<< "$urls"
done
# GitLab's main as GitLab says it, not a ref name a local branch or tag could shadow (Bastion 1072).
main="$(git ls-remote --exit-code gitlab refs/heads/main | cut -f1)" || die "cannot read main from gitlab"
[ "$main" = "$commit" ] || die "gitlab main is $main, not $commit"
git cat-file -e "$commit^{commit}" 2>/dev/null || die "$commit is not in this clone (git fetch gitlab first)"
git rev-parse -q --verify "refs/tags/$tag" >/dev/null && die "tag $tag already exists here"
for remote in gitlab gitea forgejo; do
  if git ls-remote --exit-code --tags "$remote" "refs/tags/$tag" >/dev/null 2>&1; then s=0; else s=$?; fi
  case "$s" in
    0) die "tag $tag already exists on $remote" ;;
    2) ;;
    *) die "cannot read the tags of $remote (git ls-remote exit $s)" ;;
  esac
done
signers="$(mktemp)"; trap 'rm -f -- "$signers"' EXIT
git show "$commit:.github/release-signers/allowed_signers" > "$signers" 2>/dev/null \
  || die "$commit has no .github/release-signers/allowed_signers"
grep -q -v -e '^#' -e '^[[:space:]]*$' "$signers" || die "no release signer is enrolled in $commit; refusing to tag"
name="$(git config user.name)" || die "set git config user.name"
email="$(git config user.email)" || die "set git config user.email"

git -c user.name="$name" -c user.email="$email" -c gpg.format=ssh -c gpg.ssh.program=ssh-keygen -c user.signingkey="$key.pub" \
  tag -s "$tag" "$commit" -m "forge-warden $version"
if ! git -c gpg.format=ssh -c gpg.ssh.program=ssh-keygen -c gpg.ssh.allowedSignersFile="$signers" tag -v "$tag" >/dev/null 2>&1; then
  git tag -d "$tag" >/dev/null
  die "$tag is not signed by a key in .github/release-signers/allowed_signers of $commit; deleted the local tag, pushed nothing"
fi
[ "$(git rev-parse "refs/tags/$tag^{commit}")" = "$commit" ] || die "$tag does not name $commit"
rc=0
for remote in gitlab gitea forgejo; do
  if out="$(git push "$remote" "refs/tags/$tag" 2>&1)"; then s=0; else s=$?; fi
  if printf '%s\n' "$out" | grep -qi -E 'payment|billing|quota|credit card'; then
    printf '%s\n' "$out" >&2; echo "release_tag: $remote mentions payment: stop and tell the owner" >&2; rc=5
  fi
  echo "push $remote rc=$s"
  if [ "$s" -ne 0 ]; then printf '%s\n' "$out" | tail -3 >&2; [ "$rc" -eq 5 ] || rc=1; fi
done
echo "tag object $(git rev-parse "$tag") -> commit $commit"
exit "$rc"
