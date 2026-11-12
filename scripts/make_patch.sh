#!/usr/bin/env bash
# forge-warden patch-maker v1
# Write, on stdout, the patch forge-warden should judge for the change from BASE to HEAD.
# Usage: make_patch.sh <base> <head> [<repository>]      (repository: default, the current directory)
#
# The patch is the same bytes whatever the caller's Git configuration, environment or current
# directory are. A plain `git diff` is not: `diff.noprefix` drops the `a/` `b/` prefixes (the
# judge then reads `scripts/x` as `x`), `diff.relative` in a subdirectory leaves out every change
# outside it, and colour, context and quoting settings change the bytes. So the diff runs in a
# new, empty repository that only borrows the object store: no configuration of the user, the
# system or the repository is read for it, and every option that matters is also given here.
#
# The change is `base...head`: from the merge base of the two commits to HEAD, as on a pull request.
# This does not prove to anyone else that the patch is that change; it makes it hard to get wrong.
set -euo pipefail
if [ "$#" -ne 2 ] && [ "$#" -ne 3 ]; then
  echo 'usage: make_patch.sh <base> <head> [<repository>]' >&2
  exit 2
fi
repository="${3:-.}"
# Nothing about Git comes from the caller's environment.
while IFS= read -r name; do unset "$name"; done < <(compgen -v GIT_)
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null GIT_ATTR_NOSYSTEM=1 GIT_NO_REPLACE_OBJECTS=1
base="$(git -C "$repository" rev-parse --verify --quiet --end-of-options "$1^{commit}")" || {
  echo "make_patch: base is not a commit in $repository: $1" >&2
  exit 2
}
head="$(git -C "$repository" rev-parse --verify --quiet --end-of-options "$2^{commit}")" || {
  echo "make_patch: head is not a commit in $repository: $2" >&2
  exit 2
}
objects="$(git -C "$repository" rev-parse --path-format=absolute --git-path objects)"
scratch="$(mktemp -d)"
trap 'rm -rf -- "$scratch"' EXIT
git init --quiet --bare "$scratch/diff.git"
printf '%s\n' "$objects" > "$scratch/diff.git/objects/info/alternates"
# Attributes come from the empty tree, not from any work tree: a `.gitattributes` saying `-diff`
# or `binary` would otherwise turn added text into a binary patch the judge cannot read.
# --attr-source needs Git 2.40 or later; an older Git refuses the option.
empty_tree="$(git hash-object -t tree /dev/null)"
git --git-dir="$scratch/diff.git" --attr-source="$empty_tree" -c core.attributesFile=/dev/null \
  -c core.quotePath=true diff --no-ext-diff --no-textconv --no-color --no-relative --binary \
  --no-renames --src-prefix=a/ --dst-prefix=b/ "$base...$head"
