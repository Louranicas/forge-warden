# forge-warden

**Change policy as code for [deep-diff-forge] reviews.** deep-diff-forge tells you what a change
does and where its risk is. forge-warden takes that sealed ranking and answers the next question:
*does this change meet the standards this repository wrote down?*

```bash
git diff --binary --no-renames origin/main... > change.patch
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
forge-warden 0.1.0: FAIL (exit 7 policy_fail, next: change_input)
  patch bbf580931c67, policy 9a08b3feaf2b, ranked by deep-diff-forge 0.7.0 (supplied, unverified), on 2026-11-05
  4 file(s), 9 rule(s): 4 pass, 0 waived, 4 hold, 1 fail
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
  [NOTE] rank_unverified the ranking was supplied, not produced by forge-warden; it can inform, never approve
```

Real output of the advisory command above: `fixtures/demo.patch`, its recorded deep-diff-forge
0.7.0 ranking, and `examples/starter.warden.toml`. The full JSON is the golden file
`fixtures/demo.verdict.json`, and a test fails if it drifts. A second test holds authority mode
to the same judgement.

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
cargo install --git https://github.com/Louranicas/forge-warden forge-warden-cli --locked
cargo install --git https://github.com/Louranicas/deep-diff-forge deep-diff-forge-cli --locked
forge-warden init > warden.toml     # a starter policy
forge-warden lint warden.toml       # lists every problem at once
```

MSRV is Rust 1.88. At run time forge-warden needs Git (`/usr/bin/git` by default; choose another
absolute path with `--git`), which reads the patch's facts. Authority mode also needs the
deep-diff-forge binary that your policy admits.

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
segments. The signal vocabulary is deep-diff-forge 0.7.0's risk and code-evidence signals;
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
the ranking reports, never remove them. Pass `git diff --binary --no-renames` output; see the
[known costs of failing closed](docs/POLICY_REFERENCE.md#known-costs-of-failing-closed).

## Verdicts

`forge-warden check --json` prints a `forge-warden.verdict.v1` document
([schema](schemas/verdict.v1.schema.json)):

- `seals`:
  - `policy_sha256`, `rank_sha256` and `patch_sha256`;
  - `patch_facts_sha256`: what Git read from the patch, or `null` when Git refused it;
  - `provenance`: `produced` in authority mode, `supplied` in advisory mode;
  - `ranked_by`: `{version, sha256}`, where `sha256` is the ranker binary's digest, or `null` for a
    supplied ranking;
  - `git`: the Git that read the patch, recorded for audit;
  - optionally `claims`: `--base` and `--head` as given, always `verified: false` and never used.
- `verdict`: `pass`, `hold` or `fail`, the strictest of every rule, note and expired waiver.
- `rules`: one result per rule in policy order, each with its violations in deep-diff-forge's
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

The same policy bytes, ranking bytes and date always give byte-identical output. Results go to
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
- run: git diff --binary --no-renames "origin/${BASE_REF}...HEAD" > change.patch
  env: { BASE_REF: "${{ github.base_ref }}" }
- run: forge-warden check --policy base/warden.toml --patch change.patch --ranker "$RANKER" --json > verdict.json
```

Take the policy from the **base** checkout, not from the pull request. Otherwise a pull request
can loosen the rules that judge it. This repository's own workflow
([`ci.yml`](.github/workflows/ci.yml), job `dogfood`) runs [`scripts/dogfood.sh`](scripts/dogfood.sh)
from a separate base checkout. The candidate is only diff data, and the driver picks authority or
advisory mode explicitly ([ADR 0006](docs/adr/0006-trusted-dogfood-driver.md)). A workflow file in
the repository is not an independent approval boundary, so the trust root must be enforced
outside the pull request (see [`SECURITY.md`](SECURITY.md)).

