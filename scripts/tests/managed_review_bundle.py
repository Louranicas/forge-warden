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
        self.import_objects()

    def import_objects(self):
        # Fixture setup imports controlled objects BEFORE candidate config is poisoned. The
        # production controller owns object acquisition; the helper never fetches from a candidate.
        self.git(self.trusted, "fetch", "--no-tags", str(self.candidate), self.head)

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

    def test_modified_worktree_instructions_are_ignored(self):
        original = (self.trusted / "CLAUDE.md").read_bytes()
        (self.trusted / "CLAUDE.md").write_text("dirty")
        self.assertEqual(self.run_bundle().returncode, 0)
        self.assertEqual((self.root / "bundle/CLAUDE.md").read_bytes(), original)

    def test_modified_helper_refuses(self):
        with (self.trusted / FILES[0]).open("a") as stream:
            stream.write("\n# uncommitted helper change\n")
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

    def extension_attack(self, kind):
        (self.candidate / ".gitattributes").write_text("CLAUDE.md filter=probe diff=probe\n")
        self.git(self.candidate, "add", ".gitattributes")
        self.git(self.candidate, "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                 "-c", "commit.gpgsign=false", "commit", "-qm", "attributes")
        self.head = self.git(self.candidate, "rev-parse", "HEAD").strip()
        self.import_objects()
        marker = self.root / (kind + "-ran")
        hook = self.root / (kind + "-hook")
        hook.write_text("#!/bin/sh\nprintf executed > '" + str(marker) + "'\n" +
                        ('cat "$@"\n' if kind in ("clean", "smudge", "textconv") else "exit 0\n"))
        hook.chmod(0o700)
        command = ["status", "--porcelain"]
        if kind in ("clean", "smudge"):
            self.git(self.candidate, "config", "filter.probe." + kind, str(hook))
            if kind == "smudge":
                (self.candidate / "CLAUDE.md").unlink()
                command = ["checkout", "--", "CLAUDE.md"]
        elif kind in ("textconv", "diff-driver", "external-diff"):
            key = {"textconv": "diff.probe.textconv", "diff-driver": "diff.probe.command",
                   "external-diff": "diff.external"}[kind]
            self.git(self.candidate, "config", key, str(hook))
            command = ["diff", "--ext-diff", "--textconv", "HEAD", "--", "CLAUDE.md"]
        elif kind == "hooksPath":
            hooks = self.root / "hooks"
            hooks.mkdir()
            shutil.copyfile(hook, hooks / "post-commit")
            (hooks / "post-commit").chmod(0o700)
            self.git(self.candidate, "config", "core.hooksPath", str(hooks))
            command = ["-c", "core.hooksPath=" + str(hooks), "-c", "user.name=Fixture",
                       "-c", "user.email=fixture@example.invalid", "-c", "commit.gpgsign=false",
                       "commit", "--allow-empty", "-qm", "positive-control"]
        else:
            included = self.root / "included-config"
            included.write_text('[core]\nfsmonitor = "' + str(hook) + '"\n')
            key = "include.path" if kind == "include" else "includeIf.gitdir:" + str(self.candidate) + "/.path"
            self.git(self.candidate, "config", key, str(included))
        if kind == "clean":
            tracked = self.candidate / "CLAUDE.md"
            tracked.write_bytes(tracked.read_bytes())
            os.utime(tracked, (2000000000, 2000000000))
        elif kind != "smudge":
            with (self.candidate / "CLAUDE.md").open("a") as stream:
                stream.write("\nworktree is not the review subject\n")
        result = self.run_bundle()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(marker.exists(), kind + " executed during bundle generation")
        # Positive control: prove this candidate extension really executes on its triggering
        # worktree command. This deliberate unsafe command exists only in the disposable fixture.
        self.git(self.candidate, *command)
        self.assertTrue(marker.exists(), kind + " fixture did not trigger its positive control")

    def test_clean_filter(self):
        self.extension_attack("clean")

    def test_smudge_filter(self):
        self.extension_attack("smudge")

    def test_textconv(self):
        self.extension_attack("textconv")

    def test_external_diff(self):
        self.extension_attack("external-diff")

    def test_candidate_attributes_diff_driver(self):
        self.extension_attack("diff-driver")

    def test_hooks_path(self):
        self.extension_attack("hooksPath")

    def test_include_path(self):
        self.extension_attack("include")

    def test_include_if(self):
        self.extension_attack("includeIf")


if __name__ == "__main__":
    unittest.main()
