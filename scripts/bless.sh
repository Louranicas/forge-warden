#!/usr/bin/env bash
# Regenerate golden verdicts after an intended change, then review `git diff fixtures/`.
# A golden file is a contract: bless only what you meant to change.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
cargo build --quiet --bin forge-warden
set +e
./target/debug/forge-warden check --policy examples/starter.warden.toml \
  --rank fixtures/demo.rank.json --patch fixtures/demo.patch --today 2026-11-05 --json \
  > fixtures/demo.verdict.json
code=$?
set -e
[ "$code" -eq 7 ] || { echo "bless: expected exit 7 for the demo, got $code" >&2; exit 1; }
git --no-pager diff --stat -- fixtures/
