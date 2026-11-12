---
name: forge-correct
description: Turn a defect that got past the gate into a permanent corpus case and a proposed rule, so the same mistake is caught mechanically next time. Use when a bug, weakened test or bad change was found by a person or later than it should have been.
---

# Correct at the highest level

When a defect escapes, fixing it once is the least valuable response. Fix the thing that let it
through. Prefer, in order: a check or policy rule → a test → a line in this skill. More prose is
the last resort, because prose is not enforced.

## 1. Capture the defect as a case

Get the bad change as a patch (the offending commit, or the working tree as it was):

```bash
git diff --binary --no-renames <before>..<after> > /tmp/escape.patch
forge corpus add --id unwrap-in-request-path \
  --origin "PR #212: agent replaced ? with unwrap in the handler; crashed on bad input" \
  --expect panic-path-added:src/handler.rs --patch /tmp/escape.patch
```

`--expect CHECK:PATH` names the check that **should** flag it and the file it should flag.
Check names are the ones `forge gate` prints.

## 2. See whether the gate catches it now

```bash
forge corpus run        # CAUGHT · ESCAPED · KNOWN (declared miss) · STALE
```

- **CAUGHT**: the gate already sees it. The escape was a process failure (the gate was not run,
  or a hold was waved through). Say which, to the person.
- **ESCAPED**: no check sees it. Go to step 3. If it cannot be closed today, re-add the case
  with `--known-miss` so the gap is declared, not hidden. A declared miss that later gets caught
  shows as STALE until the flag is removed.

## 3. Propose the fix, do not apply it

`forge-corpus/`, the policy and the checks are protected: a person approves changes to them.
Write the proposal:

- **A policy rule**, if an existing signal or path pattern covers it (`warden.toml`:
  `forbid_signal`, `require_companion`, `protect_paths`, `max_score`, `max_files`).
- **A new integrity check or deep-diff-forge signal**, if it is a pattern in the changed lines.
  Give the pattern, one true positive (this case) and one hard negative that must not match.
- **A test in the project**, if the defect is specific to this codebase's behaviour.

State what the proposal would **not** catch. A check that claims more than it does is worse
than a declared miss.
