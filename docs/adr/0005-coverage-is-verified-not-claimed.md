# ADR 0005: Coverage is verified, not claimed

**Status:** proposed, 2026-11-06 (WP-C0). Author: Keel (O2). To be threat-modelled by Bastion (O4), attacked by
Touchstone (O3) and adjudicated by Lodestar (O1). Prototype and evidence:
`mission4/lanes/keel/C0/` (`oracle_proto.py`, `c0.cases.json`, `c0.results.json`).

## Context

forge-warden judges a `deep-diff-forge.rank.v0` document. `--patch` checks only that
`sha256(patch) == rank.input_sha256`, a value the ranking itself carries. Everything else in the ranking is
taken as given. Mission 3 measured what that allows (R-01, MEASURED by three agents):

| Ranking supplied with the real `fixtures/demo.patch` | Verdict |
|---|---|
| the real ranking | FAIL, exit 7 |
| the `.github/workflows/ci.yml` entry deleted | HOLD, exit 8 |
| one harmless `tests/it.rs` entry, score 0 | **PASS, exit 0**, `patch_verified: true` |

Related findings: R-05 (no `--patch` passes), R-06 (non-canonical paths evade `src/**`), R-14 (the seal names no
producer; `ranked_by` accepts any string).

The WP-C0 prototype measured four more facts that any design must respect:

1. **`git apply --numstat -z` is not a sufficient oracle on its own.** It reports only the *new* path of a rename
   (the `old\0new` form belongs to `git diff`, not `git apply`), and it accepts `./src/lib.rs` (rc 0). Only
   `git apply --check` inside a repository refuses that path.
2. **`git apply --summary` ignores `-z`** and writes renames as `src/{old => new}`. Names can't be parsed from it
   safely. Exact line matches for `create mode`, `delete mode` and `mode change` are safe once paths containing a
   newline are refused.
3. **deep-diff-forge 0.7.0 does not mark binary deletions.** A deleted `logo.png` is ranked
   `binary_changed` with `["binary_change"]` and no `deleted_file`, in both `--binary` and plain form. The
   regression test `binary_deletions_and_unknowable_binary_changes_are_not_companions` uses a synthetic
   `["deleted_file", "binary_change"]` that the real producer never emits. A rule such as
   `forbid_signal = ["deleted_file"]` misses every binary deletion today.
4. **The producer's version string doesn't identify the producer.** DDF tag `v0.7.0` (3823fcf) does **not**
   contain the #100 parser-smuggle fix (7eb6f89); `587a6af`, which forge-warden was built against, does. Both
   report `0.7.0`. A `min_version = "0.7.0"` would admit the unpatched parser.

DDF's ranking is byte-deterministic: the same patch gives identical bytes across runs and with a stripped
environment, equal to the committed fixture, at about 9 MB RSS and negligible time. Producing the ranking inside
forge-warden is therefore cheap.

## Options considered

| Option | Closes | Leaves open | Verdict |
|---|---|---|---|
| A. Path-set oracle (`git apply --numstat -z`) | omitted and invented files (R-01 coverage) | rename sources, dot paths, forged status, forged **signals** | rejected: fact 1 |
| B. A + Git-derived status, fail closed on renames, copies, newline and non-canonical names | A, plus forged status (deleted, added, mode), dot paths, fact 3 | forged **signals** and scores: a ranking can still drop `secret_like` from a file it lists | necessary, not sufficient |
| C. **Trusted production:** `check --patch` runs a pinned deep-diff-forge itself, identified by **binary SHA-256**, and judges that ranking; a supplied `--rank` is cross-checked, never trusted | everything in the ranking, relative to the pinned producer | the producer's own bugs (DDF's job; B cross-checks its parse) | **chosen, with B as an independent check** |

## Decision (proposed)

1. **Two modes, never confused.**
   - **Authority mode** (`forge-warden check --patch P [--ranker PATH]`): forge-warden spawns the ranker through a
     new door (`doors::rank_patch`) and judges only the ranking it produced. The ranker's binary is hashed before it
     is executed, and the hash must equal `[producer] sha256` in the policy (an allowlist, so upgrades are a reviewed
     policy diff). This mode alone may exit 0.
   - **Advisory mode** (`--rank R` without a producer that verifies): the verdict can never be `pass`; the best it
     can be is `hold`, with the note `rank_unverified`. This closes R-01 and R-05 by construction: a supplied ranking
     cannot approve.
