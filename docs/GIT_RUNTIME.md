# Git runtime admission

Authority checks require an owner-enrolled Git executable. The CLI reads only
`/etc/forge-warden/git-allowlist.toml`; there is no policy-path flag, environment override,
repository policy, automatic enrollment, or path-execution fallback. An absent policy gives
`no Git allowlist enrolled` and no authority verdict. A malformed or unsafe policy refuses
both modes. A genuinely absent policy permits advisory output only: `git.admission` is
`unpinned`, `allowlist_sha256` is null, and the `rank_unverified` note explicitly names the
unapproved Git identity. Advisory evidence cannot authorize an acknowledgement.

This is the Mission 6 R2 amendment to ADR 0011: hosted advisory dogfood can report evidence
without enrolling the runner's Git. Authority still requires the owner policy. Since the
repository's own policy pins a producer, `scripts/dogfood.sh` would choose authority mode; the
hosted job therefore passes `--hosted-advisory`, written in the workflow, and the driver says why
in its output and in `provenance.txt`. Without that argument a pinned policy and no enrolled
allowlist end in exit 3 (`no Git allowlist enrolled`), never in a quiet downgrade. Current
output is verdict v2; the historical v1 schema is retained, not emitted.

## Owner enrollment

Luke alone enrolls artifacts. Obtain each approved executable digest from an independent,
reviewed artifact/provisioning record. Hashing whichever Git happens to be installed is an
identity measurement, **not approval**. Compare that measurement with the approved record
before enrolling it. Agent test fixtures never modify the effective policy.

The policy is UTF-8 TOML with exactly these keys:

```toml
schema = "forge-warden.git-allowlist.v1"
revision = 1
sha256 = ["0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"]
```

The displayed digest is an example, not an approved artifact. Digests must be unique,
64-character lowercase hex strings; the list must be nonempty. Revision must be a positive
integer. Luke tracks revision increases in the provisioning record: the CLI has no persistent
anti-rollback counter. Unknown fields and duplicate keys are refused. Maximum policy size is
64 KiB; maximum executable size is 512 MiB.

After preparing and reviewing a real `git-allowlist.toml` locally, Luke can install it with:

```sh
sudo install -d -o root -g root -m 0755 /etc/forge-warden
sudo install -o root -g root -m 0644 git-allowlist.toml /etc/forge-warden/git-allowlist.next
sudo mv -T /etc/forge-warden/git-allowlist.next /etc/forge-warden/git-allowlist.toml
```

These are operator instructions, not commands run by the implementation or its tests.
Every policy and executable ancestor must be a real root-owned directory, writable by
neither group nor others. The final files must be regular, root-owned, and not writable by
group or others. Symlinks are refused. The executable must have an execute bit and ELF magic;
scripts are refused. Unsupported descriptor execution fails without retrying the path.

## Snapshot and revocation

Each evaluation opens and hashes one executable descriptor and reads one policy snapshot.
Git version, numstat, and summary all execute through that descriptor. The verdict records
the executable digest and the SHA-256 of the exact policy bytes, including whitespace.
Policy, patch, rank, and patch-facts seal definitions remain unchanged.

Replacing a pathname does not replace an already open descriptor. Modifying its inode in
place can change its bytes: root/provisioning must never mutate an in-flight executable.
To revoke, install a reviewed next policy and drain or cancel evaluations admitted under
the previous snapshot. New admissions use the new policy. The trusted acknowledgement
controller also reads the current policy and refuses stale snapshot hashes, revoked Git
digests, unpinned identity, and v1 downgrade. The candidate's JSON is evidence, not enrollment.

This is not a sandbox or an immutable execution image. The kernel, loader, libraries, Git
support files, root/provisioner, and trusted controller remain trusted. It provides no new
same-uid, L3 runner, or L4 owner-compromise containment guarantee.

## Test-only fixture mechanism

Tests compile the same CLI with `cargo rustc --bin forge-warden -- --cfg test --cfg
fw_fixture_cli` into a separate target directory and select its executable from Cargo JSON.
Every policy override is behind `cfg(test)`; the second cfg only excludes unit-test helper
modules from that standalone fixture executable. Ordinary unit tests retain those modules.
The production artifact has no fixture policy interface. Cargo declares the cfg name, and
repository checks keep fixture compiler switches out of workflows, scripts, recipes, and
Cargo configuration. Release-artifact refusal and string checks are separate evidence.

Why this mechanism: `harness=false` omits test functions while retaining their helper modules,
which correctly fails the deny-warnings build. The explicit fixture compilation preserves
the lint wall and gives integration tests a disposable owner policy without a production
feature or effective-host enrollment. See ADR 0011 and the Mission 6 GIT-PIN R3 refinement.


Every check-mode integration test supplies a fresh policy root: empty for advisory,
explicitly populated for authority positive controls, and empty for authority refusal controls. The fixture refuses a missing root rather than reading
the host policy. The tutorial and golden producer use that same test-only artifact.
The Rust helper publishes an artifact SHA-256, exact cfg set and a content-addressed
source-input subject. The operator passes all three artifact-pointer fields explicitly;
the producer verifies the receipt before descriptor execution. This is local reproducibility
evidence, not a signature or a production admission mechanism. Production absence and override
refusal probes remain separate from the fixture suite.

Why a source-input subject: dirty checkouts, Git-less exports and cargo-mutants copies must
remain testable. The source digest does not assert a commit; the verifier separately binds
the checkout to the frozen subject. Admission and the policy hash remain visible in golden
output; only the measured executable digest and version are normalized.

The core's Verdict does not implement Clone. Reporting borrows the authority result or the
advisory wrapper's shared verdict directly; cloning that value to edit it fails compilation.
The exact T-GITPIN-01 clone-and-shadow case is a compiler plant, independent of goldens.
The AST check is only a secondary check for the current direct capture and single report
shape. It does not establish general data-flow integrity; Q-TYPES-2's private fields supply
the separate field-mutation barrier.

## Authority wiring proof

The CLI exposes two named admission constructors. `admit_git_pinned` returns a private-field
`PinnedGit` proof after required policy admission; `admit_git_advisory` returns `AdmittedGit`,
which may record an absent policy. There is no mode flag on either constructor and no
conversion from general admission to the proof. The authority facts builder requires
`&PinnedGit`; replacing its constructor with advisory admission fails compilation. The
same admitted descriptor supplies version and facts. Core stays independent of CLI I/O types.

`authority_requires_git_policy_before_producing_a_verdict` exercises the full fixture CLI
with an admitted ranker and an explicitly empty policy root: exit 3, named admission refusal,
and no verdict. The `git-authority-mode-wiring` compiler plant checks constructor substitution;
`git-missing-policy` separately checks the admission implementation's required-policy branch.
These controls do not claim arbitrary data-flow integrity or protect against changes to the
trusted proof-minting implementation itself without review and testing.
