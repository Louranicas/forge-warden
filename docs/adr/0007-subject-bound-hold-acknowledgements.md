# ADR 0007: HOLD acknowledgements name exact evidence

Status: Accepted
Owner-deployment: Required

Accepted = design contract, not deployment; see [ADR status](../ADR_STATUS.md).

Historical status record: Status: proposed implementation, WP-D2. Author: Bastion (O4).

A persistent PR label is not fresh consent. An acknowledgement is a detached OpenSSH signature in namespace `forge-warden-ack`
by a reviewer enrolled in the trusted base, over a receipt naming repository, full base and head
IDs, SHA256 of the exact verdict bytes, every held rule ID, and a UTC Unix-seconds expiry. Changing any of these invalidates
it. The raw verdict stays HOLD; successful admission is recorded separately. FAIL, expired
waivers, unverified patch, unknown schema and **any verdict note** cannot be acknowledged.
This deliberately excludes unknown/missing coverage and other incomplete analysis, even if a
reviewer signs it. Only explicit policy hold rules are acknowledgeable. C1's verdict.v1 requires
an explicit reviewed adapter; there is no permissive schema fallback.

`scripts/hold_ack.py prepare --base BASE --head HEAD --verdict verdict.json --signer ID --expires-at UNIX_SECONDS`
prints an unsigned receipt. This is not approval. The reviewer obtains exact evidence through
the trusted controller, reviews it, then signs the receipt outside the candidate environment:

```sh
python3 -I /trusted/scripts/hold_ack.py prepare --base "$BASE" --head "$HEAD" \
  --verdict verdict.json --signer "$REVIEWER" --expires-at "$EXPIRY" > ack.json
ssh-keygen -Y sign -f /reviewer/private-key -n forge-warden-ack - < ack.json > ack.sig
```

The owner enrolls public keys and reviewer principals in the trusted base file
`.github/hold-reviewers/allowed_signers`, with namespace restrictions. Example line format:
`reviewer-id namespaces="forge-warden-ack" ssh-ed25519 PUBLIC_KEY_BASE64`.
No production keys ship with this change; absent keys fail closed. Private keys never enter the
repository, workflow, artifacts or test logs. The fixture suite creates disposable local keys
only. Reviewer enrollment/revocation and authentication are owner-controlled operations, not
inferences from a filename or claimed username. Revocation must update the controller's accepted
base; an old base is not acceptable merely because its Git object remains available.

The trusted controller passes optional fifth/sixth arguments `ack.json ack.sig` to the base
`dogfood.sh`. Only an actual judge exit 8 invokes verification. The helper reads bounded immutable
snapshots of verdict, receipt, signature and allowed-signers file; rejects duplicate JSON keys and extra
receipt fields; checks the subject/evidence/rule set; requires canonical JSON, rejects expired receipts using the trusted runner clock,
verifies the OpenSSH signature with `/usr/bin/ssh-keygen`; and
emits an audit envelope including exact signed bytes, signature and allowed-signers digest. Missing or
failed verifier leaves HOLD 8. Judge failures are never overridden. No code or key from the
candidate tree is used. Both Python and `/usr/bin/ssh-keygen`, the runner and the trusted base remain
in the runtime trust boundary. No same-uid arbitrary-write isolation is claimed.

## Deployment and invalidation

The external owner-controlled required workflow or merge-admission service from ADR 0006 must
supply authenticated **current** repository/base/head and freshly generated trusted evidence,
transport the receipt, and check the subject again atomically at merge admission. Never accept
head/base strings from the receipt as the expected subject. A synchronize/push creates a different
head; equality then rejects the old receipt automatically, without relying on label removal.
Concurrent push after a check requires platform subject-bound admission; this local helper cannot
atomically lock a remote branch. Before expiry, a rollback to the same base/head and identical verdict is the
same acknowledged subject, not a new freshness guarantee. Key revocation and approval withdrawal
must be enforced by that external controller. Expiry is enforced locally; no remote revocation service is claimed.

