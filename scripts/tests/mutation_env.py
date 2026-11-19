#!/usr/bin/env python3
"""Measure the environment received by the mutation child at both real entrypoints."""
import hashlib
import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class MutationEnvironment(unittest.TestCase):
    def test_both_entrypoints_drop_inherited_target(self):
        commands = {"ci_local": ["bash", str(ROOT / "scripts/ci_local.sh")],
                    "workflow": ["python3", "-I", "scripts/mutation_ci.py", "run", "diff"],
                    "baseline": ["python3", "-I", "scripts/mutation_ci.py", "run", "full"]}
        with tempfile.TemporaryDirectory(prefix="mutation-env-") as directory:
            root = Path(directory)
            (root / "bin").mkdir()
            (root / "target").mkdir()
            (root / "scripts/tests").mkdir(parents=True)
            (root / "plants").mkdir()
            shutil.copyfile(ROOT / "scripts/mutation_ci.py", root / "scripts/mutation_ci.py")
            for name in ("scripts/tool_versions.py", "scripts/tests/perimeter.py",
                         "scripts/tests/runner_routing.py", "plants/test_runner.py"):
                (root / name).write_text("# explicit unrelated-gate fixture stub\n")
            (root / "scripts/check_guardrails.py").write_text("# fixture admission stub\n")
            (root / "scripts/make_patch.sh").write_text('if [ "$1" = --resolve-comparison ]; then echo "base base"; fi\n')
            scripts = {
                "git": '#!/bin/sh\nif [ "$1" = --no-replace-objects ]; then shift; fi\nwhile [ "$1" = -c ]; do shift 2; done\ncase "$1" in\nrev-parse) printf "%s\\n" "$TEST_ROOT";;\nshow-ref) echo tip;;\ndiff) echo mutation-fixture;;\n*) exit 90;;\nesac\n',
                "cargo": '#!/usr/bin/env python3\nimport json,os,sys\nif sys.argv[1] == "mutants":\n if "--list" in sys.argv:\n  print("[]"); sys.exit(0)\n with open(os.environ["RECEIPT"],"w") as f: json.dump({"target":os.environ.get("CARGO_TARGET_DIR"),"args":sys.argv[2:]},f)\n sys.exit(2)\n',
            }
            for name in ("cargo-deny", "cargo-mutants", "zizmor"):
                scripts[name] = "#!/bin/sh\nexit 0\n"
            for name, text in scripts.items():
                path = root / "bin" / name
                path.write_text(text)
                path.chmod(0o700)
            for entry, command in commands.items():
                for inherited in (False, True):
                    with self.subTest(entry=entry, inherited=inherited):
                        receipt = root / "receipt.json"
                        if receipt.exists():
                            receipt.unlink()
                        env = {**os.environ, "PATH": str(root / "bin") + os.pathsep + os.environ["PATH"],
                               "TEST_ROOT": str(root), "RECEIPT": str(receipt),
                               "BASE_SHA": "a"*40, "SHARD_INDEX": "0", "SHARD_COUNT": "1",
                               "MUTANT_TOTAL": "0", "SHARD_MINUTES": "13",
                               "INVENTORY_DIGEST": hashlib.sha256(b"[]").hexdigest()}
                        env.pop("CARGO_TARGET_DIR", None)
                        if inherited:
                            env["CARGO_TARGET_DIR"] = "/sentinel/must-not-reach-mutants"
                        result = subprocess.run(command, cwd=root, env=env, capture_output=True)
                        self.assertEqual(result.returncode, 2 if entry == "ci_local" else 1, result.stderr)
                        actual = json.loads(receipt.read_text())
                        self.assertEqual("--in-diff" in actual["args"], entry != "baseline")
                        if entry == "baseline":
                            self.assertIn("--workspace", actual["args"])
                            self.assertIn("--no-config", actual["args"])
                            self.assertNotIn("--test-workspace", actual["args"])
                        self.assertIsNone(actual["target"], "shared target leaked into mutation child")
                        index = actual["args"].index("--cap-lints")
                        self.assertEqual(actual["args"][index + 1], "true")


if __name__ == "__main__":
    unittest.main()
