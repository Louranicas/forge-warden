"""Shared plumbing: processes, Git, state, configuration, globs and safe text.

Python 3.11+ standard library only. Nothing here decides a verdict.
"""

from __future__ import annotations

import hashlib
import json
import os
import re
import shutil
import subprocess
import tempfile
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

EXIT_PASS, EXIT_USAGE, EXIT_ENV, EXIT_FAIL, EXIT_HOLD = 0, 2, 3, 7, 8
PACK_ROOT = Path(__file__).resolve().parent.parent
CONFIG_NAME = "forge-stack.toml"
DEFAULT_PROTECTED = [
    ".claude/**", ".github/**", ".githooks/**", "forge-corpus/**",
    CONFIG_NAME, "warden.toml", "clippy.toml", "deny.toml", "rust-toolchain.toml",
]
TEST_PATH = re.compile(
    r"(^|/)(tests?|__tests__|spec)/|(^|/)test_[^/]+\.py$|_test\.[A-Za-z0-9]+$"
    r"|\.(test|spec)\.[cm]?[jt]sx?$|_spec\.rb$|(^|/)conftest\.py$"
)


class EnvError(Exception):
    """The environment cannot support the request (exit 3): a missing tool, no repository."""


def run(args: list[str], *, cwd: Path | None = None, stdin: bytes | None = None,
        timeout: int = 120, env: dict | None = None) -> subprocess.CompletedProcess:
    """Run a child with no shell. A missing binary or a timeout is an environment error."""
    try:
        return subprocess.run(args, cwd=cwd, input=stdin, capture_output=True, timeout=timeout,
                              check=False, env=env)
    except FileNotFoundError as exc:
        raise EnvError(f"cannot run {args[0]}: not found") from exc
    except subprocess.TimeoutExpired as exc:
        raise EnvError(f"{args[0]} did not finish within {timeout}s") from exc


def git(root: Path, *args: str, ok: tuple[int, ...] = (0,)) -> bytes:
    done = run(["git", "-c", "core.quotepath=off", *args], cwd=root)
    if done.returncode not in ok:
        raise EnvError(f"git {args[0]} failed: {done.stderr.decode(errors='replace').strip()[:300]}")
    return done.stdout


def repo_root(start: Path | None = None) -> Path:
    done = run(["git", "rev-parse", "--show-toplevel"], cwd=start or Path.cwd())
    if done.returncode != 0:
        raise EnvError("not inside a Git repository")
    return Path(done.stdout.decode().strip())


def state_dir(root: Path) -> Path:
    """Per-clone state lives inside the Git directory, so it never appears in a diff."""
    raw = git(root, "rev-parse", "--git-common-dir").decode().strip()
    path = (root / raw).resolve() / "forge"
    path.mkdir(parents=True, exist_ok=True)
    return path


def read_json(path: Path, default):
    try:
        return json.loads(path.read_text())
    except (OSError, ValueError):
        return default


def write_json(path: Path, value) -> None:
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")
    os.replace(tmp, path)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def safe(text: str, limit: int = 160) -> str:
    """Patch-author text is data: escape everything outside printable ASCII and bound it."""
    out = "".join(ch if " " <= ch <= "~" else f"\\u{{{ord(ch):x}}}" for ch in text.strip())
    return out if len(out) <= limit else out[: limit - 1] + "…"


def glob_regex(pattern: str) -> re.Pattern[str]:
    parts = []
    for part in pattern.strip("/").split("/"):
        if part == "**":
            parts.append("(?:.*)")
        else:
            parts.append("".join("[^/]*" if c == "*" else re.escape(c) for c in part))
    expr = "/".join(parts).replace("/(?:.*)/", "/(?:.*/)?").replace("(?:.*)/", "(?:.*/)?")
    return re.compile(expr + r"\Z")


def matches(path: str, patterns: list[str]) -> bool:
    for pattern in patterns:
        if pattern.endswith("/**") and path == pattern[:-3]:
            return True
        if glob_regex(pattern).match(path):
            return True
    return False


def is_test_path(path: str) -> bool:
    return bool(TEST_PATH.search(path))


def find_tool(name: str, env: str) -> str | None:
    given = os.environ.get(env)
    if given:
        return given if Path(given).is_file() else None
    return shutil.which(name)


def is_commit(root: Path, rev: str) -> bool:
    return run(["git", "cat-file", "-e", f"{rev}^{{commit}}"], cwd=root).returncode == 0


def head(root: Path) -> str | None:
    done = run(["git", "rev-parse", "--verify", "-q", "HEAD"], cwd=root)
    return done.stdout.decode().strip() if done.returncode == 0 else None


