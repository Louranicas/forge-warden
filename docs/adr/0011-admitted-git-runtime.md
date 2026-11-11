# ADR 0011 — Admit the Git runtime once and record its identity

Status: Accepted
Owner-deployment: Required

Accepted = design contract, not deployment; see [ADR status](../ADR_STATUS.md).

Historical status record: Status: **Accepted** by Lodestar O1 on 2026-11-08, with amendments A1-A3. Author: Bastion O4. Luke exclusively owns allowlist enrollment/revocation as an L4-class root. This decision changes no effective trust entry or runtime by itself. Implementation is queued after Q-TYPES-1 merges: builder Keel O2, independent verifier Touchstone O3, boundary reviewer Bastion O4. Builder and verifier remain distinct.

Reference baseline: hub main 2ac9f596ac39a4c12c9177529d5451fa98e181c3. This refines WP-GIT-PIN and the Mission5 proposal; acceptance does not make the unimplemented controls current behavior.

## 1. Schema and seal compatibility

**Use a minimal, explicitly selected verdict v2. Do not add fields to the existing closed v1 contract.** Current `schemas/verdict.v1.schema.json` closes every object, including `seals.git` (`additionalProperties: false`, only path/version). An optional digest still breaks an old strict reader whenever present. `scripts/hold_ack.py` also explicitly requires the v1 identifier. Rust non_exhaustive annotations do not relax the wire schema.

V2 changes only the schema identifier and Git identity record needed for this guarantee. Retain path and diagnostic version. Add required lowercase SHA256 strings `seals.git.sha256` and `seals.git.allowlist_sha256` when Git evidence is present. The latter hashes the exact bounded, owner-provisioned allowlist bytes used for admission, before parsing; it identifies the admission snapshot, not a signer or proof of approval. The v2 authoritative route cannot PASS without an admitted Git record. Represent absence explicitly for nonauthoritative/failed evidence; do not synthesize zero hashes. Final schema conditions and fixtures must enforce this distinction.

Keep existing policy, rank, patch and patch-facts digest definitions unchanged. In particular patch_facts_sha256 stays SHA256(numstat bytes || NUL || summary bytes); do not concatenate the Git hash into an existing digest under an unchanged name. These are input digests, not a signature of the verdict. The complete verdict bytes already bound by HOLD acknowledgement digest will include the new record; any byte change requires a new acknowledgement. The controller must validate provenance against its trusted allowlist, not believe a candidate-written JSON field.

Release sequencing (A1): v1 remains in Unreleased and has not shipped in a tagged release; 0.1.0 carries v0. If GIT-PIN lands before the 0.2.0 tag, 0.2.0 ships v2 directly and no tagged release carries v1. Otherwise 0.2.0 ships v1 and v2 follows in 0.3.0, with required-v2 downgrade refusal. The R-0.2.0 release checklist must record this branch decision before tagging. Preserve v1 fixtures for compatibility evidence; keeping legacy output available is not a requirement to ship an otherwise unnecessary tagged v1 release.

Migration:

- Preserve v1 schema and byte fixtures. Version-aware tooling explicitly requests/accepts v2 (proposed CLI `--verdict-schema v2`); old strict readers continue receiving v1 on the legacy path, and reject v2 clearly if misrouted. Never silently add v2 fields under a v1 label.
- The new Git-admitted authority contract requires v2. If a controller requires this contract and the caller requests v1, refuse that combination rather than strip the evidence and emit an admitted PASS. Legacy v1 remains legacy and must not be presented as satisfying ADR0011.
- Update trusted CI/dogfood consumers, schema command and HOLD acknowledgement validation together before requiring v2. The acknowledgement payload schema can remain v1 if its exact existing fields/signature rules remain unchanged: its verdict_sha256 binds either supported verdict's exact bytes, while the trusted validator adds explicit v2 admission checks. Do not weaken existing subject, provenance, expiry or note checks.
- No automatic golden regeneration/blessing. Review the small v1/v2 fixture delta and prove old v1 unchanged, unknown fields refused, v2 missing/malformed identity refused and downgrade refused under a v2-required controller.

Rejected: additive v1 fields (strict-reader break); sidecar as the sole authoritative identity (two-file association and accidental omission); changing patch-facts hashing (unnecessary semantic break). V2 is the smallest honest wire change for carrying this evidence. Rollback may disable the new mode while leaving old readers intact, but a controller requiring v2 must stop/refuse until restored, never silently fall back to legacy acceptance.

## 2. Runtime trust contract

**Luke is the sole allowlist owner. The hive may propose entries and evidence, never enroll or revoke them.** Lodestar reviews/integrates code and the ADR; that does not delegate Luke's enrollment authority. No task, repository policy, generated fixture or successful hash comparison can confer it.

Use `/etc/forge-warden/git-allowlist.toml` as the deployment-default external trust file, root-owned and not writable by the runtime/candidate account, with protected parent directories and bounded regular-file/no-symlink admission. It is outside the candidate checkout. A trusted service/controller may mount this owner-provisioned file at a fixed runtime path; an alternative path must be deployment configuration, not candidate CLI/environment/CWD discovery. Do not rely on a filename alone to prove owner control. Luke's administration installs/replaces it; runtime only reads it.

Proposed small closed document: schema `forge-warden.git-allowlist.v1`, monotonically tracked owner revision, and a list of approved executable SHA256 digests. Reject duplicates, unknown fields, malformed digests, empty/missing policy and oversized files. Read/hash/parse one snapshot per verdict. Build origin, review and revocation reasons belong in the owner change record, not an automatically trusted version string. No trust-on-first-use, PATH enrollment, downloading an executable to match a version, candidate-controlled alternate allowlist, or automatic update entries.

