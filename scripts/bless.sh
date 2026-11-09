#!/usr/bin/env bash
# Operator-only regeneration. Ordinary cargo tests are always read-only.
# An independent verifier builds the reviewed subject, regenerates, compares exact bytes,
# and retains the binary/output hashes for the owner-held golden review.
set -euo pipefail
if [ "${CI+x}" = x ] || [ "${GITHUB_ACTIONS+x}" = x ]; then
  echo 'golden regeneration is refused in CI' >&2
  exit 2
fi
if [ "$#" -ne 6 ] || [ "$1" != --binary ] || [ "$3" != --receipt ] || [ "$5" != --subject ]; then
  echo 'usage: scripts/bless.sh --binary PATH --receipt PATH --subject source-sha256:HEX' >&2
  exit 2
fi
binary=$2
receipt=$4
subject=$6
cd "$(git rev-parse --show-toplevel)"
backup_dir=$(mktemp -d /tmp/fw-golden-backups.XXXXXXXX)
printf 'Golden backups: %s\n' "$backup_dir"
python3 -I scripts/golden_output.py --binary "$binary" --receipt "$receipt" --subject "$subject" --write --backup-dir "$backup_dir"
cargo test --quiet --locked -p forge-warden-cli --test cli golden_verdict_for_the_demo_change
git --no-pager diff --stat -- fixtures/
