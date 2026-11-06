# ADR 0005: Coverage is verified, not claimed

**Status:** proposed v2, 2026-11-06 (WP-C0). Author: Keel (O2). v1 (b962c8e) was reviewed by Bastion
(threat model, REQUEST CHANGES, 6 items) and Touchstone (attack, REVISE, 2 counterexamples). This version answers
each item by number (§7). Lodestar adjudicates. Evidence: `mission4/lanes/keel/C0/`: `oracle_proto_v3.py`,
`c0v3.cases.json` (43 cases), `c0v3.v3.results.json`, `MATRIX.md`, `v3-inputs/`.

## 1. Context

forge-warden judges a `deep-diff-forge.rank.v0` document. `--patch` checks only `sha256(patch) == rank.input_sha256`,
a value the ranking itself carries; everything else is taken as given. Mission 3 measured the consequence three
times (R-01): with the real `fixtures/demo.patch`, a ranking that lists one harmless file PASSes with
`patch_verified: true`. Related: R-05 (no `--patch` passes), R-06 (non-canonical paths), R-14 (no producer identity).

Measured facts (WP-C0 prototypes v1–v3, `MATRIX.md`):

1. `git apply --numstat -z` alone is not an oracle: it reports only a rename's new path and accepts `./src/lib.rs`.
2. `git apply --summary` ignores `-z` and abbreviates renames (`src/{a => b}`). Its `create`/`delete`/`mode change`
   lines have a fixed format and *can* be parsed whole-line (v3). Suffix matching is wrong (Touchstone C0-O3-01).
3. deep-diff-forge 0.7.0 marks neither binary additions (`new_file`) nor binary deletions (`deleted_file`); a content
   plus mode change is plain `modified`; a file→symlink change is two ranked entries for one path.
4. The producer's version string does not identify it: tag v0.7.0 (3823fcf) lacks the #100 parser fix (7eb6f89),
   587a6af has it, and both say `0.7.0`.
5. deep-diff-forge's ranking is byte-deterministic (identical bytes across runs and a stripped environment, equal to
   the committed fixture), at about 9 MB RSS. Producing it inside forge-warden is cheap.
6. Git and deep-diff-forge can disagree on a patch's identities: a CRLF-mangled patch is refused by Git (128) but
   ranked by deep-diff-forge with a path ending in `\r`.

## 2. Decision

### 2.1 Two modes, distinguished by type

- **Authority mode** (default): `forge-warden check --patch P --ranker R`. forge-warden runs the ranker itself on the
  patch bytes it holds (§2.3) and judges only the ranking that run produced. Only this mode can PASS.
- **Advisory mode** (explicit): `forge-warden check --patch P --rank FILE --advisory`. A supplied ranking is judged,
  but the verdict can be at best HOLD (note `rank_unverified`); it can never PASS.
- `--patch` is **required** in both modes. Without it, `check` is a usage error (exit 2).
- `--rank` given in authority mode is compared with the produced ranking: byte-equal → nothing to note;
  different → HOLD `rank_mismatch`. The produced ranking is the one judged and sealed, and the verdict says so.
  A supplied ranking is never silently replaced while being presented as verified.

### 2.2 Git is the oracle for file identity and kind (both modes)

A door runs `git apply --numstat -z` and `git apply --summary` on the **same** patch bytes and builds one typed record
per file:

```text
FileChange { path: RepoPath, kind: Added | Deleted | Modified,
             old_mode: Option<Mode>, new_mode: Option<Mode>,   // Mode = File | Exec | Symlink
             binary: bool, content: bool }
```

- `numstat -z` supplies the path (raw bytes, NUL-terminated), `binary` (`-\t-`) and `content` (any added or deleted
  line, or binary).
- `--summary` is parsed **whole-line by fixed format**: `" create mode " MODE " " PATH`, `" delete mode " MODE " " PATH`,
  `" mode change " MODE " => " MODE " " PATH`, where MODE is exactly one of `100644`, `100755`, `120000` and PATH is
  the entire rest of the line, compared byte-for-byte with a numstat path. Never a suffix match.
