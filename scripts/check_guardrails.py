#!/usr/bin/env python3
"""CI admission for the protected-path manifest, lint floor and expectation ledger."""
import collections
import importlib.util
import json
import re
import sys
import tomllib
from pathlib import Path

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("guardrail_attributes", ROOT / "scripts/guardrail_attributes.py")
attributes = importlib.util.module_from_spec(spec)
spec.loader.exec_module(attributes)
spec = importlib.util.spec_from_file_location("guardrail_sources", ROOT / "scripts/guardrail_sources.py")
sources = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sources)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def protected_paths(root):
    paths = [line.strip() for line in (root / "protected-paths.txt").read_text().splitlines()
             if line.strip() and not line.lstrip().startswith("#")]
    require(paths and paths == sorted(set(paths)), "protected-paths.txt must be sorted, unique and nonempty")
    for path in paths:
        parts = path.split("/")
        require(all(part not in ("", ".", "..") and re.fullmatch(r"[A-Za-z0-9_.-]+|\*|\*\*", part)
                    for part in parts) and "**" not in parts[:-1], "unsupported protected path pattern")
    return paths


def check_paths(root):
    paths = protected_paths(root)
    require({".cargo/**", "crates/*/tests/**", "fixtures/**", "plants/**"} <= set(paths),
            "gate perimeter ownership missing")
    policy = tomllib.loads((root / "warden.toml").read_text())
    rules = [rule for rule in policy["rule"] if rule["id"] == "quality-config-is-owned"]
    require(len(rules) == 1 and rules[0]["kind"] == "protect_paths" and rules[0]["severity"] in ("hold", "fail"),
            "quality ownership rule missing or weakened")
    require(rules[0]["paths"] == paths and "except" not in rules[0], "policy protected paths drift")
    owners = [line.split() for line in (root / ".github/CODEOWNERS").read_text().splitlines()
              if line.strip() and not line.lstrip().startswith("#")]
    require(owners == [["*", "@Louranicas"]] + [["/" + path, "@Louranicas"] for path in paths], "CODEOWNERS protected paths drift")
    settings = json.loads((root / ".claude/settings.json").read_text())
    deny = settings["permissions"]["deny"]
    for tool in ("Edit", "Write"):
        actual = [entry for entry in deny if entry.startswith(tool + "(")]
        require(actual == [tool + "(/" + path + ")" for path in paths], tool + " deny paths drift")
    require(settings["hooks"]["PreToolUse"][0]["matcher"] == "*", "pre-hook must see unknown mutation tools")
    # Inspect the actual loaded guard, not a second static path list. Hook execution remains convenience.
    spec = importlib.util.spec_from_file_location("checked_guard", root / ".claude/hooks/guard.py")
    guard = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(guard)
    require(guard.load_protected(root) == paths, "guard protected paths drift")
    for pattern in paths:
        probe = pattern.replace("**", "nested/probe").replace("*", "member")
        require(guard.is_protected(probe, root), "guard fails canonical protected-path probe")
    require(not guard.is_protected("src/ordinary.rs", root), "guard overmatches ordinary source")


def lint_setting(value):
    entry = value if isinstance(value, dict) else {"level": value, "priority": 0}
    levels = {"allow": 0, "warn": 1, "deny": 2, "forbid": 3}
    require(entry.get("level") in levels and type(entry.get("priority", 0)) is int, "invalid lint setting")
    return levels[entry["level"]], entry.get("priority", 0)


def check_floor(root):
    current = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["lints"]
    floor = tomllib.loads((root / "lint-floor.toml").read_text())["workspace"]["lints"]
    require(floor and "rust" in floor and "clippy" in floor, "empty lint floor")
    for group, entries in floor.items():
        for name, expected in entries.items():
            require(name in current.get(group, {}), f"missing lint floor entry {group}.{name}")
            minimum, priority = lint_setting(expected)
            actual, actual_priority = lint_setting(current[group][name])
            require(actual >= minimum and priority == actual_priority, f"lint floor relaxed: {group}.{name}")
    for group, entries in current.items():
        for name, value in entries.items():
            if name not in floor.get(group, {}):
                require(lint_setting(value)[0] >= 2, f"new weak lint exception: {group}.{name}")
    for manifest in (root / "crates").rglob("Cargo.toml"):
        data = tomllib.loads(manifest.read_text())
        if "package" in data:
            require(data.get("lints", {}).get("workspace") is True, f"member lint inheritance missing: {manifest}")


def source_expectations(root, source_names):
    result = collections.Counter()
    for name in source_names:
        source = root / name
        require(not source.is_symlink(), "Rust source symlinks are unsupported by ledger")
        path = source.relative_to(root).as_posix()
        for lint, reason in attributes.expectations(source.read_text()):
            result[(path, lint, reason)] += 1
    return result


def check_expectations(root, source_names=None):
    ledger = tomllib.loads((root / "lint-expectations.toml").read_text())
    require(ledger["schema"] == "forge-warden.lint-expectations.v1", "unsupported expectation ledger")
    expected = collections.Counter()
    for row in ledger["expectation"]:
        require(set(row) == {"file", "lint", "source_reason", "review_reason"}, "unknown expectation ledger field")
        require(all(isinstance(value, str) for value in row.values()), "ledger fields must be strings")
        require(len(row["review_reason"].strip()) >= 20, "expectation review reason must have at least 20 characters")
        expected[(row["file"], row["lint"], row["source_reason"])] += 1
    if source_names is None:
        data_inputs = {}
        for row in ledger.get("data_input", []):
            require(set(row) == {"file", "sha256", "review_reason"}, "invalid compiler data ledger")
            require(len(row["review_reason"].strip()) >= 20, "short compiler data review reason")
            require(row["file"] not in data_inputs and re.fullmatch(r"[0-9a-f]{64}", row["sha256"]), "invalid or duplicate compiler data hash")
            data_inputs[row["file"]] = row["sha256"]
        source_names = sources.discover(root, data_inputs)
    actual = source_expectations(root, source_names)
    require(actual == expected, f"expectation ledger drift: unlisted={list((actual-expected).elements())}; stale={list((expected-actual).elements())}")


def main():
    try:
        check_paths(ROOT)
        check_floor(ROOT)
        check_expectations(ROOT)
    except (ValueError, OSError, KeyError, TypeError, AttributeError) as error:
        print("guardrails: " + str(error), file=sys.stderr)
        return 1
    print("guardrails: protected paths, lint floor and expectation ledger agree")
    return 0


if __name__ == "__main__":
    sys.exit(main())
