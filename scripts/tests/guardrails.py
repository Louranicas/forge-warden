#!/usr/bin/env python3
"""Plant independent configuration drifts; each check must reject its own breach."""
import importlib.util
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("checks", ROOT / "scripts/check_guardrails.py")
checks = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checks)


class Guardrails(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="guardrails-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for directory in ("crates", ".claude"):
            shutil.copytree(ROOT / directory, self.root / directory)
        (self.root / ".github").mkdir()
        for name in ("protected-paths.txt", "lint-floor.toml", "lint-expectations.toml", "Cargo.toml",
                     "warden.toml", ".github/CODEOWNERS"):
            shutil.copyfile(ROOT / name, self.root / name)

    def change(self, name, before, after):
        path = self.root / name
        original = path.read_text()
        self.assertIn(before, original)
        path.write_text(original.replace(before, after, 1))

    def test_baseline_agrees(self):
        checks.check_paths(self.root)
        checks.check_floor(self.root)
        checks.check_expectations(self.root)

    def test_policy_only_addition_refuses(self):
        self.change("warden.toml", '  ".github/**",', '  ".github/**",\n  "new-protected/**",')
        with self.assertRaisesRegex(ValueError, "policy protected paths drift"):
            checks.check_paths(self.root)

    def test_cli_refuses_policy_drift(self):
        (self.root / "scripts").mkdir()
        for name in ("check_guardrails.py", "guardrail_attributes.py"):
            shutil.copyfile(ROOT / "scripts" / name, self.root / "scripts" / name)
        self.change("warden.toml", '  ".github/**",', '  ".github/**",\n  "new-protected/**",')
        out = subprocess.run([sys.executable, "-I", str(self.root / "scripts/check_guardrails.py")], capture_output=True)
        self.assertEqual(out.returncode, 1, out.stderr)
        self.assertIn(b"policy protected paths drift", out.stderr)

    def test_codeowners_drift_refuses(self):
        self.change(".github/CODEOWNERS", "/Cargo.lock @Louranicas\n", "")
        with self.assertRaisesRegex(ValueError, "CODEOWNERS"):
            checks.check_paths(self.root)

    def test_settings_write_omission_refuses(self):
        path = self.root / ".claude/settings.json"
        data = json.loads(path.read_text())
        data["permissions"]["deny"].remove("Write(/Cargo.lock)")
        path.write_text(json.dumps(data))
        with self.assertRaisesRegex(ValueError, "Write deny paths drift"):
            checks.check_paths(self.root)

    def test_guard_consumer_bypass_refuses(self):
        self.change(".claude/hooks/guard.py", "def is_protected(rel: str, root: Path) -> bool:",
                    "def is_protected(rel: str, root: Path) -> bool:\n    return False")
        with self.assertRaisesRegex(ValueError, "guard fails"):
            checks.check_paths(self.root)

    def test_floor_warn_and_priority_and_removed_lint(self):
        original = (self.root / "Cargo.toml").read_text()
        for old, new in [('unwrap_used = "deny"', 'unwrap_used = "warn"'),
                         ('priority = -1', 'priority = 1'),
                         ('unsafe_code = "forbid"', '# removed unsafe floor')]:
            with self.subTest(old=old):
                (self.root / "Cargo.toml").write_text(original.replace(old, new, 1))
                with self.assertRaises(ValueError):
                    checks.check_floor(self.root)

    def test_structural_lint_table_cannot_bypass_floor(self):
        self.change("Cargo.toml", 'unwrap_used = "deny"\n', '')
        with (self.root / "Cargo.toml").open("a") as stream:
            stream.write('\n[workspace.lints.clippy.unwrap_used]\nlevel = "warn"\n')
        with self.assertRaisesRegex(ValueError, "clippy.unwrap_used"):
            checks.check_floor(self.root)

    def test_member_inheritance_cannot_disappear(self):
        self.change("crates/forge-warden-core/Cargo.toml", '[lints]\nworkspace = true', '')
        with self.assertRaisesRegex(ValueError, "inheritance"):
            checks.check_floor(self.root)

    def test_unlisted_or_changed_expectation_refuses(self):
        path = self.root / "crates/forge-warden-core/src/lib.rs"
        with path.open("a") as stream:
            stream.write('\n#[expect(clippy::unwrap_used, reason = "ok")]\nfn planted() {}\n')
        with self.assertRaisesRegex(ValueError, "ledger drift"):
            checks.check_expectations(self.root)

    def test_removed_expectation_refuses(self):
        self.change("crates/forge-warden-cli/src/doors.rs",
                    '#[expect(clippy::disallowed_methods, reason = "door: read_file")]', '')
        with self.assertRaisesRegex(ValueError, "stale="):
            checks.check_expectations(self.root)

    def test_empty_reason_and_multiline_cfg_allow_refuse(self):
        path = self.root / "crates/forge-warden-core/src/lib.rs"
        original = path.read_text()
        for plant in ('#[expect(clippy::unwrap_used, reason = "")]',
                      '#[cfg_attr(\nall(),\nallow (dead_code)\n)]'):
            with self.subTest(plant=plant):
                path.write_text(original + '\n' + plant + '\nfn planted() {}\n')
                with self.assertRaises(ValueError):
                    checks.check_expectations(self.root)

    def test_short_review_explanation_refuses(self):
        path = self.root / "lint-expectations.toml"
        text = path.read_text()
        start = text.index('review_reason = ')
        end = text.index('\n', start)
        path.write_text(text[:start] + 'review_reason = "ok"' + text[end:])
        with self.assertRaisesRegex(ValueError, "at least 20"):
            checks.check_expectations(self.root)

    def test_lexer_ignores_comments_and_strings(self):
        sample = '''// #[allow(dead_code)]
/* outer /* #[allow(dead_code)] */ nested */
const S: &str = r###"#[expect(x, reason = "fake")]"###;
#[cfg_attr(feature = "x", expect(clippy::unwrap_used, reason = r#"reviewed reason with (parentheses)"#))]
fn real() {}
'''
        self.assertEqual(checks.attributes.expectations(sample),
                         [("clippy::unwrap_used", "reviewed reason with (parentheses)")])


if __name__ == "__main__":
    unittest.main()
