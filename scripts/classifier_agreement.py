#!/usr/bin/env python3
"""Compare D6 and C1c on the exact Cargo run exported by the Rust consumer."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tomllib

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("sources", ROOT / "scripts/guardrail_sources.py")
sources = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sources)


def compare(evidence, expected, target):
    metadata = subprocess.run(["cargo", "metadata", "--no-deps", "--locked", "--offline", "--format-version=1"],
                              cwd=ROOT, capture_output=True, text=True, check=True)
    members = set(json.loads(metadata.stdout)["workspace_members"])
    ledger = tomllib.loads((ROOT / "lint-expectations.toml").read_text())
    data = {row["file"]: row["sha256"] for row in ledger.get("data_input", [])}
    # The Rust consumer owns this build. Cargo may report cached artifacts in its
    # discovery target; both implementations consume those SAME artifact files.
    # Production D6 discovery still requires an independently fresh target.
    records = sources.records_from_messages(evidence, members, target, require_fresh=False)
    actual = sources.classify(ROOT, records, data)
    sources.require(actual == expected, "classifier disagreement: Python=" + json.dumps(actual) +
                    " Rust=" + json.dumps(expected))
    return actual


def main():
    try:
        sources.require(len(sys.argv) == 4, "expected evidence JSONL, Rust sets JSON, target directory")
        compare(Path(sys.argv[1]).read_text(), json.loads(Path(sys.argv[2]).read_text()), Path(sys.argv[3]))
    except (ValueError, OSError, KeyError, TypeError, subprocess.CalledProcessError) as error:
        print("classifier agreement: " + str(error), file=sys.stderr)
        return 1
    print("classifier agreement: identical production and non-production sets from the same build")
    return 0


if __name__ == "__main__":
    sys.exit(main())
