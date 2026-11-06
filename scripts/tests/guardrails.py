#!/usr/bin/env python3
"""Plant independent configuration drifts; each check must reject its own breach."""
import importlib.util
import hashlib
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
        checks.check_expectations(self.root, [p.relative_to(self.root).as_posix() for p in (self.root / "crates").rglob("*.rs")])

    def test_policy_only_addition_refuses(self):
        self.change("warden.toml", '  ".github/**",', '  ".github/**",\n  "new-protected/**",')
        with self.assertRaisesRegex(ValueError, "policy protected paths drift"):
            checks.check_paths(self.root)

    def test_cli_refuses_policy_drift(self):
        (self.root / "scripts").mkdir()
        for name in ("check_guardrails.py", "guardrail_attributes.py", "guardrail_sources.py"):
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
            checks.check_expectations(self.root, [p.relative_to(self.root).as_posix() for p in (self.root / "crates").rglob("*.rs")])

    def test_removed_expectation_refuses(self):
        self.change("crates/forge-warden-cli/src/doors.rs",
                    '#[expect(clippy::disallowed_methods, reason = "door: read_file")]', '')
        with self.assertRaisesRegex(ValueError, "stale="):
            checks.check_expectations(self.root, [p.relative_to(self.root).as_posix() for p in (self.root / "crates").rglob("*.rs")])

    def test_empty_reason_and_multiline_cfg_allow_refuse(self):
        path = self.root / "crates/forge-warden-core/src/lib.rs"
        original = path.read_text()
        for plant in ('#[expect(clippy::unwrap_used, reason = "")]',
                      '#[cfg_attr(\nall(),\nallow (dead_code)\n)]'):
            with self.subTest(plant=plant):
                path.write_text(original + '\n' + plant + '\nfn planted() {}\n')
                with self.assertRaises(ValueError):
                    checks.check_expectations(self.root, [p.relative_to(self.root).as_posix() for p in (self.root / "crates").rglob("*.rs")])

    def test_short_review_explanation_refuses(self):
        path = self.root / "lint-expectations.toml"
        text = path.read_text()
        start = text.index('review_reason = ')
        end = text.index('\n', start)
        path.write_text(text[:start] + 'review_reason = "ok"' + text[end:])
        with self.assertRaisesRegex(ValueError, "at least 20"):
            checks.check_expectations(self.root, [p.relative_to(self.root).as_posix() for p in (self.root / "crates").rglob("*.rs")])

    def test_lexer_ignores_comments_and_strings(self):
        sample = '''// #[allow(dead_code)]
/* outer /* #[allow(dead_code)] */ nested */
const S: &str = r###"#[expect(x, reason = "fake")]"###;
#[cfg_attr(feature = "x", expect(clippy::unwrap_used, reason = r#"reviewed reason with (parentheses)"#))]
fn real() {}
'''
        self.assertEqual(checks.attributes.expectations(sample),
                         [("clippy::unwrap_used", "reviewed reason with (parentheses)")])


