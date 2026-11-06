#!/usr/bin/env python3
"""Owned gate contracts: exact tool inputs, complete plant receipts and merge subjects."""
import importlib.util
import json
from pathlib import Path
import shutil
import sys
sys.dont_write_bytecode = True
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


TOOLS = load("tool_versions")
PLANTS = load("check_plants")


class Contracts(unittest.TestCase):
    def test_version_copies_match_canonical_inputs(self):
        TOOLS.check(ROOT)

    def test_each_tool_or_compiler_drift_is_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for file in ("scripts/tool-versions.toml", "rust-toolchain.toml", "Cargo.toml", ".github/workflows/ci.yml"):
                (root / file).parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / file, root / file)
            workflow = root / ".github/workflows/ci.yml"
            original = workflow.read_text()
            versions = TOOLS.versions(ROOT)
            pins = [name.replace("_", "-") + "@" + versions[name]
                    for name in ("zizmor", "cargo_mutants", "cargo_deny")]
            for pin in [*pins, versions["rust"], versions["ddf_rev"]]:
                with self.subTest(pin=pin):
                    self.assertIn(pin, original)
                    workflow.write_text(original.replace(pin, "9.9.9"))
                    with self.assertRaises(ValueError):
                        TOOLS.check(root)
            workflow.write_text(original)

    def test_receipt_requires_exact_subject_complete_unique_cases_and_caught(self):
        good = {"revision": "subject", "results": [{"id": "one", "status": "CAUGHT"}, {"id": "two", "status": "CAUGHT"}], "pending_guardrails": [{"id": "G-99", "status": "PENDING"}]}
        PLANTS.validate_report(good, "subject", ["one", "two"])
        for field, value in (("revision", "other"), ("results", []),
                             ("results", [good["results"][0]] * 2),
                             ("results", [{"id": "one", "status": "ESCAPED"}, good["results"][1]])):
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                PLANTS.validate_report(dict(good, **{field: value}), "subject", ["one", "two"])

    def test_merge_queue_jobs_bind_both_subjects(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        for job, end in (("mutation", "dogfood"), ("dogfood", "vocabulary")):
            body = workflow.split("\n  " + job + ":\n", 1)[1].split("\n  " + end + ":\n", 1)[0]
            self.assertIn("if: github.event_name == 'pull_request' || github.event_name == 'merge_group'", body)
            self.assertIn("github.event.pull_request.base.sha || github.event.merge_group.base_sha", body)
            if job == "dogfood":
                self.assertIn("github.event.pull_request.head.sha || github.event.merge_group.head_sha", body)

    def test_required_perimeter_is_owned(self):
        paths = set((ROOT / "protected-paths.txt").read_text().splitlines())
        self.assertTrue({".cargo/**", "crates/*/tests/**", "fixtures/**", "plants/**"} <= paths)


if __name__ == "__main__":
    unittest.main()
