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

The reviewed exclusions are `.git` metadata directories at any depth and root `target/` and
`mutants.out*/` output directories (including output-directory symlinks). Nested source directories
named `target` and ordinary Markdown files named `mutants.out.*.md` remain visible. Other source
symlinks refuse rather than hiding or following a document subtree. No ignore file controls this walk.

Executable examples use the documented shell-fence and literal workflow subset. Inline prose
fragments and data fences remain outside command parsing. Tests parse arguments with the actual
CLI without executing shell expansions or promising example paths or tools are available.

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
