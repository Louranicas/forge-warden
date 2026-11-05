# forge-warden

**Change policy as code for [deep-diff-forge] reviews.** deep-diff-forge tells you what a change
does and where its risk is. forge-warden takes that sealed ranking and answers the next question:
*does this change meet the standards this repository wrote down?*

```bash
git diff origin/main... > change.patch
deep-diff-forge --stdin-patch --rank --json < change.patch \
  | forge-warden check --policy warden.toml --patch change.patch
```

```text
forge-warden 0.1.0: FAIL (exit 7 policy_fail, next: change_input)
  patch bbf580931c67 (verified), policy 9a08b3feaf2b, ranked by deep-diff-forge 0.7.0, on 2026-11-05
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
```

Real output: `fixtures/demo.patch` ranked by deep-diff-forge 0.7.0 and judged by
`examples/starter.warden.toml`. The full JSON is `fixtures/demo.verdict.json`, and a test fails if
either drifts.

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

MSRV is Rust 1.88. There are no runtime dependencies beyond the two binaries.

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

## Verdicts

`forge-warden check --json` prints a `forge-warden.verdict.v0` document
([schema](schemas/verdict.v0.schema.json)):

- `seals`: `policy_sha256`, `rank_sha256`, `patch_sha256` (deep-diff-forge's seal of the patch),
  `patch_verified` (true when `--patch` was given and its bytes hashed to that seal), `ranked_by`.
- `verdict`: `pass`, `hold` or `fail`, the strictest of every rule, note and expired waiver.
- `rules`: one result per rule in policy order, each with its violations in deep-diff-forge's
  risk order. Waived violations stay listed with `waived_by`, so exceptions remain visible.
- `expired_waivers`, `unused_waivers`, `notes`.

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
- run: git diff "origin/${BASE_REF}...HEAD" > change.patch
  env: { BASE_REF: "${{ github.base_ref }}" }
- run: |
    deep-diff-forge --stdin-patch --rank --json < change.patch > rank.json
    forge-warden check --policy warden.toml --rank rank.json --patch change.patch --json > verdict.json
```

This repository's own workflow ([`ci.yml`](.github/workflows/ci.yml), job `dogfood`) does exactly
this on every pull request: forge-warden judges its own changes with [`warden.toml`](warden.toml).
A `hold` passes only once a maintainer adds the `warden:acknowledged` label, so a person always
makes that call.

## How this repository makes bad code hard to write

forge-warden is a gate, so it is built behind one. Nothing below is a guideline; each item is
enforced by the compiler, a test, CI, or a hook, and [`CLAUDE.md`](CLAUDE.md) gives agents the same
contract.

| Layer | What it stops | Where |
| --- | --- | --- |
| **Lint wall** | `unsafe`, panics (`unwrap`, `expect`, `panic!`, indexing, unchecked arithmetic, `as` casts), missing docs, wildcard matches, `print!`, `#[allow]`; every warning is an error | `Cargo.toml` `[workspace.lints]`, [ADR 0002](docs/adr/0002-the-lint-wall.md) |
| **Effect doors** | file, stdin, stdout, clock and environment access anywhere but one reviewed module; hash-ordered collections anywhere | `clippy.toml`, [`doors.rs`](crates/forge-warden-cli/src/doors.rs), [ADR 0001](docs/adr/0001-pure-core-and-effect-doors.md) |
| **A pure core** | I/O, clocks and threads in the engine: they are compile errors in `forge-warden-core` | [`crates/forge-warden-core/clippy.toml`](crates/forge-warden-core/clippy.toml) |
| **Contracts as tests** | schema, README, registry and vocabulary drift; non-determinism; unsafe output | `crates/forge-warden-cli/tests/` |
| **Properties** | whole classes of bugs: adding a rule never loosens a verdict; dates round-trip; rendered output is always printable ASCII | `proptest` in both crates |
| **Real corpus** | a reader that only works on hand-written JSON | 39 rankings from a real deep-diff-forge binary in `fixtures/corpus/` |
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

forge-warden reads one deep-diff-forge document, `deep-diff-forge.rank.v0`, and nothing else. It
never parses a patch: it hashes one, to check the ranking's seal. Its vocabulary tracks
deep-diff-forge 0.7.0 (`crates/forge-warden-core/src/vocab.rs`); a newer deep-diff-forge that
adds a signal makes verdicts hold, not pass, until forge-warden learns it. Its exit codes reuse
deep-diff-forge's action names.

## License

MIT OR Apache-2.0, at your option.

[deep-diff-forge]: https://github.com/Louranicas/deep-diff-forge
