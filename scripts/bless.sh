#!/usr/bin/env bash
# Operator-only regeneration. Ordinary cargo tests are always read-only.
# An independent verifier builds the reviewed subject, regenerates, compares exact bytes,
# and retains the binary/output hashes for the owner-held golden review.
set -euo pipefail
if [ "${CI+x}" = x ] || [ "${GITHUB_ACTIONS+x}" = x ]; then
  echo 'golden regeneration is refused in CI' >&2
  exit 2
fi
cd "$(git rev-parse --show-toplevel)"
backup_dir=$(mktemp -d /tmp/fw-golden-backups.XXXXXXXX)
printf 'Golden backups: %s\n' "$backup_dir"
cargo build --locked -p forge-warden-cli --bin forge-warden
python3 -I scripts/golden_output.py --binary "${CARGO_TARGET_DIR:-target}/debug/forge-warden" --write --backup-dir "$backup_dir"
cargo test --quiet --locked -p forge-warden-cli --test cli golden_verdict_for_the_demo_change
git --no-pager diff --stat -- fixtures/