class CompiledSources(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="compiled-ledger-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.write("Cargo.toml", '[workspace]\nmembers=["crates/probe"]\nresolver="2"\n')
        self.write("crates/probe/Cargo.toml", '[package]\nname="probe"\nversion="0.1.0"\nedition="2021"\n')
        self.write("Cargo.lock", 'version = 3\n[[package]]\nname = "probe"\nversion = "0.1.0"\n')
        self.write("crates/probe/src/lib.rs", 'pub fn control() {}\n')

    def write(self, name, text):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def discover(self):
        return checks.sources.discover(self.root, {})

    def test_ordinary_compiled_source(self):
        self.assertEqual(self.discover(), ["crates/probe/src/lib.rs"])

    def test_test_helper_is_discovered_and_scanned(self):
        self.write("fixtures/adversarial/helper.rs", '#[expect(dead_code, reason="compiled test helper")] fn hidden() {}')
        self.write("crates/probe/tests/helper.rs", '#[path="../../../fixtures/adversarial/helper.rs"] mod helper;')
        names = self.discover()
        self.assertIn("fixtures/adversarial/helper.rs", names)
        self.assertEqual(checks.source_expectations(self.root, names),
                         {("fixtures/adversarial/helper.rs", "dead_code", "compiled test helper"): 1})

    def test_direct_production_escape_refuses_after_successful_compile(self):
        self.write("fixtures/adversarial/hidden.rs", '#[expect(dead_code, reason="compiled escape")] fn hidden() {}')
        self.write("crates/probe/src/lib.rs", '#[path="../../../fixtures/adversarial/hidden.rs"] mod hidden;')
        with self.assertRaisesRegex(ValueError, "outside admitted Rust roots"):
            self.discover()

    def test_directory_symlink_refuses_after_successful_compile(self):
        self.write("fixtures/adversarial/hidden.rs", '#[expect(dead_code, reason="compiled escape")] fn hidden() {}')
        (self.root / "crates/probe/src/linked").symlink_to(self.root / "fixtures/adversarial", target_is_directory=True)
        self.write("crates/probe/src/lib.rs", '#[path="linked/hidden.rs"] mod hidden;')
        with self.assertRaisesRegex(ValueError, "symlinked compiler input"):
            self.discover()

    def test_include_non_rs_cannot_hide_source(self):
        self.write("crates/probe/src/hidden.data", '#[expect(dead_code, reason="compiled escape")] fn hidden() {}')
        self.write("crates/probe/src/lib.rs", 'include!("hidden.data");')
        with self.assertRaisesRegex(ValueError, "unreviewed non-Rust"):
            self.discover()

    def test_uncompiled_source_refuses(self):
        self.write("crates/probe/src/inactive.rs", "fn hidden() {}")
        with self.assertRaisesRegex(ValueError, "compiler/source walk disagreement"):
            self.discover()

    def test_symlinked_file_refuses(self):
        self.write("fixtures/adversarial/hidden.rs", "pub fn control() {}")
        (self.root / "crates/probe/src/hidden.rs").symlink_to(self.root / "fixtures/adversarial/hidden.rs")
        self.write("crates/probe/src/lib.rs", "mod hidden;")
        with self.assertRaisesRegex(ValueError, "symlinked compiler input"):
            self.discover()

    def test_reviewed_data_is_bound_to_exact_bytes(self):
        self.write("data.txt", "reviewed bytes")
        self.write("crates/probe/src/lib.rs", 'pub const DATA: &str = include_str!("../../../data.txt");')
        digest = hashlib.sha256(b"reviewed bytes").hexdigest()
        self.assertEqual(checks.sources.discover(self.root, {"data.txt": digest}), ["crates/probe/src/lib.rs"])
        self.write("data.txt", "changed bytes")
        with self.assertRaisesRegex(ValueError, "reviewed compiler data changed"):
            checks.sources.discover(self.root, {"data.txt": digest})

    def test_stale_data_refuses(self):
        with self.assertRaisesRegex(ValueError, "stale compiler data ledger"):
            checks.sources.discover(self.root, {"unused.txt": "0" * 64})

    def test_outside_repository_source_refuses(self):
        with tempfile.TemporaryDirectory(prefix="outside-ledger-") as outside:
            path = Path(outside) / "hidden.rs"
            path.write_text('pub fn control() {}')
            self.write("crates/probe/src/lib.rs", '#[path=' + json.dumps(str(path)) + '] mod hidden;')
            with self.assertRaisesRegex(ValueError, "out-of-root compiler input"):
                self.discover()

    def test_dep_info_is_total(self):
        self.assertEqual(checks.sources.dependencies("/tmp/a.d: a.rs b.rs\n\na.rs:\nb.rs:\n"), {"a.rs", "b.rs"})
        for text in ("a: escaped\\ space.rs", "a: colon:name.rs", "a: cash$.rs", "a: a.rs\nmalformed", "a: x\t.rs"):
            with self.subTest(text=text), self.assertRaises(ValueError):
                checks.sources.dependencies(text)


if __name__ == "__main__":
    unittest.main()
