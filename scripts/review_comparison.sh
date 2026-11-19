#!/usr/bin/env bash
set -euo pipefail
unset "${!GIT_@}"
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
    sys.exit(f"ci_local: cannot inspect comparison ref: {error}")
else:
    print("absent" if stat.S_ISDIR(mode) else "present")
PYREF
)"
if [ "$loose" = present ] || git show-ref --verify --quiet "$comparison_ref"; then
  tip="$(git show-ref --verify --hash "$comparison_ref")" || exit 1
elif [ "$?" -eq 1 ]; then
  tip=""
else
  exit 1
fi
exec bash scripts/make_patch.sh --resolve-comparison "$tip" "$1"
