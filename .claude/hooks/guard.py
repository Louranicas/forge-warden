#!/usr/bin/env python3
"""Claude Code hooks for forge-warden. One script, three modes (see .claude/settings.json):

  guard.py pre   PreToolUse (all tools; unknown tools refuse): refuse edits that loosen the quality wall, add
                 `#[allow]` or `unsafe`, or bypass the git hooks. Exit 2 blocks the tool call and
                 shows stderr to Claude.
  guard.py post  PostToolUse (Edit|Write): rustfmt the Rust file just written.
  guard.py stop  Stop: refuse to end the session while the fast gate (scripts/ci_fast.sh) is red,
                 at most MAX_STOP_BLOCKS times in a row, so a stuck session still ends.

Python 3.11+ standard library. This is a convenience filter, not a sandbox or approval authority.
CI/compiler invariants enforce source properties; the host must isolate protected files.
A project path or claimed session_id is not authenticated session identity.
"""

from __future__ import annotations

import hashlib
import json
import os
import re
import shlex
import tomllib
import subprocess
import sys
import tempfile
from pathlib import Path

MAX_STOP_BLOCKS = 3

def load_protected(root: Path) -> list[str]:
    paths = [line.strip() for line in (root / "protected-paths.txt").read_text().splitlines()
             if line.strip() and not line.lstrip().startswith("#")]
    if not paths or paths != sorted(set(paths)):
        raise ValueError("protected-path manifest must be nonempty, sorted and unique")
    for path in paths:
        parts = path.split("/")
        if any(part in ("", ".", "..") or not re.fullmatch(r"[A-Za-z0-9_.-]+|\*|\*\*", part)
               for part in parts) or "**" in parts[:-1]:
            raise ValueError("unsupported protected path pattern")
    return paths


def is_protected(rel: str, root: Path) -> bool:
    for pattern in load_protected(root):
        if pattern.endswith("/**") and rel == pattern[:-3]:
            return True
        expression = "/".join(".*" if part == "**" else "[^/]+" if part == "*" else re.escape(part)
                              for part in pattern.split("/"))
        if re.fullmatch(expression, rel):
            return True
    return False


# Shell constructs that can change a file. A command that names a protected path and contains one
# of these is refused; reading a protected file (cat, grep, git diff) is fine.
WRITES_IN_COMMAND = re.compile(
    r">|\bsed\b[^|;&]*\s-i|\bperl\b[^|;&]*\s-i|\b(mv|rm|cp|tee|truncate|chmod|ln|install|dd|patch)\b"
    r"|\bgit\s+(mv|rm|checkout|restore|apply|am|reset|stash)\b|\b(python3?|node|ruby)\b"
)

