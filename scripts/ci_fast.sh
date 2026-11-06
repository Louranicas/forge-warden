#!/usr/bin/env bash
# The fast gate: what the pre-commit hook and the Claude Code Stop hook run. Seconds, not minutes.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
bash scripts/tests/ci_local.sh
python3 -I scripts/tests/mutation_env.py
bash scripts/tests/dogfood.sh
python3 -I scripts/tests/hold_ack.py
python3 -I .claude/hooks/test_guard.py
python3 -I scripts/tests/managed_review_bundle.py
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked --quiet
cargo test --workspace --locked --quiet