- **Refusals** (each → note `patch_unverifiable` with its reason, verdict at least HOLD), all measured in v3:
  Git exits nonzero; empty numstat; a path containing `\n`; a non-canonical path (empty, `.` or `..` segment, or
  leading `/`); **the same path twice in numstat** (repeated operations; `--no-prefix` folding); a summary record whose
  path is not in numstat; **more than one summary record for a path** (type change file↔symlink); `rename`/`copy`
  in either output; mode `160000` (submodule) or any other mode; any summary line of unknown form.
- Base/head are not Git facts. `--base`/`--head`, if given, are recorded under `claims` as caller assertions and
  marked unverified. Git `--numstat` proves the patch parses, not that it applies to any particular tree.

### 2.3 One patch snapshot

The patch is read **once**, bounded at 64 MiB, into an immutable buffer. That buffer is hashed, written to the
ranker's stdin and written to Git's stdin. No phase reopens the patch path. The produced ranking's bytes are what
forge-warden parses, seals (`rank_sha256`) and judges.

### 2.4 The ranker is pinned by content, opened once

- `[producer] sha256 = ["…"]` in the policy is an allowlist (§2.6 says where the policy must come from).
- The door opens the ranker path once (`O_RDONLY | O_NOFOLLOW | O_CLOEXEC`), `fstat`s the descriptor
  (regular file; owned by root or the current uid; not group- or world-writable), hashes the bytes read from
  that descriptor, and executes **that descriptor** via `/proc/self/fd/<n>`. It does not re-resolve the path.
- Native executables only: an interpreted (`#!`) ranker fails at exec, which is a refusal.
- C1 must **measure** this path (a planted swap of the ranker path after hashing must not run the swapped file).
  If the measurement fails on the supported platform, authority mode is refused on that platform; it is never
  downgraded silently.
- Not claimed: protection against an attacker who can write files as the same uid, or who controls the dynamic
  loader, shared libraries, `/usr/bin/git` or the kernel. These are the **trusted runtime boundary**; the verdict
  records the ranker's sha256 and the Git version string so an auditor can check them, but forge-warden does not
  attest them.

### 2.5 Every child is bounded

- No shell, ever: `Command` with an absolute program path and an argument vector; patch bytes only on stdin.
- Environment: cleared, then exactly `LC_ALL=C`, `HOME=<empty 0700 temp dir>`, `GIT_CONFIG_NOSYSTEM=1`,
  `GIT_CONFIG_GLOBAL=/dev/null`, `GIT_CEILING_DIRECTORIES=<temp dir>`, `GIT_NO_REPLACE_OBJECTS=1`,
  `GIT_ATTR_NOSYSTEM=1`; cwd is that empty temp dir. No `PATH`, no `LD_*`.
- Git is `/usr/bin/git` (absolute; overridable only by a CLI flag that the verdict records).
- A wall-clock timeout (10 s) covers writing stdin **and** draining stdout and stderr concurrently. Stdout is capped
  at 64 MiB and stderr at 64 KiB; reaching a cap is a failure, never truncation.
- Each child runs in its own process group. On timeout, cap or parent error, the whole group is killed and reaped.
  A child that leaves a grandchild holding the pipes open is still bounded by the group kill.
- Any child failure (nonzero exit, signal, timeout, cap, invalid UTF-8 or JSON from the ranker) means **no PASS**:
  - a Git refusal is about the patch → note `patch_unverifiable` (HOLD; rules still run, so FAIL stays FAIL);
  - a ranker failure leaves nothing to judge → exit 3 (`check_environment`), no verdict on stdout.
- Unsupported runtime (not Linux, no `/proc/self/fd`, Git missing) → authority mode refuses with exit 3. It never
  degrades to advisory.

### 2.6 Where authority comes from

