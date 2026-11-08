#!/usr/bin/env python3
"""Exercise operator write controls only on disposable copies, never real goldens."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(dir="/tmp", prefix="fw-golden-probe-") as work:
        workspace = Path(work)
        copied = workspace / "repo"
        for name in ["scripts/golden_output.py", "scripts/bless.sh", "fixtures/quality-contract.json",
                     "fixtures/demo.patch", "fixtures/demo.rank.json", "fixtures/demo.verdict.json",
                     "examples/starter.warden.toml"]:
            target = copied / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, target)
        backups = workspace / "backups"
        backups.mkdir(mode=0o700)
        inner = copied / "ambient"
        inner.mkdir(mode=0o700)
        env = {"PATH": os.defpath, "HOME": work, "TMPDIR": str(inner), "LC_ALL": "C"}
        command = ["python3", "-I", str(copied / "scripts/golden_output.py"),
                   "--binary", str(args.binary.resolve()), "--write"]

        def run(arguments, environment):
            return subprocess.run(arguments, env=environment, cwd=copied, stdin=subprocess.DEVNULL,
                                  capture_output=True, text=True, timeout=40, check=False)

        for marker in ["CI", "GITHUB_ACTIONS"]:
            marked = dict(env, **{marker: "true"})
            readonly = run(command[:-1], marked)
            assert readonly.returncode == 0 and json.loads(readonly.stdout)["mode"] == "check", readonly
            for entry in [command + ["--backup-dir", str(backups)],
                          ["bash", str(copied / "scripts/bless.sh")]]:
                out = run(entry, marked)
                assert out.returncode == 2 and "regeneration is refused in CI" in out.stderr, out
        # Missing, inside-root, symlinked and non-private backup paths fail before execution.
        link = workspace / "link"
        link.symlink_to(backups, target_is_directory=True)
        unsafe = workspace / "unsafe"
        unsafe.mkdir(mode=0o755)
        unsafe.chmod(0o755)
        for extra in [[], ["--backup-dir", str(inner)], ["--backup-dir", str(link)],
                      ["--backup-dir", str(unsafe)]]:
            out = run(command + extra, env)
            assert out.returncode == 2 and "backup" in out.stderr, out
        before = (copied / "fixtures/demo.verdict.json").read_bytes()
        out = run(command + ["--backup-dir", str(backups)], env)
        assert out.returncode == 0, out
        record = json.loads(out.stdout)
        backup = Path(record["backup"])
        assert backup.parent == backups and backup.read_bytes() == before
        assert backup.stat().st_mode & 0o077 == 0
        assert (copied / "fixtures/demo.verdict.json").read_bytes() == before
        assert not list(inner.iterdir()), "ambient TMPDIR influenced producer placement"
    print("GOLDEN_OPERATOR_CONTROLS: both CI markers refused; private outside-root backup proven")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
