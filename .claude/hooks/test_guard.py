#!/usr/bin/env python3
"""Hook protocol and known M3 bypass regressions; never executes proposed commands."""
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

GUARD = Path(__file__).with_name("guard.py")


class Guard(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="guard-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
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
        self.run_guard(self.tool("Write", file_path="Cargo.toml", content='[workspace.lints.clippy.unwrap_used]\nlevel = "deny"\n'), expected=0)
        self.run_guard(self.tool("Write", file_path="crates/core/Cargo.toml", content='[package]\nname="core"\n'))

    def test_existing_named_lint_exceptions_can_remain(self):
        content = '[workspace.lints.clippy]\nmultiple_crate_versions = "allow"\nunwrap_used = "deny"\n'
        (self.root / "Cargo.toml").write_text(content)
        self.run_guard(self.tool("Write", file_path="Cargo.toml", content=content), expected=0)

    def test_invalid_edit_shape_refuses(self):
        for data in ({"file_path":"src/lib.rs","new_string":"x"},
                     {"file_path":"src/lib.rs","old_string":"not here","new_string":"x"},
                     {"file_path":"src/lib.rs","old_string":"f","new_string":[]},
                     {"file_path":"src/lib.rs","old_string":"f","new_string":"g","replace_all":"true"}):
            self.run_guard(self.tool("Edit", **data))


if __name__ == "__main__":
    unittest.main()
