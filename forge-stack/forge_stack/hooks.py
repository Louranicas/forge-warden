"""Claude Code hooks. Skills are advice; these are the enforcement.

  session-start  records the commit the session starts from and prints the contract
  pre-tool       before an edit or a shell command: scope, protected files, bypass attempts
  stop           runs the gate; a failing change cannot be declared finished

A hook is a filter inside the agent's own process, not a sandbox. The authority that decides
what merges is forge-warden in CI, with the policy taken from the base branch.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

from . import gate, scope as scope_mod
from .core import (EnvError, base_commit, find_tool, head, load_config, matches, read_json,
                   repo_root, safe, state_dir, write_json)

EDIT_TOOLS = {"Edit", "Write", "MultiEdit", "NotebookEdit"}
MAX_STOP_BLOCKS = 5
FORGE_CMD = 'python3 "$(git rev-parse --show-toplevel)/.claude/forge/forge.py"'

NO_VERIFY = re.compile(r"--no-verify\b|\bgit\s+commit\b[^|;&]*\s-[a-zA-Z]*n[a-zA-Z]*\b")
FORCE_PUSH = re.compile(r"\bgit\s+push\b[^|;&]*(--force\b|--force-with-lease\b|\s-[a-zA-Z]*f\b)")
STATE_TOUCH = re.compile(r"\.git/forge\b")
SCOPE_CHANGE = re.compile(r"forge(\.py)?[\"']?\s+scope\s+(widen|clear)\b")
HOOKS_PATH = re.compile(r"core\.hooksPath|\bgit\s+config\b[^|;&]*hooks", re.I)
WRITES = re.compile(r">|\btee\b|\bsed\s+(-[a-zA-Z]*i|--in-place)|\brm\b|\bmv\b|\bcp\b|\bchmod\b|\btruncate\b"
                    r"|\bdd\b|\bperl\s+-[a-zA-Z]*i|\bpython3?\s+-c\b|\bgit\s+(checkout|restore|apply|stash|reset)\b")
HARMLESS = re.compile(r"\S*\.claude/forge/forge\.py[\"']?|[12]?>&[12]|[12]?>\s*/dev/null")


def payload() -> dict:
    try:
        data = json.load(sys.stdin)
        return data if isinstance(data, dict) else {}
    except ValueError:
        return {}


def root_for(data: dict) -> Path:
    cwd = data.get("cwd")
    return repo_root(Path(cwd) if cwd and Path(cwd).is_dir() else None)


# ---------------------------------------------------------------- session-start

def session_start(data: dict) -> int:
    try:
        root = root_for(data)
    except EnvError:
        return 0  # not a repository: the pack is inactive
    state = state_dir(root)
    session = read_json(state / "session.json", {})
    session_id = str(data.get("session_id", ""))
    if not session or session.get("session_id") != session_id:
        session = {"session_id": session_id, "base": head(root)}
        write_json(state / "session.json", session)
        scope_mod.clear(root)  # a scope belongs to one task; a new session declares its own
        (state / "stop.json").unlink(missing_ok=True)
    cfg = load_config(root, session.get("base"))
    scope = scope_mod.load(root)
    missing = [name for name, env in (("deep-diff-forge", "FORGE_DDF"), ("forge-warden", "FORGE_WARDEN"))
               if not find_tool(name, env)]
    lines = [
        "# forge-stack is active in this repository",
        f"Run forge as: {FORGE_CMD} <command>",
        "The contract (enforced by hooks, not by this text):",
        f"1. Before your first edit, declare the task's scope ({'required' if cfg.scope == 'required' else 'advisory'}): "
        "forge scope declare --task \"…\" --allow \"src/area/**\" [--tests frozen]. Edits outside it are refused.",
        "2. Read changes through forge: `forge outline`, then `forge hunks <address>`. Do not paste whole diffs.",
        "3. Before you say a change is done, run `forge gate`. FAIL means fix the code. HOLD means report it to the person.",
        "4. You cannot finish a session while the gate fails. Policy, configuration, CI and hooks are not yours to edit.",
        "5. Weakening a test, adding a stub or swallowing an error fails the gate. Fix the cause instead.",
        "Skills: forge-mode (start here), forge-scope, forge-review, forge-radius, forge-prove, forge-correct.",
        f"Base commit for this session: {(session.get('base') or 'none (empty repository)')[:12]}",
        f"Scope: {safe(scope['task'], 80) + ' | ' + ', '.join(scope_mod.allowed(scope)) if scope else 'not declared'}",
    ]
    if missing:
        lines.append(f"WARNING: not installed: {', '.join(missing)}. The gate cannot pass without them; "
                     "tell the person, do not work around it.")
    print("\n".join(lines))
    return 0


# ---------------------------------------------------------------- pre-tool

def deny(reason: str) -> int:
    print(f"forge-stack refused this: {reason}", file=sys.stderr)
    return 2


def ask(reason: str) -> int:
    print(json.dumps({"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "ask",
                                             "permissionDecisionReason": f"forge-stack: {reason}"}}))
    return 0


def judge_edit(root: Path, raw: str) -> tuple[str, str]:
    """('allow' | 'ask' | 'deny', reason) for an edit to a path."""
    target = Path(raw)
    target = target if target.is_absolute() else root / target
    try:
        rel = target.resolve().relative_to(root.resolve()).as_posix()
    except ValueError:
        return "allow", ""  # outside the repository: not this gate's business
    if rel == ".git" or rel.startswith(".git/"):
        return "deny", "the Git directory holds the gate's own state"
    cfg = load_config(root, base_commit(root))
    if matches(rel, cfg.protected):
        return "ask", f"`{rel}` is protected: it decides what the other checks mean"
    scope = scope_mod.load(root)
    if not scope:
        if cfg.scope == "required":
            return "deny", ("no scope is declared. Declare what this task may touch first: "
                            "forge scope declare --task \"…\" --allow \"<glob>\"")
        return "allow", ""
    if not scope_mod.permits(scope, rel):
        why = ("tests are frozen for this task" if scope.get("tests") == "frozen" and scope_mod.is_test_path(rel)
               else f"`{rel}` is outside the declared scope ({', '.join(scope_mod.allowed(scope))})")
        return "deny", (f"{why}. If the task truly needs it, ask: forge scope widen --allow \"<glob>\" "
                        "--reason \"…\" (a person approves)")
    return "allow", ""


def judge_bash(root: Path, command: str) -> tuple[str, str]:
    if NO_VERIFY.search(command):
        return "deny", "skipping Git hooks is not allowed; fix what the hook reports"
    if FORCE_PUSH.search(command):
        return "deny", "force-pushing is not allowed"
    if STATE_TOUCH.search(command):
        return "deny", "the gate's state in .git/forge is changed only through the forge command"
    if SCOPE_CHANGE.search(command):
        return "ask", "the agent is asking to change its own declared scope"
    if HOOKS_PATH.search(command):
        return "ask", "this changes which Git hooks run"
    stripped = HARMLESS.sub(" ", command)
    if WRITES.search(stripped):
        cfg = load_config(root, base_commit(root))
        for pattern in cfg.protected:
            literal = pattern.split("*", 1)[0]
            if literal and literal in stripped:
                return "ask", f"this command may write to protected `{pattern}`"
    return "allow", ""


def pre_tool(data: dict) -> int:
    tool, tool_input = data.get("tool_name", ""), data.get("tool_input") or {}
    try:
        root = root_for(data)
    except EnvError:
        return 0
    try:
        if tool in EDIT_TOOLS:
            raw = tool_input.get("file_path") or tool_input.get("notebook_path") or ""
            decision, reason = judge_edit(root, raw) if raw else ("ask", "an edit with no path")
        elif tool == "Bash":
            decision, reason = judge_bash(root, str(tool_input.get("command", "")))
        else:
            return 0
    except Exception as exc:  # noqa: BLE001 - a broken guard must not silently allow
        return ask(f"the guard could not judge this ({type(exc).__name__}: {safe(str(exc), 120)})")
    return {"allow": lambda: 0, "ask": lambda: ask(reason), "deny": lambda: deny(reason)}[decision]()


# ---------------------------------------------------------------- stop

def message(text: str) -> int:
    print(json.dumps({"systemMessage": text}))
    return 0


def stop(data: dict) -> int:
    try:
        root = root_for(data)
    except EnvError:
        return 0
    try:
        result = gate.judge_worktree(root)
    except EnvError as exc:
        return message(f"forge gate could not run, so this change is UNJUDGED: {exc}")
    state_path = state_dir(root) / "stop.json"
    if result["verdict"] == "pass":
        state_path.unlink(missing_ok=True)
        return 0
    text = gate.render(result)
    if result["verdict"] == "hold":
        return message("forge gate: HOLD. A person must decide before this merges.\n" + text)
    state = read_json(state_path, {})
    blocks = int(state.get("blocks", 0))
    if state.get("patch") == result["patch_sha256"] or blocks >= MAX_STOP_BLOCKS:
        # Nothing changed since the last refusal (or the cap is reached): a stuck session must
        # still end, but it ends red and says so.
        return message("forge gate: FAIL. The session ended with a failing change; do not merge it.\n" + text)
    write_json(state_path, {"patch": result["patch_sha256"], "blocks": blocks + 1})
    print("The change fails the gate, so the work is not finished. Fix each FAIL below "
          "(the code, never the policy or the tests' expectations), then stop again.\n" + text, file=sys.stderr)
    return 2


def main(name: str) -> int:
    handlers = {"session-start": session_start, "pre-tool": pre_tool, "stop": stop}
    if name not in handlers:
        print(f"unknown hook: {name}", file=sys.stderr)
        return 2
    return handlers[name](payload())
