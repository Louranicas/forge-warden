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


RUSTC_SHIM = """#!/bin/sh
[ "$RUSTUP_AUTO_INSTALL" = 0 ] || exit 3
[ "$1" = -Vv ] || exit 90
[ -n "$RUSTC_RELEASE" ] || { echo "error: toolchain not installed" >&2; exit 1; }
printf 'rustc %s (fixture)\\nbinary: rustc\\nrelease: %s\\nLLVM version: 21\\n' "$RUSTC_RELEASE" "$RUSTC_RELEASE"
"""
CARGO_V_SHIM = """#!/bin/sh
[ "$RUSTUP_AUTO_INSTALL" = 0 ] || exit 3
[ "$1" = -V ] || exit 90
printf 'cargo %s (fixture 2026-01-01)\\n' "$CARGO_RELEASE"
"""


# Toolchain-site text is assembled at run time, so this file is not itself a toolchain site.
PLUS = "+"
DEFAULT = "rustup " + "default "


class Toolchain(unittest.TestCase):
    """G-16: the gate's compiler is the pinned one, and every toolchain site names a canonical version."""

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="toolchain-")
        self.root = Path(self.directory.name)
        names = {*tool_versions.TOOLCHAIN_SITES, "scripts/tool-versions.toml", "rust-toolchain.toml", "Cargo.toml"}
        for name in names:
            (self.root / name).parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, self.root / name)
        (self.root / "bin").mkdir()
        for name, text in (("rustc", RUSTC_SHIM), ("cargo", CARGO_V_SHIM)):
            (self.root / "bin" / name).write_text(text)
            (self.root / "bin" / name).chmod(0o700)
        self.pinned = tool_versions.versions(ROOT)["rust"]

    def tearDown(self):
        self.directory.cleanup()

    def env(self, **extra):
        return {"PATH": f"{self.root / 'bin'}:/usr/bin:/bin", "RUSTC_RELEASE": self.pinned,
                "CARGO_RELEASE": self.pinned, **extra}

    def test_the_pinned_compiler_is_admitted_offline(self):
        tool_versions.check_toolchain(self.root, self.env(), timeout=2)
        tool_versions.check_toolchain(self.root, self.env(**{"RUSTUP_" + "TOOLCHAIN": self.pinned}), timeout=2)

    def test_another_compiler_is_refused(self):
        for extra, pattern in [
            ({"RUSTC_RELEASE": "1.99.0"}, "rustc is 1.99.0"),
            ({"CARGO_RELEASE": "1.98.0"}, "cargo is 1.98.0"),
            ({"RUSTUP_" + "TOOLCHAIN": "1.88.0"}, "RUSTUP_" + "TOOLCHAIN=1.88.0 overrides"),
            ({"RUSTC_RELEASE": ""}, "rustc did not report one version"),
        ]:
            with self.subTest(extra=extra), self.assertRaisesRegex(ValueError, pattern):
                tool_versions.check_toolchain(self.root, self.env(**extra), timeout=2)

    def test_no_compiler_on_the_path_is_refused(self):
        empty = self.root / "empty"
        empty.mkdir()
        with self.assertRaisesRegex(ValueError, "rustc is unavailable"):
            tool_versions.check_toolchain(self.root, {"PATH": str(empty)}, timeout=2)

    def test_the_real_tree_names_only_canonical_toolchains_at_listed_sites(self):
        tool_versions.check_toolchain_sites(ROOT)
        tool_versions.check_toolchain_sites(self.root)

    def edit(self, name, old, new):
        path = self.root / name
        text = path.read_text()
        self.assertEqual(text.count(old), 1, (name, old))
        path.write_text(text.replace(old, new))

    def test_a_drifting_site_is_refused_with_its_line(self):
        self.edit("plants/msrv_boundary.py", "'" + PLUS + "1.88.0'", "'" + PLUS + "1.87.0'")
        with self.assertRaisesRegex(ValueError, r"plants/msrv_boundary\.py:\d+: toolchain 1\.87\.0 drifts"):
            tool_versions.check_toolchain_sites(self.root)

    def test_a_bump_that_misses_a_site_is_refused(self):
        bumped = self.pinned.rsplit(".", 1)[0] + ".9"
        self.edit("rust-toolchain.toml", f'channel = "{self.pinned}"', f'channel = "{bumped}"')
        with self.assertRaisesRegex(ValueError, f"toolchain {self.pinned.replace('.', '[.]')} drifts"):
            tool_versions.check_toolchain_sites(self.root)

    def test_an_unlisted_or_vanished_site_is_refused(self):
        (self.root / "scripts/new_bootstrap.sh").write_text(DEFAULT + self.pinned + "\n")
        with self.assertRaisesRegex(ValueError, r"unlisted \['scripts/new_bootstrap\.sh'\]"):
            tool_versions.check_toolchain_sites(self.root)
        (self.root / "scripts/new_bootstrap.sh").unlink()
        (self.root / "plants/msrv_boundary.py").write_text("# no toolchain here\n")
        with self.assertRaisesRegex(ValueError, r"absent \['plants/msrv_boundary\.py'\]"):
            tool_versions.check_toolchain_sites(self.root)

    def test_history_and_build_output_are_not_sites(self):
        (self.root / "CHANGELOG.md").write_text("- 2020: built with `cargo " + PLUS + "1.40.0`\n")
        (self.root / "target").mkdir()
        (self.root / "target/old.sh").write_text(DEFAULT + "1.40.0\n")
        tool_versions.check_toolchain_sites(self.root)


