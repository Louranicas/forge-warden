#!/usr/bin/env python3
"""Measure the environment received by the mutation child at both real entrypoints."""
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class MutationEnvironment(unittest.TestCase):
    def test_both_entrypoints_drop_inherited_target(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        commands = [line.strip() for line in workflow.splitlines()
                    if "cargo mutants --" in line and not line.lstrip().startswith("#")]
        self.assertEqual(len(commands), 1, "identify exactly one workflow mutation command")
        baseline = (ROOT / ".github/workflows/mutation-baseline.yml").read_text()
        baseline_commands = [line.strip().removeprefix("run: ") for line in baseline.splitlines()
                             if "run: env -u CARGO_TARGET_DIR cargo mutants --" in line]
        self.assertEqual(len(baseline_commands), 1)
        with tempfile.TemporaryDirectory(prefix="mutation-env-") as directory:
            root = Path(directory)
            (root / "bin").mkdir()
            (root / "target").mkdir()
            (root / "scripts/tests").mkdir(parents=True)
            (root / "plants").mkdir()
            for name in ("scripts/tool_versions.py", "scripts/tests/perimeter.py", "plants/test_runner.py"):
                (root / name).write_text("# explicit unrelated-gate fixture stub\n")
            (root / "scripts/check_guardrails.py").write_text("# fixture admission stub\n")
            scripts = {
                "git": '#!/bin/sh\ncase "$1" in\nrev-parse) printf "%s\\n" "$TEST_ROOT";;\nmerge-base) echo base;;\ndiff) echo mutation-fixture;;\n*) exit 90;;\nesac\n',
                "cargo": '#!/usr/bin/env python3\nimport json,os,sys\nif sys.argv[1] == "mutants":\n with open(os.environ["RECEIPT"],"w") as f: json.dump({"target":os.environ.get("CARGO_TARGET_DIR"),"args":sys.argv[2:]},f)\n sys.exit(2)\n',
            }
            for name in ("cargo-deny", "cargo-mutants", "zizmor"):
                scripts[name] = "#!/bin/sh\nexit 0\n"
            for name, text in scripts.items():
                path = root / "bin" / name
                path.write_text(text)
                path.chmod(0o700)
            for entry in ("ci_local", "workflow", "baseline"):
                for inherited in (False, True):
                    with self.subTest(entry=entry, inherited=inherited):
                        receipt = root / "receipt.json"
                        if receipt.exists():
                            receipt.unlink()
                        env = {**os.environ, "PATH": str(root / "bin") + os.pathsep + os.environ["PATH"],
                               "TEST_ROOT": str(root), "RECEIPT": str(receipt)}
                        env.pop("CARGO_TARGET_DIR", None)
                        if inherited:
                            env["CARGO_TARGET_DIR"] = "/sentinel/must-not-reach-mutants"
                        command = (["bash", str(ROOT / "scripts/ci_local.sh")] if entry == "ci_local"
                                   else ["bash", "-c", baseline_commands[0] if entry == "baseline" else commands[0]])
                        result = subprocess.run(command, cwd=root, env=env, capture_output=True)
                        self.assertEqual(result.returncode, 2, result.stderr)
                        actual = json.loads(receipt.read_text())
                        if entry == "baseline":
                            self.assertIn("--workspace", actual["args"])
                            self.assertIn("--no-config", actual["args"])
                            self.assertNotIn("--in-diff", actual["args"])
                            self.assertNotIn("--test-workspace", actual["args"])
                        self.assertIsNone(actual["target"], "shared target leaked into mutation child")
                        index = actual["args"].index("--cap-lints")
                        self.assertEqual(actual["args"][index + 1], "true")


if __name__ == "__main__":
    unittest.main()
