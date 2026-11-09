#!/usr/bin/env python3
"""Explicit golden regeneration/checking, separate from ordinary read-only tests."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
GOLDENS = ("fixtures/demo.verdict.json", "fixtures/cli-stderr/io.txt",
           "fixtures/cli-stderr/refused.txt", "fixtures/cli-stderr/over-budget.txt")


def normalize(stdout):
    """Preserve every input seal; only the machine-specific Git version is normalized."""
    value = json.loads(stdout)
    value["seals"]["git"]["version"] = "<git --version>"
    return json.dumps(value, ensure_ascii=False, indent=2) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--golden", choices=GOLDENS, default=GOLDENS[0], help="reviewed output to check or regenerate")
    parser.add_argument("--write", action="store_true", help="operator-only explicit regeneration")
    parser.add_argument("--backup-dir", type=Path, help="existing private outside-root backup directory")
    args = parser.parse_args()
    if args.write and ("CI" in os.environ or "GITHUB_ACTIONS" in os.environ):
        parser.error("golden regeneration is refused in CI")
    if args.write:
        directory = args.backup_dir
        if directory is None or not directory.is_absolute():
            parser.error("--write requires an absolute --backup-dir")
        if directory.resolve() != directory or not directory.is_dir():
            parser.error("backup directory must exist without symlinks")
        info = directory.stat()
        if directory.is_relative_to(ROOT) or info.st_uid != os.getuid() or info.st_mode & 0o077:
            parser.error("backup directory must be private, owned, and outside the repository")
    contract = json.loads((ROOT / "fixtures/quality-contract.json").read_text())
    golden = args.golden
    if golden not in contract["goldens"]:
        parser.error("producer output is not in the reviewed golden registry")
    destination = ROOT / golden
    if destination.resolve() != destination.absolute() or not destination.is_file():
        parser.error("golden destination must be an existing regular file without symlinks")
    binary = args.binary.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix="fw-golden-home-", dir="/tmp") as home:
        env = {"PATH": os.defpath, "HOME": home, "LC_ALL": "C", "LANG": "C"}
        command = [str(binary), "check", "--policy", "examples/starter.warden.toml",
                   "--patch", "fixtures/demo.patch", "--advisory", "--rank",
                   "fixtures/demo.rank.json", "--today", "2026-11-05", "--json"]
        if golden == GOLDENS[0]:
            out = subprocess.run(command, cwd=ROOT, env=env, stdin=subprocess.DEVNULL,
                                 capture_output=True, timeout=30, check=False)
            if out.returncode != 7 or out.stderr:
                raise ValueError("golden producer did not return the expected demo judgement")
            result = normalize(out.stdout).encode()
        else:
            if golden == "fixtures/cli-stderr/io.txt":
                with open("/dev/full", "wb") as full:
                    out = subprocess.run([str(binary), "init"], cwd=ROOT, env=env,
                                         stdin=subprocess.DEVNULL, stdout=full,
                                         stderr=subprocess.PIPE, timeout=30, check=False)
                expected = 3
            elif golden == "fixtures/cli-stderr/refused.txt":
                out = subprocess.run([str(binary), "lint", "."], cwd=ROOT, env=env,
                                     stdin=subprocess.DEVNULL, capture_output=True,
                                     timeout=30, check=False)
                expected = 4
            else:
                command.remove("--json")
                command[command.index("fixtures/demo.rank.json")] = "-"
                out = subprocess.run(command, cwd=ROOT, env=env,
                                     input=b" " * (67108864 + 1), capture_output=True,
                                     timeout=30, check=False)
                expected = 4
            if out.returncode != expected or out.stdout or not out.stderr:
                raise ValueError("golden producer did not return the expected stderr failure")
            result = out.stderr
    if args.write:
        with tempfile.NamedTemporaryFile(dir=args.backup_dir, prefix="fw-golden-backup-", delete=False) as backup:
            backup.write(destination.read_bytes())
            backup_path = backup.name
        with tempfile.NamedTemporaryFile(dir=destination.parent, prefix=".golden-", delete=False) as temp:
            temp.write(result)
            proposal = Path(temp.name)
        proposal.replace(destination)
    elif result != destination.read_bytes():
        raise ValueError("GOLDEN_BYTES_DIFFER: independent regeneration does not match")
    print(json.dumps({"golden": golden, "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                      "output_sha256": hashlib.sha256(result).hexdigest(),
                      "mode": "write" if args.write else "check",
                      "backup": backup_path if args.write else None}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