def base_commit(root: Path, override: str | None = None) -> str | None:
    """The commit the change is measured from: an override, the session's start, else HEAD."""
    if override:
        if not is_commit(root, override):
            raise EnvError(f"base {override!r} is not a commit")
        return git(root, "rev-parse", override).decode().strip()
    recorded = read_json(state_dir(root) / "session.json", {}).get("base")
    if recorded and is_commit(root, recorded):
        return recorded
    return head(root)


def show_at(root: Path, base: str | None, rel: str) -> str | None:
    """A file's text as it was at the base commit, or None when it did not exist there."""
    if not base:
        return None
    done = run(["git", "show", f"{base}:{rel}"], cwd=root)
    return done.stdout.decode(errors="replace") if done.returncode == 0 else None


EMPTY_TREE = "4b825dc642cb6eb9a060e54bf8d69288fbee4904"


def build_patch(root: Path, base: str | None) -> bytes:
    """Everything since the base: commits, staged and unstaged edits, and untracked files.

    The working tree is staged into a throwaway copy of the index, so one `git diff` describes
    the whole change (binary and new files included) and the real index is never touched.
    """
    with tempfile.TemporaryDirectory(prefix="forge-index-") as tmp:
        index = Path(tmp) / "index"
        real = Path(git(root, "rev-parse", "--git-path", "index").decode().strip())
        real = real if real.is_absolute() else root / real
        if real.is_file():
            shutil.copyfile(real, index)
        env = {**os.environ, "GIT_INDEX_FILE": str(index)}
        added = run(["git", "add", "-A"], cwd=root, env=env)
        if added.returncode != 0:
            raise EnvError(f"git add failed: {added.stderr.decode(errors='replace').strip()[:300]}")
        done = run(["git", "-c", "core.quotepath=off", "diff", "--cached", "--binary", "--no-renames",
                    "--no-ext-diff", base or EMPTY_TREE], cwd=root, env=env)
        if done.returncode != 0:
            raise EnvError(f"git diff failed: {done.stderr.decode(errors='replace').strip()[:300]}")
        return done.stdout


@dataclass
class Config:
    policy: str = "warden.toml"
    scope: str = "required"
    protected: list[str] = field(default_factory=lambda: list(DEFAULT_PROTECTED))
    oracle_mode: str = "off"
    oracle_model: str = ""
    oracle_hold_at: float = 0.7
    source: str = "built-in defaults"
    problems: list[str] = field(default_factory=list)


def load_config(root: Path, base: str | None) -> Config:
    """Configuration is read from the base commit, so a change cannot loosen its own gate.

    A repository with no configuration at the base gets the strict defaults.
    """
    cfg = Config()
    text = show_at(root, base, CONFIG_NAME)
    if text is None:
        return cfg
    cfg.source = f"{CONFIG_NAME} at base {base[:12]}"
    try:
        doc = tomllib.loads(text)
    except tomllib.TOMLDecodeError as exc:
        cfg.problems.append(f"{CONFIG_NAME} at the base does not parse: {exc}")
        return cfg
    known = {"schema", "policy", "scope", "protected", "oracle"}
    cfg.problems += [f"unknown key `{k}` in {CONFIG_NAME}" for k in sorted(set(doc) - known)]
    if isinstance(doc.get("policy"), str):
        cfg.policy = doc["policy"]
    if doc.get("scope") in ("required", "advisory"):
        cfg.scope = doc["scope"]
    elif "scope" in doc:
        cfg.problems.append("`scope` must be \"required\" or \"advisory\"")
    extra = doc.get("protected", [])
    if isinstance(extra, list) and all(isinstance(p, str) for p in extra):
        cfg.protected = sorted(set(DEFAULT_PROTECTED) | set(extra))  # additions only
    else:
        cfg.problems.append("`protected` must be a list of globs")
    oracle = doc.get("oracle", {})
    if isinstance(oracle, dict):
        if oracle.get("mode", "off") in ("off", "advisory", "required"):
            cfg.oracle_mode = oracle.get("mode", "off")
        else:
            cfg.problems.append("`oracle.mode` must be off, advisory or required")
        cfg.oracle_model = str(oracle.get("model", ""))
        hold_at = oracle.get("hold_at", 0.7)
        if isinstance(hold_at, (int, float)) and 0.5 <= hold_at <= 1:
            cfg.oracle_hold_at = float(hold_at)
        else:
            cfg.problems.append("`oracle.hold_at` must be between 0.5 and 1")
    return cfg


def load_policy(root: Path, base: str | None, cfg: Config) -> tuple[str, str]:
    """The policy text and where it came from: the base commit, else the bundled default."""
    text = show_at(root, base, cfg.policy)
    if text is not None:
        return text, f"{cfg.policy} at base {base[:12]}"
    return (PACK_ROOT / "defaults" / "warden.toml").read_text(), "bundled default policy"
