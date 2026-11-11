# Reviewed CI job budgets

WP-Q-CI-BOUNDS.1 supersedes the fixed mutation budgets below. The required check remains exactly `diff mutation (cargo-mutants)` (job `mutation`), now aggregating planning and every shard. The periodic audit retains its `full workspace mutation baseline` aggregate name. Other job names and budgets are unchanged.

Every job in ci.yml and mutation-baseline.yml has an explicit timeout-minutes. Only the mutation shard jobs admit the exact expression `${{ matrix.minutes }}`, produced by the reviewed planner; other budgets remain literal. The protected check_ci contract in scripts/plant_tools.py requires the exact reviewed mapping below. Missing, duplicate, zero, expression-valued or unreviewed budgets refuse. New jobs or workflow shapes require review; the checker admits our block-style workflow subset, not arbitrary YAML. Multiline quoted/flow scalars, quoted/aliased job definitions and duplicate root/job fields are unsupported rather than silently ignored. Steps retain their own existing contracts.

## Historical measurements and headroom (fixed mutation budgets superseded)

Job durations are completedAt minus startedAt from GitHub Actions job records fetched with gh run view on 2026-11-08 UTC. They include setup and teardown, but not pre-job queue time. Successful timings are distinguished from failed runs; this is a sizing observation, not a future-duration guarantee.

| Workflow/job | Measured job duration | Budget | Reason |
| --- | --- | --- | --- |
| mutation-baseline/route (WP-RUNNER-UP) | No hosted run yet; one hosted step of shell comparisons and two printf calls | 5min provisional | It does no checkout and no install, so job setup dominates. Five minutes bounds a stuck runner. If it times out, the routed jobs (mutation-plan and mutation-shards) still run hosted (`!cancelled()` and an `|| 'ubuntu-latest'` fallback). On runner-up the shard budgets stay the hosted-measured `matrix.minutes`; the guest bootstrap runs inside that budget's setup allowance. |
| ci/gate | 1086s successful at 98 plants; cancelled at the old 20min limit at 101 plants (see "The gate job grows with the plant catalogue") | 45min | 1.5 times the modelled worst run at 140 plants, the catalogue size `scripts/plant_tools.py` admits. |
| ci/msrv | 21s and 33s successful | 10min | Allows substantial toolchain provisioning/network variability while bounding hangs. |
| ci/supply-chain | 14s and 17s successful | 10min | Allows advisory/action provisioning variability; no unlimited wait. |
| ci/workflow-audit | 4s and 7s successful | 10min | Tool installation and runner variability dominate these short observed runs. |
| ci/mutation | 648s and 1448s successful | 60min | More than twice the larger diff run; a larger future diff may need a separately reviewed budget. |
| ci/dogfood | 100s and 103s, HOLD exits | 10min | Roughly six times the observed judge run; HOLD is not relabeled a successful verdict. |
| ci/vocabulary | 6s successful in both runs | 10min | Leaves room for the pinned-source fetch and external service delays. |
| mutation-baseline/full-workspace | No hosted run available yet | 120min provisional | Existing local full baseline on6947afe took13m42s for556 mutants. The24m08s hosted diff is a cautionary comparator, not a full-baseline measurement. Two hours provides substantial initial headroom while reducing the old six-hour bound; measure the first successful scheduled hosted baseline and review if inadequate. |