No hosted integration, signing-key provisioning or external settings are changed here. Repository
CI remains fail-closed on HOLD until the owner provisions the controller and trusted reviewer keys.
It does not fetch a PR artifact and trust its claimed signer. This avoids adding a privileged
candidate-controlled workflow. R-03 has an in-repo verifier/driver implementation; deployment
closure remains unmeasured and must be recorded separately from the local fixtures.

Rungs: separate trusted keys/code vs candidate data is architecture; exact subject/digest matching,
receipt parsing and signature verification are runtime admission. Types cannot prove freshness or
authorship of external bytes. Tests plant removed head/digest checks and signature bypass; a passing
fixture does not establish external deployment authority. The receipt is not a replacement for
coverage verification (C1) or permission to turn unknown evidence into PASS.

## Amendment (M7-Q61, 2026-11-15): who can acknowledge, and for how long

The text above is unchanged; this section corrects two readings of it.

**Who.** The acknowledgement is a feature of this repository's driver, not of the product. The
`forge-warden` binary has no acknowledgement option. `scripts/hold_ack.py` writes
`"repository": "Louranicas/forge-warden"` into every receipt and reads `allowed_signers` from the trusted
checkout it sits in, and `scripts/dogfood.sh` is the only caller. A deployer of the released binary
therefore has no acknowledgement path: a hold stays a hold until the change or the policy changes in
review, or until the deployer builds its own admission step outside the pull request. Whether the
product should carry one is an open owner decision (ledger Q61).

**How long.** The receipt binds the SHA-256 of the exact verdict bytes, and the verdict carries
`evaluated_on`, the UTC date of the run (`dogfood.sh` passes no `--today`). A run on a later UTC day
writes different bytes, so a receipt matches only runs on the UTC day it was prepared for; after
00:00 UTC it is refused with "acknowledgement does not match current subject and verdict", whatever
`expires_at` says. The effective lifetime is the earlier of `expires_at` and the end of that UTC day.
This fails closed. "Before expiry, a rollback to the same base/head and identical verdict is the same
acknowledged subject" above holds only within that day, because the next day's verdict is not
identical. Measured 2026-11-14 00:27Z (Keel, HOLD-ACK-TEST on 7bd9e655): a receipt accepted at
23:31Z the day before, replayed on the same change about 23 hours before its `expires_at`, was refused
(exit 8); the two verdicts differed only in `evaluated_on`.

**One enrolled reviewer can admit another.** A change to `allowed_signers` is held like any other
`.github/**` change (rule `quality-config-is-owned`), and that hold is itself acknowledgeable, so a
reviewer already enrolled in the trusted base can acknowledge a change that enrols a second one. This
is the "separately reviewed trusted-base change" of `.github/hold-reviewers/README.md`; the owner
should know that the review is one reviewer's signature unless the external controller asks for more.
(Read from `warden.toml` and this ADR; not measured.)

**First enrolment (HOLD-REVIEWER, 2026-11-15).** "No production keys ship with this change" above
describes the change that introduced the helper. One reviewer is now enrolled in
`.github/hold-reviewers/allowed_signers`: principal `luke`, the owner's key, fingerprint
`SHA256:ynROoorgFjRNx4689OUBe4eff4atYzIWvzuskmuF79E`, namespace `forge-warden-ack` only. The owner
made the key and placed its public half for enrolment; the private key never left the owner's machine.
The external controller of "Deployment and invalidation" still does not exist, so the enrolment
makes the local path usable, not the hosted one.

## Amendment (M7-RETIRE-GITHUB-GUARDS, 2026-11-16): the repository named in a receipt

Owner decision D3(c): the receipt's `repository` field is now `"lukeomahoney/forge-warden"`, the GitLab
repository, where this project's merges are admitted; it was `"Louranicas/forge-warden"`, the GitHub one. The
schema id stays `forge-warden.hold-ack.v1` and the field keeps its place, so only its value changes: a receipt
signed over the old value no longer verifies, and a new acknowledgement is signed over the new one. The path
`.github/hold-reviewers/` stays (D3(d)): moving it is a wire change for no gain. The "Who" paragraph above
names the old value; this amendment replaces it.
