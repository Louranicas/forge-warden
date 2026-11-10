#!/usr/bin/env python3
"""G-18: a skipped gate must not read as a passed gate. Shims only; no tool is downloaded or run."""
import importlib.util
import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("tool_versions", ROOT / "scripts/tool_versions.py")
tool_versions = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tool_versions)

CARGO_SHIM = """#!/bin/sh
case "$1 $2" in
 "deny --version") [ -n "$DENY_OUT" ] && printf '%b' "$DENY_OUT" && exit 0; exit 101 ;;
 "mutants --version") [ "$MUTANTS_HANG" = 1 ] && exec sleep 30
  [ -n "$MUTANTS_OUT" ] && printf '%b' "$MUTANTS_OUT" && exit 0; exit 101 ;;
esac
exit 90
"""


class LocalTools(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="local-tools-")
        self.root = Path(self.directory.name)
        (self.root / "scripts").mkdir()
        (self.root / "bin").mkdir()
        for name in ("scripts/tool-versions.toml", "scripts/ci_local.sh", "rust-toolchain.toml", "Cargo.toml"):
            shutil.copyfile(ROOT / name, self.root / name)
        shim = self.root / "bin/cargo"
        shim.write_text(CARGO_SHIM)
        shim.chmod(0o700)
        values = tool_versions.versions(ROOT)
        self.deny = f"cargo-deny {values['cargo_deny']}\\n"
        self.mutants = f"cargo-mutants {values['cargo_mutants']}\\n"

    def tearDown(self):
        self.directory.cleanup()

    def env(self, **extra):
        return {"PATH": f"{self.root / 'bin'}:/usr/bin:/bin", "DENY_OUT": self.deny,
                "MUTANTS_OUT": self.mutants, "MUTANTS_HANG": "0", **extra}

    def refused(self, pattern, **extra):
        with self.assertRaisesRegex(ValueError, pattern):
            tool_versions.check_local_tools(self.root, self.env(**extra), timeout=2)

    def test_the_pinned_tools_are_admitted(self):
        tool_versions.check_local_tools(self.root, self.env(), timeout=2)

    def test_a_missing_required_tool_is_refused_by_name(self):
        self.refused(r"required tool missing: cargo-deny", DENY_OUT="")
        self.refused(r"required tool missing: cargo-mutants", MUTANTS_OUT="")

    def test_another_version_is_refused(self):
        self.refused(r"cargo-deny reports 'cargo-deny 0\.0\.1'", DENY_OUT="cargo-deny 0.0.1\\n")
        self.refused(r"cargo-mutants reports .*pinned", MUTANTS_OUT=self.mutants.replace("cargo-mutants", "cargo-mutantz"))
        self.refused(r"cargo-deny reports", DENY_OUT=self.deny.replace("\\n", "-beta\\n"))
        self.refused(r"cargo-deny reports", DENY_OUT=self.deny + "extra line\\n")

    def test_a_probe_that_does_not_answer_is_refused(self):
        self.refused(r"required tool unavailable: cargo-mutants", MUTANTS_HANG="1")

    def test_no_cargo_at_all_is_refused(self):
        (self.root / "bin/cargo").unlink()
        empty = self.root / "empty"
        empty.mkdir()
        with self.assertRaisesRegex(ValueError, r"required tool (missing|unavailable): cargo-deny"):
            tool_versions.check_local_tools(self.root, {"PATH": str(empty)}, timeout=2)

    def write_gate(self, text):
        (self.root / "scripts/tool-versions.toml").write_text(
            (ROOT / "scripts/tool-versions.toml").read_text().split("[local_gate]")[0] + "[local_gate]\n" + text)

    def test_the_local_gate_table_is_closed(self):
        for text, pattern in [
            ('required = ["cargo-deny"]\n', "exactly"),
            ('required = ["cargo-deny"]\nhosted_only = ["cargo-deny"]\n', "both required and hosted-only"),
            ('required = ["cargo-deny", "cargo-deny"]\nhosted_only = []\n', "distinct"),
            ('required = ["rustfmt"]\nhosted_only = []\n', "no version probe: rustfmt"),
            ('required = []\nhosted_only = ["cargo-deny"]\n', "no recorded reason: cargo-deny"),
            ('required = []\nhosted_only = []\nextra = 1\n', "exactly"),
        ]:
            with self.subTest(text=text):
                self.write_gate(text)
                with self.assertRaisesRegex(ValueError, pattern):
                    tool_versions.local_gate(self.root)

    def test_the_real_ci_local_passes_the_structural_check(self):
        tool_versions.check_ci_local(ROOT)

    def plant(self, old, new, pattern):
        source = (ROOT / "scripts/ci_local.sh").read_text()
        self.assertEqual(source.count(old), 1, old)
        (self.root / "scripts/ci_local.sh").write_text(source.replace(old, new))
        with self.assertRaisesRegex(ValueError, pattern):
            tool_versions.check_ci_local(self.root)

    def test_a_skippable_required_tool_is_refused(self):
        self.plant("step deny; cargo deny check\n",
                   "if command -v cargo-deny >/dev/null; then step deny; cargo deny check; fi\n",
                   "not hosted-only.*cargo-deny")

    def test_an_echoed_skip_is_refused(self):
        self.plant('else hosted_only_not_run+=" zizmor"; fi', 'else echo "skip zizmor"; fi', "echoes a skip")

    def test_a_missing_local_tools_step_is_refused(self):
        self.plant("step local-tools; python3 -I scripts/tool_versions.py --check-local-tools\n", "",
                   "local-tools step exactly once")

    def test_a_required_tool_cannot_leave_the_required_list(self):
        for text, pattern in (
            ('required = []\nhosted_only = ["zizmor", "deep-diff-forge"]\n', "not in \\[local_gate\\] required"),
            ('required = ["cargo-mutants"]\nhosted_only = ["zizmor", "deep-diff-forge", "cargo-deny"]\n',
             "no recorded reason: cargo-deny"),
        ):
            with self.subTest(text=text):
                self.write_gate(text)
                with self.assertRaisesRegex(ValueError, pattern):
                    tool_versions.check_ci_local(self.root)

    def test_the_old_success_claim_is_refused(self):
        self.plant("ci_local: all gates passed", "ci_local: all available gates passed", "must not claim")


if __name__ == "__main__":
    os.environ.pop("CARGO_TARGET_DIR", None)
    unittest.main()