Provisioning sequence: hive submits exact build digest plus reproducible/package provenance and test evidence; Luke independently approves the entry; the owner-controlled deployment atomically installs the reviewed allowlist and approved runtime artifact. An agent may prepare a proposal file but must not write the effective trust file or translate general zero-touch coding authority into enrollment.

First enrollment (A3): Luke installs the first approved Git entry using root administration. The hive never substitutes a bootstrap entry. Missing `/etc/forge-warden/git-allowlist.toml` must fail closed, with an explicit regression test; absence is not legacy-mode permission when the controller requires the new contract.

Revocation: Luke removes the digest and records why/replacement policy. New evaluations reload and refuse it. An evaluation already holding a snapshot is not retroactively revoked by editing a file. For urgent revocation, the operator cancels/drains in-flight jobs and their outputs; the integrator revalidates historical admission against current owner policy before acting. Bind the observed allowlist digest in v2 for audit, but do not treat an old allowed hash as indefinitely authorized. Never auto-restore a revoked entry during rollback.

Artifact stability: initially require an owner-provisioned, root-controlled read-only runtime artifact/mount inaccessible to candidate writes, with its relevant parent paths controlled. Existing ranker pinning allows current-uid-owned files and owner write permission; that predicate alone is insufficient for a stronger Git same-bytes claim. An open descriptor pins an inode, not immutable contents. Trusted root can still change runtime state; protected provisioning and not modifying in-flight artifacts remain explicit L4 assumptions. No sealed-copy mechanism is mandated in this smallest first implementation; designing one later requires separate execution/loader compatibility evidence.

The loader, libraries, Git installation/support files, kernel and controller remain trusted. A Git executable digest does not attest that entire closure or sandbox Git. Same-uid controller compromise is outside the claim. Missing or unusable runtime policy is a runtime refusal, not a candidate patch HOLD that a reviewer can acknowledge into PASS.

## 3. Admitted Git type and execution boundary

**Use a private CLI-door type `AdmittedGit`, distinct from `PinnedExecutable` and from the serialized `GitOracle`.** Its fields are private: the one held executable descriptor, executable digest, owner-policy snapshot digest, and diagnostic source path. No public constructors, Deserialize, From<Path>, or digest-only token constructor. Only a fallible admission function in the trusted effect door creates it after checking the external policy and artifact. A generic pinned ranker is not automatically admitted Git.

Read the external owner policy once; open/hash the executable once with bounded regular-file/ownership/mode and symlink checks; compare its descriptor-derived digest with the owner snapshot; retain that descriptor. `git_version`, numstat and summary/facts operations accept `&AdmittedGit` and launch that same descriptor. They never reopen the diagnostic path or accept an alternate Path in the production call chain. Linux descriptor execution and `/proc/self/fd` availability are required; unsupported/script execution modes refuse explicitly rather than silently falling back. Tests may use reviewed test-only fixture constructors, never a production feature that bypasses owner admission.

Carry Git identity and facts from the same operation context into v2. A pure-core `GitOracle` record remains data: constructing one must not constitute executable admission. Prefer a CLI-local result object grouping facts and admitted identity, consumed by the v2 verdict-building path, to avoid matching separate arbitrary digest strings after the fact. The pure core does not gain filesystem/process access or a claim that it independently observed Git. The trusted controller and effect door establish runtime provenance.

Preserve ADR0010 environment/config/TMPDIR isolation, immutable patch input snapshot, child lifetime/output/timeout limits, and existing typed refusals. Executable admission failure refuses before evaluation; Git refusing patch syntax retains the existing no-PASS evidence semantics. These are distinct outcomes.

## Acceptance evidence and allocation

Before implementation is READY, an independent verifier must demonstrate:

1. Same admitted descriptor across facts/version despite pathname replacement; approved artifact succeeds, a different binary spoofing the version and missing/revoked/invalid policy refuse.
2. Wrong-owner, candidate-writable runtime, symlink/FIFO/oversize and unsupported execution refusal. Demonstrate the protected artifact provisioning assumption; do not call a descriptor-only test proof against in-place writes.
3. Compile-time refusal for unadmitted-path entrypoints and no exposed constructor; legitimate admitted route succeeds. Review all production Git call sites.
4. Exact v2 executable/allowlist digests and unchanged patch-facts digest; strict v1 fixtures unchanged, v2 schema/ack/controller migration proven, required-v2 downgrade rejected.
5. Revocation applies to the next evaluation, and canceled in-flight evidence cannot be integrated under current revoked policy.
6. ADR0010 regressions plus the current mandatory fw-verify gate (plants, mutants, MSRV); controlled fixture policy is test-only and is not enrollment into Luke's deployment.

Implementation sequencing (A2): begin only after Q-TYPES-1 merges because both change the authority/provenance boundary. Keel builds; Touchstone independently verifies; Bastion reviews the boundary as ADR author, not builder. Freeze the actual post-merge implementation base before work starts. Luke's first and subsequent enrollment remain separate explicit owner acts. This accepted decision contains no enrolled hashes and performs no runtime deployment.

## Amendment (2026-11-10): what shipped differs from the migration plan

The implementation emits `forge-warden.verdict.v2` unconditionally. The `--verdict-schema v2` flag proposed under
Migration was not built, and no code path emits a v1 or v0 verdict (no `verdict.v1` or `verdict.v0` string in
`crates/*/src`). So a caller cannot request v1, and the downgrade the Migration bullets guard against has no route.
Consumers that still read v1 must move to v2; the 0.2.0 upgrade notes state it. Git admission itself is as designed:
the allowlist is read from `/etc/forge-warden/git-allowlist.toml` (`doors/child/git.rs`) and the verdict records
the admission (`Approved` or `Unpinned`, `verdict.rs`).
