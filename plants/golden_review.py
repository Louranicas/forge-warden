#!/usr/bin/env python3
"""Evaluate behaviour plus golden changes as data under the immutable trusted policy."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
GOLDEN = "fixtures/demo.verdict.json"
SOURCE = "crates/forge-warden-core/src/lib.rs"


def run(argv, cwd, env, accepted=(0,)):
    result = subprocess.run(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                            capture_output=True, timeout=120, check=False)
    if result.returncode not in accepted:
        raise ValueError(f"golden plant prerequisite failed: {argv[0]} exit {result.returncode}")
    return result


def main():
    env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
    env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL="/dev/null",
               GIT_NO_REPLACE_OBJECTS="1", CARGO_NET_OFFLINE="true", RUSTUP_AUTO_INSTALL="0")
    original = run(["git", "show", "HEAD:" + GOLDEN], ROOT, env).stdout
    policy = run(["git", "show", "HEAD:warden.toml"], ROOT, env).stdout
    source = run(["git", "show", "HEAD:" + SOURCE], ROOT, env).stdout
    current = (ROOT / GOLDEN).read_bytes()
    run(["cargo", "build", "--quiet", "--bin", "forge-warden", "--locked", "--offline"], ROOT, env)
    target = Path(env.get("CARGO_TARGET_DIR", ROOT / "target"))
    if not target.is_absolute():
        target = ROOT / target
    binary = (target / "debug/forge-warden").resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix="fw-golden-policy-") as directory:
        work = Path(directory)
        candidate = work / "candidate"
        candidate.mkdir()
        for path, data in [(GOLDEN, original), (SOURCE, source), ("warden.toml", policy)]:
            output = candidate / path
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_bytes(data)
        run(["git", "init", "-q"], candidate, env)
        run(["git", "add", "--", GOLDEN, SOURCE, "warden.toml"], candidate, env)
        run(["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
             "commit", "-qm", "trusted data"], candidate, env)
        anchor = b'pub const TOOL_NAME: &str = "forge-warden";'
        if source.count(anchor) != 1:
            raise ValueError("golden behaviour plant anchor must occur exactly once")
        (candidate / SOURCE).write_bytes(source.replace(anchor, b'pub const TOOL_NAME: &str = "forge-warden-planted";'))
        (candidate / GOLDEN).write_bytes(current)
        # A candidate-controlled policy is deliberately ignored: only HEAD's base policy is used.
        (candidate / "warden.toml").write_text('schema = "forge-warden.policy.v0"\n')
        patch = run(["git", "-c", "core.attributesFile=/dev/null", "diff", "--binary",
                     "--no-renames", "--no-ext-diff", "--no-textconv", "HEAD"], candidate, env).stdout
        paths = run(["git", "diff", "--name-only", "-z", "HEAD"], candidate, env).stdout
        names = [name.decode() for name in paths.split(b"\0") if name]
        patch_path = work / "change.patch"
        patch_path.write_bytes(patch)
        trusted_policy = work / "trusted.toml"
        trusted_policy.write_bytes(policy)
        ranking = {"schema": "deep-diff-forge.rank.v0", "input_sha256": hashlib.sha256(patch).hexdigest(),
                   "tool": {"name": "deep-diff-forge", "version": "0.7.0"},
                   "ranked": [{"path": name, "status": "modified", "score": 1, "signals": [], "evidence": []}
                              for name in names]}
        rank_path = work / "rank.json"
        rank_path.write_text(json.dumps(ranking))
        verdict = run([str(binary), "check", "--policy", str(trusted_policy), "--patch", str(patch_path),
                       "--advisory", "--rank", str(rank_path), "--today", "2026-11-05", "--json"],
                      candidate, env, accepted=(7, 8))
        value = json.loads(verdict.stdout)
        rule = next((r for r in value["rules"] if r["id"] == "golden-output-needs-review"), None)
        if rule is None:
            raise ValueError("golden owner rule absent from trusted base")
        if rule["outcome"] != "pass":
            print("GOLDEN_REVIEW_REQUIRED: behaviour plus golden update held by trusted policy")
            return 1
        if current != original:
            raise ValueError("golden update escaped the trusted owner rule")
        print("PASS: behaviour-only data does not trigger the named golden rule")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
