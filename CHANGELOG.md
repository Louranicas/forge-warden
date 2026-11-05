# Changelog

All notable changes are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). A change to a published schema
(`forge-warden.policy.v0`, `forge-warden.verdict.v0`) or to the exit-code registry is always
listed, because consumers parse them.

## [Unreleased]

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
