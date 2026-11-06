#!/usr/bin/env bash
# Build from authenticated Git objects in the trusted repository; never invoke Git in the candidate.
# Usage: managed_review_bundle.sh <base-sha> <head-sha> <candidate-checkout> <new-absolute-bundle-dir>
set -euo pipefail
[ "$#" -eq 4 ] || { echo 'review bundle: four arguments required' >&2; exit 2; }
base="$1"; head="$2"; candidate="$3"; out="$4"
[[ "$base" =~ ^[0-9a-f]{40}$ && "$head" =~ ^[0-9a-f]{40}$ ]] || exit 2
trusted="$(cd "$(dirname "$0")/.." && pwd -P)"
candidate="$(cd "$candidate" && pwd -P)"
# The controller imports both commit objects before launch, into its own trusted repository.
# No fetch/upload-pack, status, checkout, index refresh or candidate .git/config access occurs here.
trusted_git() {
  /usr/bin/env -i PATH=/usr/bin:/bin HOME=/nonexistent LC_ALL=C \
    GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null GIT_ATTR_NOSYSTEM=1 \
    GIT_NO_REPLACE_OBJECTS=1 GIT_OPTIONAL_LOCKS=0 \
    /usr/bin/git --no-replace-objects -C "$trusted" \
      -c core.attributesFile=/dev/null -c core.hooksPath=/dev/null -c core.fsmonitor=false "$@"
}
[ "$candidate" != "$trusted" ] || exit 2
[ "$(trusted_git rev-parse --verify HEAD)" = "$base" ] || exit 2
[ "$(trusted_git rev-parse --verify "$head^{commit}")" = "$head" ] || exit 2
# Source consistency only: the external controller must authenticate the helper itself.
expected_script="$(trusted_git show "$base:scripts/managed_review_bundle.sh" | /usr/bin/sha256sum)"
actual_script="$(/usr/bin/sha256sum < "$trusted/scripts/managed_review_bundle.sh")"
[ "$actual_script" = "$expected_script" ] || { echo 'review bundle: helper differs from trusted base' >&2; exit 2; }
[[ "$out" = /* && ! -e "$out" && ! -L "$out" ]] || exit 2
# Refuse missing or symlink instruction/policy blobs before writing a bundle. No candidate fallback.
for file in agents/managed/reviewer.agent.json agents/managed/REVIEW_CONTRACT.md CLAUDE.md warden.toml; do
  entry="$(trusted_git ls-tree "$base" -- "$file")"
  [[ "$entry" = "100644 blob "* ]] || { echo "review bundle: missing regular trusted blob $file" >&2; exit 2; }
done
umask 077
mkdir -- "$out"
out="$(cd "$out" && pwd -P)"
case "$out/" in "$trusted/"*|"$candidate/"*) echo 'review bundle: output must be outside both checkouts' >&2; exit 2;; esac
for file in agents/managed/reviewer.agent.json agents/managed/REVIEW_CONTRACT.md CLAUDE.md warden.toml; do
  trusted_git show "$base:$file" > "$out/$(basename "$file")"
done
/usr/bin/python3 -I - "$base" "$head" "$out" <<'PY'
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
