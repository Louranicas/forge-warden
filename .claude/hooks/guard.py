#!/usr/bin/env python3
"""Claude Code hooks for forge-warden. One script, three modes (see .claude/settings.json):

  guard.py pre   PreToolUse (Edit|Write|Bash): refuse edits that loosen the quality wall, add
                 `#[allow]` or `unsafe`, or bypass the git hooks. Exit 2 blocks the tool call and
                 shows stderr to Claude.
  guard.py post  PostToolUse (Edit|Write): rustfmt the Rust file just written.
  guard.py stop  Stop: refuse to end the session while the fast gate (scripts/ci_fast.sh) is red,
                 at most MAX_STOP_BLOCKS times in a row, so a stuck session still ends.

Standard library only: the hook must run anywhere the repository is checked out.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

MAX_STOP_BLOCKS = 3

# Files that decide what "good" means. Changing them is the owner's call (warden.toml rule
# `quality-config-is-owned`, CODEOWNERS). Cargo.toml is handled separately: its dependencies may
# change, its lint wall may only tighten.
PROTECTED = (
    re.compile(r"^clippy\.toml$"),
    re.compile(r"^crates/[^/]+/clippy\.toml$"),
    re.compile(r"^deny\.toml$"),
    re.compile(r"^rustfmt\.toml$"),
    re.compile(r"^warden\.toml$"),
    re.compile(r"^\.github/"),
    re.compile(r"^\.githooks/"),
    re.compile(r"^\.claude/settings\.json$"),
    re.compile(r"^\.claude/hooks/"),
    re.compile(r"^crates/forge-warden-cli/tests/repo_invariants\.rs$"),
    re.compile(r"^crates/forge-warden-cli/src/doors\.rs$"),
)

RUST_REFUSALS = (
    (re.compile(r"#!?\[allow\("), "use #[expect(lint, reason = \"...\")] instead of #[allow]: it fails once it is no longer needed (ADR 0002)"),
    (re.compile(r"\bunsafe\s*(\{|fn\b|impl\b|trait\b|extern\b)"), "unsafe code is forbidden in this workspace (unsafe_code = \"forbid\")"),
    (re.compile(r"#!?\[expect\((?![^\]]*reason\s*=)[^\]]*\]"), "every #[expect] needs reason = \"...\" saying why the lint is wrong here"),
)

BASH_REFUSALS = (
    (re.compile(r"--no-verify\b"), "do not skip the git hooks; fix what they report"),
    (re.compile(r"\bgit\s+commit\b[^\n]*\s-n\b"), "do not skip the git hooks (-n is --no-verify); fix what they report"),
    (re.compile(r"\bgit\s+push\b[^\n]*(--force\b|--force-with-lease\b|\s-f\b)"), "do not force-push; history is evidence"),
    (re.compile(r"\bcore\.hooksPath\b(?!\s+\.githooks\b)"), "the hooks path is .githooks; do not point it elsewhere"),
    (re.compile(r"\bscripts/bless\.sh\b|\bjust\s+bless\b"), "blessing golden files changes a contract; ask the human to run it and review the diff"),
)


def project_dir() -> Path:
    return Path(os.environ.get("CLAUDE_PROJECT_DIR") or Path(__file__).resolve().parents[2])


def refuse(message: str) -> None:
    print(f"forge-warden guard: {message}", file=sys.stderr)
    sys.exit(2)


def relative(path_text: str) -> str:
    root = project_dir().resolve()
    path = Path(path_text)
    if not path.is_absolute():
        path = root / path
    try:
        return path.resolve().relative_to(root).as_posix()
    except ValueError:
        return path.as_posix()


def proposed_text(tool: str, tool_input: dict, current: str) -> str:
    """The file's text after the tool call, as far as it can be known before it runs."""
    if tool == "Write":
        return tool_input.get("content", "")
    old, new = tool_input.get("old_string", ""), tool_input.get("new_string", "")
    if tool_input.get("replace_all"):
        return current.replace(old, new)
    return current.replace(old, new, 1)


