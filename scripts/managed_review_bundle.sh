#!/usr/bin/env bash
# Build a managed-review instruction bundle from a clean trusted base, outside both checkouts.
# Usage: managed_review_bundle.sh <base-sha> <head-sha> <candidate-checkout> <new-absolute-bundle-dir>
set -euo pipefail
[ "$#" -eq 4 ] || { echo 'review bundle: four arguments required' >&2; exit 2; }
base="$1"; head="$2"; candidate="$3"; out="$4"
[[ "$base" =~ ^[0-9a-f]{40}$ && "$head" =~ ^[0-9a-f]{40}$ ]] || exit 2
trusted="$(cd "$(dirname "$0")/.." && pwd -P)"
candidate="$(cd "$candidate" && pwd -P)"
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null GIT_NO_REPLACE_OBJECTS=1 GIT_OPTIONAL_LOCKS=0
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES GIT_CONFIG_COUNT GIT_CONFIG_PARAMETERS
[ "$candidate" != "$trusted" ] || exit 2
[ "$(git -C "$trusted" rev-parse --verify HEAD)" = "$base" ] || exit 2
[ "$(git -C "$candidate" rev-parse --verify HEAD)" = "$head" ] || exit 2
[ -z "$(git -c core.fsmonitor=false -C "$trusted" status --porcelain --untracked-files=all --ignored)" ] || exit 2
[ -z "$(git -c core.fsmonitor=false -C "$candidate" status --porcelain --untracked-files=all --ignored)" ] || exit 2
[[ "$out" = /* && ! -e "$out" && ! -L "$out" ]] || exit 2
# Refuse missing or symlink instruction/policy blobs before writing a bundle. No candidate fallback.
for file in agents/managed/reviewer.agent.json agents/managed/REVIEW_CONTRACT.md CLAUDE.md warden.toml; do
  entry="$(git -C "$trusted" ls-tree "$base" -- "$file")"
  [[ "$entry" = "100644 blob "* ]] || { echo "review bundle: missing regular trusted blob $file" >&2; exit 2; }
done
umask 077
mkdir -- "$out"
out="$(cd "$out" && pwd -P)"
case "$out/" in "$trusted/"*|"$candidate/"*) echo 'review bundle: output must be outside both checkouts' >&2; exit 2;; esac
for file in agents/managed/reviewer.agent.json agents/managed/REVIEW_CONTRACT.md CLAUDE.md warden.toml; do
  git -C "$trusted" show "$base:$file" > "$out/$(basename "$file")"
done
python3 -I - "$base" "$head" "$out" <<'PY'
import hashlib, json, sys
from pathlib import Path
base, head, directory = sys.argv[1:]
root = Path(directory)
files = {name: hashlib.sha256((root / name).read_bytes()).hexdigest()
         for name in ("reviewer.agent.json", "REVIEW_CONTRACT.md", "CLAUDE.md", "warden.toml")}
(root / "subject.json").write_text(json.dumps({"schema": "forge-warden.review-bundle.v1",
    "repository": "Louranicas/forge-warden", "base": base, "head": head, "sha256": files},
    sort_keys=True, indent=2) + "\n")
PY
printf 'review bundle: prepared %s; provider isolation and subject admission still required\n' "$out"
