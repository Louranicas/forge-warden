# Upgrading

From 0.2.0 to 0.2.1: [the last section](#020-to-021). From 0.1.0 to 0.2.0: sections 1 to 7 below.

## From 0.1.0 to 0.2.0

0.2.0 is not a drop-in replacement. 0.1.0 judged a ranking it was handed; 0.2.0 only passes a
change when it ran the ranker itself, on a patch it holds, with a Git the owner enrolled. This
page lists every change a 0.1.0 user has to act on, and what to do. The full list of changes is
in the [changelog](../CHANGELOG.md); the reasons are in ADRs 0005 to 0011
([status](ADR_STATUS.md)).

If you only want yesterday's behaviour with the least work, read section 1 and use advisory
mode. It reports the same rule results as before, and it can no longer print PASS.

### 1. `check` needs a patch and an explicit mode

0.1.0: `forge-warden check --policy warden.toml --rank rank.json`, with `--patch` optional.

0.2.0: `--patch <file>` is required, and exactly one of two modes:

| Mode | You pass | Can it PASS? | What it needs |
| --- | --- | --- | --- |
| Authority | `--ranker <path>` | yes | the ranker's SHA-256 in the policy's `[producer]` table (section 3) and an owner-enrolled Git allowlist (section 4) |
| Advisory | `--advisory --rank <file>` | no: at best HOLD (exit 8) | nothing more |

What to do:

- Save the patch to a file; `--patch -` is refused (stdin is reserved for `--rank -`). Make it
  with `scripts/make_patch.sh <base> <head>` from the revision you installed
  ([runbook](RUNBOOK.md), steps 2 and 6). A patch whose paths do not start with `a/` and `b/`
  is now unverifiable (exit 8): that is what a plain `git diff` writes when your Git
  configuration has `diff.noprefix` or `diff.mnemonicPrefix`. A plain `git diff` in a
  subdirectory with `diff.relative` leaves out every change outside it, and a plain `git diff`
  leaves out a submodule change when `diff.ignoreSubmodules` is set or the work tree's
  `.gitmodules` says `ignore = all` (a file the change itself can commit); no check of the
  patch can see either omission. The patch-maker reads none of those settings. A verdict is about the patch
  bytes it was given: forge-warden does not check that they are the whole change between two
  commits. A `git format-patch` message line that looks like a mode header makes the patch
  unverifiable too.
- Pick a mode. `--rank` alone is now a usage error (exit 2).
- A pipeline that treated exit 0 as "merge" and used a supplied ranking will now see exit 8 on
  every change. That is intended: a supplied ranking can inform, never approve. To get exit 0
  again, move that pipeline to authority mode.

### 2. The verdict document is `forge-warden.verdict.v2`

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

### 3. Authority mode needs a pinned producer

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

### 4. Authority mode needs an owner-enrolled Git

forge-warden asks Git what the patch changes, and in authority mode it runs only a Git whose
executable digest the machine's owner has enrolled in `/etc/forge-warden/git-allowlist.toml`.
There is no flag and no environment variable for another location.

What to do: follow the [runbook](RUNBOOK.md), step 4, once per machine (it needs root);
[Git runtime identity](GIT_RUNTIME.md) explains the rules behind it.
Without the allowlist, authority mode stops with exit 3, "no Git allowlist enrolled", and
advisory mode marks its evidence as unpinned. After a Git upgrade the digest changes and the
allowlist has to be updated before authority mode works again.

A hosted CI runner has no owner-enrolled allowlist. The hosted job in this repository's example
workflow therefore runs in advisory mode by decision and cannot PASS; authority mode is for a machine
you control. This repository's own gate uses no hosted job (CONTRIBUTING, "Where the gate runs").

### 5. Exit codes: same numbers, wider meanings

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

### 6. Policies

- A policy with no rules is refused (exit 4). Add at least one rule.
- A policy file over 1 MiB is refused (exit 4). 0.1.0 read up to 64 MiB.
- Duplicate paths in a ranking hold the verdict.
- `lint --json` prints `forge-warden.lint.v1`.

### 7. If you use the crates as a library

`forge-warden-core` is not a stable API yet. Since 0.1.0: `Policy`, `RankDoc`, `RankedFile`
and `Evidence` have private fields and are built only through the parsers; rule kinds are the
enum `Kind` (non-exhaustive: match it with a wildcard arm); `Verdict` is no longer `Clone`; a ranking is judged only through the
authority and advisory entry points, which take a checked ranking.

### Not covered here

Moving a signed HOLD acknowledgement workflow from an unreleased v1 build; there was no release
that produced one. Platforms other than x86_64 Linux: nothing is pinned or tested for them.

## 0.2.0 to 0.2.1

0.2.1 is a drop-in replacement for 0.2.0: the verdict, policy and Git-allowlist schemas and the exit codes are
unchanged. Act on two things: the starter rules below, if you adopted the starter, and three refusals that
read differently or are new.

### Starter rules to copy by hand

`forge-warden init` prints the starter policy of the version you run. A `warden.toml` you adopted from
an older starter keeps its own rules: nothing changes until you edit it. The starter has gained, in
0.2.1 (changelog, 0.2.1):
- rule `binary-text-is-hidden-text` (hold): a file that is normally text, carried by the patch as a
  binary section. Copy the whole rule from `examples/starter.warden.toml`; its three globs are generated
  from the lists in `crates/forge-warden-core/tests/authority.rs` (extensions; exact names, ten of Git's
  hook names, `configure` and `allowed_signers` among them, while Git's other hook names such as
  `post-commit` are not listed; name prefixes `LICENSE-`, `COPYING.`,
  `Dockerfile.`). **Not covered:** an executable with another name (a `bin/tool` with no extension) sent
  as a binary section is not held by this rule: bash runs a file as a script unless it finds a NUL byte in
  its first line (within the first 80 bytes), so a file Git calls binary can still be a script. Protect such paths with a `protect_paths` rule (`bin/**`, `tools/**`). A hold for
  any binary section on an executable file needs the file mode and is planned for verdict v3 (ledger Q64).
- in rule `ci-is-owned`, the two `.claude` globs (any letter case, the entry itself and everything under it).

### The ranker pin

The ranker pin stays deep-diff-forge 0.8.2; its advisory GHSA-rq5r-p3g9-phff (published 2026-11-11) is fixed in 0.8.3.
Measured with 0.2.1 in authority mode on the advisory's shapes (Touchstone, receipt 88e2f464): the bare-header
shape and the three submodule-mode shapes are held on forge-warden's own checks (the header check; "unsupported
file mode"); the `new file mode` then `new mode` shape gets the same verdict as Git's own patch for that change,
because forge-warden takes the executable and symlink signals from Git's reading, not from the ranker. Moving
the pin is the owner's decision (ledger Q22), planned as 0.3.0 work (M7-RANKER-PIN).

### Refusals that read differently
- A file replaced by a symlink, or the reverse, is still refused (`patch_unverifiable`); the detail now says
  the path "changes type" instead of "git lists PATH more than once". Automation that matched the old
  text should match the note code instead.
- A patch without `diff --git a/PATH b/PATH` headers is still refused (`patch_unverifiable`); the detail now
  says to make the patch with `scripts/make_patch.sh <base> <head>` instead of `git diff --src-prefix=a/
  --dst-prefix=b/`. Match the note code, not the text.
- New: Git's numstat counts are read strictly (1 to 20 decimal digits, or `-` `-` for a binary file); any
  other count is refused as unreadable Git output, where 0.2.0 read it as a content change. Git does not
  write one, so a patch Git made is unaffected.

## 0.2.1 to 0.3.0 (unreleased)

- **Library users of forge-warden-core: `Ranking::attest(doc, producer, key)`** takes a `MintKey` and consumes it
  (EFFECT-WALL step 1). Make one with `MintKey::new()`, which exists only with the core feature `mint`. The CLI
  is unchanged for its users.
- **To require bound mode, move the policy to `forge-warden.policy.v1`** and add the table `[claims]` with `require_verified = true`
  (POLICY_REFERENCE). A policy that stays on v0 is read unchanged. A forge-warden older than 0.3.0 refuses a v1
  policy, so upgrade the binary before the policy. Under the key, `check` without `--repo` holds (`claims_unverified`); a `claims_unverified` hold cannot be acknowledged; re-run in bound mode.
- **`forge-warden init` now writes a v1 policy with `require_verified = true`** (ruling 1663: a new deployment
  gets bound mode on). An existing policy is untouched until you change it; to adopt the same, set
  `schema = "forge-warden.policy.v1"` and add these two lines:

  ```toml
  [claims]
  require_verified = true
  ```

  Then run authority mode in bound mode: make the patch with `scripts/make_patch.sh BASE HEAD` and pass
  `--repo "$(git rev-parse --path-format=absolute --git-common-dir)" --base BASE --head HEAD` (full ids).

- **The verdict document is `forge-warden.verdict.v3`.** Only `seals.claims` changed. Without `--repo`
  it is v2's object exactly (`{base?, head?, verified: false}`); a bound run (`--repo`) that matched
  records `{base, head, merge_base, verified: true}`. Validate against
  [`schemas/verdict.v3.schema.json`](../schemas/verdict.v3.schema.json); a reader that checks the
  `schema` member for `forge-warden.verdict.v2` must accept v3.
- **`scripts/hold_ack.py` reads v3 only.** Acknowledge open holds before upgrading, or run the check
  again with 0.3.0 and acknowledge that verdict.
- **Patches for bound mode come from `scripts/make_patch.sh` of 0.3.0** (patch-maker v2, `--full-index`).
  A patch made by 0.2.x's script has abbreviated `index` lines, so `--repo` refuses it (exit 4); make it
  again.
- **The ranker pin is deep-diff-forge v0.8.3** (`[producer]` in `warden.toml`, `scripts/tool-versions.toml`;
  owner decision D4: the newest release, with its stated limits). A policy that pins 0.8.2's binary keeps working
  with 0.8.2; to move, list the v0.8.3 binary's SHA-256 from the [runbook](RUNBOOK.md), step 3. 0.8.3 refuses
  some patches 0.8.2 ranked (malformed `index`-line modes; in authority mode: exit 3, no verdict) and ranks every
  other measured patch identically (vocabulary, statuses, paths, scores). Stated limit: 0.8.3 has two open high
  findings (G1 `Files … differ` after an `index` line; G2 several sections on one path); forge-warden holds both
  shapes itself, and `fixtures/coverage` keeps one case of each as its test.
- **Library users:** `Claims` has private fields and two forms (read it with `base()`, `head()`,
  `merge_base()`, `verified()`); `Unverified` is gone. Verified claims come only from
  `BoundClaims::from_comparison`.
- **A new hold no policy can remove: `builtin:binary-on-executable`** (ledger Q64; the gap the 0.2.1
  notes above call "not covered"). A binary section on a file whose mode after the change is `100755` or
  `120000` now holds whatever the policy says, listed in `rules` with kind `builtin` (a new value in v3's
  rule `kind`; a rule id may now start with `builtin:`). A repository that commits executable binaries (a
  vendored tool, a prebuilt helper) gets one hold per change to them: acknowledge it like a policy hold
  (`scripts/hold_ack.py`), or keep those files out of the patch. It cannot be waived. Measured on four
  repositories' whole history (Q63's set), it holds no commit there. A binary edit's mode is read from its
  `index` line, which is Git's own only in bound mode (`--repo`); without `--repo` a falsely stated
  `100644` is not caught. 0.3.0 also reads a binary section's header lines only up to `GIT binary patch`.
- **Starter rule `ci-is-owned` to copy by hand** (ledger Q85): the starter now also holds `.gitlab-ci.yml`, `.gitlab`,
  `.gitea` and `.forgejo` in any letter case, root or nested. A `warden.toml` adopted from an older starter keeps its
  own list: copy the seven `**/.[gG]…` and `**/.[fF]…` globs from `examples/starter.warden.toml` if your CI runs there.
