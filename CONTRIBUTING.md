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

## Branch protection (for the maintainer)

Protect `main` with: require a pull request; require branches to be up to date; require these
checks: `gate (fmt, clippy, test, doc)`, `declared Rust 1.88 compatibility`,
`supply chain (cargo-deny)`, `workflow audit (zizmor)`, `diff mutation (cargo-mutants)`,
`dogfood (forge-warden judges this change)`, `vocabulary tracks deep-diff-forge`; require review
from code owners; disallow bypass. Create the label `warden:acknowledged` and grant it only to
maintainers: it is how a person accepts a `hold`.

## Decisions

Architecture decisions are in [`docs/adr/`](docs/adr). A change that contradicts one starts with
a new ADR, not a code change.
