"""`forge init`: vendor the pack into a repository's `.claude/` so it is versioned and reviewed.

Prints the plan by default and writes nothing; `--apply` writes. An existing file that
differs is never overwritten without `--force`. Only a person should run `--apply`: the
pre-tool hook treats `.claude/**` as protected once the pack is installed.
"""

from __future__ import annotations

import json
from pathlib import Path

from .core import CONFIG_NAME, PACK_ROOT, find_tool, run

HOOKS = {
    "SessionStart": (None, "session-start"),
    "PreToolUse": ("Edit|Write|MultiEdit|NotebookEdit|Bash", "pre-tool"),
    "Stop": (None, "stop"),
}
CONFIG_TEXT = '''# forge-stack configuration. Read from the BASE commit: a change cannot loosen its own gate.
schema = "forge-stack.config.v0"
policy = "warden.toml"      # the forge-warden policy that judges changes
scope = "required"          # "required": no edit before a declared scope. "advisory": warn only.
protected = []              # extra globs a person must approve (the defaults always apply)

[oracle]                    # Jev (TypeSafe). Uploads hunks to a third party when on.
mode = "off"                # "off" | "advisory" | "required" (required: no answer means hold)
model = ""                  # a pinned version, e.g. "jev-1.14.0"; "jev-latest" is refused
hold_at = 0.7               # probability at which an answer holds the change
'''
CORPUS_README = '''# forge-corpus

Each directory is one defect that reached (or nearly reached) this repository: `change.patch`
and a `case.json` naming the check that must catch it. `forge corpus run` replays them all;
an escaped case fails. Add one with the `forge-correct` skill whenever a defect gets through.
'''


def command(name: str) -> str:
    return f'python3 "$CLAUDE_PROJECT_DIR/.claude/forge/forge.py" hook {name}'


def merged_settings(existing: str | None) -> str:
    settings = json.loads(existing) if existing else {}
    hooks = settings.setdefault("hooks", {})
    for event, (matcher, name) in HOOKS.items():
        entries = hooks.setdefault(event, [])
        if any(f"forge.py\" hook {name}" in h.get("command", "") for e in entries for h in e.get("hooks", [])):
            continue
        entry = {"hooks": [{"type": "command", "command": command(name), "timeout": 120}]}
        if matcher:
            entry = {"matcher": matcher, **entry}
        entries.append(entry)
    return json.dumps(settings, indent=2) + "\n"


def plan(root: Path) -> list[tuple[Path, str]]:
    """Every file the pack wants in the repository, with its content."""
    wanted: list[tuple[Path, str]] = []
    claude = root / ".claude"
    for source in [PACK_ROOT / "forge.py", *sorted((PACK_ROOT / "forge_stack").glob("*.py")),
                   PACK_ROOT / "defaults" / "warden.toml"]:
        wanted.append((claude / "forge" / source.relative_to(PACK_ROOT), source.read_text()))
    wanted.append((claude / "forge" / ".gitignore", "__pycache__/\n"))
    for skill in sorted((PACK_ROOT / "skills").glob("*/SKILL.md")):
        wanted.append((claude / "skills" / skill.parent.name / "SKILL.md", skill.read_text()))
    for agent in sorted((PACK_ROOT / "agents").glob("*.md")):
        wanted.append((claude / "agents" / agent.name, agent.read_text()))
    settings = claude / "settings.json"
    wanted.append((settings, merged_settings(settings.read_text() if settings.is_file() else None)))
    if not (root / CONFIG_NAME).exists():
        wanted.append((root / CONFIG_NAME, CONFIG_TEXT))
    if not (root / "warden.toml").exists():
        warden = find_tool("forge-warden", "FORGE_WARDEN")
        starter = run([warden, "init"]).stdout.decode() if warden else ""
        wanted.append((root / "warden.toml", starter or (PACK_ROOT / "defaults" / "warden.toml").read_text()))
    if not (root / "forge-corpus" / "README.md").exists():
        wanted.append((root / "forge-corpus" / "README.md", CORPUS_README))
    return wanted


def init(root: Path, apply: bool, force: bool) -> int:
    conflicts = 0
    for path, content in plan(root):
        rel = path.relative_to(root)
        if not path.exists():
            state = "create"
        elif path.read_text() == content:
            state = "same"
        elif path.name == "settings.json" or force:
            state = "update"
        else:
            state, conflicts = "CONFLICT (differs; use --force)", conflicts + 1
        print(f"  {state:8} {rel}")
        if apply and state in ("create", "update"):
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content)
    if not apply:
        print("Nothing was written. Run again with --apply to write these files.")
        return 0
    print("Written. Review and COMMIT these files: the gate reads its policy and configuration from the\n"
          "base commit, so until they are committed it uses the bundled strict defaults.\n"
          "Then check the setup with: forge doctor")
    return 4 if conflicts else 0
