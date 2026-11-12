---
name: forge-radius
description: Find what else a change can break by listing where the names it touches are used elsewhere in the repository. Use after changing a function, type or public interface, and before calling a bug fix or refactor done.
---

# Blast radius

A small diff can break code it never mentions. Check the callers before the gate, not after
the bug report.

```bash
forge radius            # or: forge radius --json
```

It lists each function, type or class the change defines or sits inside, and every other place
that name appears.

This is a **textual search** (`git grep -w`), not a call graph. It over-reports common names
and misses dynamic use (reflection, string-built calls, macros). It tells you where to look,
never what is safe.

For each name with uses elsewhere:

1. Read the callers that matter (`forge hunks` for changed ones, the file for the rest).
2. Decide whether the change alters what they rely on: arguments, return value, errors,
   ordering, side effects.
3. If it does, either the caller is in scope and you update it with a test, or it is not and
   you stop and ask (see `forge-scope`).
4. Prove it by **running** something: the caller's tests, or a new test at the boundary.
   "I read it and it looks fine" is a belief, not a verification.

A name with zero uses elsewhere that you added is a candidate for deletion: unused code is not
a feature.
