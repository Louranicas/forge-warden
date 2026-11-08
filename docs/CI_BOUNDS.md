# Reviewed CI job budgets

WP-Q-CI-BOUNDS closes the timeout part of Q3-11. Job IDs and display names are unchanged: required-check names remain the owner's ruleset contract. No job is split or renamed.

Every job in ci.yml and mutation-baseline.yml has an explicit, literal timeout-minutes. The protected check_ci contract in scripts/plant_tools.py requires the exact reviewed mapping below. Missing, duplicate, zero, expression-valued or unreviewed budgets refuse. New jobs or workflow shapes require review; the checker admits our block-style workflow subset, not arbitrary YAML. Multiline quoted/flow scalars, quoted/aliased job definitions and duplicate root/job fields are unsupported rather than silently ignored. Steps retain their own existing contracts.

## Measurements and headroom

Job durations are completedAt minus startedAt from GitHub Actions job records fetched with gh run view on 2026-11-08 UTC. They include setup and teardown, but not pre-job queue time. Successful timings are distinguished from failed runs; this is a sizing observation, not a future-duration guarantee.

| Workflow/job | Measured job duration | Budget | Reason |
| --- | --- | --- | --- |
| ci/gate | 374s successful; 384s before a plant failure | 20min | More than three times the successful run, accommodating cold build/cache and plant variability. |
| ci/msrv | 21s and 33s successful | 10min | Allows substantial toolchain provisioning/network variability while bounding hangs. |
| ci/supply-chain | 14s and 17s successful | 10min | Allows advisory/action provisioning variability; no unlimited wait. |
| ci/workflow-audit | 4s and 7s successful | 10min | Tool installation and runner variability dominate these short observed runs. |
| ci/mutation | 648s and 1448s successful | 60min | More than twice the larger diff run; a larger future diff may need a separately reviewed budget. |
| ci/dogfood | 100s and 103s, HOLD exits | 10min | Roughly six times the observed judge run; HOLD is not relabeled a successful verdict. |
| ci/vocabulary | 6s successful in both runs | 10min | Leaves room for the pinned-source fetch and external service delays. |
| mutation-baseline/full-workspace | No hosted run available yet | 120min provisional | Existing local full baseline on6947afe took13m42s for556 mutants. The24m08s hosted diff is a cautionary comparator, not a full-baseline measurement. Two hours provides substantial initial headroom while reducing the old six-hour bound; measure the first successful scheduled hosted baseline and review if inadequate. |

Hosted sources: [run42484786431 on6947afe](https://github.com/Louranicas/forge-warden/actions/runs/42484786431) and [run42478310228 on5a25ef3](https://github.com/Louranicas/forge-warden/actions/runs/42478310228). The scheduled workflow was absent from GitHub's default branch when queried, so no hosted full-baseline duration is claimed. Local full-baseline evidence is mission6/lanes/bastion/BASELINE-6947afe/BASELINE.json in the review workspace; it is not a hosted benchmark.

Rung: workflow enforcement plus a gate contract. Domain types cannot bound an external GitHub runner job; the workflow owns cancellation, and the contract prevents a missing or unjustified value from quietly restoring the service default. Timeouts are failures/cancellations, never successful incomplete coverage. These budgets do not bound queue time or prove immediate termination of external services.
