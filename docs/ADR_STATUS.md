# Architecture decision status

Every numbered ADR has exactly one active line: `Status: Proposed`, `Status: Accepted`,
`Status: Superseded-by-NNNN`, or `Status: Deprecated`. Historical status records are preserved
below it. Acceptance records a design decision; it does not assert deployment, completion or
release. ADR 0011 is accepted design with implementation still queued, as its text states.
ADRs 0006–0010 describe accepted contracts already used by the gate; their earlier proposal
labels remain dated history. External deployment and owner provisioning remain separate.

An Accepted ADR is current within its stated scope. Proposed is under discussion and is not
binding; Deprecated is retired without a replacement. Superseded-by-NNNN retires the whole ADR
in favor of an existing later numbered decision. Every such replacement has a reciprocal
`Supersedes: NNNN` line in the replacement ADR; multiple numbers are comma-separated. These
fields are the only whole-ADR replacement claims. Refinements of one section remain explicitly
scoped prose and do not silently retire another whole decision. Replacement chains must move
forward and end in an Accepted decision, so there is one current endpoint per chain.

The documentation contract rejects missing/duplicate/unknown statuses, dangling or backward
replacement links, mismatched reciprocal links, and an older Accepted ADR claimed by a later
replacement. It does not infer semantic contradiction from arbitrary prose; authors must state
whole-decision replacement using the structured fields. Changing the decision still needs review.

ADR 0001's effect-door boundary includes child process execution: see ADR 0005 and ADR 0010
for the bounded child and isolated Git-data contracts. This clarification extends the original
examples; it does not move effects into the pure core. Current decision text is in [the ADR
set](adr/0001-pure-core-and-effect-doors.md), starting at 0001 through 0011.

Every ADR also declares exactly one `Owner-deployment: Required` or
`Owner-deployment: Not-required` field. Required means the decision specifies separately
owned runtime, repository, gate, or trust configuration; it is not an implementation-complete
flag. The reviewed Required baseline is ADRs 0006 through 0011. ADRs 0001–0005 describe
source-level architecture, lint and evaluation contracts without a separate deployment claim.
New ADRs must declare their classification explicitly; the checker does not infer it from prose.
Every Accepted Required ADR carries the exact design-versus-deployment pointer used in those
six records. Missing/duplicate fields, downgrading a baseline record, and missing pointers fail.
