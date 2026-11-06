#!/usr/bin/env python3
"""Real Git fixtures; no managed service or candidate code execution."""
import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FILES = ("scripts/managed_review_bundle.sh", "agents/managed/reviewer.agent.json",
         "agents/managed/REVIEW_CONTRACT.md", "CLAUDE.md", "warden.toml")


class ReviewBundle(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="review-bundle-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.trusted = self.root / "trusted"
        self.candidate = self.root / "candidate"
        self.env = {**os.environ, "HOME": str(self.root), "GIT_CONFIG_NOSYSTEM": "1",
                    "GIT_CONFIG_GLOBAL": "/dev/null"}
        for key in ("GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_CONFIG_COUNT", "GIT_CONFIG_PARAMETERS"):
            self.env.pop(key, None)
        self.trusted.mkdir()
        for name in FILES:
            path = self.trusted / name
            path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, path)
        self.git(self.trusted, "init", "-q")
        self.commit(self.trusted)
        self.base = self.git(self.trusted, "rev-parse", "HEAD").strip()
        self.git(self.root, "clone", "-q", "--no-local", str(self.trusted), str(self.candidate))
        for name in FILES[1:]:
            (self.candidate / name).write_text("CANDIDATE-INSTRUCTION-OR-POLICY\n")
        self.commit(self.candidate)
        self.head = self.git(self.candidate, "rev-parse", "HEAD").strip()

    def git(self, where, *args):
        return subprocess.check_output(["git", "-c", "core.hooksPath=/dev/null", "-C", str(where),
                                        *args], env=self.env, stderr=subprocess.PIPE).decode()

    def commit(self, where):
        self.git(where, "add", "--", *FILES)
        self.git(where, "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                 "-c", "commit.gpgsign=false", "commit", "-qm", "fixture", "--", *FILES)

    def run_bundle(self, out=None):
        return subprocess.run(["bash", str(self.trusted / FILES[0]), self.base, self.head,
                               str(self.candidate), str(out or self.root / "bundle")],
                              env=self.env, capture_output=True)

    def test_candidate_cannot_supply_instructions_or_policy(self):
        result = self.run_bundle()
        self.assertEqual(result.returncode, 0, result.stderr)
        bundle = self.root / "bundle"
        for name in FILES[1:]:
            self.assertEqual((bundle / Path(name).name).read_bytes(), (self.trusted / name).read_bytes())
        subject = json.loads((bundle / "subject.json").read_text())
        self.assertEqual(subject["base"], self.base)
        self.assertEqual(subject["head"], self.head)

    def test_missing_trusted_policy_never_falls_back(self):
        (self.trusted / "warden.toml").unlink()
        self.commit(self.trusted)
        self.base = self.git(self.trusted, "rev-parse", "HEAD").strip()
        result = self.run_bundle()
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertFalse((self.root / "bundle").exists())

    def test_missing_trusted_contract_refuses(self):
        (self.trusted / "agents/managed/REVIEW_CONTRACT.md").unlink()
        self.commit(self.trusted)
        self.base = self.git(self.trusted, "rev-parse", "HEAD").strip()
        self.assertEqual(self.run_bundle().returncode, 2)

    def test_symlink_contract_refuses(self):
        path = self.trusted / "agents/managed/REVIEW_CONTRACT.md"
        path.unlink()
        path.symlink_to(self.candidate / "agents/managed/REVIEW_CONTRACT.md")
        self.commit(self.trusted)
        self.base = self.git(self.trusted, "rev-parse", "HEAD").strip()
        self.assertEqual(self.run_bundle().returncode, 2)

    def test_wrong_head_or_base_refuses(self):
        original = self.head
        self.head = "1" * 40
        self.assertEqual(self.run_bundle().returncode, 2)
        self.head = original
        self.base = "2" * 40
        self.assertEqual(self.run_bundle().returncode, 2)

    def test_dirty_trusted_refuses(self):
        (self.trusted / "CLAUDE.md").write_text("dirty")
        self.assertEqual(self.run_bundle().returncode, 2)

    def test_output_inside_candidate_refuses(self):
        result = self.run_bundle(self.candidate / "bundle")
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertFalse((self.candidate / "bundle/REVIEW_CONTRACT.md").exists())

    def test_candidate_fsmonitor_cannot_execute(self):
        marker = self.root / "fsmonitor-ran"
        hook = self.root / "fsmonitor"
        hook.write_text("#!/bin/sh\nprintf executed > '" + str(marker) + "'\n")
        hook.chmod(0o700)
        self.git(self.candidate, "config", "core.fsmonitor", str(hook))
        result = self.run_bundle()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(marker.exists(), "candidate fsmonitor executed")


if __name__ == "__main__":
    unittest.main()
