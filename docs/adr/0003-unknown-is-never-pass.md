# ADR 0003: Unknown is never pass

**Status:** accepted, 2026-11-05

## Context

A gate fails open when it treats "I could not tell" as "nothing wrong". deep-diff-forge adds
vocabulary additively without renaming its schema, so forge-warden will meet signals and
statuses it has never seen. Empty input is the other classic hole: a gate that judges nothing
reports nothing wrong.

## Decision

- An empty ranking, an unknown signal and an unknown status each add a `note` and hold the
  verdict (exit 8). They never fail it (the change may be fine) and never pass it.
- An expired waiver fails the verdict whether or not it still matches anything, so exemptions
  cannot outlive their date silently.
- A policy with no rules, or naming a signal the vocabulary lacks, is refused at load (exit 4):
  the author meant something the tool cannot enforce.
- The overall verdict is the maximum of every contribution under `pass < hold < fail`. The
  property `adding_a_rule_never_loosens_the_verdict` checks monotonicity over random inputs.

## Consequences

Upgrading deep-diff-forge before forge-warden produces holds, not silent passes. CI's
`vocabulary` job compares `KNOWN_SIGNALS` with deep-diff-forge's source at the pinned commit, so
drift is caught before it reaches users.