A `hold` is accepted only with an OpenSSH-signed acknowledgement. The receipt binds the exact
base, head, verdict digest and held rules, signed by a reviewer enrolled in the base branch's
`.github/hold-reviewers/allowed_signers` ([ADR 0007](docs/adr/0007-subject-bound-hold-acknowledgements.md)).
No reviewer keys ship, so until the owner enrols one, CI fails closed on every hold. A label
grants nothing.

## How this repository makes bad code hard to write

forge-warden is a gate, so it is built behind one. Nothing below is a guideline; each item is
enforced by the compiler, a test, CI, or a hook, and [`CLAUDE.md`](CLAUDE.md) gives agents the same
contract.

| Layer | What it stops | Where |
| --- | --- | --- |
| **Lint wall** | `unsafe`, panics (`unwrap`, `expect`, `panic!`, indexing, unchecked arithmetic, `as` casts), missing docs, wildcard matches, `print!`, `#[allow]`; every warning is an error | `Cargo.toml` `[workspace.lints]`, [ADR 0002](docs/adr/0002-the-lint-wall.md) |
| **Effect doors** | file, stdin, stdout, clock, environment and child-process access anywhere but one reviewed module (the pinned ranker and Git are its only children); minting a ranking or judging raw input outside it; hash-ordered collections anywhere | `clippy.toml`, [`doors.rs`](crates/forge-warden-cli/src/doors.rs), [ADR 0001](docs/adr/0001-pure-core-and-effect-doors.md) |
| **A pure core** | I/O, clocks and threads in the engine: they are compile errors in `forge-warden-core` | [`crates/forge-warden-core/clippy.toml`](crates/forge-warden-core/clippy.toml) |
| **Contracts as tests** | schema, README, registry and vocabulary drift; non-determinism; unsafe output | `crates/forge-warden-cli/tests/` |
| **Properties** | whole classes of bugs: adding a rule never loosens a verdict; dates round-trip; rendered output is always printable ASCII | `proptest` in both crates |
| **Real corpus** | a reader that only works on hand-written JSON | 40 rankings from a real deep-diff-forge binary in `fixtures/corpus/` |
| **Repository invariants** | crates that skip the lint wall, unexplained suppressions, doors outside `doors.rs`, untracked TODOs | [`repo_invariants.rs`](crates/forge-warden-cli/tests/repo_invariants.rs) |
| **Mutation testing** | tests that pass whatever the code does: every surviving mutant in a PR's diff fails it | CI job `mutation` |
| **Supply chain** | unknown registries and git sources, copyleft licences, yanked or advisory-flagged crates | [`deny.toml`](deny.toml), CI job `supply-chain` |
| **Workflow audit** | template injection, credential persistence, unpinned actions | `zizmor` in CI; every action pinned to a commit SHA |
| **Dogfooding** | forge-warden's own changes skipping its own policy | CI job `dogfood`, [`warden.toml`](warden.toml) |
| **Local hooks** | committing unformatted, unlinted or failing code | [`.githooks/`](.githooks), `just install-hooks` |
| **Agent guardrails** | an agent loosening lints, adding `#[allow]` or `unsafe`, skipping hooks, or stopping with a red gate | [`.claude/`](.claude), [`CLAUDE.md`](CLAUDE.md) |

Run the whole gate locally with `just ci` (or `scripts/ci_local.sh`).

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
one. It hashes the patch to check the ranking's seal. It does not parse the patch itself: an
isolated Git reads the patch's facts ([ADR 0010](docs/adr/0010-isolated-git-data-contract.md)).
Its vocabulary tracks
deep-diff-forge 0.7.0 (`crates/forge-warden-core/src/vocab.rs`); a newer deep-diff-forge that
adds a signal makes verdicts hold, not pass, until forge-warden learns it. Its exit codes reuse
deep-diff-forge's action names.

## License

MIT OR Apache-2.0, at your option.

[deep-diff-forge]: https://github.com/Louranicas/deep-diff-forge
