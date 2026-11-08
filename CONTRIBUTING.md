# Contributing

Thank you. forge-warden is a gate, so its own bar is high, and most of that bar is automated:
you find out in seconds, not in review.

## Setup

```bash
just install-hooks   # pre-commit: fast gate; commit-msg: useful subjects; pre-push: full gate
just fast            # fmt + clippy (the full lint wall) + tests
just ci              # everything CI runs that works offline
```

Optional tools the full gate uses when present: `cargo-deny`, `cargo-mutants`, `zizmor`,
`deep-diff-forge`.

## What a pull request needs

The [pull request template](.github/pull_request_template.md) is the checklist. In short: the
gate is green, new behaviour has a test that fails without it, contract changes (schemas, exit
codes, vocabulary) are in the CHANGELOG, and the dogfood verdict passes or every hold is
explained.

Read [`CLAUDE.md`](CLAUDE.md) even if you are not an agent: it is the shortest statement of the
rules, with what to do instead of each forbidden move.

## Command examples

The documentation contract walks source files directly and works without Git metadata, including
in archives and cargo-mutants copies. Untracked `.md` files (including `.MD`) are visible immediately.
Every source Markdown file and the justfile needs an exact invocation count in
`docs/command-sources.tsv`; update that inventory with an intentional example change.

The reviewed exclusions are `.git` metadata entries at any depth and root `target/`,
`mutants.out/`, and `mutants.out.old/` output directories (including output-directory symlinks).
Root `target/` is reserved for Cargo
build artifacts, including generated documentation; source documentation must live elsewhere.
`mutants.out.old/` is cargo-mutants’ rotated previous output. These are exact directory names;
`mutants.outline/` and `mutants.out.backup/` remain source-visible. Nested source directories
named `target` and ordinary Markdown files named `mutants.out.*.md` remain visible. Other source
symlinks refuse rather than hiding or following a document subtree. No ignore file controls this walk.

The test-only pulldown-cmark parser classifies Markdown at every nesting depth. Every code
block receives a language decision: indented code is parsed as shell; fenced code uses the
explicit supported shell, literal YAML, or data language. Unknown or absent fence languages
refuse. Continued shell lines are parsed together; use `text` for recorded output.

Every raw HTML event (block or inline) containing a `pre`, `xmp`, `listing`, `code`, `samp`, or
`kbd` opener refuses. There is no raw-HTML allowlist. Real inline backtick code is a distinct
parser event and remains prose; so do ordinary paragraphs. A fence-looking line inside an
HTML block is HTML, not a data exemption. Genuine data fences after the HTML block ends remain
supported. The parser determines those boundaries; tests do not infer them from tag strings.

Per-file invocation counts remain exact. The shell lexer and literal YAML subset still have
explicit limits: no command expansion or example subprocess is executed, and example paths or
tools need not exist. These tests check actual CLI arguments, not general shell semantics.

## Oracles and golden review

`fixtures/quality-contract.json` declares independent Rust oracle sources and blessed output paths.
Oracle sources are self-contained fixtures: the syn check rejects production crate/parent paths,
non-standard imports, includes/out-of-line modules, and unreviewed attributes/macros. Pure
standard-library macros use explicit `std::` or `core::` paths. This detects structural coupling;
it does not prove that two independently written algorithms cannot share the same conceptual bug.

Ordinary golden tests never write expected output, even if `FORGE_WARDEN_BLESS` is inherited.
An independent verifier builds the reviewed subject and checks its output with
`python3 -I scripts/golden_output.py --binary target/debug/forge-warden`.
The explicit operator-only `scripts/bless.sh` entrypoint can regenerate after intended changes;
it refuses CI and records binary/output hashes plus an outside-repository backup. The verifier
must compare exact bytes; the builder's own re-blessing is not independent evidence.

All blessed outputs belong under the reviewed `fixtures/` namespace. The trusted base's
`golden-output-needs-review` HOLD covers that entire namespace, including new registered outputs;
it is separate from generic quality ownership. A candidate cannot waive that obligation by
editing its own policy. This new rule becomes authoritative after integration into the trusted
base; an older base retains its existing generic fixture-ownership HOLD. Owner review and the
existing trusted acknowledgement mechanism remain required; an unsigned rationale grants nothing.

## Branch protection (for the maintainer)

Protect `main` with: require a pull request; require branches to be up to date; require these
checks: `gate (fmt, clippy, test, doc)`, `declared Rust 1.88 compatibility`,
`supply chain (cargo-deny)`, `workflow audit (zizmor)`, `diff mutation (cargo-mutants)`,
`dogfood (trusted base judges candidate data)`, `vocabulary tracks deep-diff-forge`; require
review from code owners; disallow bypass.

A `hold` is accepted only with an OpenSSH-signed acknowledgement from a reviewer enrolled in
`.github/hold-reviewers/allowed_signers` on the base branch (ADR 0007). A label grants nothing.
Until reviewer keys are enrolled and an external trust root enforces the check (ADR 0006), CI
fails closed on every hold.

## Periodic full mutation baseline

`mutation-baseline.yml` runs each Monday at 04:23 UTC and can be started with
`workflow_dispatch`. It is deliberately not a required pull-request check. Unlike diff mutation,
it selects the full workspace with no diff filter or repository mutation config. It keeps
`--cap-lints true`, clears inherited `CARGO_TARGET_DIR`, and uses the canonical tool pins checked
by `scripts/tool_versions.py`. Tests run for each mutated package (no `--test-workspace`).

Any MISSED mutant makes the job fail; there is no survivor budget or automatic waiver. The
always-uploaded artifact contains `mutants.out` and exact-subject/tool provenance, including failed
runs. Review caught, missed, unviable and timeout counts separately. An interrupted run is partial
evidence, not a completed baseline. The weekly job does not replace diff mutation on pull requests.

## Decisions

Architecture decisions are in [`docs/adr/`](docs/adr). A change that contradicts one starts with
a new ADR, not a code change.
