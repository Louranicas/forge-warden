#!/usr/bin/env python3
"""Gate every runnable plant on a clean exact subject, retaining pending coverage honestly."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def validate_report(report, revision, ids):
    rows = report["results"]
    if report["revision"] != revision or len(rows) != len(ids) or {row["id"] for row in rows} != set(ids):
        raise ValueError("plant receipt subject or case coverage mismatch")
    if any(row["status"] != "CAUGHT" for row in rows):
        raise ValueError("not every runnable plant was caught")
    print(f"plants gate: {len(rows)} CAUGHT; pending coverage remains explicit: {report['pending_guardrails']}")


def run():
    env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
    env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL="/dev/null", GIT_NO_REPLACE_OBJECTS="1")
    def git(*args):
        return subprocess.check_output(["git", "-c", "core.hooksPath=/dev/null", *args],
                                       cwd=ROOT, env=env, text=True).strip()
    if git("status", "--porcelain", "--untracked-files=all"):
        raise ValueError("plants gate requires a clean committed subject")
    revision = git("rev-parse", "HEAD")
    cases = json.loads((ROOT / "plants/catalog.json").read_text())["cases"]
    ids = [case["id"] for case in cases]
    if not ids or len(ids) != len(set(ids)):
        raise ValueError("plant catalog must have unique nonempty cases")
    (ROOT / "target").mkdir(exist_ok=True)
    output = Path(tempfile.mkdtemp(prefix="plants-", dir=ROOT / "target")) / "evidence"
    argv = [sys.executable, "-I", str(ROOT / "plants/run.py"), "--source", str(ROOT),
            "--revision", revision, "--output", str(output)]
    for case in ids:
        argv.extend(["--case", case])
    result = subprocess.run(argv, cwd=ROOT, env=env, check=False)
    if result.returncode:
        raise ValueError("plant harness failed; see " + str(output))
    report = json.loads((output / "report.json").read_text())
    validate_report(report, revision, ids)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(run())
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError) as error:
        sys.exit("plants gate: " + str(error))
