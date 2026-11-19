#!/usr/bin/env bash
# forge-warden patch-maker v2
# make_patch.sh [--resolve-comparison] <base> <head> [<repository>]
# By default, stdout contains the patch from the merge base to the supplied head commit.
# With --resolve-comparison, stdout contains the base tip and merge-base IDs, in that order.
# An empty base argument in this mode selects the single root of HEAD's complete history.
# The repository argument defaults to the current directory.
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
mode=patch
if [[ "${1:-}" = --review-comparison && "$#" = 1 ]]; then
  mode=review
  repository=.
  head_ref=HEAD
else
  if [[ "${1:-}" = --resolve-comparison ]]; then
    mode=resolve
    shift
  fi
  if [[ "$#" != 2 && "$#" != 3 ]]; then
    echo 'usage: make_patch.sh [--resolve-comparison] <base> <head> [<repository>] | --review-comparison' >&2
    exit 2
  fi
  repository="${3:-.}"
  base_ref="$1"
  head_ref="$2"
fi
# Nothing about Git comes from the caller's environment.
while IFS= read -r name; do unset "$name"; done < <(compgen -v GIT_)
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null GIT_ATTR_NOSYSTEM=1 GIT_NO_REPLACE_OBJECTS=1
CDPATH= cd -P -- "$repository"
start="$PWD"
while [[ "$mode" != review && "$PWD" != / && ! -L .git && ! -e .git && -x . ]]; do
  cd -P ..
done
if [[ -L .git || ( ! -d .git && ! -f .git ) ]]; then
  echo "make_patch: invalid checkout .git entry in $PWD" >&2
  exit 2
fi
if [[ "$(git -C "$start" rev-parse --show-toplevel && printf .)" != "$PWD"$'\n.' ]]; then
  echo "make_patch: Git worktree differs from the selected checkout $PWD" >&2
  exit 2
fi
head="$(git rev-parse --verify --quiet --end-of-options "$head_ref^{commit}")" || {
  echo "make_patch: head is not a commit in $PWD: $head_ref" >&2
  exit 2
}
if [[ "$mode" = review ]]; then
  comparison_ref=refs/remotes/origin/main
  ref_path="$(git rev-parse --git-path "$comparison_ref")"
  loose="$(python3 -I - "$ref_path" <<'PYREF'
import os
import stat
import sys

try:
    mode = os.lstat(sys.argv[1]).st_mode
except (FileNotFoundError, NotADirectoryError):
    print("absent")
except OSError as error:
    sys.exit(f"make_patch: cannot inspect comparison ref: {error}")
else:
    print("absent" if stat.S_ISDIR(mode) else "present")
PYREF
  )"
  if [ "$loose" = present ] || git show-ref --verify --quiet "$comparison_ref"; then
    base_ref="$(git show-ref --verify --hash "$comparison_ref")" || exit 1
  elif [ "$?" -eq 1 ]; then
    base_ref=""
  else
    exit 1
  fi
fi
objects="$(git rev-parse --path-format=absolute --git-path objects)"
scratch="$(mktemp -d)"
trap 'rm -rf -- "$scratch"' EXIT
# The private repository has the store's object format (40 hex digits: SHA-1, 64: SHA-256), as the
# door's bound mode does; a SHA-1 repository cannot read a SHA-256 store's objects (H18).
case "${#head}" in
  40) format=sha1 ;;
  64) format=sha256 ;;
  *) echo "make_patch: unexpected object id length ${#head}: $head" >&2; exit 2 ;;
esac
git init --quiet --bare --template= --object-format="$format" "$scratch/diff.git"
printf '%s\n' "$objects" > "$scratch/diff.git/objects/info/alternates"
# The caller's commit-graph and multi-pack index are not read (they can lie about parents; bound mode's
# staging does the same, ADR 0012 D3.1), and several merge bases are refused as bound mode refuses
# them (D7): `git diff A...B` would pick one with only a warning.
nograph=(-c core.commitGraph=false -c core.multiPackIndex=false)
if [[ "$mode" != patch && -z "$base_ref" ]]; then
  if [ "$(git rev-parse --is-shallow-repository)" != false ]; then
    echo 'make_patch: root comparison requires complete history' >&2
    exit 1
  fi
  base="$(git --git-dir="$scratch/diff.git" "${nograph[@]}" rev-list --max-parents=0 "$head")"
  if [[ -z "$base" || "$base" == *$'\n'* ]]; then
    echo 'make_patch: root comparison requires a single root' >&2
    exit 2
  fi
else
  base="$(git rev-parse --verify --quiet --end-of-options "$base_ref^{commit}")" || {
    echo "make_patch: base is not a commit in $PWD: $base_ref" >&2
    exit 2
  }
fi
# `merge-base` exits 1 and says nothing when the two have no common history; any other failure is
# Git's (most often a shallow or partial clone missing commits between them: this private repository
# borrows the object store, not the clone's shallow list), and its first line is shown, not renamed.
status=0
bases="$(git --git-dir="$scratch/diff.git" "${nograph[@]}" merge-base --all "$base" "$head" 2> "$scratch/merge-base.err")" \
  || status=$?
if [ "$status" -eq 1 ] && [ ! -s "$scratch/merge-base.err" ]; then
  echo "make_patch: base and head have no common history" >&2
  exit 2
elif [ "$status" -ne 0 ]; then
  echo "make_patch: git merge-base failed (exit $status): $(head -n 1 "$scratch/merge-base.err")" >&2
  echo "make_patch: a shallow or partial clone lacks commits between base and head; fetch them (git fetch --unshallow)" >&2
  exit 2
fi
count="$(printf '%s\n' "$bases" | grep -c .)"
if [ "$count" -ne 1 ]; then
  echo "make_patch: base and head have $count merge bases; name the merge base as --base" >&2
  exit 2
fi
case "$mode" in
  review) printf '%s %s %s\n' "$base" "$bases" "$head"; exit 0 ;;
  resolve) printf '%s %s\n' "$base" "$bases"; exit 0 ;;
esac
# Attributes come from the empty tree, not from any work tree: a `.gitattributes` saying `-diff`
# or `binary` would otherwise turn added text into a binary patch the judge cannot read.
# --attr-source needs Git 2.41 or later; an older Git refuses the option.
empty_tree="$(git --git-dir="$scratch/diff.git" hash-object -t tree /dev/null)"
# A submodule change is always written as its `diff --git` section with mode 160000. In a work
# tree, `diff.ignoreSubmodules` or `ignore = all` in `.gitmodules` (a file the change itself can
# commit) leaves it out, and `diff.submodule=log` writes a line no patch reader takes as a change.
git --git-dir="$scratch/diff.git" "${nograph[@]}" --attr-source="$empty_tree" -c core.attributesFile=/dev/null \
  -c core.quotePath=true \
  diff --no-ext-diff --no-textconv --no-color --no-relative --binary --no-renames --full-index --src-prefix=a/ --dst-prefix=b/ --submodule=short --ignore-submodules=none \
  "$base...$head"
