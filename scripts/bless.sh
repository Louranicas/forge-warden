#!/usr/bin/env bash
# Operator-only regeneration. Ordinary cargo tests are always read-only.
# An independent verifier builds the reviewed subject, regenerates, compares exact bytes,
# and retains the binary/output hashes for the owner-held golden review.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
if [ "${CI+x}" = x ]; then
  echo 'golden regeneration is refused in CI' >&2
  exit 2
fi
cargo build --locked -p forge-warden-cli --bin forge-warden
python3 -I scripts/golden_output.py --binary "${CARGO_TARGET_DIR:-target}/debug/forge-warden" --write
cargo test --quiet --locked -p forge-warden-cli --test cli golden_verdict_for_the_demo_change
git --no-pager diff --stat -- fixtures/
