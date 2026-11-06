#!/usr/bin/env bash
# Regenerate golden verdicts after an intended change, then review `git diff fixtures/`.
# A golden file is a contract: bless only what you meant to change.
#
# The golden verdict is advisory mode on the demo patch and deep-diff-forge 0.7.0's recorded ranking;
# a second test holds authority mode (the fixture ranker replaying that recording) to the same
# judgement. The Git version depends on the machine, so it is a placeholder; every input seal is
# real. The test itself writes the file and still checks the exit status (7).
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
FORGE_WARDEN_BLESS=1 cargo test --quiet --locked -p forge-warden-cli --test cli golden_verdict_for_the_demo_change
git --no-pager diff --stat -- fixtures/