RUST_REFUSALS = (
    (re.compile(r"#\s*!?\s*\[\s*(cfg_attr\s*\([^\]]*)?allow\s*\("), "use #[expect(lint, reason = \"...\")] instead of #[allow]: it fails once it is no longer needed (ADR 0002)"),
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
        refuse("path escapes the configured project root")


def required_text(data: dict, key: str, nonempty: bool = False) -> str:
    value = data.get(key)
    if not isinstance(value, str) or (nonempty and not value) or "\0" in value:
        refuse(f"malformed tool input: {key} must be a string" + (" and nonempty" if nonempty else ""))
    return value


def proposed_text(tool: str, data: dict, current: str) -> str:
    if tool == "Write":
        return required_text(data, "content")
    old, new = required_text(data, "old_string", True), required_text(data, "new_string")
    replace_all = data.get("replace_all", False)
    if type(replace_all) is not bool or old not in current or (not replace_all and current.count(old) != 1):
        refuse("edit cannot be simulated unambiguously")
    return current.replace(old, new) if replace_all else current.replace(old, new, 1)


def check_cargo_toml(current: str, proposed: str) -> None:
    before, after = tomllib.loads(current), tomllib.loads(proposed)
    if before.get("lints", {}).get("workspace") is True and after.get("lints", {}).get("workspace") is not True:
        refuse("member manifest must keep workspace lint inheritance")
    levels = {"allow": 0, "warn": 1, "deny": 2, "forbid": 3}
    old = before.get("workspace", {}).get("lints", {})
    new = after.get("workspace", {}).get("lints", {})
    for group in old.keys() | new.keys():
        for lint in old.get(group, {}).keys() | new.get(group, {}).keys():
            prior = old.get(group, {}).get(lint)
            value = new.get(group, {}).get(lint)
            def setting(raw):
                obj = raw if isinstance(raw, dict) else {"level": raw, "priority": 0}
                return levels.get(obj.get("level"), -1), obj.get("priority", 0)
            previous, previous_priority = setting(prior)
            level, priority = setting(value)
            if level < (previous if prior is not None else 2) or (prior is not None and priority != previous_priority):
                refuse(f"manifest lint floor relaxed or priority changed: {group}.{lint}")


def shell(command: str) -> None:
    lexer = shlex.shlex(command, posix=True, punctuation_chars=True)
    lexer.whitespace_split = True
    tokens = list(lexer)  # Concatenated quotes and absolute paths are checked after tokenization.
    normal = " ".join(tokens)
    for pattern, why in BASH_REFUSALS:
        if pattern.search(normal):
            refuse(why)
    if not WRITES_IN_COMMAND.search(normal):
        return
    if any(mark in command for mark in ("$", "`", "\n")):
        refuse("dynamic shell mutation cannot be checked here; use a supported structured edit")
    for token in tokens:
        if token.startswith("-") or token in (";", "&&", "||", "|", ">", ">>", "<"):
            continue
        # Only literal path tokens are resolved. This is not a general shell/interpreter analyzer.
        if not token or any(char.isspace() for char in token):
            continue
        rel = relative(token)
        if is_protected(rel, project_dir()) or rel.endswith((".rs", "Cargo.toml")):
            refuse(f"shell mutation may change protected or Rust source path {rel}; use structured edits")


READ_ONLY_TOOLS = frozenset({"Read", "Glob", "Grep", "WebFetch", "WebSearch", "AskUserQuestion", "TodoWrite", "TaskOutput"})


def pre(event: dict) -> None:
    tool = required_text(event, "tool_name", True)
    data = event.get("tool_input")
    if not isinstance(data, dict):
        refuse("tool_input must be an object")
    if tool in READ_ONLY_TOOLS:
        return
    if tool == "Bash":
        shell(required_text(data, "command", True))
        return
    if tool not in ("Edit", "Write"):
        refuse(f"unsupported tool {tool!r}: no checked mutation adapter (including multi-edit/patch tools)")
    rel = relative(required_text(data, "file_path", True))
    if is_protected(rel, project_dir()):
        refuse(f"{rel} is protected quality configuration; use the owner-controlled change path")
    path = project_dir() / rel
    current = path.read_text() if path.exists() else ""
    proposed = proposed_text(tool, data, current)
    if path.name == "Cargo.toml":
        check_cargo_toml(current, proposed)
    if rel.endswith(".rs"):
        for pattern, why in RUST_REFUSALS:
            if pattern.search(proposed):
                refuse(f"{rel}: {why}")


def post(event: dict) -> None:
    data = event.get("tool_input")
    if not isinstance(data, dict):
        refuse("tool_input must be an object")
    rel = relative(required_text(data, "file_path", True))
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
    session = required_text(event, "session_id", True)
    root = project_dir()
    dirty = subprocess.run(["git", "status", "--porcelain"], cwd=root, capture_output=True, text=True, check=False)
    if dirty.returncode == 0 and not dirty.stdout.strip():
        return  # nothing changed in this session's working tree
    identity = hashlib.sha256((str(root.resolve()) + "\0" + session).encode()).hexdigest()
    counter = Path(tempfile.gettempdir()) / f"forge-warden-stop-{identity}"
    if counter.is_symlink():
        refuse("stop counter must not be a symlink")
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


def unique_pairs(items):
    result = {}
    for key, value in items:
        if key in result:
            refuse("duplicate event key")
        result[key] = value
    return result


def main() -> None:
    try:
        if len(sys.argv) != 2 or sys.argv[1] not in ("pre", "post", "stop"):
            refuse("unknown hook mode")
        raw = sys.stdin.buffer.read(1024 * 1024 + 1)
        if len(raw) > 1024 * 1024:
            refuse("event exceeds byte limit")
        event = json.loads(raw, object_pairs_hook=unique_pairs, parse_constant=lambda _: refuse("non-finite JSON"))
        if not isinstance(event, dict):
            refuse("event must be an object")
        {"pre": pre, "post": post, "stop": stop}[sys.argv[1]](event)
    except (ValueError, TypeError, OSError, AttributeError, RecursionError) as error:
        refuse(f"malformed event or unavailable check: {error}")


if __name__ == "__main__":
    main()