2. **The patch is mandatory for `check`.** Without patch bytes there is nothing to verify, so it is a usage error
   (exit 2). This replaces "`patch_verified: false` but pass".
3. **Git is the oracle for file identity and status (option B), in both modes.** A door runs
   `git apply --numstat -z` and `git apply --summary` on the patch bytes: environment cleared, no config, `cwd` an
   empty temporary directory under `GIT_CEILING_DIRECTORIES`, bounded input (64 MiB), bounded output, a timeout,
   and a typed error. The core receives a validated `PatchFacts { files: BTreeMap<RepoPath, FileChange> }` and
   compares:
   - path in Git, not in the ranking → note `rank_incomplete` (HOLD);
   - path in the ranking, not in Git → note `rank_extraneous` (HOLD);
   - Git's `added`/`deleted`/`mode_changed`/binary disagrees with the ranking → note `status_mismatch` (HOLD);
   - Git reports a rename or copy, a path with a newline, or a non-canonical path → note `patch_unverifiable`
     (HOLD). Pipelines rank `git diff --no-renames`, as they already do.
   Rules read the change kind from `PatchFacts`, not from the ranking's `status`, so a deletion is a deletion even
   when the producer says `binary_changed` (fact 3). Scores and signals still come from the ranking.
4. **The verdict seals the producer.** `seals.ranked_by` becomes `{ version, sha256 }` of the binary actually run
   (authority mode) or `{ version, sha256: null }` (advisory mode), and `seals.patch_facts_sha256` binds the
   oracle's view. `base`/`head` commits are recorded when the caller provides them (`--base`, `--head`); they
   are informational and unverified (this ADR does not authenticate them).
5. **No version-string gate.** `[producer] min_version` is dropped in favour of `[producer] sha256 = [...]`
   (fact 4).

## Rungs

| Guarantee | Rung | Why not higher |
|---|---|---|
| A supplied ranking can never approve | 2, types: `Verdict` constructed from `RankProvenance::Supplied` cannot hold `Outcome::Pass` (the type has no such path) | already the top rung reachable in the core |
| Rules see Git's change kind | 2, types: `evaluate` takes `PatchFacts`; `RankedFile` loses public `status` | — |
| Ranking ⟷ patch coverage | 3, admission: the comparison produces notes before any rule runs | it depends on runtime input, so no type can carry it |
| The ranker is the pinned binary | 3, admission at the door: hash before exec | TOCTOU between hash and exec remains (open the file once, `fexecve` via `/proc/self/fd`, is a candidate; to be measured) |
| No `--patch`, no check | 4, clap: a required argument | — |

## Consequences

- Breaking change: verdict schema v0 → v1 (`ranked_by` object, `patch_facts_sha256`, new note codes), CLI
  (`--patch` required; `--rank` now advisory). CHANGELOG 0.2.0, README and the schemas change with it.
- Golden files change (needs Luke's L1 answer before blessing).
- forge-warden gains two effects (spawn `git`, spawn the ranker): two new doors, each with bounds and a timeout.
  The core stays pure. A Git version floor must be measured (prototype ran on Git 2.56.0).
- The `binary_deletions…` test is replaced by one built from a real DDF ranking of a real binary deletion.
- Upstream (DDF, owner Luke): report fact 3 (no `deleted_file` on binary deletions) and fact 4 (v0.7.0 lacks
  #100). Neither is forge-warden's to fix, and this design does not depend on either being fixed.

## Open questions for review

1. Bastion: is spawning `git` and a hashed ranker acceptable at the trust boundary? The ranker runs on
   attacker-controlled patch bytes, the same as today in CI, but now inside forge-warden's process tree.
2. Touchstone: attack the line-match status derivation (fact 2), `-p` prefix handling (`a/`/`b/` vs `--no-prefix`
   patches), CRLF patches, mode-only changes on symlinks, and type changes (file ↔ symlink).
3. Lodestar or Luke: is a v1 breaking change acceptable at 0.2.0, or should advisory mode be the default with an
   opt-in to authority mode?
