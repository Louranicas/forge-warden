# Score-family qualification (WP-JEV-Z3 B6)

This document is normative for how forge-warden evidence about a producer's risk scores may be
stated. It is a **specification**: it adds no policy member, exit code, runtime check or receipt
checker. Each requirement below is labelled with what currently enforces it. Where the label is
**specification only, not tested enforcement**, nothing in this repository fails when the
requirement is broken.

Source design: Touchstone's Z3 B6 design v1 (`SCORE-FAMILY.md`, SHA-256 `77463d72…`), reviewed
in the Z3 contract review.

## 1. The problem

A producer allowlist admits **bytes**: the exact SHA-256 of a deep-diff-forge executable. Being
admitted says nothing about what that executable's integer scores mean. Two producers can emit
structurally valid, sealed rankings with the same tool version string and different score units.
A `max_score` limit written for one is then silently wrong for the other. Admission must not be
read as qualification of score semantics.

## 2. Decision

- Qualification is an **offline, reviewed receipt** per producer digest, policy digest and
  fixture revision. It is evidence for an owner's enrollment decision, not a runtime input.
- A candidate with no matching receipt is **NOT QUALIFIED for owner enrollment**. That is a
  release/admission state for people. It is not a forge-warden verdict, HOLD or exit code.
- No receipt is inherited. A new producer digest, policy bytes, task family or fixture revision
  needs its own receipt, even when the version string, output schema or sampled scores agree.
- **Nothing automatically updates a pin.** No receipt, script or test may add an allowlist
  digest, change a limit, rescale scores or transfer thresholds. Only the owner enrolls a digest
  or changes trusted limits; agents prepare reviewable evidence.
- Supporting several incompatible score families at once would need an approved
  domain/compatibility ADR and a wire/schema migration first. Until then, unknown policy fields
  remain errors.

## 3. Receipt contents

Status: **specification only, not tested enforcement.** No receipt format or checker exists.

A receipt records:

- the forge-warden source or release subject and verifier version;
- the producer executable SHA-256, tool version and source revision;
- the policy SHA-256 (including its allowlist);
- the fixture manifest revision and SHA-256, and the expected-assertion manifest SHA-256;
- the runner SHA-256, the exact argv, locale and environment, platform and runtime pins,
  budgets, and the seed if sampled;
- the emitted ranking digest and patch seal for each fixture, and the raw result and log digests,
  not only a pass/fail summary;
- the declared task family: patch encoding, supported text/binary/mode shapes, signal and status
  vocabulary, the units and meaning of the integer score, ordering, per-file versus change-sum
  aggregation, and scope; and the shapes that are unsupported and so HOLD or refuse;
- who wrote the expectations, who reviewed them, the dated owner enrollment decision (pending
  until it is actually made), the result, its limits and exclusions, and the predecessor receipt.

A receipt keeps these states distinct: source-inspected, offline-passing, owner-admitted,
deployed, useful. Hashes identify bytes. They do not authenticate an owner or prove that a score
is true. Deep-diff-forge integers are not probabilities or confidence bands, and deterministic
fixture comparisons support no statistical or calibration claim.

## 4. Fixture families and their status

### 4.1 Policy arithmetic (forge-warden's side)

Status: **tested.** The Z3 frozen cases, judged through the public authority API by
`crates/forge-warden-core/tests/z3_authority.rs` against the independent oracle
`fixtures/adversarial/z3_oracle.rs`:

| Case | Boundary | Expected |
| --- | --- | --- |
| P12 | file score 10, per-file limit 10 | pass (strictly greater fails) |
| P13 | score 0, limit 0 | pass |
| P14 | 6 + 4 across two files, change limit 10 | pass |
| P15 | score and limit `u32::MAX` | pass |
| N22 | score 1, per-file limit 0 | fail |
| N23 | change sum above `u32::MAX` | fail (the sum does not wrap) |
| N24 | score 11, limit 10 | fail |
| N25 | change sum 11, limit 10 | fail |

The guardrail plants `z3-ge-instead-of-gt` and `z3-u32-change-sum` (G-45) show that the harness
catches a `>=` limit and a 32-bit change sum. These cases fix how forge-warden **interprets**
supplied integers. They say nothing about a producer's scoring algorithm.

### 4.2 Real producer scores

Status: **BLOCKED.** Numeric qualification of a real deep-diff-forge executable needs an
owner-reviewed score contract for that producer and source revision: what each integer means and
how it is computed. None exists. Expected values must come from reviewed scoring rules or
independent hand calculation, never from the candidate's own run, and no true-risk labels may be
invented to fill the gap. When a contract exists, the fixtures run the exact admitted executable
offline, one identity or semantic fault at a time.

### 4.3 Family separation

Status: **specification only, not tested enforcement.** The control: producers A and B both emit
valid sealed rankings, B's scores are A's times 10 over a non-overflowing domain, and both report
the same version string. Under one limit of 10, A's 10 clears and B's 100 violates, so schema
validity and allowlist membership cannot certify shared semantics. A and B have distinct receipt
identities. B may later get its own receipt and an owner-approved policy; nothing rescales or
tunes limits to make a fixture pass.

What Z3 does test nearby: a producer digest outside the allowlist is refused at admission (N28),
and seal fields claiming a different producer or version do not change provenance (P20).

### 4.4 Scope and shape

Status: **partly tested** by Z3. A correct score does not excuse an independent hazard (N02,
N11, N15), a missing or extra path (N04, N05), refused Git facts (N10, N20, N21) or supplied
provenance (N01). Deleted and binary shapes keep their companion and signal behaviour (N14, N17,
N18). Unknown vocabulary holds (N12, N13). The generated laws in the same test file cover these
properties beyond the fixed rows:
`law_one_coverage_fault_names_itself_and_never_loosens_a_failure`,
`law_a_supplied_ranking_never_passes_and_a_failure_stays_failed`,
`law_adding_a_rule_or_a_diagnostic_never_loosens` and
`law_an_omitted_git_hazard_still_reaches_its_rule_and_producer_hazards_survive`. Z3 always scopes
rules to `**`, so it makes **no** claim that out-of-scope scores stay out of an in-scope budget.

### 4.5 Receipt refusal

Status: **specification only, not tested enforcement.** The control: change only the producer
digest, the policy digest, the fixture revision or bytes, or the expected-manifest digest, and the
receipt matcher must find the old receipt inapplicable. Two versions with identical scores on
every selected example still need separately identified evidence. If owner signatures are later
required, they need their own trust design, not filename matching.

## 5. What this does not claim

Passing core fixtures does not prove that a real producer ran. An offline recipe does not prove
that a hosted admission rule exists. No measured score family shows that a score predicts risk.
Those need separate evidence, recorded in a receipt as above.
