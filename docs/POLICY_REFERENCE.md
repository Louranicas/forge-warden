# Policy and verdict reference (`forge-warden.policy.v0`, `forge-warden.verdict.v2`)

The authority on a policy is `forge-warden lint`. The JSON Schema `schemas/policy.v0.schema.json`
gives editors the same rules as you type, and `schemas/verdict.v2.schema.json` describes every
verdict; tests keep both equal to the code, and keep the note and seal tables below complete.

Contents: the policy file (top level, `[producer]`, rules, kinds, signals, globs, waivers), then
how `check` runs (modes), the verdict's seals, notes, what is refused and what is held, and the
known costs of failing closed.

## Top level

| Key | Required | Meaning |
| --- | --- | --- |
| `schema` | yes | Exactly `"forge-warden.policy.v0"`. |
| `[[rule]]` | yes, one or more | An empty policy is refused: it would pass every change. |
| `[[waiver]]` | no | Dated, owned exemptions. |
| `[producer]` | for authority mode | The rankers `check --ranker` may run (below). |

Unknown keys anywhere are refused, not ignored: a misspelt key would otherwise silently disable
what it meant to configure.

## `[producer]`: the ranker allowlist

```toml
[producer]
sha256 = ["<64 lowercase hex digits>"]   # sha256sum of the deep-diff-forge binary
```

It pins the **bytes of the ranker executable**. In authority mode forge-warden opens the
`--ranker` path once without following a symlink, refuses it unless it is a regular file owned by
root or the current user and writable by neither group nor others, hashes the bytes read through
that descriptor, and runs exactly that descriptor only if the hash is listed here. Swapping the
path after the hash changes nothing. The list must be non-empty, every entry 64 lowercase hex
digits, none repeated; a policy without `[producer]` can only be used in advisory mode.

Updating it is a reviewed diff like any other policy change: build or obtain the new ranker in
the trusted environment, take `sha256sum` of the binary, add the digest (keep the old one until
nothing uses it), and merge the policy change. The verdict seals both the policy's SHA-256 and the
ranker's, so a reviewer can see which allowlist admitted which binary. Where authority comes from
is outside the file: the policy and the forge-warden binary must come from the trusted base or a
pinned release, never from the change being judged (ADR 0005 §2.6).

## Every rule

| Key | Meaning |
| --- | --- |
| `id` | Kebab-case (`[a-z][a-z0-9]*(-[a-z0-9]+)*`), at most 64 characters, unique. Appears in verdicts and waivers. |
| `why` | At least 12 characters. Shown to whoever the rule stops; write it for them. |
| `kind` | One of the kinds below. |
| `severity` | `fail` (exit 7) or `hold` (exit 8). No default. |

## Kinds

### `forbid_signal`
Violated by each in-scope file carrying any of `signals`. Each violation cites deep-diff-forge's
evidence for that signal (hunk address, side, line) when deep-diff-forge listed any.
Fields: `signals` (required), `paths` (default `["**"]`), `except`.

### `require_companion`
If any non-deleted file in `when` (minus `except`) changes, at least one file matching `require`
must change too, and that companion must still exist afterwards and have a knowable change. A
file is deleted when its status is `deleted` **or** it carries the `deleted_file` signal
(deep-diff-forge labels a deleted binary `binary_changed`). A `binary_changed` file counts as a
companion only if it is a `new_file`. One violation per triggering file.
Fields: `when`, `require` (both required), `except`.

### `protect_paths`
Violated by each in-scope file that changes in any way, deletion included.
Fields: `paths` (required), `except`.

### `max_score`
`per = "file"`: violated by each in-scope file whose risk score exceeds `limit`.
`per = "change"`: violated once (no path) when the summed score of in-scope files exceeds `limit`.
Fields: `limit`, `per` (both required), `paths`, `except`.

### `max_files`
Violated once (no path) when more than `limit` in-scope files change. `limit = 0` is refused:
use `protect_paths` to say "nothing may change here".
Fields: `limit` (required, at least 1), `paths`, `except`.

## Signals

