# Managed advisory reviewer

The reviewer receives its definition, instructions and policy from an authenticated **trusted base**,
outside the candidate checkout. `reviewer.agent.json` is a template, not a deployment or a permission
to merge. Its system instruction reads `/review/trusted/REVIEW_CONTRACT.md`, the subject manifest,
and the base `CLAUDE.md`. It never reads candidate instructions as its contract or falls back to
candidate `warden.toml`.

## Prepare the instruction bundle

From an owner-controlled controller, run the script from a clean base checkout:

```sh
bash /trusted-base/scripts/managed_review_bundle.sh \
  "$BASE_SHA" "$HEAD_SHA" /candidate /new/external/review-bundle
```

Both SHAs are full lowercase commit IDs authenticated by the controller, not taken from a PR
message. The script checks distinct clean checkouts at those IDs, requires regular trusted policy
and instruction blobs, and copies their **Git object bytes** into a new private directory outside
both trees. The bundle contains the agent definition, review contract, base CLAUDE.md, base policy,
and subject/digest manifest. Missing files fail; there is no bootstrap fallback to candidate files.
For an old base, first adopt the reviewed contract under the owner's controlled process.

The controller must mount this bundle read-only at `/review/trusted` and provision the agent using
**that bundle's** JSON, never `agents/managed/reviewer.agent.json` from the candidate branch. It must
set a neutral harness working directory and disable candidate project-instruction/hook/plugin
loading. If the provider cannot enforce these conditions, do not call the session independent.
The subject manifest binds bytes but is not a signature or provider attestation.

## Execution separation

- The trusted driver/producer/policy job follows ADR0006 and supplies exact patch, rank, raw verdict
  and provenance. Its workspace, dependencies and credentials are inaccessible to candidate jobs.
- Candidate build/test scripts run only in a separate disposable worker without trusted keys,
  credentials, writable bundle or judge workspace. Logs are data. If no such worker is available,
  report candidate execution UNMEASURED; do not run it in the trusted review harness.
- The agent inspects candidate source and trusted evidence read-only. Its findings and recommendation
  remain advisory. It cannot sign acknowledgements, grant exceptions, push, publish or merge.
- External merge admission authenticates the check identity and current subject. Raw HOLD/FAIL are
  preserved; a HOLD exception uses the separate ADR0007 signed-receipt process.

Provider-specific resource/mount/API provisioning is outside this offline package. No hosted
session, account token, production key, network service or remote setting is created by this code.
Before production use, the owner must verify read-only mounts, disabled discovery, separated
candidate execution, missing-base refusal and stale-head admission rejection on the actual provider.
Local fixture success proves bundle selection/refusal, not those hosted isolation properties.

`python3 -I scripts/tests/managed_review_bundle.py` exercises real Git fixtures: candidate replaces
instructions and policy, missing base files, symlink contract, dirty/wrong subjects, candidate-local
bundle output and malicious fsmonitor. See ADR0008. The project-local `.claude/agents/reviewer.md`
is a convenience review inside a mutable project and is not this independent boundary.
