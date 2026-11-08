# ADR 0007: HOLD acknowledgements name exact evidence

Status: Accepted

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
