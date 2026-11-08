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
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


TOOLS = load("tool_versions")
PLANTS = load("check_plants")
CAPS = load("plant_tools")


class Contracts(unittest.TestCase):
    def test_version_copies_match_canonical_inputs(self):
        TOOLS.check(ROOT)

    def test_each_tool_or_compiler_drift_is_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for file in ("scripts/tool-versions.toml", "scripts/plant_tools.py", "plants/catalog.json", "rust-toolchain.toml", "Cargo.toml", ".github/workflows/ci.yml", ".github/workflows/mutation-baseline.yml"):
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
            baseline = root / ".github/workflows/mutation-baseline.yml"
            baseline_original = baseline.read_text()
            for pin in ("cargo-mutants@" + versions["cargo_mutants"], versions["rust"]):
                with self.subTest(baseline_pin=pin):
                    baseline.write_text(baseline_original.replace(pin, "9.9.9"))
                    with self.assertRaises(ValueError):
                        TOOLS.check(root)
            baseline.write_text(baseline_original)

    def test_catalog_tools_are_provisioned_in_gate(self):
        CAPS.check_ci(ROOT)

    def test_gate_tool_install_cannot_be_removed_moved_or_revised(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for file in ("plants/catalog.json", "scripts/tool-versions.toml", ".github/workflows/ci.yml"):
                (root / file).parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / file, root / file)
            path = root / ".github/workflows/ci.yml"
            original = path.read_text()
            install = ("      - uses: taiki-e/install-action@" + TOOLS.versions(ROOT)['install_action']
                       + " # v2.90.1\n        with:\n          tool: cargo-mutants@"
                       + TOOLS.versions(ROOT)['cargo_mutants'] + "\n")
            self.assertIn(install, original)
            gate_only_removed = original.replace(install, '', 1)
            changed = [gate_only_removed,
                       original.replace(install, install.replace('27.1.0', '0.0.0'), 1),
                       original.replace(install, install.replace(TOOLS.versions(ROOT)['install_action'], '0'*40), 1),
                       gate_only_removed.replace('      - run: python3 -I scripts/check_plants.py',
                                                '      - run: python3 -I scripts/check_plants.py\n'+install, 1)]
            for extra in ("        if: false\n", "        if: github.event_name == 'push'\n",
                          "        continue-on-error: true\n", "        env:\n          PATH: /missing\n",
                          "        timeout-minutes: 1\n"):
                changed.append(original.replace(install, install + extra, 1))
            changed.append(original.replace(install, install.replace('      - uses:',
                '      - if: false\n        uses:'), 1))
            for command in ('      - run: python3 -I scripts/check_plants.py',
                            '      - run: rustup toolchain install ' + TOOLS.versions(ROOT)['rust']
                            + ' --profile minimal --component rustfmt,clippy && rustup default '
                            + TOOLS.versions(ROOT)['rust']):
                changed.append(original.replace(command, command + '\n        if: false', 1))
            for text in changed:
                with self.subTest(text=text[:60]):
                    path.write_text(text)
                    with self.assertRaises(ValueError):
                        CAPS.check_ci(root)

    def test_catalog_tool_declarations_are_closed_and_match_direct_commands(self):
        for case in ({'id': 'missing'}, {'id': 'empty', 'required_tools': []},
                     {'id': 'unknown', 'required_tools': ['unknown']},
                     {'id': 'duplicate', 'required_tools': ['cargo','cargo']},
                     {'id': 'wrong', 'cargo': ['clippy'], 'required_tools': ['cargo']},
                     {'id': 'command', 'command': ['bash','x.sh'], 'required_tools': ['python3']}):
            with self.subTest(case=case), self.assertRaises(ValueError):
                CAPS.requirements([case])

    def test_missing_mutants_is_a_named_preflight_refusal(self):
        with tempfile.TemporaryDirectory() as directory:
            tool = Path(directory) / "cargo"
            tool.write_text("#!/bin/sh\necho 'no such command: mutants' >&2\nexit 101\n")
            tool.chmod(0o700)
            with self.assertRaisesRegex(ValueError, "plant preflight: unavailable tool cargo-mutants"):
                CAPS.preflight({'cargo-mutants'}, ROOT, {'PATH': directory})

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
