#!/usr/bin/env python3
"""Hook protocol and known M3 bypass regressions; never executes proposed commands."""
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

GUARD = Path(__file__).with_name("guard.py")


class Guard(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="guard-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "protected-paths.txt").write_bytes(GUARD.parents[2].joinpath("protected-paths.txt").read_bytes())
        (self.root / "crates/core").mkdir(parents=True)
        (self.root / "src").mkdir()
        (self.root / "src/lib.rs").write_text("pub fn f() {}\n")
        (self.root / "Cargo.toml").write_text('[workspace.lints.clippy]\nunwrap_used = "deny"\n')
        (self.root / "crates/core/Cargo.toml").write_text('[lints]\nworkspace = true\n')

    def run_guard(self, event, mode="pre", expected=2):
        raw = event if isinstance(event, bytes) else json.dumps(event).encode()
        result = subprocess.run([sys.executable, "-I", str(GUARD), mode], input=raw,
                                env={**os.environ, "CLAUDE_PROJECT_DIR": str(self.root)}, capture_output=True)
        self.assertEqual(result.returncode, expected, (event, result.stderr))
        return result

    def tool(self, name, **data):
        return {"tool_name": name, "tool_input": data}

    def test_malformed_events_fail_closed(self):
        for raw in (b"", b"{", b"[]", b"null", b"{}", b'{"tool_name": "Read", "tool_input": [], "tool_name":"Write"}',
                    b'{"tool_name":"Write","tool_input":null}', b"x" * (1024 * 1024 + 1)):
            with self.subTest(raw=raw[:60]):
                self.run_guard(raw)
        self.run_guard({}, mode="unknown")
        self.run_guard({}, mode="stop")

    def test_protected_paths_all_spellings(self):
        for path in ("warden.toml", "./warden.toml", str(self.root / "warden.toml"),
                     "src/../warden.toml", ".github/workflows/ci.yml"):
            with self.subTest(path=path):
                self.run_guard(self.tool("Write", file_path=path, content="x"))
        (self.root / "alias").symlink_to(self.root / "warden.toml")
        self.run_guard(self.tool("Write", file_path="alias", content="x"))
        self.run_guard(self.tool("Write", file_path="../outside", content="x"))

    def test_stop_runs_gate_for_hidden_and_untracked_changes(self):
        event = {"session_id": "stop-visibility"}
        scripts = self.root / "scripts"
        scripts.mkdir()
        gate = scripts / "ci_fast.sh"
        gate.write_text("#!/bin/sh\ntouch .git/gate-ran\n")
        gate.chmod(0o755)
        ran = self.root / ".git/gate-ran"
        source = self.root / "src/lib.rs"
        original = source.read_bytes()
        env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
        with tempfile.TemporaryDirectory() as home, patch.dict(os.environ, {
                **env, "HOME": home, "XDG_CONFIG_HOME": home, "GIT_CONFIG_NOSYSTEM": "1"}, clear=True):
            def git(*args):
                subprocess.run(["git", *args], cwd=self.root, check=True)

            git("init", "-q", "--template=")
            template = self.root / "crates/core/src/template.txt"
            template.parent.mkdir()
            template.write_bytes(b"embedded\n")
            git("add", ".")
            git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                "commit", "-qm", "base")
            git("config", "status.showUntrackedFiles", "no")
            self.run_guard(event, mode="stop", expected=0)
            self.assertFalse(ran.exists())
            attributes = self.root / ".git/info/attributes"
            attributes.parent.mkdir(exist_ok=True)
            attributes.write_text("*.rs -filter -ident -text eol=lf working-tree-encoding=UTF-8\n")
            self.run_guard(event, mode="stop", expected=0)
            self.assertFalse(ran.exists(), "disabled conversions")
            git("config", "filter.unset.clean", "cat")
            attributes.write_text("*.rs filter=unset\n")
            self.run_guard(event, mode="stop", expected=0)
            self.assertTrue(ran.exists(), "configured filter named unset")
            ran.unlink()
            for eol, edited in (("lf", b"embedded\r\n"), ("crlf", b"embedded\n")):
                with self.subTest(eol=eol):
                    attributes.write_text(f"*.txt text eol={eol}\n")
                    template.write_bytes(edited)
                    git("add", "--", "crates/core/src/template.txt")
                    status = subprocess.check_output(["git", "--no-optional-locks", "status", "--porcelain"],
                                                     cwd=self.root)
                    self.assertEqual(status, b"")
                    index = self.root / ".git/index"
                    original_index = index.read_bytes()
                    self.run_guard(event, mode="stop", expected=0)
                    self.assertTrue(ran.exists())
                    self.assertEqual(template.read_bytes(), edited)
                    self.assertEqual(index.read_bytes(), original_index)
                    ran.unlink()
            attributes.unlink()
            restricted = self.root / "restricted"
            restricted.mkdir()
            (restricted / "new.rs").touch()
            restricted.chmod(0)
            try:
                if not os.access(restricted, os.R_OK):
                    self.run_guard(event, mode="stop", expected=0)
                    self.assertTrue(ran.exists(), "unreadable directory")
                    ran.unlink()
            finally:
                restricted.chmod(0o700)
            (restricted / "new.rs").unlink()
            restricted.rmdir()
            for flag in ("assume-unchanged", "skip-worktree"):
                git("update-index", "--" + flag, "src/lib.rs")
                source.write_text("pub fn changed() {}\n")
                self.run_guard(event, mode="stop", expected=0)
                self.assertTrue(ran.exists(), flag)
                ran.unlink()
                source.write_bytes(original)
                git("update-index", "--no-" + flag, "src/lib.rs")
            (self.root / "tests").mkdir()
            (self.root / "tests/new.rs").touch()
            self.run_guard(event, mode="stop", expected=0)
            self.assertTrue(ran.exists())
            (self.root / "tests/new.rs").unlink()
            ran.unlink()

            git("init", "-q", "--template=", "module")
            nested = self.root / "module"
            (nested / "file").write_text("submodule content\n")
            git("-C", "module", "add", "file")
            git("-C", "module", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                "commit", "-qm", "nested base")
            git("submodule", "add", "./module", "module")
            git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                "commit", "-qm", "add module")
            nested_index = subprocess.check_output(
                ["git", "rev-parse", "--path-format=absolute", "--git-path", "index"], cwd=nested)
            indexes = [self.root / ".git/index", Path(os.fsdecode(nested_index.rstrip(b"\n")))]
            before = [index.read_bytes() for index in indexes]
            os.utime(nested / "file", (1_000_000_000, 1_000_000_000))
            self.run_guard(event, mode="stop", expected=0)
            self.assertTrue(ran.exists(), "initialized submodule")
            self.assertEqual([index.read_bytes() for index in indexes], before)

    def test_shell_tokenization_and_rust_writes(self):
        for command in ("echo x > war''den.toml", "echo x > " + str(self.root / "warden.toml"),
                        "echo 'unsafe fn x() {}' > src/lib.rs", "git commit -n", "git push --force-with-lease",
                        'echo x > "$DEST"', "echo x > 'unterminated"):
            with self.subTest(command=command):
                self.run_guard(self.tool("Bash", command=command))

    def test_read_and_ordinary_edit_positive_controls(self):
        self.run_guard(self.tool("Read", file_path="warden.toml"), expected=0)
        self.run_guard(self.tool("Bash", command="cat warden.toml"), expected=0)
        self.run_guard(self.tool("Bash", command="cargo test --workspace"), expected=0)
        self.run_guard(self.tool("Write", file_path="src/new.rs", content="pub fn safe() {}"), expected=0)
        self.run_guard(self.tool("Edit", file_path="src/lib.rs", old_string="f()", new_string="g()"), expected=0)

    def test_alternative_edit_tools_refuse(self):
        for name in ("MultiEdit", "NotebookEdit", "apply_patch", "mcp__filesystem__edit_file", "Task", "UnknownTool"):
            with self.subTest(name=name):
                self.run_guard(self.tool(name, file_path="warden.toml", content="x"))

    def test_rust_spacing_and_complete_edit(self):
        for content in ('#[cfg_attr(all(), allow (dead_code))] fn f() {}',
                        '#[cfg_attr(\n all(),\n allow (dead_code)\n)] fn f() {}', 'unsafe fn f() {}'):
            self.run_guard(self.tool("Write", file_path="src/lib.rs", content=content))
        (self.root / "src/lib.rs").write_text("#[cfg_attr(all(), PLACEHOLDER)] fn f() {}")
        self.run_guard(self.tool("Edit", file_path="src/lib.rs", old_string="PLACEHOLDER", new_string="allow (dead_code)"))

    def test_toml_semantics_and_member_inheritance(self):
        for content in ('[workspace.lints.clippy]\nunwrap_used = "warn"\n',
                        '[workspace.lints.clippy.unwrap_used]\nlevel = "warn"\n', "[workspace]\n", "["):
            self.run_guard(self.tool("Write", file_path="Cargo.toml", content=content))
        self.run_guard(self.tool("Write", file_path="Cargo.toml", content='[workspace.lints.clippy.unwrap_used]\nlevel = "deny"\n'))
        self.run_guard(self.tool("Write", file_path="crates/core/Cargo.toml", content='[package]\nname="core"\n'))

    def test_owner_protected_manifest_refuses_even_unchanged_content(self):
        content = '[workspace.lints.clippy]\nmultiple_crate_versions = "allow"\nunwrap_used = "deny"\n'
        (self.root / "Cargo.toml").write_text(content)
        self.run_guard(self.tool("Write", file_path="Cargo.toml", content=content))

    def test_invalid_edit_shape_refuses(self):
        for data in ({"file_path":"src/lib.rs","new_string":"x"},
                     {"file_path":"src/lib.rs","old_string":"not here","new_string":"x"},
                     {"file_path":"src/lib.rs","old_string":"f","new_string":[]},
                     {"file_path":"src/lib.rs","old_string":"f","new_string":"g","replace_all":"true"}):
            self.run_guard(self.tool("Edit", **data))


if __name__ == "__main__":
    unittest.main()
