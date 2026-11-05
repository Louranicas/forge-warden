#!/usr/bin/env bash
# The fast gate: what the pre-commit hook and the Claude Code Stop hook run. Seconds, not minutes.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked --quiet
cargo test --workspace --locked --quiet