class ToolchainKeelAttacks(unittest.TestCase):
    """Keel's REQUEST CHANGES e6f07f96 on G-16, F2-F5, kept as planted cases (Toolchain's fixture)."""

    setUp = Toolchain.setUp
    tearDown = Toolchain.tearDown
    env = Toolchain.env

    def test_f2_a_same_number_prerelease_is_refused(self):
        for extra, pattern in [({"RUSTC_RELEASE": self.pinned + "-nightly"}, "rustc did not report one version"),
                               ({"CARGO_RELEASE": self.pinned + "-beta"}, "cargo did not report one version")]:
            with self.subTest(extra=extra), self.assertRaisesRegex(ValueError, pattern):
                tool_versions.check_toolchain(self.root, self.env(**extra), timeout=2)

    def test_f3_a_compiler_override_is_refused(self):
        for name in tool_versions.COMPILER_OVERRIDES:
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, name + " is set"):
                tool_versions.check_toolchain(self.root, self.env(**{name: "/usr/bin/false"}), timeout=2)

    def test_f3_a_cargo_config_compiler_key_is_refused(self):
        (self.root / ".cargo").mkdir()
        (self.root / ".cargo/config.toml").write_text('[build]\nrustc = "/usr/bin/false"\n')
        with self.assertRaisesRegex(ValueError, "build.rustc overrides the compiler"):
            tool_versions.check_toolchain(self.root, self.env(), timeout=2)
        (self.root / ".cargo/config.toml").write_text("[build\n")
        with self.assertRaisesRegex(ValueError, "cargo config cannot be read"):
            tool_versions.check_toolchain(self.root, self.env(), timeout=2)
        (self.root / ".cargo/config.toml").unlink()
        home = self.root / "home"
        (home / ".cargo").mkdir(parents=True)
        (home / ".cargo/config.toml").write_text('[build]\nrustc-wrapper = "sccache"\n')
        with self.assertRaisesRegex(ValueError, "build.rustc-wrapper overrides the compiler"):
            tool_versions.check_toolchain(self.root, self.env(HOME=str(home)), timeout=2)

    def test_f4_every_site_form_is_seen(self):
        site = self.root / "scripts/new_site.sh"
        for text in [
            "RUSTUP_" + 'TOOLCHAIN: "1.80.0"\n',
            "          tool" + "chain: 1.80.0\n",
            "rustup " + "install 1.80.0\n",
            "rustup " + "override set 1.80.0\n",
            "cargo " + PLUS + "nightly build\n",
            "tool" + "chain: stable\n",
        ]:
            with self.subTest(text=text):
                site.write_text(text)
                with self.assertRaisesRegex(ValueError, r"scripts/new_site\.sh:1: toolchain \S+ drifts"):
                    tool_versions.check_toolchain_sites(self.root)
        site.unlink()
        tool_versions.check_toolchain_sites(self.root)

    def test_f5_untracked_files_are_not_sites_in_a_work_tree(self):
        import subprocess
        git = ["git", "-C", str(self.root), "-c", "user.name=t", "-c", "user.email=t@local.invalid"]
        subprocess.run([*git, "init", "-q"], check=True)
        subprocess.run([*git, "add", "-A"], check=True)
        subprocess.run([*git, "commit", "-q", "-m", "fixture"], check=True)
        vendored = self.root / "vendor/somecrate/Cargo.toml"
        vendored.parent.mkdir(parents=True)
        vendored.write_text("[package]\nrust-" + 'version = "1.70"\n')
        tool_versions.check_toolchain_sites(self.root)
        subprocess.run([*git, "add", "vendor"], check=True)
        with self.assertRaisesRegex(ValueError, r"vendor/somecrate/Cargo\.toml:2: toolchain 1\.70 drifts"):
            tool_versions.check_toolchain_sites(self.root)
        shutil.rmtree(self.root / ".git")
        with self.assertRaisesRegex(ValueError, r"vendor/somecrate/Cargo\.toml"):
            tool_versions.check_toolchain_sites(self.root)


if __name__ == "__main__":
    os.environ.pop("CARGO_TARGET_DIR", None)
    unittest.main()
