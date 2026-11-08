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
GOLDEN = "fixtures/demo.verdict.json"


def normalize(stdout):
    """Preserve every input seal; only the machine-specific Git version is normalized."""
    value = json.loads(stdout)
    value["seals"]["git"]["version"] = "<git --version>"
    return json.dumps(value, ensure_ascii=False, indent=2) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--write", action="store_true", help="operator-only explicit regeneration")
    args = parser.parse_args()
    if args.write and "CI" in os.environ:
        parser.error("golden regeneration is refused in CI")
    contract = json.loads((ROOT / "fixtures/quality-contract.json").read_text())
    if GOLDEN not in contract["goldens"]:
        parser.error("producer output is not in the reviewed golden registry")
    destination = ROOT / GOLDEN
    if destination.resolve() != destination.absolute() or not destination.is_file():
        parser.error("golden destination must be an existing regular file without symlinks")
    binary = args.binary.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix="fw-golden-home-") as home:
        env = {"PATH": os.defpath, "HOME": home, "LC_ALL": "C", "LANG": "C"}
        out = subprocess.run([str(binary), "check", "--policy", "examples/starter.warden.toml",
                              "--patch", "fixtures/demo.patch", "--advisory", "--rank",
                              "fixtures/demo.rank.json", "--today", "2026-11-05", "--json"],
                             cwd=ROOT, env=env, stdin=subprocess.DEVNULL,
                             capture_output=True, text=True, timeout=30, check=False)
    if out.returncode != 7 or out.stderr:
        raise ValueError("golden producer did not return the expected demo judgement")
    result = normalize(out.stdout).encode()
    if args.write:
        with tempfile.NamedTemporaryFile(prefix="fw-golden-backup-", delete=False) as backup:
            backup.write(destination.read_bytes())
            backup_path = backup.name
        with tempfile.NamedTemporaryFile(dir=destination.parent, prefix=".golden-", delete=False) as temp:
            temp.write(result)
            proposal = Path(temp.name)
        proposal.replace(destination)
    elif result != destination.read_bytes():
        raise ValueError("GOLDEN_BYTES_DIFFER: independent regeneration does not match")
    print(json.dumps({"golden": GOLDEN, "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                      "output_sha256": hashlib.sha256(result).hexdigest(),
                      "mode": "write" if args.write else "check",
                      "backup": backup_path if args.write else None}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
