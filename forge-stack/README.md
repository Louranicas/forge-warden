# forge-stack

**Agent-first tooling for [deep-diff-forge] and [forge-warden], batteries included.**
One command, three hooks, six skills and two review subagents that make it hard for a coding
agent to write bad code, at the speed an agent works (a gate run takes about 0.2 s).

```text
deep-diff-forge   senses    patch truth, risk ranking, evidence
forge-warden      judges    the repository's policy, read from the base commit
forge-stack       conducts  scope before the edit, the gate after it, hooks that enforce both
Jev (optional)    advises   a hosted second opinion that can only add a hold
```

Skills are advice. Hooks and the gate are enforcement. Nothing here relies on the agent
choosing to behave.

## Install

Requirements: Python 3.11+, Git, and the two binaries on `PATH`
(or `FORGE_DDF` / `FORGE_WARDEN` set to their paths).

```bash
cargo install --git https://github.com/Louranicas/deep-diff-forge deep-diff-forge-cli --locked
cargo install --git https://github.com/Louranicas/forge-warden forge-warden-cli --locked

cd your-repository
python3 /path/to/forge-warden/forge-stack/forge.py init            # prints the plan, writes nothing
python3 /path/to/forge-warden/forge-stack/forge.py init --apply    # writes it
git add -A && git commit -m "Install forge-stack"     # required: see below
python3 .claude/forge/forge.py doctor
```

`init` vendors the pack into `.claude/forge/`, adds the skills and subagents, merges three
hooks into `.claude/settings.json` (existing hooks are kept), and creates `forge-stack.toml`,
`warden.toml` and `forge-corpus/` when they are missing. It never overwrites a file that
differs unless you pass `--force`.

**Commit the result.** The gate reads its policy and configuration from the *base commit*, so
a change can never loosen the rules that judge it. Until they are committed, the bundled strict
defaults apply. A person runs `init`; once installed, `.claude/**` is protected from the agent.

## What an agent experiences

| Moment | What happens | Enforced by |
| --- | --- | --- |
| Session starts | The base commit is recorded and the contract is printed | `SessionStart` hook |
| First edit | Refused until a scope is declared: `forge scope declare --task … --allow …` | `PreToolUse` hook |
| Edit outside the scope | Refused; widening needs a reason and a person's approval, and is logged | `PreToolUse` hook |
| Edit to policy, CI, hooks, `.claude/` | Sent to the person to approve | `PreToolUse` hook |
| `--no-verify`, force-push, touching gate state | Refused | `PreToolUse` hook |
| Says "done" | The gate runs. FAIL blocks the session from ending; HOLD is reported to the person | `Stop` hook |

A session that keeps failing still ends (after the same failing patch twice, or five blocks),
but it ends red and says so.

## The gate

`forge gate` judges everything since the session began: commits, staged and unstaged edits and
new files. Exit `0` pass, `7` fail, `8` hold. `--json` prints `forge-stack.gate.v0`.

| Source | Checks | Severity |
| --- | --- | --- |
| forge-warden | Every rule in `warden.toml` (secrets, CI privilege, tests travel with code, …) | as the rule says |
| Integrity | `test-skipped`, `test-trivial-assert`, `stub-added` | fail |
| Integrity | `assertions-removed`, `expectations-changed`, `error-swallowed`, `panic-path-added`, `guard-removed`, `marker-added` | hold |
| Scope | A file outside the declared scope; a test changed while tests are frozen | fail |
| Scope | No scope declared; a scope of `**` | hold |
| Protected | A change to policy, configuration, CI, hooks or the corpus | hold |
| Jev | A probability at or above `hold_at` on any question | hold only |

Rules the gate keeps:

- **Unknown is never pass.** A missing tool, a refused patch or an unreadable configuration
  holds or fails.
- **The change cannot loosen its own gate.** Policy and configuration come from the base commit.
- **A hold is a person's decision.** The agent reports it and does not retry it away.
- **Patch-author text is data.** Paths and lines are escaped before they are printed.

## Commands

```text
forge gate [--json] [--base REV]        judge the session's change
forge scope declare|show|widen|clear    say what the task may touch
forge outline | forge hunks ADDRESS…    read the change cheaply, then exactly
forge findings FILE                     anchor-check review findings (JSONL)
forge radius [--json]                   where else the changed names are used
forge corpus add|run                    escaped defects, replayed for ever
forge route "TASK"                      pick a playbook (Jev, when configured)
forge doctor | forge init [--apply]     check or install the setup
```

## Skills and subagents

| Skill | Use it |
| --- | --- |
| `forge-mode` | At the start of a task: picks the playbook and the order |
| `forge-scope` | Before the first edit, and when an edit is refused |
| `forge-review` | To review a change; only anchored findings survive |
| `forge-radius` | After changing a function or type: what else can break |
| `forge-prove` | Before saying "done": the gate and an evidence-backed summary |
| `forge-correct` | When a defect escaped: corpus case first, then a proposed rule |

Subagents: `forge-falsifier` (tries to refute one claim) and `forge-reviewer` (reviews cold).

## Jev (optional)

Jev is TypeSafe's hosted decision model. It is **off** by default. To use it, set in
`forge-stack.toml` (and commit):

```toml
[oracle]
mode = "advisory"        # or "required": no answer means hold
model = "jev-1.14.0"     # must be pinned; "jev-latest" is refused
hold_at = 0.7
```

and export `TYPESAFE_API_KEY`. Each hunk is asked whether it removes a guard, weakens a test,
swallows an error, leaves a stub, or is unrelated to the declared task.

- It can only add a hold. It cannot pass, fail or clear anything.
- Files that deep-diff-forge marks `secret_like` or `hazardous_text` are never sent.
- Answers are cached by hunk content, question set and model.
- **Turning it on uploads your code to a third party.** Check TypeSafe's terms first.
- **It has not been tested against the live service.** The tests use a recorded-shape transport.

## The corpus

`forge-corpus/` holds one directory per defect that got through: the patch and the check that
must catch it. `forge corpus run` replays them all: `CAUGHT`, `ESCAPED` (fails), `KNOWN` (a
declared miss) or `STALE` (a declared miss that is now caught: remove the flag).

## What this does not do

- **The integrity checks are lexical.** They read changed lines. A pattern inside a comment or
  a string can match, and code assembled at run time is missed. `guard-removed` and
  `expectations-changed` are heuristics that hold, never fail, for that reason. An honest change
  to a test's expectation gets a hold so a person confirms it.
- **`forge radius` is a text search, not a call graph.**
- **The hooks are a filter inside the agent's process, not a sandbox.** The shell-command
  checks are pattern matches and can be evaded by a determined agent. The authority for what
  merges is forge-warden's authority mode in CI, with the policy from the base branch.
- **The local gate runs forge-warden in advisory mode** and treats its `rank_unverified` note as
  expected, because the gate ran the ranker itself. It is a fast inner loop, not an approval.
- **Claude Code only.** The hooks use Claude Code's hook contract.

## Tests

```bash
cd forge-stack && python3 -B -m unittest discover -s tests -v
```

35 tests. Ten run the real binaries end to end and are skipped, not passed, when either binary
is absent. Last run against deep-diff-forge 0.8.2 and forge-warden 0.2.0.

[deep-diff-forge]: https://github.com/Louranicas/deep-diff-forge
[forge-warden]: https://github.com/Louranicas/forge-warden