def lint_lines(cargo_toml: str) -> list[str]:
    """Non-comment lines inside [workspace.lints.*] tables."""
    lines, inside = [], False
    for raw in cargo_toml.splitlines():
        line = raw.split("#", 1)[0].strip()
        if line.startswith("["):
            inside = line.startswith("[workspace.lints")
            continue
        if inside and line:
            lines.append(line)
    return lines


def check_cargo_toml(tool: str, tool_input: dict, path: Path) -> None:
    current = path.read_text() if path.exists() else ""
    before, after = lint_lines(current), lint_lines(proposed_text(tool, tool_input, current))
    removed = [line for line in before if line not in after]
    loosened = [line for line in after if line not in before and re.search(r'"(allow|warn)"', line)]
    if removed or loosened:
        detail = "; ".join([*(f"removes `{l}`" for l in removed), *(f"adds `{l}`" for l in loosened)])
        refuse(f"Cargo.toml: the lint wall may only tighten ({detail}). Ask the human if a lint is wrong.")


def pre(event: dict) -> None:
    tool, tool_input = event.get("tool_name", ""), event.get("tool_input") or {}
    if tool == "Bash":
        command = tool_input.get("command", "")
        for pattern, why in BASH_REFUSALS:
            if pattern.search(command):
                refuse(why)
        return
    if tool not in ("Edit", "Write"):
        return
    rel = relative(tool_input.get("file_path", ""))
    if any(p.search(rel) for p in PROTECTED):
        refuse(
            f"{rel} is part of the quality wall (warden.toml rule `quality-config-is-owned`). "
            "Do not edit it: tell the human what you need changed and why, and let them decide."
        )
    if rel == "Cargo.toml":
        check_cargo_toml(tool, tool_input, project_dir() / rel)
    if rel.endswith(".rs"):
        text = tool_input.get("content") if tool == "Write" else tool_input.get("new_string")
        for pattern, why in RUST_REFUSALS:
            if text and pattern.search(text):
                refuse(f"{rel}: {why}")


def post(event: dict) -> None:
    rel = relative((event.get("tool_input") or {}).get("file_path", ""))
    if not rel.endswith(".rs"):
        return
    result = subprocess.run(
        ["rustfmt", "--edition", "2024", str(project_dir() / rel)],
        capture_output=True, text=True, check=False,
    )
    if result.returncode != 0:
        print(f"rustfmt could not format {rel}:\n{result.stderr[-2000:]}", file=sys.stderr)
        sys.exit(2)


def stop(event: dict) -> None:
    root = project_dir()
    dirty = subprocess.run(["git", "status", "--porcelain"], cwd=root, capture_output=True, text=True, check=False)
    if dirty.returncode == 0 and not dirty.stdout.strip():
        return  # nothing changed in this session's working tree
    counter = Path(tempfile.gettempdir()) / f"forge-warden-stop-{event.get('session_id', 'unknown')}"
    blocks = int(counter.read_text()) if counter.exists() else 0
    gate = subprocess.run([str(root / "scripts/ci_fast.sh")], cwd=root, capture_output=True, text=True, check=False)
    if gate.returncode == 0:
        counter.unlink(missing_ok=True)
        return
    if blocks >= MAX_STOP_BLOCKS:
        counter.unlink(missing_ok=True)
        print(
            f"forge-warden guard: the fast gate is still red after {blocks} attempts; stopping. "
            "Tell the human exactly what fails and what you tried.",
            file=sys.stderr,
        )
        return
    counter.write_text(str(blocks + 1))
    tail = "\n".join((gate.stdout + gate.stderr).splitlines()[-60:])
    refuse(
        "the fast gate (scripts/ci_fast.sh: fmt, clippy, tests) is red. Fix it before finishing; "
        f"do not weaken a check to make it pass.\n{tail}"
    )


def main() -> None:
    mode = sys.argv[1] if len(sys.argv) > 1 else ""
    try:
        event = json.load(sys.stdin)
    except json.JSONDecodeError:
        event = {}
    {"pre": pre, "post": post, "stop": stop}.get(mode, lambda _e: None)(event)


if __name__ == "__main__":
    main()
