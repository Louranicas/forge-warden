# Changelog

All notable changes are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). A change to a published schema
(today `forge-warden.policy.v0`, `forge-warden.verdict.v3` and the Git allowlist
`forge-warden.git-allowlist.v1`; 0.1.0 shipped `forge-warden.verdict.v0`, 0.2.x `forge-warden.verdict.v2`) or to the exit-code
registry is always listed, because consumers parse them.

## [Unreleased]

- The effect doors are their own crate, `forge-warden-doors` (EFFECT-WALL step 3): the ten door modules moved
  unchanged from `crates/forge-warden-cli/src/doors/` to `crates/forge-warden-doors/src/`, and the binary's `main.rs`
  forbids `clippy::disallowed_methods` and `clippy::disallowed_types`, so no attribute, macro or included file in the
  binary can lower them. The core feature `mint` is enabled by the doors crate alone. Policy, CODEOWNERS and the agent
  hooks protect the new path; `scripts/release_prepare.sh` checks the new crate's locked version too.
- The CLI builds its command line with clap's builder instead of its derive (EFFECT-WALL step 3): the derive emits
  `allow(clippy::style)`, which a crate forbidding the effect lints cannot accept. Help, usage errors and exit codes are
  unchanged byte for byte (pinned by `fixtures/cli-surface`). clap's `derive` feature is off, so `clap_derive` and
  `heck` leave the dependency tree.
- `forge-warden-core` is `#![no_std]` (with `alloc`, EFFECT-WALL step 2): an effect of any spelling is a compile
  error in the core, not only the ones `clippy.toml` lists. It matches policy globs with its own matcher, which
  reproduces globset 0.4.21 (the same patterns refused with the same words, the same answers); globset is no longer a
  dependency of the core. Library users: the core's dependencies are built without `std` features unless another
  crate in your build turns them on.
- Core API (EFFECT-WALL step 1, ruling R1): `Ranking::attest` takes a `MintKey` it consumes; `MintKey::new` exists
  only with the new core feature `mint`, which only the CLI enables. The binary's door hands out one key per run, and
  `clippy.toml` denies `MintKey::new` outside it, so a second mint is refused in every spelling; the text count of
  `attest(` in the repository invariants is gone.
- The starter policy (`forge-warden init`, `examples/starter.warden.toml`) is `forge-warden.policy.v1`
  with `[claims] require_verified = true`, and its `#:schema` line names the GitLab raw URL of
  `policy.v1`: an authority run of a new deployment counts only in bound mode (`--repo`); without it
  the verdict holds with `claims_unverified` (ruling 1663). Existing policies are untouched. This
  repository's own `warden.toml` does the same, and `scripts/dogfood.sh` judges with `--repo` and
  refuses (exit 4) an authority verdict whose claims are not verified.
- `scripts/make_patch.sh` reports Git's error, with the remedy, when `merge-base` fails (a shallow or
  partial clone lacking the commits between base and head); "no common history" now means only that.
- **Ranker pin: deep-diff-forge v0.8.3** (owner decision D4: the newest release, with its stated limits), from
  v0.8.2: `warden.toml` `[producer]` lists the v0.8.3 x86_64-unknown-linux-gnu binary
  (`4cafd0ad0f44417eb0e5b8feb591109a3e055428928f2a045316046c434f0ef1`); `scripts/tool-versions.toml` names tag
  commit `a74bf91f` and the archive's SHA-256 (`b7734168…`, the release's own `.sha256`); the runbook, README
  install line and CI's `DDF_REV` follow. Measured before the move, on every fixture patch: the same signal and
  status vocabulary, identical rows (path, status, signals, score) on 71 of 74 patches, the hostile-input rankings
  unchanged; 0.8.3 refuses three more (padded `index`-line modes, its GHSA-rq5r-p3g9-phff fix). Stated limit: two
  open high findings of 0.8.3 (G1 `Files … differ` after an `index` line; G2 several sections on one path);
  forge-warden holds both itself, and four `ddf083-*` cases in `fixtures/coverage` keep that as a test. The
  coverage corpus's other rankings stay 0.7.0's: the honest ones are byte-equal under 0.8.3 and the forged ones are
  forgeries by design.
- `scripts/dogfood.sh` takes `--ranker-archive FILE`: a local copy of the pinned deep-diff-forge release
  archive, copied into the evidence directory and checked against the same pinned SHA-256 (other bytes:
  exit 3). Only a run with `--hosted-advisory` downloads the archive; a run with neither argument is a
  usage error (exit 2) before any effect, so a local run never reaches the network (ledger Q67). The
  provenance file says `ranker_archive=local` or `downloaded`. The hosted job is unchanged.
