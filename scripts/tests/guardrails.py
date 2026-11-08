#!/usr/bin/env python3
"""Plant independent configuration drifts; each check must reject its own breach."""
import importlib.util
import hashlib
import json
import os
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

    def test_raw_attribute_names_and_paths_match_ordinary_spelling(self):
        for attr in ("#[r#allow(dead_code)]", "#[r#cfg_attr(test, r#allow(dead_code))]",
                     "#[r#expect(dead_code)]"):
            with self.subTest(attr=attr), self.assertRaises(ValueError):
                checks.attributes.expectations(attr)
        ordinary = '#[expect(clippy::unwrap_used, reason = "test")]'
        raw = '#[r#expect(r#clippy::r#unwrap_used, r#reason = "test")]'
        self.assertEqual(checks.attributes.expectations(raw),
                         checks.attributes.expectations(ordinary))

    def test_independent_attribute_scanner_matches_shared_corpus(self):
        cases = json.loads((ROOT / "fixtures/attribute-policy.json").read_text())
        self.assertGreaterEqual(len(cases), 30)
        for case in cases:
            with self.subTest(case=case["name"]):
                if case["expectations"] is None:
                    with self.assertRaises(ValueError):
                        checks.attributes.expectations(case["source"])
                else:
                    actual = checks.attributes.expectations(case["source"])
                    self.assertEqual(actual, [tuple(pair) for pair in case["expectations"]])

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
                    '#[expect(clippy::disallowed_methods, reason = "door: read_stdin")]', '')
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

    def test_shipping_kind_matrix(self):
        for kind in ("lib", "rlib", "bin", "proc-macro", "dylib", "cdylib", "staticlib", "example", "bench", "test"):
            for test in (False, True):
                with self.subTest(kind=kind, test=test):
                    actual = checks.sources.production_artifact({"target": {"kind": [kind]}, "profile": {"test": test}})
                    self.assertEqual(actual, not test and kind not in ("example", "bench", "test"))
        for kind in ("custom-build", "unknown"):
            with self.assertRaisesRegex(ValueError, "unsupported artifact kind"):
                checks.sources.production_artifact({"target": {"kind": [kind]}, "profile": {"test": False}})

    def test_examples_and_benches_are_scanned_as_nonproduction(self):
        manifest = self.root / "crates/probe/Cargo.toml"
        with manifest.open("a") as stream:
            stream.write('\n[[example]]\nname="helper"\npath="tests/support/example.rs"\ntest=false\n')
            stream.write('\n[[bench]]\nname="probe"\nharness=false\n')
        self.write("crates/probe/tests/support/example.rs", 'fn main() {}')
        self.write("crates/probe/examples/ordinary.rs", 'fn main() {}')
        self.write("crates/probe/benches/probe.rs", 'fn main() {}')
        names = self.discover()
        for name in ("tests/support/example.rs", "examples/ordinary.rs", "benches/probe.rs"):
            self.assertIn("crates/probe/" + name, names)

    def test_tests_component_in_production_is_refused(self):
        self.write("crates/probe/src/tests/helper.rs", "pub fn helper() {}")
        self.write("crates/probe/src/lib.rs", '#[path="tests/helper.rs"] mod helper;')
        with self.assertRaisesRegex(ValueError, "test source compiled in production"):
            self.discover()

    def test_production_observation_wins_over_nonproduction(self):
        classes = checks.sources.classify(self.root, {("crates/probe/src/lib.rs", True),
                                                      ("crates/probe/src/lib.rs", False)}, {})
        self.assertEqual(classes, {"production": ["crates/probe/src/lib.rs"], "test_only": []})

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


