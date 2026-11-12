# Upgrading from 0.1.0 to 0.2.0

0.2.0 is not a drop-in replacement. 0.1.0 judged a ranking it was handed; 0.2.0 only passes a
change when it ran the ranker itself, on a patch it holds, with a Git the owner enrolled. This
page lists every change a 0.1.0 user has to act on, and what to do. The full list of changes is
in the [changelog](../CHANGELOG.md); the reasons are in ADRs 0005 to 0011
([status](ADR_STATUS.md)).

If you only want yesterday's behaviour with the least work, read section 1 and use advisory
mode. It reports the same rule results as before, and it can no longer print PASS.

## 1. `check` needs a patch and an explicit mode

0.1.0: `forge-warden check --policy warden.toml --rank rank.json`, with `--patch` optional.

0.2.0: `--patch <file>` is required, and exactly one of two modes:

| Mode | You pass | Can it PASS? | What it needs |
| --- | --- | --- | --- |
| Authority | `--ranker <path>` | yes | the ranker's SHA-256 in the policy's `[producer]` table (section 3) and an owner-enrolled Git allowlist (section 4) |
| Advisory | `--advisory --rank <file>` | no: at best HOLD (exit 8) | nothing more |

What to do:

- Save the patch to a file; `--patch -` is refused (stdin is reserved for `--rank -`). Use
  `git diff --binary --no-renames --src-prefix=a/ --dst-prefix=b/` output. A patch whose paths
  do not start with `a/` and `b/` is unverifiable (exit 8): if your Git configuration has
  `diff.noprefix` or `diff.mnemonicPrefix`, the two prefix options are what makes the command
  work. A `git format-patch` message line that looks like a mode header makes the patch
  unverifiable too.
- Pick a mode. `--rank` alone is now a usage error (exit 2).
- A pipeline that treated exit 0 as "merge" and used a supplied ranking will now see exit 8 on
  every change. That is intended: a supplied ranking can inform, never approve. To get exit 0
  again, move that pipeline to authority mode.

## 2. The verdict document is `forge-warden.verdict.v2`

The v0 document 0.1.0 printed is withdrawn; v1 existed only between releases and is neither
produced nor accepted (`schemas/verdict.v1.schema.json` stays in the tree for reference only). A consumer that parses `--json` output must change:

- `seals.patch_verified` is gone. Read `seals.provenance` (`produced` or `supplied`) and
  `seals.patch_facts_sha256` instead.
- `seals.ranked_by` is an object with `version` and `sha256`, not a string.
- `seals.git` records the Git that read the patch: its executable digest, its admission state
  and the digest of the allowlist that admitted it.
- `notes` can carry new codes: `duplicate_paths`, `rank_incomplete`, `rank_extraneous`,
  `status_mismatch`, `patch_unverifiable`, `rank_unverified`, `rank_mismatch`. A verdict with any
  note is at least HOLD; a FAIL stays FAIL.
- An optional `claims` object records `--base` and `--head` as given, with `verified: false`.

What to do: validate against [`schemas/verdict.v2.schema.json`](../schemas/verdict.v2.schema.json)
(`forge-warden schema verdict` prints it), and treat any note as "a person must look".

On failure with `--json`, `check`, `lint`, `explain` and `schema` now print one error object on
stderr (`exit`, `name`, `message`, `next`). Reject stdout whenever the exit code is an error.

## 3. Authority mode needs a pinned producer

A policy that should be able to PASS lists the SHA-256 of each ranker executable it admits in a
`[producer]` table ([policy reference](POLICY_REFERENCE.md)). forge-warden hashes the bytes it is
about to run and refuses any other (exit 4).

What to do:

- Use a released deep-diff-forge binary and pin its digest. This repository pins v0.8.2 for
  x86_64 Linux. A binary you build yourself has other bytes (the build host's paths, linker and
  libc go into it), so pin what you actually run, and expect to re-pin after rebuilding.
  The [runbook](RUNBOOK.md), step 3, has the download, check and install commands.
- deep-diff-forge 0.8.2 refuses some patches 0.7.0 ranked. In authority mode a patch the pinned
  ranker refuses ends as exit 3 with no verdict, never as a pass.
- A policy without `[producer]` still works, in advisory mode only.

## 4. Authority mode needs an owner-enrolled Git

forge-warden asks Git what the patch changes, and in authority mode it runs only a Git whose
executable digest the machine's owner has enrolled in `/etc/forge-warden/git-allowlist.toml`.
There is no flag and no environment variable for another location.

What to do: follow the [runbook](RUNBOOK.md), step 4, once per machine (it needs root);
[Git runtime identity](GIT_RUNTIME.md) explains the rules behind it.
Without the allowlist, authority mode stops with exit 3, "no Git allowlist enrolled", and
advisory mode marks its evidence as unpinned. After a Git upgrade the digest changes and the
allowlist has to be updated before authority mode works again.

A hosted CI runner has no owner-enrolled allowlist. This repository's own hosted job therefore
runs in advisory mode by decision and cannot PASS; authority mode is for a machine you control.

## 5. Exit codes: same numbers, wider meanings

The registry is unchanged: 0 `pass`, 2 `usage`, 3 `io`, 4 `invalid_input`, 7 `policy_fail`,
8 `policy_hold`, 101 `internal`. What changed is when you see them:

- 0 can only come from authority mode.
- 2 now also means a missing mode or `--rank` without `--advisory`.
- 3 now also means the ranker or Git could not be run: the ranker file is missing or
  unreadable, it exits nonzero or times out, or no Git allowlist is enrolled. No verdict is
  written.
- 4 now also means the ranker itself is refused as input: its digest is not in `[producer]`,
  the file is a symlink, not a regular file, or writable by group or others; or authority mode
  was asked of a policy that has no `[producer]`.
- 8 is what advisory mode returns when nothing failed.

What to do: a wrapper that mapped "not 7" to success must be rewritten to accept only 0.

## 6. Policies

- A policy with no rules is refused (exit 4). Add at least one rule.
- A policy file over 1 MiB is refused (exit 4). 0.1.0 read up to 64 MiB.
- Duplicate paths in a ranking hold the verdict.
- `lint --json` prints `forge-warden.lint.v1`.

## 7. If you use the crates as a library

`forge-warden-core` is not a stable API yet. Since 0.1.0: `Policy`, `RankDoc`, `RankedFile`
and `Evidence` have private fields and are built only through the parsers; rule kinds are the
enum `Kind` (non-exhaustive: match it with a wildcard arm); `Verdict` is no longer `Clone`; a ranking is judged only through the
authority and advisory entry points, which take a checked ranking.

## Not covered here

Moving a signed HOLD acknowledgement workflow from an unreleased v1 build; there was no release
that produced one. Platforms other than x86_64 Linux: nothing is pinned or tested for them.
