# Trusted HOLD reviewers

No production keys are provisioned. A repository owner may enroll public keys and reviewer
principals in `allowed_signers` through a separately reviewed trusted-base change. For example:
`reviewer-id namespaces="forge-warden-ack" ssh-ed25519 PUBLIC_KEY_BASE64`.
Private keys stay with reviewers, never in Git or CI. Signer IDs in receipts contain only
lowercase ASCII letters, digits and hyphens. The trusted base's file is authoritative; candidate
files cannot enroll a reviewer. Revocation requires the external admission controller to use the
new trusted base. See ADR 0007. A bare label, username or unsigned receipt grants nothing.