class DependencyPrecondition(unittest.TestCase):
    def test_missing_cargo_refuses_before_agreement_fixtures(self):
        with tempfile.TemporaryDirectory(prefix="guardrails-empty-path-") as empty_path:
            result = subprocess.run(
                [sys.executable, "-I", str(Path(__file__).resolve()), "AgreementFixtures"],
                cwd=ROOT, env=dict(os.environ, PATH=empty_path), capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("GUARDRAILS_TOOL_UNAVAILABLE", result.stderr)
        self.assertIn("Ran 0 tests", result.stderr)

    def test_empty_cargo_home_fails_before_agreement_fixtures(self):
        with tempfile.TemporaryDirectory(prefix="guardrails-empty-cargo-") as cargo_home:
            env = dict(os.environ, CARGO_HOME=cargo_home, CARGO_NET_OFFLINE="true")
            result = subprocess.run(
                [sys.executable, "-I", str(Path(__file__).resolve()), "AgreementFixtures"],
                cwd=ROOT, env=env, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("GUARDRAILS_DEPENDENCIES_UNAVAILABLE", result.stderr)
        self.assertIn("cargo fetch --locked", result.stderr)
        self.assertIn("Ran 0 tests", result.stderr)


class AgreementFixtures(unittest.TestCase):
    """Real workspace integration; first run `cargo fetch --locked` in this tree.

    Fixture execution stays offline. Check the complete locked dependency set
    before copying or modifying the workspace, even when a build cache is warm.
    """

    @classmethod
    def setUpClass(cls):
        found = shutil.which("cargo")
        if found is None:
            raise RuntimeError("GUARDRAILS_TOOL_UNAVAILABLE: install Cargo and set PATH before running source-agreement fixtures")
        cls.cargo = os.path.abspath(found)
        fetched = subprocess.run([cls.cargo, "fetch", "--locked", "--offline"], cwd=ROOT,
                                 capture_output=True, text=True)
        if fetched.returncode:
            raise RuntimeError(
                "GUARDRAILS_DEPENDENCIES_UNAVAILABLE: prepare the locked dependency set "
                "with `cargo fetch --locked` in the source workspace, using the same "
                "CARGO_HOME as these fixtures. Offline preflight failed:\n" + fetched.stderr)

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="source-contract-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "tree"
        shutil.copytree(ROOT, self.root, ignore=shutil.ignore_patterns(".git", "target", "__pycache__"))

    def run_cargo(self, *args):
        return subprocess.run([self.cargo, *args, "--target-dir", str(self.root / "target")],
                              cwd=self.root, capture_output=True, text=True)

    def agreement(self):
        return self.run_cargo("test", "-p", "forge-warden-cli", "--test", "classifier_agreement",
                              "--locked", "--offline")

    def test_extra_workspace_member_agrees(self):
        manifest = self.root / "Cargo.toml"
        text = manifest.read_text()
        marker = 'members = ["crates/forge-warden-core", "crates/forge-warden-cli"]'
        self.assertIn(marker, text)
        manifest.write_text(text.replace(marker, marker[:-1] + ', "crates/boundary-extra"]', 1))
        member = self.root / "crates/boundary-extra"
        (member / "src").mkdir(parents=True)
        (member / "Cargo.toml").write_text(
            '[package]\nname="boundary-extra"\nversion="0.1.0"\nedition="2024"\n'
            'description="Compiler context agreement fixture"\nlicense="MIT"\n'
            '[lints]\nworkspace=true\n')
        (member / "src/lib.rs").write_text('//! Compiled workspace identity control.\n/// A public control.\npub fn control() {}\n')
        lock = subprocess.run([self.cargo, "generate-lockfile", "--offline"], cwd=self.root,
                              capture_output=True, text=True)
        self.assertEqual(lock.returncode, 0, lock.stderr)
        result = self.agreement()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_non_rs_include_refused_by_both(self):
        main = self.root / "crates/forge-warden-cli/src/main.rs"
        main.write_text(main.read_text() + '\ninclude!("boundary_input.inc");\n')
        (main.parent / "boundary_input.inc").write_text(
            '#[expect(dead_code, reason = "compiled source admission fixture")]\nfn boundary_input() {}\n')
        compiled = self.run_cargo("check", "--workspace", "--all-targets", "--locked", "--offline")
        self.assertEqual(compiled.returncode, 0, compiled.stderr)
        rust = self.agreement()
        self.assertNotEqual(rust.returncode, 0, rust.stdout + rust.stderr)
        self.assertIn("boundary_input.inc", rust.stdout + rust.stderr)
        # A refusal only from the Python comparison is not evidence that Rust
        # admitted the input correctly: require the Rust consumer's own panic.
        self.assertIn("tests/common/mod.rs:", rust.stdout + rust.stderr)
        python = subprocess.run([sys.executable, "-I", "scripts/check_guardrails.py"], cwd=self.root,
                                capture_output=True, text=True)
        self.assertEqual(python.returncode, 1, python.stdout + python.stderr)
        self.assertIn("unreviewed non-Rust compiler input", python.stderr)
        self.assertIn("boundary_input.inc", python.stderr)


if __name__ == "__main__":
    unittest.main()