- Bound mode, first slice (ADR 0012, accepted): `check --repo DIR --base BASE --head HEAD` (authority
  mode, full commit ids) makes `git diff merge-base..head` from a copy of the repository's objects
  that Git re-hashes (a staging repository enumerates; `index-pack` copies into a repository with no
  alternate; forge-warden cuts history at the copy's edge) and refuses a `--patch` that is not
  byte-equal (exit 4). A store whose objects are not what their ids name, a linked worktree's `.git`
  file, a caller alternates or shallow file, a control character in the path, several merge bases:
  refused. Every bound-mode Git runs under `/usr/bin/prlimit` (1 GiB data, 4 GiB address space) and a
  60 s budget.
- **New policy schema `forge-warden.policy.v1` (published schema change; ADR 0012 D8).** v1 is v0 plus an optional
  `[claims]` table: `require_verified = true` holds an authority run whose claims bound mode did not verify, with the
  new note `claims_unverified` (verdict v3's note list). v0 policies are read unchanged; `[claims]` under v0 is refused
  naming v1; `forge-warden schema policy` prints v1 (`schemas/policy.v1.schema.json`); v0's schema stays as history.
- **Verdict schema `forge-warden.verdict.v2` -> `forge-warden.verdict.v3` (published schema change).**
  Only `seals.claims` changed: without `--repo` it is v2's object exactly (`{base?, head?, verified:
  false}`); a bound run that matched records `{base, head, merge_base, verified: true}`. The core's
  `Claims` has private fields and two forms; verified claims come only from
  `BoundClaims::from_comparison` (the byte comparison, now in the core), and the CLI calls it once.
  `Unverified` leaves the public API (semver: 0.3.0). `schemas/verdict.v3.schema.json` is new;
  v2 stays byte-for-byte as history. `scripts/hold_ack.py` reads v3 only: acknowledge open holds
  before upgrading, or run the check again.
- `scripts/make_patch.sh` (patch-maker v2) and the documented by-hand command write `--full-index`:
  abbreviated ids depend on how many objects a repository holds, so bound mode could not reproduce
  them. Make patches again with the new script.
- Release scripts (ledger Q42): `scripts/release_prepare.sh VERSION` checks the version everywhere a
  release states it and that HEAD is GitLab `main`, and writes the source archive, notes and checksums
  under `dist/`; `scripts/release_tag.sh VERSION COMMIT`, run by the owner, signs the tag, keeps it only
  if it verifies against `.github/release-signers/allowed_signers` in that commit, and pushes it to
  GitLab, Gitea and Forgejo only. No release signer is enrolled yet; the script refuses until one is.
- A binary section must state its file mode where Git states it (ledger Q64, M7-Q64-MODE): a binary edit
  that keeps its mode carries it on the `index` line (`index A..B 100755`), and a creation, deletion or mode
  change carries it on its own header line with none on the `index` line. A patch that drops the mode or
  states it in both places is refused as unverifiable (`patch_unverifiable`, detail "is a binary section
  whose file mode is not stated as Git states it"). Git applies such a patch and keeps an executable file
  executable, so before, a binary edit of an executable could read as one of a plain file. Git never writes
  one: a patch made by `scripts/make_patch.sh` or `git diff` is unaffected. Still not covered without bound
  mode: an edit stating the wrong mode (`100644` for an executable); bound mode (ADR 0012) compares the patch
  with Git's own diff.

- **Built-in rule `builtin:binary-on-executable` (ledger Q64, M7-Q64; verdict v3 before its release).**
  A file that Git reads as binary, that the change does not delete, and whose mode after the change is
  `100755` or `120000` holds whatever the policy says: Git's summary gives the mode of a creation or a mode
  change, the `index` line that of a binary edit keeping its mode (Git's own in bound mode). It is a rule
  outcome (`kind: "builtin"`, severity and outcome `hold`), listed after the policy's rules only when it
  holds and counted in `summary`, also when a policy rule holds the same path. No policy
  can declare or waive it (policy ids are kebab-case); `scripts/hold_ack.py` acknowledges it as any hold.
  `verdict.v3.schema.json`: a rule id may start with `builtin:`, `kind` gains `builtin`, and kind
  `builtin` requires severity and outcome `hold`. The core's `FileChange` gains `resulting_mode()`.
  Measured: no commit in four repositories' history (Q63's set) is held by it.
- A binary section's header lines are read only up to its `GIT binary patch` line (Touchstone, M7-Q64-MODE
  verify): an `index` line appended after the body no longer states the section's mode.

## [0.2.1] - 2026-11-16

- No change to a published schema or to the exit-code registry: `forge-warden.policy.v0`,
  `forge-warden.verdict.v2`, `forge-warden.git-allowlist.v1` and the exit codes are as in 0.2.0. Two
  starter-policy changes below need a hand edit in a policy adopted earlier (Upgrading).
- New rule `binary-text-is-hidden-text` in the starter policy and in this repository's `warden.toml`
  (ledger Q63): it holds a file that is normally text (125 extensions, 40 exact names including ten of
  Git's hook names (`pre-commit`, `commit-msg`, `pre-push`, `post-checkout` and others; Git's other hook
  names, such as `post-commit` and `post-rewrite`, are not listed), `configure` and `allowed_signers`, and the name prefixes `LICENSE-`, `COPYING.` and
  `Dockerfile.`; any letter case) when the patch carries it as a binary section. An executable with
  another name is not covered (protect it with a path rule; Upgrading). Git's own numstat says "binary" (`binary_change`), so
  any patch maker that hides text this way, such as a `.gitattributes` the change commits, is held for a
  person instead of passing every signal rule. A repository that adopted the starter earlier adds the
  rule by hand (Upgrading). The demo verdict gains the rule (it passes there) and a new policy digest.
- A file replaced by a symlink, or the reverse, is refused as a type change: the
  `patch_unverifiable` detail now says the path "changes type: Git deletes it and creates it again
  as a symlink or a file" instead of "git lists PATH more than once" (ledger Q60). What holds is
  unchanged; only the reason is clearer. Any other path Git lists twice keeps the old message.
- Git's numstat counts are read strictly: each is a decimal of 1 to 20 digits, or both are `-` for a binary file;
  anything else is refused as unreadable Git output (`patch_unverifiable`). Before, an
  unreadable count was read as a content change. Git does not write one.
- The refusal for a patch without `diff --git a/PATH b/PATH` headers, and the policy reference,
  now name `scripts/make_patch.sh` as the way to make the patch.
- `scripts/dogfood.sh` downloads the ranker with `--proto-redir =https` as well as
  `--proto =https`, so a redirect cannot leave HTTPS (the archive's pinned SHA-256 was already
  checked).
- The owner's hold reviewer key is enrolled in `.github/hold-reviewers/allowed_signers`
  (principal `luke`, namespace `forge-warden-ack`, fingerprint
  `SHA256:ynROoorgFjRNx4689OUBe4eff4atYzIWvzuskmuF79E`; intention 124). A signed acknowledgement
  from that key is now accepted by an authority-mode run of `scripts/dogfood.sh`. Hosted CI runs
  dogfood in advisory mode and still fails closed on every hold. README, CONTRIBUTING, the
  hold-reviewers README and ADR 0007 say so.
- Documents say who can acknowledge a hold, and for how long (ledger Q61). The `forge-warden`
  binary has no acknowledgement path; the signed acknowledgement of ADR 0007 belongs to this
  repository's own driver (`scripts/dogfood.sh`). A receipt binds the verdict's bytes, which carry
  the UTC date of the run, so it matches only runs on the UTC day it was prepared for, whatever its
  `expires_at` says. The runbook's exit-8 row, the README, `.github/hold-reviewers/README.md` and a
  dated amendment to ADR 0007 say so. `scripts/hold_ack.py` now names the fields a refused receipt
  differs in (and the UTC-day cause when only the verdict bytes differ) and says "signature is not
  valid for an enrolled reviewer" instead of printing a failed command line; `scripts/dogfood.sh`
  no longer leaves an `acknowledgement.json` behind when it refuses one.
- The starter policy (`forge-warden init`, `examples/starter.warden.toml`) protects a coding
  agent's configuration: rule `ci-is-owned` now also holds a change to `.claude` or anything under
  it (hooks, permissions, skills), at the root or in a subdirectory, in any letter case (a
  case-insensitive checkout reads `.CLAUDE` as `.claude`), and a `.claude` symlink itself. A repository that adopted the starter
  earlier keeps its own copy; add the path to its policy by hand. The demo verdict's
  `policy_sha256` changes with the policy bytes; this change alters nothing else in it.
- For contributors (gap G9): the fast gate runs `scripts/script_commands.py`, which reads every
  tracked shell and Python script, here-documents included, and names each command it calls that is
  not in the offline gate image's declared list (that image has no `cmp`, `diff`, `jq`, `xxd` or
  `file`), within the limits its docstring states. A construct the reader does not handle is a failure,
  not a skip. It reads a source export
  without `.git` as well as a checkout. `GateImageCommands` in `scripts/tests/perimeter.py` runs it,
  and CONTRIBUTING says how to add a command. No change to the `forge-warden` binary.
- The demo verdict (`fixtures/demo.verdict.json`) names tool version 0.2.1; it was regenerated from
  the built binary.

## [0.2.0] - 2026-11-13

- The documented by-hand `git diff` (README, runbook) now takes attributes from the empty tree
  (`--attr-source=<empty tree> -c core.attributesFile=/dev/null`, Git 2.40 or later), as
  `scripts/make_patch.sh` already did. Without them, a `.gitattributes` the change commits
  (`*.rs -diff`) made Git write the edited text as binary sections, and signal rules saw nothing.
  The dogfood test commits such a file and fails if the documents drop the options.
- Advisory [FW-2026-001](docs/advisories/FW-2026-001.md) records the wrong PASS on every commit
  published before 0.2.0 (non-default Git prefixes, `diff.relative` patches, submodule changes
  left out of the patch, text hidden by a committed `.gitattributes`), what 0.2.0 fixes for patches made with `scripts/make_patch.sh`, and what
  it does not fix for patches made by hand. Builds of `dcce82c` and `7f36ba2a` already say 0.2.0
  but are affected. The install, clone and patch-maker download lines, the reporting route and the
  crate metadata now name the repository of record on GitLab; the GitHub repository is no longer
  updated.
- New `scripts/make_patch.sh <base> <head> [<repository>]` writes the patch to judge: the change
  from the merge base to head, the same bytes whatever the caller's Git configuration,
  environment and current directory are (the diff runs in a new repository that only borrows the
  object store). `scripts/dogfood.sh` uses it, and the runbook, README and upgrade notes
  document it in place of a plain `git diff`. `just judge` and the reviewer agent now judge
  committed changes only.
- The patch-maker and the by-hand `git diff` in the README and the runbook pass
  `--submodule=short --ignore-submodules=none`. Without them the by-hand command left a
  submodule change out of the patch when the work tree's `.gitmodules` said `ignore = all` (a
  file the change itself can commit), or `diff.ignoreSubmodules` or `diff.submodule=log` was
  set, and authority mode could pass the rest of the change. `scripts/make_patch.sh` was not
  affected. A submodule change that is in the patch holds, as before.
- **Security fix, and breaking for some setups:** a patch is believed only if every file has a
  `diff --git a/PATH b/PATH` header (`b/PATH a/PATH` from `git diff -R` is read too). A patch
  made with `diff.noprefix`, `diff.mnemonicPrefix` or other prefixes, or without Git headers, now
  holds with `patch_unverifiable` (new refusal `Unverifiable::HeaderPaths`). Before, Git and the
  ranker both dropped the first component of an unprefixed path (`scripts/x` became `x`), agreed
  with each other, and no path rule matched: authority mode could pass a change to a protected
  file. No attacker was needed, only `[diff] noprefix = true` in the Git configuration. Make
  patches with the new `scripts/make_patch.sh <base> <head>` (see the next entry); the documented
  commands now do. Not detectable from a patch: prefixes
  that themselves begin `a/` and `b/`, and a patch that is only part of a change (`diff.relative`
  in a subdirectory). A verdict is about the patch bytes it was given.
- A usage error (exit 2) is printed on the argument parser's own lines again. It was one line with
  `\u{000a}` where each line break belonged. Characters outside printable ASCII in an argument
  are still escaped, and if any argument contains a line feed the whole message stays on one
  line, so argument text cannot start a line of its own. `--json` errors are unchanged.
- Schemas in this release: the verdict is `forge-warden.verdict.v2`. 0.1.0 shipped v0; v1 existed
  only between releases and no tagged release carries it (ADR 0011, amendment A1).
- The policy schema stays `forge-warden.policy.v0`. The `[producer]` table is optional and
  additive: a policy without it still works, in advisory mode only. Two 0.1.0 policies are now
  refused all the same (no rules; a file over 1 MiB): see
  [Upgrading from 0.1.0](docs/UPGRADING.md), section 6.
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
