# Trusted HOLD reviewers

No production keys are provisioned. A repository owner may enroll public keys and reviewer
principals in `allowed_signers` through a separately reviewed trusted-base change. For example:
`reviewer-id namespaces="forge-warden-ack" ssh-ed25519 PUBLIC_KEY_BASE64`.
Private keys stay with reviewers, never in Git or CI. Signer IDs in receipts contain only
lowercase ASCII letters, digits and hyphens. The trusted base's file is authoritative; candidate
files cannot enroll a reviewer. Revocation requires the external admission controller to use the
new trusted base. See ADR 0007. A bare label, username or unsigned receipt grants nothing.
An acknowledgement's `expires_at` must be in the future (`scripts/hold_ack.py`), and the signer
chooses it. It is not the only limit: the receipt binds the SHA-256 of the verdict's bytes, and the
verdict carries the UTC date of the run (`evaluated_on`). A run on a later UTC day writes a different
verdict, so the receipt stops matching at 00:00 UTC after the day it was prepared for, whatever
`expires_at` says (measured 2026-11-14: a valid receipt replayed 23 hours before its expiry, on the next
UTC day, was refused). It also stops matching when its signer is removed from the trusted base.
This acknowledgement works only for this repository's own driver (`scripts/dogfood.sh`); the
`forge-warden` binary has no acknowledgement path.
