#!/usr/bin/env bash
# CI job `dogfood`: rank this pull request with deep-diff-forge and judge it with warden.toml.
#   exit 7 (fail)  -> the job fails.
#   exit 8 (hold)  -> the job fails unless a maintainer labelled the PR `warden:acknowledged`.
# Usage: scripts/dogfood.sh <base sha> <acknowledged: true|false>
set -euo pipefail
base="${1:?base sha}"
acknowledged="${2:-false}"
# --no-renames: rank.v0 reports only a renamed file's new path, so a rename is split into a deletion
# and an addition that every path rule can see.
git diff --binary --no-renames "${base}...HEAD" > change.patch
deep-diff-forge --stdin-patch --rank --json < change.patch > rank.json
# Judge with the BASE branch's policy: a pull request must not be able to loosen the rules that
# judge it. (Its own warden.toml change is judged too: `quality-config-is-owned` holds it.)
# A base without a policy is refused rather than replaced by the pull request's own: otherwise a
# change that deleted warden.toml would let the next one choose its judge.
if ! git cat-file -e "${base}:warden.toml" 2>/dev/null; then
  echo "dogfood: the base commit has no warden.toml; restore it on main before merging anything" >&2
  exit 1
fi
git show "${base}:warden.toml" > policy.toml
set +e
./target/release/forge-warden check --policy policy.toml --rank rank.json --patch change.patch --json > verdict.json
code=$?
set -e
./target/release/forge-warden check --policy policy.toml --rank rank.json --patch change.patch > verdict.txt || true
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  { echo '## forge-warden verdict'; echo; echo '```text'; cat verdict.txt; echo '```'; } >> "$GITHUB_STEP_SUMMARY"
fi
cat verdict.txt
case "$code" in
  0) exit 0 ;;
  8) if [ "$acknowledged" = "true" ]; then echo "hold acknowledged by a maintainer (label warden:acknowledged)"; exit 0; fi
     echo "HOLD: a maintainer must review the held rules and add the label warden:acknowledged" >&2; exit 1 ;;
  *) exit "$code" ;;
esac
