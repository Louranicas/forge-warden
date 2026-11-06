# Changelog

All notable changes are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). A change to a published schema
(`forge-warden.policy.v0`, `forge-warden.verdict.v0`) or to the exit-code registry is always
listed, because consumers parse them.

## [Unreleased]

### Changed (breaking)

- **The verdict schema is now `forge-warden.verdict.v1`** (`schemas/verdict.v1.schema.json`;
  the v0 schema is withdrawn). The `seals` object changes shape:
  - `patch_verified` is removed;
  - `patch_facts_sha256` (SHA-256 of Git's own reading of the patch), `provenance`
    (`produced` | `supplied`) and `git` (the Git that read the patch, recorded for audit) are
    added;
  - `ranked_by` becomes an object `{version, sha256}`.

  An optional `claims` object records `--base`/`--head` with `verified: false`, and is never
  used. New verdict notes: `duplicate_paths`, `rank_incomplete`, `rank_extraneous`,
  `status_mismatch`, `patch_unverifiable`, `rank_unverified`, `rank_mismatch`. Each one holds
  the verdict (ADR 0005).
- **`check` now needs a patch, and has two modes** (ADR 0005):
  - Authority mode (`--ranker <path>`) runs a ranker whose bytes are pinned by the policy's new
    `[producer] sha256` list, through a bounded child door, then judges the ranking it produced.
    It is the only mode that can PASS.
  - Advisory mode (`--rank`) judges a supplied ranking. It can HOLD or FAIL, never PASS.
- Patch facts come from Git through an isolated child process (ADR 0010). They can only add
  change-kind signals, never remove them (the union rule).
- **`git format-patch` output is refused** when a commit-message line starts with a mode
  keyword. Pass `git diff --binary --no-renames` output instead.
- The library's rule kinds are an exhaustive enum: `policy::KINDS`/`RULE_FIELD_KINDS` become
  `Kind::ALL`, `Field::ALL` and `Kind::fields`, and `Check::kind()` returns `Kind`.
- Duplicate paths in a ranking now hold the verdict. Policy values are validated types
  (`RepoPath`, non-empty rule lists).

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
