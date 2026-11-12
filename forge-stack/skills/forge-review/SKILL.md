---
name: forge-review
description: Review a change adversarially and report only findings that are anchored to real lines of the patch. Use when asked to review a diff, a branch or your own work, and before a risky change is called done.
---

# Review a change

A review is a set of claims about specific lines. A claim that points at nothing is noise, so
every finding is checked against the patch before anyone reads it.

## 1. Look before you read

```bash
forge outline                  # files in risk order, each hunk's address, signals and line ranges
forge hunks <address> [<address>…]     # exactly those hunks, verbatim
forge hunks --path src/auth/guard.rs   # or every hunk of one file
```

Read the highest-risk hunks first. Do not paste the whole diff into your context.
`forge gate` shows what the deterministic checks already found; do not repeat those.

## 2. Try to break it

For a change that matters, give the work to the two subagents and keep only their findings:

- **forge-falsifier**: one claim in ("this guard still runs on every path"), an attempt to
  refute it out. Use it on each load-bearing claim.
- **forge-reviewer**: reads the change cold and reports defects by severity.

Look for what the gate cannot see: wrong logic, a missing case, a check moved to where it no
longer runs, a test that passes for the wrong reason, behaviour the task did not ask for.
`forge-radius` tells you what else the change can reach.

## 3. Write findings as JSONL, one per line

Write them **outside the repository** (a file inside it would become part of the change):
`F="${TMPDIR:-/tmp}/findings.jsonl"`.

```json
{"agent":"claude","severity":"high","title":"Admin check removed","body":"guard() now returns true for every user.","path":"src/auth/guard.rs","line":2,"side":"new"}
```

`severity` is `low`, `medium` or `high`. `path` and `line` must be a line of the patch;
`side` is `new` (default) or `old` for a removed line.

## 4. Verify them

```bash
forge findings "$F"
```

Each finding is accepted with its anchor or **refused** with a reason (`unknown_path`,
`line_outside_hunk`, `stale_review`, …). Fix or drop every refused finding. Report only
accepted ones, and say plainly that they are anchored, not proven: a claim is yours to
demonstrate (a failing test is the best demonstration).
