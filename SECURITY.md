# Security

## Threat model

forge-warden reads two untrusted inputs: a ranking (written by deep-diff-forge from a patch
someone else wrote) and, with `--patch`, the patch bytes. It trusts its policy file, which lives
in the repository under review and is owned (CODEOWNERS, `protect_paths`).

| Threat | Mitigation |
| --- | --- |
| Terminal-escape injection or Trojan-Source bidi text in a path | Human output escapes every character outside printable ASCII (`render::terminal_safe`, property-tested). JSON output is RFC 8259-escaped by serde_json. |
| Memory exhaustion | Inputs over 64 MiB are refused before parsing (`MAX_INPUT_BYTES`), using a bounded read. |
| A verdict for one patch attached to another | `--patch` recomputes the seal and refuses a mismatch (exit 4). |
| Fail-open on new or empty input | ADR 0003: unknown vocabulary and empty rankings hold; they never pass. |
| A pull request weakening the gate that judges it | The dogfood job judges every pull request with the **base branch's** `warden.toml`, so a PR cannot loosen the rules applied to itself. Changes to the policy, lints, CI and hooks are `protect_paths` holds and CODEOWNERS-owned. |
| Supply chain | `cargo-deny` (sources, licences, advisories, bans), Dependabot, actions pinned by SHA, `zizmor` workflow audit. |
| Panics as denial of service | No `unwrap`/`expect`/`panic`/indexing in production code (lint wall); a panic exits 101 and is a bug. |

forge-warden has no `unsafe` code (forbidden), spawns no processes, opens no network
connections and writes no files.

## Reporting

Please report vulnerabilities privately through GitHub's "Report a vulnerability" on this
repository rather than in a public issue.