Hosted sources: [run42484786431 on6947afe](https://github.com/Louranicas/forge-warden/actions/runs/42484786431) and [run42478310228 on5a25ef3](https://github.com/Louranicas/forge-warden/actions/runs/42478310228). The scheduled workflow was absent from GitHub's default branch when queried, so no hosted full-baseline duration is claimed. Local full-baseline evidence is Bastion's baseline record for 6947afe in the project's coordination records (not in this repository); it is not a hosted benchmark.

Rung: workflow enforcement plus a gate contract. Domain types cannot bound an external GitHub runner job; the workflow owns cancellation, and the contract prevents a missing or unjustified value from quietly restoring the service default. Timeouts are failures/cancellations, never successful incomplete coverage. These budgets do not bound queue time or prove immediate termination of external services.

## The gate job grows with the plant catalogue (WP-CI-GATE-BUDGET, ledger Q49)

`ci/gate` runs every catalogued plant (`scripts/check_plants.py`), so its duration grows with `plants/catalog.json`. The 374 s recorded on 2026-11-08 was measured on a much smaller catalogue. Two hosted runs on 2026-11-11 (step durations are completedAt minus startedAt from `gh run view --json jobs`):

| Run | Commit | Plants | Job | `cargo test --workspace` | `check_plants.py` | Everything else |
| --- | --- | --- | --- | --- | --- | --- |
| [42924334392](https://github.com/Louranicas/forge-warden/actions/runs/42924334392) | 26e7357a | 98 | 1086 s, success | 288 s | 672 s, complete | 126 s |
| [42943254426](https://github.com/Louranicas/forge-warden/actions/runs/42943254426) | e1c70582 | 101 | cancelled at 1216 s (the 20min limit) | 367 s | 695 s, not finished | 154 s |

Measured: 672 s for 98 plants is 6.9 s per plant, and the part of the job before the plants took 414 s in the first run and 521 s in the second. The second run was the slower runner throughout (its `cargo test` step took 1.27 times as long), and it also carried the hostile-input harness for the first time.

Inferred, not measured: the second run's plant step never finished, so its rate is taken as 6.9 s times that same 1.27, which is 8.7 s per plant.

The rule, from the slower run:

```text
seconds(plants) = 525 + 8.7 * plants
budget_minutes  = ceil(1.5 * seconds(capacity) / 60)
```

For the admitted capacity of 140 plants that is 525 + 1218 = 1743 s, and 1.5 times it is 2615 s: 45 minutes. At today's 101 plants the model gives 1404 s (23.4 min), so the old 20-minute limit could not have held. `scripts/plant_tools.py` refuses a catalogue above 140 cases (`GATE_PLANT_CAPACITY`), in the fast gate: growing past it means re-deriving this budget from a new hosted run, in the same reviewed change as the workflow, its pin and the two budget tables.

Limits: two runs, one of them incomplete; the fixed part will also grow as tests are added, which this rule does not model. The local parity model has a composite budget and no per-job hosted limit, which is why it did not see this.

## Scaling mutation budgets (WP-Q-CI-BOUNDS.1)

The PR #3 hosted diff on5478daa found305 mutants. Its copied baseline took24s build plus337s test. The job was cancelled at60min with most mutants still untested. In the archived run log (kept in the project's coordination records, not in this repository), the largest completed mutant took1s build plus193s test. The original60min bound was demonstrated RED, not merely estimated to be inadequate.

Both mutation workflows now list the complete inventory using cargo-mutants27.1.0, `--no-config --cap-lints true --workspace --no-shuffle --jobs 1 --cargo-arg=--locked`. The PR workflow also uses `--in-diff pr.diff`, derived from the PR/merge-group base SHA and the existing production Rust path filter. Each shard rediscovers the same inventory, checks its digest/count/denominator/budget against the plan, and tests its slice. No baseline is skipped. Every shard preserves nonzero cargo-mutants exits, including MISSED; cancellation and timeout are not success.

The protected planner `scripts/mutation_ci.py` uses:

```text
shards = max(1, ceil(mutant_count / 20))
max_shard_size = ceil(mutant_count / shards)
minutes = ceil((180 + 1.5 * (361 + 194 * max_shard_size)) / 60)
```

361s is the observed baseline,194s the largest observed completed mutant,1.5 the explicit headroom multiplier, and180s a provisioning reserve (the observed setup-to-inventory was about16s). Even an empty shard receives13min for setup/baseline;20-mutant shards receive110min. Planning has10min and aggregation5min. Counts above5120 refuse explicitly at the256-job matrix ceiling; they never truncate the inventory. This bounded capacity must be reviewed if exceeded. Budgets do not include queue time or promise completion under arbitrary runner load.

305 mutants produce16 shards. A real cargo-mutants list-only dry run measured sizes `[20,20,20,20,20,20,20,20,20,20,20,20,20,20,20,5]`; the union exactly equals all305 mutants without duplicates. At the observed cost envelope plus margin, the largest shard models6541.5s, below6600s. Serial worst-observed-cost replay is59531s, well above60min and6h. These are **inventory/capacity dry-run results, not successful executions of all305 mutants or a hosted throughput benchmark**. Unobserved mutants and future suites can cost more; collect the first hosted shard timings and revise the envelope if necessary.

Parallelism is across runners, one cargo-mutants worker per runner. No speedup from unmeasured `--jobs2` or`4` is assumed. Sixteen shards replicate baseline/setup work and may queue subject to account concurrency and billing limits. The scheduled full-workspace audit uses the same scaling contract, avoiding its former fixed120min assumption.

The aggregator uses `always()` on eligible events and requires both planning and the entire shard matrix to report success. Failed, cancelled, skipped, missing or unknown dependency results refuse. `fail-fast: false` collects every shard result. Shards have no conditional skip or continue-on-error. Empty inventories get one verified shard, not an implicit missing-job success. A byte-empty Git diff maps explicitly to an empty inventory because cargo-mutants emits no JSON in that case; missing or malformed JSON for a nonempty diff refuses. Uploads use distinct shard names and run on failure too.

`check_ci` pins the reviewed orchestration blocks and budget-driver bytes, in addition to the job timeout contract; drift in coverage flags, matrix wiring, aggregate result checks or budget arithmetic requires review. These candidate-side pins do not replace trusted owner review under ADR0006. Tests plant missing shards/dependencies, skipped execution, altered budgets and ignored failure exits, and test count/capacity boundaries. The normal fast gate runs the planner tests.

References: [cargo-mutants sharding](https://mutants.rs/shards.html) documents zero-based indices and equal discovery arguments; [GitHub workflow syntax](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax) defines matrix dependencies, `always()` and fail-fast behavior.
