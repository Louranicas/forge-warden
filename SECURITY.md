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
| A pull request weakening the gate that judges it | The dogfood driver builds the judge and loads policy from a separate, clean **base checkout**; the candidate is diff data only. This requires a trusted workflow/runner enforced outside the candidate PR. A mutable repository-local workflow is not an independent approval boundary. See ADR 0006 for bootstrap, isolation and deployment prerequisites. |
| A substituted ranker or Git | Authority mode runs only a deep-diff-forge whose SHA-256 is listed in the policy's `[producer] sha256`, and only a Git whose SHA-256 is enrolled in the owner's allowlist `/etc/forge-warden/git-allowlist.toml` (ADR 0011); with no allowlist, or an unlisted digest, it refuses. Each must be an executable ELF, run from the descriptor that was hashed. With no allowlist installed, advisory mode still runs Git and records `Unpinned` in the verdict; it can at best hold. With an allowlist installed, an unenrolled Git is refused in both modes. |
| Supply chain | `cargo-deny` (sources, licences, advisories, bans), Dependabot, actions pinned by SHA, `zizmor` workflow audit. |
| Panics as denial of service | No `unwrap`/`expect`/`panic`/indexing in production code (lint wall); a panic exits 101 and is a bug. |

forge-warden has no `unsafe` code (forbidden) and opens no network connections. Every child
process it starts goes through the door modules (`src/doors/`). Two run inside the bounded wrapper, with a cleared
environment, a timeout and capped output:
- in authority mode, the deep-diff-forge binary the policy admits by SHA-256;
- Git, which reads the patch's facts inside a private 0700 temporary directory that it removes
  afterwards (ADR 0005, ADR 0010). In authority mode Git must be enrolled by SHA-256 in the
  owner-installed allowlist (ADR 0011); the verdict records the admission.

The third, `/usr/bin/kill`, cleans up the child's process group after every run, whether it
finished, was refused, or timed out, and also when the child's pipes could not be set up. It has
a cleared environment and its own limit of 2 seconds (`KILL_TIMEOUT` in `doors/child/process.rs`):
a kill that cannot start, exits nonzero or does not finish in time is reported as an error. The
one exception is the path where the child's pipes could not be set up: that run already fails,
and the kill's own result is not checked there.

It writes no other files.

## Reporting

Please report vulnerabilities privately through GitHub's "Report a vulnerability" on this
repository rather than in a public issue.

## Agent hooks are convenience checks (WP-D4)

The PreToolUse hook matches all tools, validates event shapes, and refuses unsupported mutation
tools (including MultiEdit, NotebookEdit, patch tools and delegation without a checked adapter).
Supported Edit/Write operations simulate the complete proposed text; manifest lint checks parse
TOML. Shell tokenization catches known literal-path spellings, including concatenated quotes and
absolute paths, but does not interpret arbitrary shell programs, scripts, aliases or interpreters.
Rust text checks are conservative heuristics, not a Rust parser: the hook's shell refusals are a
list of known spellings, and forms outside it (for example `#[ expect ]` with spaces, a `#[warn]`, or
the allowed `cargo clippy --fix`) pass the hook and are left to the compiler and CI checks below.
A hook is not a sandbox: bypassing
the host, changing hook configuration or running an opaque script can avoid it.

Compiler/clippy and CI repo invariants are the enforcing checks for unsafe code, suppressions and
member lint inheritance; D6 adds configuration equality, lint floors and an expectation ledger.
The host should mount protected files read-only for ordinary agent work and use a separate
owner-controlled change path. No read-only mount or provider setting is configured by this hook.
The configured project directory and supplied session_id are routing/accounting inputs, not
authenticated session identity. Stop retries are a usability counter, never approval authority.
Hook failure now refuses with exit2; a successful hook check is not proof a change is safe.
Python3.11+ is required for structural TOML parsing.
