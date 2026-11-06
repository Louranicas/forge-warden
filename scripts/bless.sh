#!/usr/bin/env bash
# Regenerate golden verdicts after an intended change, then review `git diff fixtures/`.
# A golden file is a contract: bless only what you meant to change.
#
# The golden verdict is authority mode on the demo patch: the fixture ranker (built by `cargo test`)
# replays deep-diff-forge 0.7.0's recorded ranking. Seals that depend on the build or the machine
# (the fixture ranker's hash, the policy that admits it, the Git version) are placeholders, so the
# file is the same everywhere. The test itself writes the file and still checks the exit status (7).
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
FORGE_WARDEN_BLESS=1 cargo test --quiet --locked -p forge-warden-cli --test cli golden_verdict_for_the_demo_change
git --no-pager diff --stat -- fixtures/
