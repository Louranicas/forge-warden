#!/usr/bin/env bash
# The fast gate: what the pre-commit hook and the Claude Code Stop hook run. Seconds, not minutes.
set -euo pipefail
# Hook-exported Git state must not redirect scratch-repository commands.
while IFS= read -r name; do unset "$name"; done < <(compgen -v GIT_)
cd "$(git rev-parse --show-toplevel)"
python3 -I scripts/tests/mutation_budget.py
bash scripts/tests/ci_local.sh
python3 -I scripts/tests/mutation_env.py
python3 -I scripts/tests/local_tools.py
bash scripts/tests/dogfood.sh
bash scripts/tests/release.sh
python3 -I scripts/tests/hold_ack.py
python3 -I .claude/hooks/test_guard.py
python3 -I scripts/tests/managed_review_bundle.py
python3 -I scripts/tests/guardrails.py
python3 -I scripts/tests/runner_routing.py
python3 -I scripts/check_guardrails.py
python3 -I plants/anchors.py
python3 -I scripts/script_commands.py
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked --quiet
cargo test --workspace --locked --quiet