forge-warden cannot tell who wrote a file. The contract is that in authoritative CI the **policy** (including the
producer allowlist) and the **forge-warden binary** come from the trusted base or a pinned release, never from the
candidate change (WP-D1 enforces the invocation; Bastion owns it). The verdict seals `policy_sha256`, the producer's
sha256 and the patch sha256, so a reviewer can check that the policy and producer were the trusted ones. WP-D2
acknowledgements bind to the verdict digest and head sha.

### 2.7 Proof-carrying types in the core

- `Coverage::verify(&RankDoc, &PatchFacts) -> Result<Covered, CoverageGap>` is a pure core function. `Covered` has
  a private constructor; it is the only way to obtain the value that `evaluate` needs for an authoritative verdict.
- Provenance is a type parameter: `Ranking<Produced>` and `Ranking<Supplied>`. `evaluate` returns `Verdict<P>`.
  The outcome type of `Verdict<Supplied>` is `AdvisoryOutcome { Hold, Fail }`, **which has no `Pass` variant**, so
  "a supplied ranking can never approve" is a type fact, not a test fact.
- `Ranking<Produced>` is minted only by `Produced::attest(rank_bytes, producer_sha256)`. The core cannot know that
  a door ran; so this one is rung 4: a `repo_invariants` test refuses any call to `attest` outside
  `crates/forge-warden-cli/src/doors.rs`. A *library* caller can still call `attest`. The guarantee is the binary's,
  and the doc comment says so.

### 2.8 Rules read Git's facts

`evaluate` receives the `Covered` facts and rules read a file's kind (added, deleted, modified) and `binary` from
the Git `FileChange`, not from the ranking's `status`. Signals and scores still come from the ranking. This closes
fact 3: a deleted binary test file is a deletion even when the producer says `binary_changed`.

### 2.9 Compatibility matrix (ranking vs Git), exhaustive

For each path in both, the ranking must match the measured producer behaviour (`MATRIX.md`); otherwise note
`status_mismatch` (HOLD):

| Git `FileChange` | allowed ranking status | required signals | forbidden signals |
|---|---|---|---|
| Added, text | `added` | `new_file` (+ `symlink_change` if Symlink) | `deleted_file` |
| Added, binary | `binary_changed` | — | `deleted_file` |
| Deleted, text | `deleted` | `deleted_file` (+ `symlink_change` if Symlink) | `new_file` |
| Deleted, binary | `binary_changed` | — | `new_file` |
| Modified, binary | `binary_changed` | — | `new_file`, `deleted_file` |
| Modified, mode changed, no content | `mode_changed` | — | `new_file`, `deleted_file` |
| Modified, content (± mode) | `modified` | — | `new_file`, `deleted_file` |

The product implements this as one exhaustive `match` over `(kind, binary, mode_changed, content)`. Adding a field
to `FileChange` breaks it at compile time.

## 3. Options rejected

| Option | Why rejected (measured) |
|---|---|
| Path-set oracle (`numstat -z` only) | v1: hides rename sources, accepts `./src`, can't see forged status (17/19) |
| Path + status by suffix match (v2) | Touchstone C0-O3-01: a mode change on `other src/a` was also given to `src/a` |
| added/deleted-only comparison (v2) | Touchstone C0-O3-02: forged mode and text-as-binary classified `covered` |
| A version floor (`min_version`) | fact 4 |
| forge-warden parses patch headers itself | it would add a third parser to the disagreement class (fact 6); Git is the tool that will apply the change |

## 4. Rungs

| Guarantee | Rung | Why not higher |
|---|---|---|
| A supplied ranking cannot PASS | **2, type** (`AdvisoryOutcome` has no `Pass`) | — |
| An authoritative verdict has verified coverage | **2, type** (`Covered` private constructor) | — |
| Only the door mints `Produced` | 4, repo invariant | the core cannot observe that a process ran |
| A new `FileChange` axis is compared | 2, type (exhaustive match) | — |
| Coverage, identity, kind agree with Git | 3, admission (notes before rules) | runtime input |
| The executed ranker is the hashed bytes | 3, admission at the door; **measured in C1** | TOCTOU against same-uid writers is out of scope (§2.4) |
| Children are bounded | 3, door; planted controls in C1 | runtime behaviour |
| `--patch` required | 4, clap required argument | — |

