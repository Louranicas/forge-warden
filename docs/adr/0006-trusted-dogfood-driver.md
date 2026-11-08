# ADR 0006: The candidate is data to a trusted dogfood driver

Status: Accepted

Historical status record: Status: proposed implementation, WP-D1. Author: Bastion (O4).

## Problem

Base policy does not protect a gate if the candidate supplies the executable or driver interpreting it. The former dogfood job built the candidate and ran its script. A candidate could replace either with success. Workflow YAML is also candidate-controlled; changing checkout paths inside that YAML cannot by itself establish an immutable trust root.

## Decision

A trusted workflow supplies full base/head commit IDs and separate checkouts. The driver must run from the clean base checkout, confirm its HEAD equals base and reject a shared candidate checkout. Candidate changes are acquired only by an explicit Git diff of immutable IDs, with external diff/textconv disabled and replace objects disabled. Candidate scripts, build.rs, Cargo manifests/configuration and policy are never executed/read as judge configuration.

The base source, lockfile, build configuration and warden.toml produce the judge and its policy. A producer source revision is a literal in that base driver. It is installed into the new private evidence directory, not a user's cargo bin. Compiler and producer failures stop before judgment; every judge status is propagated, including HOLD. A bare PR label no longer acknowledges a HOLD; WP-D2 must provide a verified subject-bound attestation before any exception can be accepted.

Evidence lives outside both checkouts: exact patch/rank/policy/verdict, full base/head and producer revision, and executable/input digests. These hashes identify bytes; they are not signatures or build attestations. No candidate artifact is reused to build or rank. This job uses a fresh runner and no cross-PR build cache. The trusted driver must be executable code from the base, not the candidate. Its explicit v1 marker is checked before invocation so an old base driver cannot silently receive the changed argument protocol.

## Trust root and limits

The workflow must be enforced independently of candidate edits, e.g. an owner-controlled pinned required workflow or external gate whose identity is checked by the merge/release authority. A same-named check from a candidate-defined workflow must not satisfy approval. Repository owners must configure and verify this outside the PR, with branch/release admission rejecting a mutated workflow and stale subject. No external repository setting is changed by this WP; hosted acceptance remains UNMEASURED. Until enforced, this repository workflow is evidence automation, not independently authenticated permission to merge. Never solve the problem by executing untrusted PR code with privileged pull_request_target credentials.

The runner, Git, Rust/Cargo, base policy/code, producer revision, dependency resolution constrained by lockfiles, and artifact transport are trusted. No resistance to an arbitrary same-uid writer or compromised host is claimed. The driver rejects a dirty base, including ignored files; no build target lives there. Candidate jobs must not share this runner/workspace, credentials or mutable compiler cache. Read-only GitHub token and no persisted checkout credentials limit effects but do not constitute process isolation.

Bootstrap: adopt the reviewed driver into the trusted base under the owner's controlled process before expecting dogfood to pass; a base lacking its v1 marker fails closed. Never fall back to candidate code to break this cycle. Updates to the driver change the future judge, not the judge assessing the current change.

Current check still has the baseline rank-input trust limitations until WP-C1 is implemented. Generating rank in this trusted job reduces exposure but does not prove producer correctness or apply-to-base truth. C1's new CLI contract must update this base driver under review. This WP does not complete R-01, R-03 or the external enforcement part of R-02.

## Guarantees and verification

Highest rung is architecture: authoritative code/configuration and candidate data occupy separate trees; only the former is built. Runtime admission rejects wrong/dirty base and invalid commit IDs. Types cannot eliminate filesystem/process identity failures in a shell driver.

scripts/tests/dogfood.sh constructs real Git base/candidate commits. Candidate changes driver, build marker and policy; fake compiler asserts it is invoked in the base with the base marker. Exact patch must contain candidate changes, copied policy must equal base. PASS/FAIL/HOLD, rank/build errors and wrong/dirty base are checked with controlled subprocesses and no network. These are orchestration fixtures, not live CI or producer qualification. Planted mutants must demonstrate sensitivity to candidate compilation, policy replacement and swallowed judge failure. Hosted trust-root enforcement needs a separate owner acceptance test; static YAML review cannot prove it.

## Verdict v1 CLI migration (WP-D8)

The trusted driver copies its own policy unchanged. A `[producer]` section selects authority
mode: the judge alone pins, admits and runs the built ranker with `--ranker`. A source revision
pin is not a binary SHA-256 enrollment; the driver never inserts the freshly built hash into the
policy. An empty, malformed or mismatched producer allowlist remains an authority refusal,
never an automatic downgrade.

Without a producer section the driver explicitly uses `--advisory --rank`, reports that it
cannot PASS, and preserves HOLD/FAIL. Advisory HOLD cannot enter the signed-acknowledgement path.
Evidence records the selected mode; rank.json exists only for advisory runs. Authority evidence
records the actual producer digest in the verdict and hashes the binary in provenance.txt.
Base/head are passed as unverified claims; the separate signed receipt still binds the external
subject and exact verdict digest.

ci_local uses the same explicit selection against its working policy; it is a local development
check, not the independently trusted policy/driver authority described above. Both entrypoints
propagate ranker/admission failures; neither retries a refusal in a weaker mode.