deep-diff-forge 0.8.2's vocabulary (the same 21 signals as 0.7.0). Path and shape: `public_api_surface`, `new_file`,
`deleted_file`, `binary_change`, `large_change`, `many_hunks`, `config_or_lockfile`, `test_only`,
`generated_or_vendored`, `hazardous_text`, `symlink_change`, `executable_bit`. Code evidence:
`unsafe_added`, `process_spawn`, `network_fetch`, `dynamic_eval`, `ci_privilege`, `build_hook`,
`secret_like`, `check_suppressed`, `test_removed`. deep-diff-forge's README documents what each
detects; its code evidence is lexical, so it finds tokens and does not prove a change safe.

## Globs

Relative to the repository root; a leading `/` or `./` is refused. `*` matches within one path
segment, `**` across segments, `?` one character, `[abc]` a class, `{a,b}` alternatives, and `\`
escapes. `src/*.rs` matches `src/lib.rs` but not `src/a/lib.rs`.

## Waivers

| Key | Meaning |
| --- | --- |
| `rule` | The id of a rule in this policy. |
| `paths` | Globs. The waiver covers a violation whose path matches. |
| `reason` | At least 12 characters: why the exemption is acceptable. |
| `owner` | Who answers for it. |
| `expires` | `YYYY-MM-DD`, the last day it applies. |

A violation is covered by the first active waiver (today on or before `expires`) for its rule
whose `paths` match. Change-wide violations (`max_score` per change, `max_files`) have no path
and cannot be waived. From the day after `expires`, the waiver fails the verdict until it is
renewed or deleted. An active waiver that covered nothing is listed in `unused_waivers`.

## How `check` runs: authority and advisory

| Mode | Command | What is judged | Best outcome |
| --- | --- | --- | --- |
| Authority | `check --policy P --patch C --ranker R` | the ranking forge-warden produced by running R on C | `pass` (exit 0) |
| Advisory | `check --policy P --patch C --advisory --rank FILE` (`-` reads stdin) | the supplied ranking | `hold` (exit 8) |

There is no default mode: `check` requires exactly one of `--ranker` and `--advisory` (a required
argument group in `src/main.rs`), and refuses with a usage error (exit 2) when neither is given.

- `--patch` is required in both modes, and is read once: the same bytes are hashed, given to the
  ranker and given to Git. Produce it with `git diff --binary --no-renames` (not `format-patch`).
- Exactly one of `--ranker` and `--advisory` must be given; `--advisory` needs `--rank`.
- **Advisory can never pass.** forge-warden cannot tell who wrote a supplied ranking, so it can
  inform a person but not approve a change: every advisory verdict carries `rank_unverified`, and
  its outcome type has no pass (ADR 0005 §2.7).
- `--rank` in authority mode is a comparison, not an input: forge-warden still produces its own
  ranking, judges and seals that one, and adds `rank_mismatch` if the supplied bytes differ in any
  way (byte for byte). Identical bytes add nothing.
- In both modes Git reads the patch (`git apply --numstat -z` and `--summary`, isolated per ADR
  0010) and the ranking is checked against it: every file Git sees must be ranked, nothing else may
  be, and statuses must agree. Rules then judge each file's signals **plus** the change kinds Git
  implies (`new_file`, `deleted_file`, `binary_change`, `symlink_change`, `executable_bit`): Git
  can only add.
- `--git PATH` (default `/usr/bin/git`) must be absolute; it is recorded, with `git --version`.
- `--base REV` / `--head REV` (4–64 lowercase hex) are recorded as `claims` with
  `verified: false`. They never change an outcome.

## Verdict seals (`forge-warden.verdict.v2`)

| Member | Meaning |
| --- | --- |
| `policy_sha256` | SHA-256 of the policy bytes. |
| `rank_sha256` | SHA-256 of the judged ranking's bytes (the produced one, in authority mode). |
| `patch_sha256` | SHA-256 of the patch; the ranking's own seal must equal it. |
| `patch_facts_sha256` | SHA-256 of Git's facts: `--numstat -z` output, one 0x00 byte, `--summary` output. `null` when Git refused the patch. |
| `provenance` | `produced` (authority) or `supplied` (advisory). |
| `ranked_by` | `{version, sha256}`: the version the ranking reports, and the ranker binary's SHA-256 (`null` when supplied). |
| `git` | The Git that read the patch: `{path, version, sha256, admission, allowlist_sha256}`. `sha256` is the executable read through the descriptor every Git call used; `admission` is `approved` when that digest is enrolled in the owner's allowlist and `unpinned` only in advisory mode with no allowlist installed (with an allowlist, an unenrolled digest is refused in both modes); `allowlist_sha256` is the SHA-256 of the admitted allowlist bytes, `null` when unpinned. `path` and `version` are recorded for audit, not attested. |
| `claims` | Only when `--base`/`--head` were given: `{base?, head?, verified: false}`. |

`sha256sum` reproduces every digest. v0's `patch_verified` is gone: the patch is always hashed.

## Notes

A note is something the policy cannot judge. **Every note holds the verdict at least** (`hold`,
exit 8); rules still run, so a failing rule still fails (exit 7). Notes never make a verdict looser.

| Code | Trigger | Minimum verdict level |
| --- | --- | --- |
| `empty_ranking` | The ranking lists no file. | hold |
| `unknown_signal` | A file carries a signal this forge-warden does not know. | hold |
| `unknown_status` | A file's status is not one this forge-warden knows (including `unknown`). | hold |
| `source_path_unknown` | A `renamed` or `copied` file: rank.v0 does not say where it came from. Rank `git diff --no-renames`. | hold |
| `duplicate_paths` | The ranking lists one path more than once (deep-diff-forge does this for a type change). | hold |
| `rank_incomplete` | Git sees a file in the patch that the ranking does not list. | hold |
| `rank_extraneous` | The ranking lists a file the patch does not change. | hold |
| `status_mismatch` | The ranking's status or change-kind signals contradict Git's (e.g. deep-diff-forge 0.7.0 ranks a binary deletion without `deleted_file`). | hold; rules also see Git's change kinds |
| `patch_unverifiable` | Git refused the patch, or its facts are outside what forge-warden verifies (reason in the detail; see the costs below). Coverage cannot be checked. | hold |
| `rank_unverified` | Advisory mode: the ranking was supplied. | hold at best, never pass |
| `rank_mismatch` | Authority mode: the `--rank` bytes differ from the produced ranking. | hold; the produced ranking is the one judged |

## Refuse or hold

A **refusal** prints nothing on stdout, explains on stderr, and exits without a verdict. A
**hold** is a verdict level (exit 8) that asks a person to look.

| Situation | Result | Exit |
| --- | --- | --- |
| No mode, both modes, `--advisory` without `--rank`, no `--patch`, relative `--git`, malformed `--base`/`--head`/`--today` | refused | 2 `usage` |
| A file cannot be read; Git cannot run; the temporary directory is relative, contains `:` or is not UTF-8; `/proc/self/fd` is missing | refused | 3 `io` |
| The ranker cannot run (e.g. a script), exits nonzero, times out (10 s), overflows its output caps, prints something that is not a ranking, or seals a different patch | refused | 3 `io` |
| The ranker cannot be pinned (symlink, not a regular file, wrong owner, group/world-writable, over 512 MiB); the policy is invalid or over 1 MiB; a ranking is malformed or over 64 MiB; an advisory ranking is sealed to a different patch; the ranker's SHA-256 is not in `[producer]`; authority mode with no `[producer]` | refused | 4 `invalid_input` |
| Any note above | hold (or fail, if a rule fails) | 8 (7) |
| A rule of severity `hold` is violated | hold | 8 |
| A rule of severity `fail` is violated, or a waiver has expired | fail | 7 |

## Known costs of failing closed

forge-warden refuses what it cannot verify instead of guessing, so some honest patches hold:

- **`git format-patch` output**: a commit-message line that starts with a mode keyword (`index`,
  `new`, `deleted`, `old`) and does not have an exact header shape is refused as
  `unreadable mode header` → `patch_unverifiable`. Pass `git diff` output, which has no message.
- **Renames and copies** → `patch_unverifiable` (and `source_path_unknown` from the ranking).
  Use `git diff --no-renames`, so each side is judged as a deletion and an addition.
- **Submodules and unknown file modes** (anything but 100644, 100755, 120000) →
  `patch_unverifiable`.
- **A symlink whose target changes** → `patch_unverifiable`: the change cannot be verified per file.
- **Type changes** (e.g. file → symlink): Git and deep-diff-forge each describe one path twice →
  `patch_unverifiable` / `duplicate_paths`.
- **deep-diff-forge 0.7.0 omissions**: a binary added without `new_file` or deleted without
  `deleted_file` → `status_mismatch` until the ranker reports them (Git's kind is still applied).
- **A stricter producer** → exit 3, no verdict. deep-diff-forge 0.8.2 refuses some patches 0.7.0 ranked
  (for example `fixtures/coverage/keel-c1b-v4-mode-line-tab.patch`: "file header out of place"). In
  authority mode a patch the pinned ranker refuses ends as a ranker failure, never as a pass.

## Size limits, and what was measured

| Input | Limit | Over the limit |
| --- | --- | --- |
| Policy file | 1 MiB (1 048 576 bytes) | exit 4, before parsing |
| Ranking | 64 MiB | exit 4, before parsing |
| Patch | 64 MiB | exit 4 |
| Patch, through the pinned ranker (authority mode) | 1 000 000 lines (deep-diff-forge 0.8.2's own budget) | the ranker refuses it: exit 3, no verdict |

Measured on 2026-11-11 with a release build and the pinned deep-diff-forge 0.8.2, on one idle
16-core x86_64 Linux machine (Fedora 44 toolbox), advisory mode, each command alone:

| Case | Time | Peak memory |
| --- | --- | --- |
| Ranker, 140 000 new files (980 000 patch lines, 22 MiB) | 0.7 s | 217 MiB |
| `check` on that patch and its 12.6 MiB ranking | 0.9 s | 172 MiB |
| The same with 200 `**`-heavy globs in a `protect_paths` rule | 1.7 s | 174 MiB |
| `check` on one file of 990 000 added lines | 0.1 s | 29 MiB |
| A patch or ranking one byte over 64 MiB | 0.04 s | 67 MiB |
| `lint` on a policy just under 1 MiB (8 152 small rules) | 0.1 s | 69 MiB |
| `check` with that policy | 0.2 s | 82 MiB |
| `check` on a hand-written ranking at the 64 MiB limit (about 440 000 entries, none in the patch) | 1.0 s | 368 MiB, and a 74 MiB verdict |

Not measured here: the time and memory of authority mode (it has been run once end to end, in a
container with an agent-installed allowlist: see [RUNBOOK.md](RUNBOOK.md) step 6), a loaded or smaller
machine, the worst case of the glob matcher, and the child timeouts (10 s each for the ranker
and for Git). Before the policy limit existed, `lint` on a 64 MiB policy of 506 626 rules took
6.5 s and 4 GiB; that is why the policy has its own limit.

Canonical terminology and the distinction between a document and its level are in the
[glossary](GLOSSARY.md).

### Git executable admission

The v2 `git` object records `path`, `version`, executable `sha256`, `admission`, and
`allowlist_sha256`. Approved authority binds the exact external owner-policy bytes. Missing
policy permits only unpinned advisory output with an explicit `rank_unverified` note; malformed
or unsafe policy refuses both modes. See [Git runtime admission](GIT_RUNTIME.md) for owner
enrollment, revision/revocation rules, and execution-boundary limitations. GitOracle in the
pure core is serialized evidence, not a capability to open or execute an artifact.

## This repository's own policy (`warden.toml`)

The rules this repository judges its own changes with, and how strictly. Lowering a severity from
`fail` to `hold`, or changing a kind, is a change to the gate: the table must change in the same
commit, and `documented_rule_severities_equal_the_policies` (`crates/forge-warden-cli/tests/docs_inventory.rs`)
fails when the two disagree. The same test holds the README's sample output to
`examples/starter.warden.toml`.

| Rule | Kind | Severity |
| --- | --- | --- |
| `no-secrets` | `forbid_signal` | `fail` |
| `no-hidden-text` | `forbid_signal` | `fail` |
| `no-unsafe` | `forbid_signal` | `fail` |
| `core-stays-pure` | `forbid_signal` | `fail` |
| `no-ci-privilege` | `forbid_signal` | `fail` |
| `no-silent-suppression` | `forbid_signal` | `hold` |
| `keep-the-tests` | `forbid_signal` | `hold` |
| `tests-travel-with-code` | `require_companion` | `hold` |
| `schema-changes-are-versioned` | `require_companion` | `fail` |
| `quality-config-is-owned` | `protect_paths` | `hold` |
| `reviewable-size` | `max_score` | `hold` |
| `one-thing-at-a-time` | `max_files` | `hold` |
| `golden-output-needs-review` | `protect_paths` | `hold` |
