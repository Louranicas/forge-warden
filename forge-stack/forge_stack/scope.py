"""Declared scope: the agent says what a task may touch before it edits.

The declaration lives in the Git directory, so it is never part of the change it limits.
It is declared once per session. Later additions go through `widen`, which is logged and
shown in every gate result, and which the pre-tool hook sends to the person for approval.
"""

from __future__ import annotations

import datetime
from pathlib import Path

from .core import is_test_path, matches, read_json, state_dir, write_json

UNBOUNDED = {"**", "*", "**/*", "./**"}


def now() -> str:
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def path_of(root: Path) -> Path:
    return state_dir(root) / "scope.json"


def load(root: Path) -> dict | None:
    scope = read_json(path_of(root), None)
    return scope if isinstance(scope, dict) and scope.get("allow") else None


def validate(globs: list[str]) -> list[str]:
    problems = []
    for glob in globs:
        if not glob or glob.startswith(("/", "../")) or "/../" in glob or glob.startswith("-"):
            problems.append(f"`{glob}` must be a path glob relative to the repository root")
    return problems


def declare(root: Path, task: str, allow: list[str], tests: str, base: str | None) -> dict:
    if load(root):
        raise ValueError("a scope is already declared for this session; use `scope widen`")
    if len(task.strip()) < 12:
        raise ValueError("--task must describe the task (12+ characters)")
    problems = validate(allow)
    if not allow or problems:
        raise ValueError("; ".join(problems) or "at least one --allow glob is required")
    scope = {"schema": "forge-stack.scope.v0", "task": task.strip(), "allow": sorted(set(allow)),
             "tests": tests, "base": base, "declared_at": now(), "widenings": []}
    write_json(path_of(root), scope)
    return scope


def widen(root: Path, globs: list[str], reason: str) -> dict:
    scope = load(root)
    if not scope:
        raise ValueError("no scope is declared; use `scope declare`")
    if len(reason.strip()) < 12:
        raise ValueError("--reason must say why the task needs this (12+ characters)")
    problems = validate(globs)
    if not globs or problems:
        raise ValueError("; ".join(problems) or "at least one --allow glob is required")
    for glob in globs:
        scope["widenings"].append({"glob": glob, "reason": reason.strip(), "at": now()})
    write_json(path_of(root), scope)
    return scope


def clear(root: Path) -> None:
    path_of(root).unlink(missing_ok=True)


def allowed(scope: dict) -> list[str]:
    return scope["allow"] + [w["glob"] for w in scope.get("widenings", [])]


def permits(scope: dict, path: str) -> bool:
    """Whether the scope admits a path. Test files are admitted unless tests are frozen."""
    if is_test_path(path):
        return scope.get("tests") != "frozen"
    return matches(path, allowed(scope))


def judge(scope: dict | None, required: bool, paths: list[str]) -> list[dict]:
    """Scope items for the changed paths, in the order given."""

    def item(severity: str, path: str | None, why: str, fix: str) -> dict:
        return {"check": "scope", "severity": severity, "path": path, "line": None, "side": None,
                "address": "", "text": "", "why": why, "fix": fix, "source": "scope"}

    if not scope:
        if required and paths:
            return [item("hold", None, "No scope was declared for this change, so nothing says what it was meant to touch.",
                         "Declare one before editing: forge scope declare --task … --allow …")]
        return []
    out = []
    if any(glob in UNBOUNDED for glob in allowed(scope)):
        out.append(item("hold", None, "The declared scope admits every path, so it limits nothing.",
                        "Declare the directories this task needs."))
    for path in paths:
        if is_test_path(path) and scope.get("tests") == "frozen":
            out.append(item("fail", path, "Tests are frozen for this task and this test file changed.",
                            "Revert it; if the test is wrong, stop and ask."))
        elif not permits(scope, path):
            out.append(item("fail", path, "This file is outside the scope declared for the task.",
                            "Revert it, or ask to widen: forge scope widen --allow … --reason …"))
    return out
