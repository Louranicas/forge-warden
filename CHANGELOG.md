# Changelog

All notable changes are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). A change to a published schema
(today `forge-warden.policy.v0`, `forge-warden.verdict.v2` and the Git allowlist
`forge-warden.git-allowlist.v1`; 0.1.0 shipped `forge-warden.verdict.v0`) or to the exit-code
registry is always listed, because consumers parse them.

## [Unreleased]

- **Breaking:** a policy file over 1 MiB is refused (exit 4, `invalid_input`, before parsing);
  it shared the ranking's 64 MiB budget before. New constant `MAX_POLICY_BYTES`. Measured: a
  64 MiB policy of half a million rules parsed in 6.5 s at a peak of 4 GiB. Size limits and
  measurements are in [the policy reference](docs/POLICY_REFERENCE.md).
- [Upgrading from 0.1.0](docs/UPGRADING.md) lists every change a 0.1.0 user must act on.
- `scripts/dogfood.sh` (driver v2) takes `--hosted-advisory` as an optional first argument. The
  repository's hosted `dogfood` job passes it: a hosted runner has no owner-enrolled Git
  allowlist, so with the producer now pinned in `warden.toml` it could not judge in authority mode
  (exit 3). Hosted dogfood is advisory by decision and cannot PASS; without the argument a pinned
  policy still means authority mode. No schema or exit code changes.
- A patch Git refuses is now judged in one code path that cannot finish without its
  `patch_unverifiable` note (Q25; `Verdict::held_by`, crate-internal). For consumers nothing
  changes: the verdict document, its schema and the exit codes are the same, a refused patch still
  holds at least, and a failing rule on it still fails.
- Verdict no longer implements Clone; CLI reporting borrows judged authority/advisory results without copying them (T-GITPIN-01).

### CLI machine contract

- `lint --json` emits `forge-warden.lint.v1`; `check`, `lint`, `explain` and `schema` with
  `--json` emit one error envelope on stderr (`exit`, `name`, `message`, `next`) on failure,
  including command-line parse failures. Messages are printable ASCII before JSON encoding.
  Failed stdout writes can leave partial bytes; consumers must reject output on an error exit.
  `init` remains a TOML producer; help/version remain human text. Schema commands `lint`,
  `error`, `exit-entry` and `exit-codes-schema` expose the corresponding v1 schemas.
- `--patch -` now fails explicitly as `patch_stdin_unsupported`; help documents that stdin
  is reserved for `--rank -`. Save patches to a file. Door hints distinguish I/O resources,
  host environment, input size and refused input; numeric registry exits are unchanged.
- `today_source` remains deferred; the CLI-contract change itself did not change verdict fields.

### Git runtime identity (WP-GIT-PIN)

- Verdict v2 records the executable SHA-256, admission state, and exact Git owner-policy
  snapshot digest. Historical v1 is no longer produced or accepted for acknowledgements.
- Authority requires a root-owned external Git allowlist; absent policy only permits
  explicitly unpinned advisory evidence. Every Git call uses one admitted descriptor.
- The acknowledgement controller checks current policy and refuses revoked or stale Git
  admission. See [operator enrollment and limits](docs/GIT_RUNTIME.md).

### Changed (breaking)