## 5. Consequences

- Breaking (0.2.0, accepted by Lodestar #57): verdict schema v1 (`ranked_by` becomes `{version, sha256}`; new
  `patch_facts_sha256`, `provenance`, `claims`; notes `rank_unverified`, `rank_mismatch`, `rank_incomplete`,
  `rank_extraneous`, `status_mismatch`, `patch_unverifiable`); CLI (`--patch` required, `--ranker`, `--advisory`,
  `--git`). Goldens change (Luke L1). CHANGELOG/README are Lodestar's.
- Two new doors (`rank_patch`, `git_facts`), each with the bounds of §2.5. The core stays pure; `PatchFacts`,
  `Covered` and the matrix are core types.
- `binary_deletions_and_unknowable_binary_changes_are_not_companions` is replaced by a test on a real binary
  deletion ranked by the real producer (MATRIX row).
- **WP-B2 change:** duplicate ranked paths are a real producer output (type change), so the rank parser must not
  refuse them as malformed (exit 4). The coverage check turns them into `patch_unverifiable` HOLD. A3's
  `duplicate_ranked_paths_are_refused` should expect HOLD, not refusal (Touchstone owns it).
- Upstream for Luke (DDF): facts 3, 4 and 6.

## 6. Planted controls C1 must pass (Bastion's list, each with an expected result)

| Plant | Expected |
|---|---|
| swap the ranker path (rename/symlink) after hashing | the hashed bytes run, or refusal; the swapped file never runs |
| swap the patch file after the first read | no effect: the one buffer is used throughout |
| a supplied rank with the real file set but `ci_privilege` removed | advisory: cannot PASS; authority: `rank_mismatch` HOLD |
| `PATH`, `LD_PRELOAD`, `GIT_CONFIG_*`, `~/.gitconfig` poisoned in the parent environment | no effect on either child |
| the ranker exits nonzero but prints valid-looking JSON | exit 3, no verdict |
| a child that sleeps, floods stdout or stderr, or forks a grandchild that holds the pipes | timeout or cap → group killed and reaped; no leaked process (checked with `pgrep -g`) |
| malformed or oversized ranker output | exit 3 |
| `--rank` differing from the produced ranking | HOLD `rank_mismatch` |
| fake `--base`/`--head` | recorded under `claims`, `verified: false`; no effect on the outcome |
| an allowlist edited to admit the ranker under test | the policy sha changes and is sealed; the trust rule is WP-D1's |
| Touchstone's held-out C0 patches + the 43 v3 cases | as in `c0v3.cases.json` |

## 7. Review items answered

| Item | Where |
|---|---|
| Bastion 1: pin the executed object; runtime boundary; no same-uid claim | §2.4 |
| Bastion 2: one snapshot; seal produced bytes; child failure precludes PASS | §2.3, §2.5 |
| Bastion 3: bound the child; env, config, discovery; refuse unsupported runtimes | §2.5 |
| Bastion 4: precise Git semantics; repeated operations; type changes; base/head informational | §2.2 (refusal list, measured) |
| Bastion 5: trusted authority source; `--rank` mismatch handling | §2.1, §2.6 |
| Bastion 6: proof-carrying types with private constructors | §2.7, §4 |
| Touchstone C0-O3-01: suffix collision | §2.2 whole-line parse; v3 case `ts-mode-collision-real` (facts per path correct) |
| Touchstone C0-O3-02: mode and binary axes | §2.9 matrix; v3 cases `ts-forged-mode`, `ts-text-as-binary` → `status_mismatch` |
| Touchstone: `--no-prefix` must HOLD, not auto-repair | v3 refuses (`repeated_operation_on_path`) |
| Touchstone: bounded capture, typed refusals, empty ≠ valid | §2.5; `empty_patch` refusal |
