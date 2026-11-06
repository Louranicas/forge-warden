#!/usr/bin/env bash
# The fast gate: what the pre-commit hook and the Claude Code Stop hook run. Seconds, not minutes.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
bash scripts/tests/ci_local.sh
bash scripts/tests/dogfood.sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked --quiet
cargo test --workspace --locked --quiet
