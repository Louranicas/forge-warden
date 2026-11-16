# forge-warden

**Change policy as code for [deep-diff-forge] reviews.** deep-diff-forge tells you what a change
does and where its risk is. forge-warden takes that sealed ranking and answers the next question:
*does this change meet the standards this repository wrote down?*

```bash
bash make_patch.sh origin/main HEAD > change.patch   # scripts/make_patch.sh from the revision you installed
forge-warden check --policy warden.toml --patch change.patch --ranker "$(command -v deep-diff-forge)"
```

In this **authority mode**, forge-warden runs the deep-diff-forge binary itself on the patch. The
policy must admit that binary by its SHA-256 (see [Authority and advisory mode](#authority-and-advisory-mode)),
and only this mode can pass. To judge a ranking you already have, use **advisory mode**. It can
hold or fail, never pass:

```bash
forge-warden check --policy examples/starter.warden.toml --patch fixtures/demo.patch \
  --advisory --rank fixtures/demo.rank.json
```

```text
forge-warden 0.2.1: FAIL (exit 7 policy_fail, next: change_input)
  patch bbf580931c67, policy 55c357961fc8, ranked by deep-diff-forge 0.7.0 (supplied, unverified), on 2026-11-05
  4 file(s), 10 rule(s): 5 pass, 0 waived, 4 hold, 1 fail
  [PASS  ] no-secrets (forbid_signal, fail)
  [PASS  ] no-hidden-text (forbid_signal, fail)
  [FAIL  ] no-ci-privilege (forbid_signal, fail)
           why: pull_request_target, write-all and secret inheritance hand untrusted code the keys to the repository.
           - .github/workflows/ci.yml: carries forbidden signal `ci_privilege`
               at new line 2 (hunk 210b042e0873)
               at new line 3 (hunk 210b042e0873)
  [HOLD  ] unsafe-needs-review (forbid_signal, hold)
           why: Every new unsafe block needs a reviewer who checks its safety argument.
           - src/lib.rs: carries forbidden signal `unsafe_added`
               at new line 3 (hunk 063a52daefc5)
  [HOLD  ] no-silent-suppression (forbid_signal, hold)
  …
  [NOTE] rank_unverified the ranking was supplied, not produced by forge-warden; it can inform, never approve; Git is unpinned: no owner allowlist enrolled; its recorded executable digest is not approved
```

Real output of the advisory command above: `fixtures/demo.patch`, its recorded deep-diff-forge
0.7.0 ranking, and `examples/starter.warden.toml`. The full JSON is the golden file
`fixtures/demo.verdict.json`, and a test fails if it drifts. A second test holds authority mode
to the same judgement. `…` marks lines left out. The date is the day the command runs (this block
was made with `--today 2026-11-05`), and the Git part of the NOTE line is what a machine without an
enrolled Git allowlist prints.

---

## Why

Review standards usually live in people's heads, a CONTRIBUTING file nobody re-reads, or a bot
whose rules are scattered across YAML. Each pull request then re-litigates them. forge-warden
puts them in one versioned file, `warden.toml`, next to the code they govern:

- **Every rule says why.** The `why` line is shown to whoever the rule stops.
- **Every exception is dated and owned.** A waiver fails the build the day after it expires, so
  "temporary" cannot quietly become permanent.
- **Every verdict is sealed.** It carries the SHA-256 of the policy, the ranking and the patch, so
  you can prove which bytes were judged by which rules.
- **Unknown is never pass.** An empty change, a signal this version does not know, or a status it
  has never seen holds the verdict for a person. Nothing passes by default.

It is built for the same world deep-diff-forge is: large, often AI-written changes, reviewed by
people, scripts, CI and agents together.

## Install

```bash
cargo install --git https://gitlab.com/lukeomahoney/forge-warden --tag v0.2.1 forge-warden-cli --locked
cargo install --git https://gitlab.com/lukeomahoney/deep-diff-forge --tag v0.8.3 deep-diff-forge-cli --locked
forge-warden init > warden.toml     # a starter policy
forge-warden lint warden.toml       # lists every problem at once
```

The starter policy holds changes to CI, supply-chain configuration, the policy itself and a coding
agent's configuration (`.claude` and everything under it) for a person's review. It also holds a text
file sent as a binary section, whose lines no signal rule can read (an executable whose name the rule
does not list needs a path rule of your own); read `examples/starter.warden.toml`
before you adopt it, and add what your repository owns.

Both lines pin a release tag; pin what you deploy (`--rev` with a commit works too). 0.2.1 is a
source release: there is no forge-warden binary to download. Authority mode runs only the
deep-diff-forge binary whose SHA-256 your policy lists, and a binary you build yourself has other
bytes than the released one ([runbook](docs/RUNBOOK.md), step 3).

The repository of record is GitLab (`gitlab.com/lukeomahoney/forge-warden`); the release tags
(`v0.2.0`, and `v0.2.1` once the owner has signed it) are there and on its mirrors, not on GitHub. The GitHub repository is no longer updated, and
its `main` predates the fixes in 0.2.0: see advisory
[FW-2026-001](docs/advisories/FW-2026-001.md).

Coming from 0.2.0 or 0.1.0? [Upgrading](docs/UPGRADING.md) lists what changed and what to do: from 0.2.0,
one new starter rule and two globs in `ci-is-owned` to copy by hand.

forge-warden runs on Linux only: its doors read `/proc/self`, `/proc/self/fd` and `/proc`, use
Unix file metadata, clean up child process groups with `/usr/bin/kill`, and cap bound mode's Git
children with `/usr/bin/prlimit` (util-linux). It is built, tested and
measured on x86_64 Linux only; the deep-diff-forge release this repository pins exists for that
platform alone.

MSRV is Rust 1.88. At run time forge-warden needs Git (`/usr/bin/git` by default; choose another
absolute path with `--git`), which reads the patch's facts. Authority mode also needs the
deep-diff-forge binary that your policy admits and [owner-enrolled Git](docs/GIT_RUNTIME.md).
With no Git allowlist, only advisory mode runs, records unpinned identity, and cannot pass.

## The policy

```toml
#:schema ./schemas/policy.v0.schema.json
schema = "forge-warden.policy.v0"

[[rule]]
id = "tests-travel-with-code"
why = "Behaviour that changes without a test change is behaviour nobody pinned down."
kind = "require_companion"
severity = "fail"
when = ["src/**"]
require = ["tests/**"]

[[waiver]]
rule = "tests-travel-with-code"
paths = ["src/generated/**"]
reason = "Generated from schema.proto; covered by the round-trip test in tests/proto.rs."
owner = "@louranicas"
expires = "2027-02-01"
```

| `kind` | Violated when | Fields |
| --- | --- | --- |
| `forbid_signal` | a file in scope carries one of `signals` (cites deep-diff-forge's evidence: hunk address and line) | `signals`, `paths`?, `except`? |
| `require_companion` | a file in `when` changes and no file in `require` changes too. Deleted files are neither triggers nor companions: deleting the tests does not satisfy "tests must change". | `when`, `require`, `except`? |
| `protect_paths` | any file in scope changes | `paths`, `except`? |
| `max_score` | a file's risk score (`per = "file"`) or the change's total (`per = "change"`) exceeds `limit` | `limit`, `per`, `paths`?, `except`? |
| `max_files` | more than `limit` files in scope change | `limit`, `paths`?, `except`? |

Every rule also needs `id` (kebab-case, unique), `why` (12+ characters), `kind` and `severity`:
`fail` (exit 7, the change must not merge as it is) or `hold` (exit 8, a person decides). There are
no defaults for these: a rule says what it means.

Globs are relative to the repository root. `*` stays within one path segment and `**` crosses
segments. The signal vocabulary is deep-diff-forge 0.8.3's risk and code-evidence signals;
naming one that does not exist is refused with a suggestion (`"secrets_like"`: *did you mean
`"secret_like"`?*). The full reference is [`docs/POLICY_REFERENCE.md`](docs/POLICY_REFERENCE.md).

**The policy is held to the standard of code.** `forge-warden lint` refuses unknown keys, unknown
signals, fields that do not apply to a rule's kind, invalid or rooted globs, duplicate ids, empty
policies, missing rationale, and waivers that name no rule, no owner, no reason or no valid date.
It reports all of them in one pass, each with its location. With `#:schema` on the first line,
editors that read JSON Schema (taplo, Even Better TOML) flag most of these as you type.

### Authority and advisory mode

`check` requires exactly one mode ([ADR 0005](docs/adr/0005-coverage-is-verified-not-claimed.md)).
`--rank` on its own is a usage error (exit 2).

- **Authority** (`--ranker FILE`): forge-warden pins the ranker by its bytes and runs it on the
  patch through a bounded child process. It then judges the ranking it produced, and this is the
  only mode that can pass. The policy lists the binaries it admits:

  ```toml
  [producer]
  sha256 = ["<the 64 lowercase hex digits of sha256sum on the deep-diff-forge binary>"]
  ```

  A ranker whose digest is not listed is refused (exit 4) before it runs. A `--rank` given in
  authority mode is compared with the produced ranking, and any difference holds the verdict
  (`rank_mismatch`).
- **Advisory** (`--advisory --rank FILE`, where `-` reads stdin): judges a supplied ranking. It
  cannot prove who produced it, so the verdict is at best `hold` (`rank_unverified`).

In both modes Git reads the patch independently. Its facts can only add change kinds to what
the ranking reports, never remove them. Make the patch with
[`scripts/make_patch.sh`](scripts/make_patch.sh) `<base> <head>`, taken from the revision you
installed: it writes the same bytes whatever your Git configuration, environment and current
directory are. A plain `git diff` does not: `diff.noprefix` drops the `a/` `b/` prefixes (such a
patch is refused), `diff.relative` in a subdirectory leaves out every change outside it, and
`ignore = all` in `.gitmodules` or `diff.ignoreSubmodules` leaves out a submodule change, and a
`.gitattributes` the change commits (`*.rs -diff`) turns its text into binary sections the ranker
cannot read; no check of the patch can see any of these. If you must use Git directly, run what
the script runs,
`git --attr-source="$(git hash-object -t tree /dev/null)" -c core.attributesFile=/dev/null diff --no-ext-diff --no-textconv --no-color --no-relative --binary --no-renames --full-index --src-prefix=a/ --dst-prefix=b/ --submodule=short --ignore-submodules=none <base>...<head>`,
from the top level, with Git 2.40 or later: the first two options take attributes from the empty
tree, not from the work tree (the script also reads no Git configuration). A verdict is about the patch bytes it was given, with paths read from the
repository root: forge-warden does not check that the patch is the whole change between two
commits, so whoever makes the patch is part of the trust boundary. See the [known costs of failing closed](docs/POLICY_REFERENCE.md#known-costs-of-failing-closed).

## Verdicts

`forge-warden check --json` prints a `forge-warden.verdict.v3` document
([schema](schemas/verdict.v3.schema.json)):

- `seals`:
  - `policy_sha256`, `rank_sha256` and `patch_sha256`;
  - `patch_facts_sha256`: what Git read from the patch, or `null` when Git refused it;
  - `provenance`: `produced` in authority mode, `supplied` in advisory mode;
  - `ranked_by`: `{version, sha256}`, where `sha256` is the ranker binary's digest, or `null` for a
    supplied ranking;
  - `git`: path, version, executable SHA-256, admission state, and exact owner-policy digest;
  - optionally `claims`, never used by the judgement: `--base` and `--head` as given with
    `verified: false`; or, in bound mode (`--repo`) when the patch was byte-equal to the diff
    forge-warden made, `{base, head, merge_base, verified: true}`.
- `verdict`: the verdict level `pass`, `hold` or `fail`, the strictest of every rule, note and expired waiver.
- `rules`: one rule outcome per rule in policy order, each with its violations in deep-diff-forge's
  risk order. Waived violations stay listed with `waived_by`, so exceptions remain visible.
- `expired_waivers`, `unused_waivers` and `notes`. A note marks something the policy cannot
  judge, so a verdict with any note is at least `hold`, and a `fail` stays `fail`. The notes
  are listed in the schema and in [`docs/POLICY_REFERENCE.md`](docs/POLICY_REFERENCE.md):
  - unknown vocabulary;
  - an empty, incomplete or extraneous ranking;
  - duplicate paths;
  - a status Git contradicts;
  - an unverifiable patch;
  - an unverified or mismatched ranking.

  Renames produce `source_path_unknown`, so rank with `git diff --no-renames`.

Evaluation and rendering are deterministic. Identical policy, patch, ranking, Git facts and
metadata, date, and `--base`/`--head` claims always give byte-identical output. Repeating a
command reproduces that output only if the ranker (in authority mode) and Git also produce
identical output. An admitted ranker's digest pins its identity, not its correctness. Verdict documents go to
**stdout**, diagnostics to **stderr**, and on error stdout is empty. Paths in the human output
are printed with every character outside printable ASCII escaped, so a hostile file name cannot
inject terminal escapes or reorder text with bidi controls.

## Exit codes

| Code | Name | Meaning | Next |
| --- | --- | --- | --- |
| 0 | `pass` | Every rule passed or was waived by an active waiver. | `proceed` |
| 2 | `usage` | Command-line usage error. | `fix_invocation` |
| 3 | `io` | A file or stream could not be read, or output could not be written. | `check_environment` |
| 4 | `invalid_input` | The policy or the ranking failed to parse or validate, exceeded the byte budget, or the ranking's seal does not match the patch given with --patch. | `change_input` |
| 7 | `policy_fail` | At least one rule of severity `fail` was violated, or a waiver has expired. | `change_input` |
| 8 | `policy_hold` | No rule failed, but a `hold` rule was violated or the ranking holds something the policy cannot judge (no files, unknown vocabulary): a person must decide. | `review_evidence` |
| 101 | `internal` | Internal error (a bug). Report it with the command and its input. | `report_bug` |

**Next** uses deep-diff-forge's action vocabulary, so an agent driving both tools needs one table.
`forge-warden explain <code|name>` and `forge-warden schema exit-codes` print the same registry
as JSON, and a test fails if this table drifts from it.

## In CI

```yaml
- run: bash make_patch.sh "origin/${BASE_REF}" HEAD > change.patch   # scripts/make_patch.sh, pinned like the binary
  env: { BASE_REF: "${{ github.base_ref }}" }
- run: forge-warden check --policy base/warden.toml --patch change.patch --ranker "$RANKER" --json > verdict.json
```

Take the policy from the **base** checkout, not from the pull request. Otherwise a pull request
can loosen the rules that judge it. This repository's example workflow
([`ci.yml`](.github/workflows/ci.yml), job `dogfood`) runs [`scripts/dogfood.sh`](scripts/dogfood.sh)
from a separate base checkout. The project does not run that workflow itself: it is kept for deployers on
GitHub, and this project's gate of record is the local integrator gate, with merges admitted on GitLab
(owner decision D3, 2026-11-16; where each check runs is in
[CONTRIBUTING](CONTRIBUTING.md#where-the-gate-runs)). The candidate is only diff data, and the driver picks authority or
advisory mode explicitly ([ADR 0006](docs/adr/0006-trusted-dogfood-driver.md)). On a hosted runner
it is asked for advisory mode (`--hosted-advisory`): the runner has no owner-enrolled Git allowlist
([`docs/GIT_RUNTIME.md`](docs/GIT_RUNTIME.md)), so that job reports evidence and cannot PASS. It
still fetches the pinned deep-diff-forge release and checks its bytes. Only that hosted job downloads:
a local run names a copy of the pinned release archive with `--ranker-archive FILE` (the driver copies
it and checks the copy against the same pinned SHA-256), and without either argument the driver
refuses before it does anything. Authority mode is claimed
only for a machine whose owner enrolled the allowlist. A workflow file in
the repository is not an independent approval boundary, so the trust root must be enforced
outside the pull request (see [`SECURITY.md`](SECURITY.md)).

The scheduled full mutation baseline (its plan and shards), run on merged `main` only, can use a
self-hosted runner-up pool. Like `ci.yml` it is a deployer example: this project runs no runner-up pool, and
the runner-routing guardrail (G-52) is retired from 1.0 (D3; [`plants/README.md`](plants/README.md)). In that workflow, pull requests, every ci.yml job and the
baseline's aggregate check always run hosted. The trade-offs and the residual host-loopback exposure are set out in
[`docs/runner-up.md`](docs/runner-up.md).

A `hold` is accepted only with an OpenSSH-signed acknowledgement. The receipt binds the exact
base, head, verdict digest and held rules, signed by a reviewer enrolled in the base branch's
`.github/hold-reviewers/allowed_signers` ([ADR 0007](docs/adr/0007-subject-bound-hold-acknowledgements.md)).
One reviewer is enrolled, the owner (principal `luke`). An acknowledgement is checked only in an
authority-mode run of `scripts/dogfood.sh` on a machine with an owner-enrolled Git allowlist;
a hosted runner runs it in advisory mode, which cannot be acknowledged, so a hosted run still fails
closed on every hold. A label grants nothing. This acknowledgement belongs to this repository's own driver (`scripts/dogfood.sh`);
the `forge-warden` binary has none, so a deployer of the binary has no acknowledgement path. A
receipt binds the verdict's bytes, which carry the UTC date of the run (`evaluated_on`), so it
matches only runs on the UTC day it was prepared for, whatever its `expires_at` says.

## How this repository makes bad code hard to write

forge-warden is a gate, so it is built behind one. Nothing below is a guideline; each item is
enforced by the compiler, a test, the gate, or a hook, and [`CLAUDE.md`](CLAUDE.md) gives agents the same
contract. Two limits, stated where each layer is described: the effect wall is a list of the
spellings it refuses (a form outside the list passes until it is added), and the agent hook layer
is a convenience that catches mistakes early, not a boundary (the gate and the build checks are).
[CONTRIBUTING](CONTRIBUTING.md#where-the-gate-runs) says where each part of the gate runs for this
repository. It also names the hosted checks that are in no gate today: the trusted dogfood on each
change, the vocabulary check and the workflow audit.

| Layer | What it stops | Where |
| --- | --- | --- |
| **Lint wall** | `unsafe`, panics (`unwrap`, `expect`, `panic!`, indexing, unchecked arithmetic, `as` casts), missing docs, wildcard matches, `print!`, `#[allow]`; every warning is an error | `Cargo.toml` `[workspace.lints]`, [ADR 0002](docs/adr/0002-the-lint-wall.md) |
| **Effect doors** | file, stdin, stdout, clock, environment and child-process access anywhere but one reviewed module (its only children are the pinned ranker, Git, `/usr/bin/kill` for cleanup and `/usr/bin/prlimit` as bound mode's exec wrapper); minting a ranking or judging raw input outside it; hash-ordered collections anywhere | `clippy.toml`, [`doors/`](crates/forge-warden-cli/src/doors/mod.rs), [ADR 0001](docs/adr/0001-pure-core-and-effect-doors.md) |
| **A pure core** | I/O, clocks and threads in the engine: they are compile errors in `forge-warden-core` | [`crates/forge-warden-core/clippy.toml`](crates/forge-warden-core/clippy.toml) |
| **Contracts as tests** | schema, README, registry and vocabulary drift; non-determinism; unsafe output | `crates/forge-warden-cli/tests/` |
| **Properties** | whole classes of bugs: adding a rule never loosens a verdict; dates round-trip; rendered output is always printable ASCII | `proptest` in both crates |
| **Real corpus** | a reader that only works on hand-written JSON | 40 rankings from a real deep-diff-forge binary in `fixtures/corpus/` |
| **Repository invariants** | crates that skip the lint wall, unexplained suppressions, doors outside the listed door modules (`src/doors/`), untracked TODOs | [`repo_invariants.rs`](crates/forge-warden-cli/tests/repo_invariants.rs) |
| **Mutation testing** | tests that pass whatever the code does: every surviving mutant in a change's diff fails it | `scripts/ci_local.sh` and the integrator gate (hosted job `mutation` for deployers) |
| **Supply chain** | unknown registries and git sources, copyleft licences, yanked or advisory-flagged crates | [`deny.toml`](deny.toml), `scripts/ci_local.sh` and the integrator gate (hosted job `supply-chain` for deployers) |
| **Workflow audit** | template injection, credential persistence, unpinned actions | `zizmor`: in the hosted workflow for deployers, and in `scripts/ci_local.sh` when it is installed; not in the integrator gate. Every action pinned to a commit SHA |
| **Dogfooding** | forge-warden's own changes skipping its own policy | [`scripts/dogfood.sh`](scripts/dogfood.sh) and [`warden.toml`](warden.toml); hosted job `dogfood` for deployers. Not yet run on each change of this repository |
| **Local hooks** | committing unformatted, unlinted or failing code | [`.githooks/`](.githooks), `just install-hooks` |
| **Agent guardrails** | an agent loosening lints, adding `#[allow]` or `unsafe`, skipping hooks, or stopping with a red gate | [`.claude/`](.claude), [`CLAUDE.md`](CLAUDE.md) |

Run the offline build, lint, test and audit part of the gate locally with `just ci` (or
`scripts/ci_local.sh`); it is a subset, and `scripts/ci_fast.sh` runs the script and fixture checks
beside it.

## Working with Claude

- **Claude Code** reads [`CLAUDE.md`](CLAUDE.md) and [`.claude/settings.json`](.claude/settings.json):
  hooks block edits that loosen the quality configuration, format every Rust file it writes, and
  refuse to let a session end while the fast gate is red. Three subagents ship in
  [`.claude/agents/`](.claude/agents): `falsifier`, `reviewer` and `subtractor`, plus two skills
  for writing policy and shipping a change.
- **Claude Managed Agents**: [`agents/managed/`](agents/managed) holds a reviewer agent definition
  and the documented calls to run it against a pull request in a managed sandbox.
- **Claude Projects**: [`docs/claude-project/`](docs/claude-project) holds the project
  instructions and the knowledge files to upload, so design conversations start from the same
  contract the code is held to.

## Relationship to deep-diff-forge

forge-warden reads one deep-diff-forge document, `deep-diff-forge.rank.v0`. In authority mode it
runs the admitted deep-diff-forge binary to produce that document; in advisory mode it is given
one. It hashes the patch to check the ranking's seal. An isolated Git reads the patch's facts
(`git apply --numstat` and `--summary`, [ADR 0010](docs/adr/0010-isolated-git-data-contract.md)).
forge-warden itself reads only the patch's mode-bearing header lines, with a total grammar, to
cross-check those facts. It does not interpret hunks.
Its vocabulary tracks
deep-diff-forge 0.8.3 (`crates/forge-warden-core/src/vocab.rs`; unchanged since 0.7.0); a newer deep-diff-forge that
adds a signal makes verdicts hold, not pass, until forge-warden learns it. Its exit codes reuse
deep-diff-forge's action names.

## License

MIT OR Apache-2.0, at your option.

[deep-diff-forge]: https://gitlab.com/lukeomahoney/deep-diff-forge

Terminology: [glossary](docs/GLOSSARY.md). Design status: [ADR status](docs/ADR_STATUS.md).
