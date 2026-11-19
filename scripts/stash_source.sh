#!/usr/bin/env bash
set -euo pipefail
for name in "${!GIT_@}"; do
    if [[ $name != GIT_CONFIG_NOSYSTEM ]]; then unset "$name"; fi
done
sources=':(glob)crates/*/src/**'
untracked="$(git ls-files --others --exclude-standard -- "$sources")"
ignored_rust="$(git ls-files --others --ignored --exclude-standard -- ':(glob)crates/*/src/**/*.rs')"
if [[ -n $untracked || -n $ignored_rust ]]; then
    echo 'Failing-first check unverified. Stage the new source before saving:' >&2
    printf '%s\n' ${untracked:+"$untracked"} ${ignored_rust:+"$ignored_rust"} >&2
    exit 1
fi
git status --porcelain=v1 -z --no-renames --untracked-files=no -- crates | python3 -I -c '
import os
import sys
from pathlib import Path

entries = {Path(os.fsdecode(row[3:])): row[:2] for row in sys.stdin.buffer.read().split(b"\0") if row}
for path, status in entries.items():
    if len(path.parts) < 4 or path.parts[2] != "src":
        continue
    if status == b"AD":
        sys.exit("Failing-first check unverified. Git stash cannot preserve a newly staged file missing from the working tree.")
    blocked_parent = any(parent in entries or parent.is_symlink() or
                         (parent.exists() and not parent.is_dir()) for parent in path.parents)
    if blocked_parent or (not path.is_symlink() and path.is_dir()):
        sys.exit(f"Failing-first check unverified. Source changes between a file and a directory at {str(path)!r}.")
'
marker="forge-warden-source-$(python3 -I -c 'import uuid; print(uuid.uuid4())')"
git stash push --message "$marker" -- "$sources"
git stash list --fixed-strings --grep-reflog="$marker" --format=%H