- **Earlier unreleased v1 transition (superseded above by v2)** (`schemas/verdict.v1.schema.json`;
  the v0 schema is withdrawn). The `seals` object changes shape:
  - `patch_verified` is removed;
  - `patch_facts_sha256` (SHA-256 of Git's own reading of the patch), `provenance`
    (`produced` | `supplied`) and `git` (the Git that read the patch, recorded for audit) are
    added;
  - `ranked_by` becomes an object `{version, sha256}`.

  An optional `claims` object records `--base`/`--head` with `verified: false`, and is never
  used. New verdict notes: `duplicate_paths`, `rank_incomplete`, `rank_extraneous`,
  `status_mismatch`, `patch_unverifiable`, `rank_unverified`, `rank_mismatch`. A verdict with
  any note is at least HOLD. An existing FAIL stays FAIL (ADR 0005).
- **`check` now needs a patch, and has two modes** (ADR 0005):
  - Authority mode (`--ranker <path>`) runs a ranker whose bytes are pinned by the policy's new
    `[producer] sha256` list, through a bounded child door, then judges the ranking it produced.
    It is the only mode that can PASS.
  - Advisory mode (`--advisory --rank <file>`) judges a supplied ranking. It can HOLD or FAIL,
    never PASS.

  One of `--ranker` or `--advisory` is required; `--rank` alone is a usage error (exit 2).
- Patch facts come from Git through an isolated child process (ADR 0010). They can only add
  change-kind signals, never remove them (the union rule).
- **A mode-like line that is not an exact header makes the patch unverifiable.** This includes
  a `git format-patch` commit-message line that starts with `index`, `new`, `deleted` or `old`
  but has no exact header shape (`unreadable mode header`), which produces the
  `patch_unverifiable` note. Well-formed mode lines are read as headers. Prefer
  `git diff --binary --no-renames` output, which has no message.
- The library's rule kinds are an exhaustive enum: `policy::KINDS`/`RULE_FIELD_KINDS` become
  `Kind::ALL`, `Field::ALL` and `Kind::fields`, and `Check::kind()` returns `Kind`.
- Library: `Policy`, `RankDoc`, `RankedFile` and `Evidence` fields are private. Build them
  through the parsers and checked constructors, and read them through getters. A `Policy` exists
  only through `parse_policy`. Ranking paths are validated `RepoPath`s.
- A policy with no rules is refused. Duplicate paths in a ranking hold the verdict.

### Added

- Policy schema: the optional `producer` table (`sha256`: the admitted ranker digests).
- `scripts/dogfood.sh` and `ci_local.sh` select authority or advisory mode explicitly and never
  enrol the ranker they built (ADR 0006). The trusted base judges the candidate.
- HOLD acknowledgements are OpenSSH-signed receipts bound to the exact base, head, verdict
  digest and held rules (`scripts/hold_ack.py`, ADR 0007). No reviewer keys ship; absent keys
  fail closed.
- A managed-review bundle built only from Git objects (ADR 0008). The guard hook fails
  closed.
- One protected-path manifest, a lint floor, and an expectation ledger whose sources come from
  compiler dep-info (ADR 0009). A Python and a Rust classifier must agree on what is production
  code.
- The compiler refuses minting a ranking (`Ranking::attest`) or judging raw input outside the
  CLI's door (clippy `disallowed-methods`, plus attribute-parsed door lints).
- An independent finite reference model and property laws for the policy engine, and a
  planted-red harness (`plants/`) whose plants must each be caught by their named diagnostic.
- `docs/THREAT_MODEL.md`, and ADRs 0005-0010.

### Fixed

- `ci_local.sh` and the pre-push hook propagate judge and ranker failures.
- `check_vocab.sh` finds the repository root without `.git`, and attests the named
  deep-diff-forge commit by its full SHA. The vocabulary corpus is 40 rankings covering all 21
  signals.

### Build

- The gate toolchain is pinned to Rust 1.99.1 (`rust-toolchain.toml`). The MSRV 1.88 is checked
  with `cargo +1.88.0 check --all-targets`.
- Mutation runs use `cargo mutants --cap-lints true` with `CARGO_TARGET_DIR` unset.

## [0.1.0] - 2026-11-05

### Added

- `forge-warden check`: judges a `deep-diff-forge.rank.v0` ranking against a `warden.toml`
  policy and prints a sealed `forge-warden.verdict.v0` (`--json`) or a terminal-safe summary.
  `--patch` verifies the ranking's seal against the patch bytes.
- Rule kinds `forbid_signal`, `require_companion`, `protect_paths`, `max_score`, `max_files`;
  dated, owned waivers that fail the verdict once expired.
- `forge-warden lint`, `explain`, `schema verdict|policy|exit-codes`, `init`.
- Exit-code registry: 0 `pass`, 2 `usage`, 3 `io`, 4 `invalid_input`, 7 `policy_fail`,
  8 `policy_hold`, 101 `internal`.
- JSON Schemas for the policy and the verdict, checked against the code by tests.
- Verdict notes `empty_ranking`, `unknown_signal`, `unknown_status` and `source_path_unknown`
  (renamed or copied files), each holding the verdict; `evidence_truncated` on violations whose
  evidence deep-diff-forge capped.
