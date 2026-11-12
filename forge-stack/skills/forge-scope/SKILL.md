---
name: forge-scope
description: Declare what a task may touch before editing, so out-of-scope edits are refused instead of reviewed. Use before the first edit of any task, and whenever an edit is refused as outside the scope.
---

# Declare the scope

An edit that should not exist is the cheapest defect to prevent and the hardest to notice in
review. Say what the task touches first; the pre-tool hook then refuses everything else.

## Declare (once per task)

```bash
forge scope declare --task "Fix the off-by-one in pagination cursors" \
  --allow "src/pagination/**" [--allow "docs/pagination.md"] [--tests frozen]
```

- `--task`: one sentence a reviewer could check the diff against.
- `--allow`: the narrowest globs that cover the work. `*` stays in one directory, `**` crosses.
  A scope of `**` is reported as a hold: it limits nothing.
- Test files are in scope by default (`--tests open`). Use `--tests frozen` for a refactor:
  behaviour must not change, so no test may.

Decide the scope from what you have **read**, not guessed: find the code first
(`forge-mode` → investigate), then declare.

## When an edit is refused

Stop and ask which is true:

1. **The edit is not needed.** Usually the case. Solve the task inside the scope.
2. **The task really needs it.** Ask to widen. A person approves, and every widening is listed
   in every gate result:

   ```bash
   forge scope widen --allow "src/cursor.rs" --reason "pagination calls Cursor::next, which has the bug"
   ```

3. **The file is protected** (policy, CI, hooks, `.claude/`): that is not a scope question.
   Tell the person what you would change and why, and let them do it.

Never work around a refusal with a shell command that writes the file another way.

`forge scope show` prints the current declaration.
