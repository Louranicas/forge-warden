#!/usr/bin/env bash
set -euo pipefail
for name in "${!GIT_@}"; do
    if [[ $name != GIT_CONFIG_NOSYSTEM ]]; then unset "$name"; fi
done
scope='crates/*/src/**'
sources=":(glob)$scope"
untracked="$(git ls-files --others --exclude-standard -- "$sources")"
ignored_rust="$(git ls-files --others --ignored --exclude-standard -- ':(glob)crates/*/src/**/*.rs')"
if [[ -n $untracked || -n $ignored_rust ]]; then
    echo 'Failing-first check unverified. Stage the new source before saving:' >&2
    printf '%s\n' ${untracked:+"$untracked"} ${ignored_rust:+"$ignored_rust"} >&2
    exit 1
fi
git status --porcelain=v1 -z --no-renames --untracked-files=no -- crates | python3 -I -c '
import os
import subprocess
import sys
from pathlib import Path

entries = {Path(os.fsdecode(row[3:])): row[:2] for row in sys.stdin.buffer.read().split(b"\0") if row}
staged_source = False
for path, status in entries.items():
    if len(path.parts) < 4 or path.parts[2] != "src":
        continue
    if status[:1] != b" ":
        staged_source = True
    if status == b"AD":
        sys.exit("Failing-first check unverified. Git stash cannot preserve a newly staged file missing from the working tree.")
    blocked_parent = any(parent in entries or parent.is_symlink() or
                         (parent.exists() and not parent.is_dir()) for parent in path.parents)
    if blocked_parent or (not path.is_symlink() and path.is_dir()):
        sys.exit(f"Failing-first check unverified. Source changes between a file and a directory at {str(path)!r}.")
if staged_source and not subprocess.check_output(
        ["git", "diff", "--no-ext-diff", "--no-textconv", "--name-only", "-z", "HEAD", "--", sys.argv[1]]):
    sys.exit("Failing-first check unverified. Source edits are staged, but the working source matches HEAD.")
' "$sources"
marker="forge-warden-source-$(python3 -I -c 'import uuid; print(uuid.uuid4())')"
scratch="$(mktemp -d)"
trap 'rm -rf -- "$scratch"' EXIT
index="$scratch/index"
cp -- "$(git rev-parse --git-path index)" "$index"
GIT_INDEX_FILE="$index" git reset -q HEAD -- ":(exclude,glob)$scope"
status=0
GIT_INDEX_FILE="$index" git -c apply.whitespace=nowarn stash push --message "$marker" -- "$sources" || status=$?
stored="$(git stash list --fixed-strings --grep-reflog="$marker" --format=%H)"
if [[ -n $stored ]]; then
    printf '%s\n' "$stored"
    if [[ $status == 0 ]]; then
        git reset -q "$stored^1" -- "$sources"
    fi
fi
exit "$status"
