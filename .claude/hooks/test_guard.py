#!/usr/bin/env python3
"""Hook protocol and known M3 bypass regressions; never executes proposed commands."""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

GUARD = Path(__file__).with_name("guard.py")
CHECKOUT_JUST = GUARD.parents[2] / "scripts/checkout_just.py"


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

    def run_guard(self, event, mode="pre", expected=2, guard=GUARD):
        raw = event if isinstance(event, bytes) else json.dumps(event).encode()
        result = subprocess.run([sys.executable, "-I", str(guard), mode], input=raw,
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
        guard = self.root / ".claude/hooks/guard.py"
        guard.parent.mkdir(parents=True)
        shutil.copyfile(GUARD, guard)
        shutil.copy2(GUARD.parents[2] / "scripts/make_patch.sh", scripts)
        (scripts / "worktree_diff.py").write_bytes((GUARD.parents[2] / "scripts/worktree_diff.py").read_bytes())
        (scripts / "tests").mkdir()
        direct = self.root / "direct"
        direct.mkdir()
        for entry in ("ci_fast.sh", "ci_local.sh"):
            shutil.copy2(GUARD.parents[2] / "scripts" / entry, direct / entry)
        for first_step in (scripts / "tests/mutation_budget.py", scripts / "tool_versions.py"):
            first_step.write_text("from pathlib import Path\nPath('.git/gate-ran').touch()\nraise SystemExit(91)\n")

        def assert_refused_before_gate(path):
            for expected in (2, 2, 2, 0):
                result = self.run_guard(event, mode="stop", expected=expected, guard=guard)
                self.assertIn(os.fsencode(path), result.stderr)
                self.assertFalse(ran.exists())
            self.assertIn(b"after 3 attempts", result.stderr)
            for entry in ("ci_fast.sh", "ci_local.sh"):
                with self.subTest(entry=entry):
                    result = subprocess.run(["bash", direct / entry],
                                            cwd=self.root, capture_output=True)
                    self.assertEqual(result.returncode, 2, result.stderr)
                    self.assertIn(os.fsencode(path), result.stderr)
                    self.assertFalse(ran.exists())

        source = self.root / "src/lib.rs"
        original = source.read_bytes()
        env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
        with tempfile.TemporaryDirectory() as home, patch.dict(os.environ, {
                **env, "HOME": home, "TMPDIR": home, "XDG_CONFIG_HOME": home, "GIT_CONFIG_NOSYSTEM": "1"}, clear=True):
            def git(*args):
                subprocess.run(["git", *args], cwd=self.root, check=True)

            git("init", "-q", "--template=")
            template = self.root / "crates/core/src/template.txt"
            template.parent.mkdir()
            template.write_bytes(b"embedded\n")
            for reviewed in ("clippy.toml", "crates/forge-warden-core/clippy.toml"):
                config = self.root / reviewed
                config.parent.mkdir(parents=True, exist_ok=True)
                config.touch()
            git("add", ".")
            git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                "commit", "-qm", "base")
            git("config", "status.showUntrackedFiles", "no")
            self.run_guard(event, mode="stop", expected=0)
            self.assertFalse(ran.exists())
            gate_bytes = gate.read_bytes()
            gate.write_text("#!/bin/sh\necho fixture-failure >&2\nexit 1\n")
            self.run_guard(event, mode="stop")
            counter, = Path(home).glob("forge-warden-stop-*")
            counter.write_bytes(b"")
            gate.write_bytes(gate_bytes)
            self.run_guard(event, mode="stop", expected=0)
            self.assertFalse(counter.exists())
            self.assertFalse(ran.exists())
            gate.write_text("#!/bin/sh\necho fixture-failure >&2\nexit 1\n")
            for damaged in (b"", b"2x", b"99", b"\xff"):
                counter.write_bytes(damaged)
                with counter.open("rb") as old:
                    for count in (1, 2, 3):
                        result = self.run_guard(event, mode="stop")
                        self.assertIn(b"fixture-failure", result.stderr)
                        self.assertEqual(counter.read_bytes(), str(count).encode())
                        self.assertEqual(counter.stat().st_mode & 0o777, 0o600)
                    self.assertEqual(old.read(), damaged)
                result = self.run_guard(event, mode="stop", expected=0)
                self.assertIn(b"after 3 attempts", result.stderr)
                self.assertFalse(counter.exists())
            gate.write_bytes(gate_bytes)
            counter.symlink_to(source)
            self.assertIn(b"symlink", self.run_guard(event, mode="stop").stderr)
            self.assertEqual(source.read_bytes(), original)
            counter.unlink()
            attributes = self.root / ".git/info/attributes"
            attributes.parent.mkdir(exist_ok=True)
            (attributes.parent / "exclude").write_text(
                "/crates/core/**/*.rs\n*.[mM][dD]\ncurrent\nconfig\nconfig.toml\nclippy.toml\n.clippy.toml\n"
                "/.github/workflows/\nCargo.toml\n")
            for directory in ("target", "mutants.out", "mutants.out.old", ".git", "docs/.git"):
                generated = self.root / directory / "report.md"
                generated.parent.mkdir(parents=True, exist_ok=True)
                generated.touch()
                (generated.parent / ".clippy.toml").touch()
                (generated.parent / "current").symlink_to(".")
            (self.root / "docs/config.toml").touch()
            self.run_guard(event, mode="stop", expected=0)
            self.assertFalse(ran.exists(), "excluded outputs and ordinary configuration files")
            (self.root / "docs/pending.md").touch()
            git("update-index", "--assume-unchanged", "src/lib.rs")
            recorder = Path(home) / "cargo"
            recorder.write_text("#!/bin/sh\ntouch .git/gate-ran\nexit 91\n")
            recorder.chmod(0o755)
            os.environ["PATH"] = home + os.pathsep + os.environ["PATH"]
            os.environ["RUSTC_WRAPPER"] = str(recorder)
            git("config", "filter.record.clean", "cargo")
            attributes.write_text("template.txt filter=record\n")
            template.write_bytes(b"changed\n")
            probe = scripts / "worktree_diff.py"
            saved_probe = scripts / "saved-probe.py"
            probe.rename(saved_probe)
            probe.symlink_to("tests/mutation_budget.py")
            assert_refused_before_gate("scripts/worktree_diff.py")
            probe.unlink()
            saved_probe.rename(probe)
            link = self.root / "docs/current"
            for destination in (".", "../Cargo.toml", "missing"):
                with self.subTest(destination=destination):
                    link.symlink_to(destination)
                    git("check-ignore", "-q", "--", str(link))
                    assert_refused_before_gate("docs/current")
                    link.unlink()
            for path in (".cargo/config", "crates/core/.cargo/config.toml", "crates/core/clippy.toml",
                         "docs/target/.clippy.toml"):
                with self.subTest(path=path):
                    ignored = self.root / path
                    ignored.parent.mkdir(parents=True, exist_ok=True)
                    ignored.touch()
                    git("check-ignore", "-q", "--", str(ignored))
                    assert_refused_before_gate(path)
                    ignored.unlink()
            attributes.unlink()
            template.write_bytes(b"embedded\n")
            git("update-index", "--no-assume-unchanged", "src/lib.rs")
            (self.root / "docs/pending.md").unlink()
            for path in ("docs/local-review.md", "docs/target/source.MD", "mutants.out.backup/source.Md",
                         ".github/workflows/local.yml", ".github/workflows/.note", "crates/local/Cargo.toml",
                         "crates/group/nested/Cargo.toml"):
                with self.subTest(path=path):
                    ignored = self.root / path
                    ignored.parent.mkdir(parents=True, exist_ok=True)
                    ignored.touch()
                    git("check-ignore", "-q", "--", str(ignored))
                    self.run_guard(event, mode="stop", expected=0)
                    self.assertTrue(ran.exists(), path)
                    ran.unlink()
                    ignored.unlink()
            workflow_dir = self.root / ".github/workflows/empty"
            workflow_dir.mkdir()
            self.run_guard(event, mode="stop", expected=0)
            self.assertTrue(ran.exists(), "empty workflow subdirectory")
            ran.unlink()
            workflow_dir.rmdir()
            for target in ("src/bin/probe.rs", "tests/regression.rs", "examples/demo/main.rs",
                           "benches/bench.rs", "build.rs"):
                with self.subTest(target=target):
                    ignored = self.root / "crates/core" / target
                    ignored.parent.mkdir(parents=True, exist_ok=True)
                    ignored.write_text("fn main() {}\n")
                    git("check-ignore", "-q", "--", str(ignored))
                    self.run_guard(event, mode="stop", expected=0)
                    self.assertTrue(ran.exists(), target)
                    ran.unlink()
                    ignored.unlink()
            git("config", "core.fileMode", "false")
            gate.chmod(0o644)
            self.run_guard(event, mode="stop", expected=2)
            gate.chmod(0o755)
            source.chmod(0o755)
            self.run_guard(event, mode="stop", expected=0)
            self.assertTrue(ran.exists(), "executable bit added")
            source.chmod(0o644)
            ran.unlink()

            attributes.write_text("*.rs filter=local\n")
            git("config", "filter.local.required", "false")
            for setting in ("clean", "smudge", "process"):
                git("config", "filter.local." + setting, "")
            self.run_guard(event, mode="stop", expected=0)
            self.assertFalse(ran.exists(), "optional filter with empty commands")
            git("config", "filter.local.required", "true")
            self.run_guard(event, mode="stop", expected=0)
            self.assertTrue(ran.exists(), "required filter")
            ran.unlink()
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
                    assert_refused_before_gate("restricted")
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

    def test_gate_launchers_validate_script_paths(self):
        repo = GUARD.parents[2]
        scripts = self.root / "scripts"
        scripts.mkdir()
        hooks = self.root / ".githooks"
        hooks.mkdir()
        shutil.copyfile(repo / "justfile", self.root / "justfile")
        env = {k: v for k, v in os.environ.items() if not k.startswith(("GIT_", "JUST_"))}
        env.update(GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")

        def run(*args, **overrides):
            return subprocess.run(args, cwd=self.root, env={**env, **overrides}, capture_output=True)

        run("git", "init", "-q", "--template=").check_returncode()
        ran = self.root / ".git/ran"
        recorder = self.root / ".git/record"
        recorder.write_text("#!/bin/sh\ntouch .git/ran\n")
        recorder.chmod(0o755)
        for hook, recipe, gate in (("pre-commit", "fast", "ci_fast.sh"), ("pre-push", "ci", "ci_local.sh")):
            shutil.copy2(repo / ".githooks" / hook, hooks / hook)
            launchers = [(["git", "-c", "core.hooksPath=.githooks", "hook", "run", hook], {})]
            if shutil.which("just"):
                launcher = [sys.executable, "-I", CHECKOUT_JUST, recipe]
                launchers.append((launcher, {"JUST_DRY_RUN": "true", "JUST_JUSTFILE": str(self.root / "other")}))
            for command, overrides in launchers:
                with self.subTest(command=command):
                    script = scripts / gate
                    shutil.copy2(recorder, script)
                    run(*command, **overrides).check_returncode()
                    self.assertTrue(ran.exists())
                    ran.unlink()
                    saved = self.root / ".git/scripts"
                    scripts.rename(saved)
                    scripts.symlink_to(saved)
                    result = run(*command, **overrides)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(gate.encode(), result.stderr)
                    self.assertFalse(ran.exists())
                    scripts.unlink()
                    saved.rename(scripts)
                    script.unlink()
                    script.symlink_to(recorder)
                    result = run(*command, **overrides)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(gate.encode(), result.stderr)
                    self.assertFalse(ran.exists())
                    script.unlink()

    def test_gate_checkout_identity(self):
        source = GUARD.parents[2]
        main = self.root / "main"
        linked = self.root / "linked"
        other = self.root / "other"
        env = {k: v for k, v in os.environ.items() if not k.startswith(("GIT_", "JUST_"))}
        env.update(HOME=str(self.root), TMPDIR=str(self.root), XDG_CONFIG_HOME=str(self.root / ".config"))
        for root in (main, other):
            subprocess.run(["git", "init", "-q", "--template=", root], check=True, env=env)
        for name in ("justfile", ".githooks/pre-commit", ".githooks/pre-push", ".githooks/commit-msg", "scripts/ci_fast.sh",
                     "scripts/ci_local.sh", "scripts/worktree_diff.py", "scripts/make_patch.sh"):
            path = main / name
            path.parent.mkdir(exist_ok=True)
            shutil.copy2(source / name, path)
        (main / "scripts/tests").mkdir()
        for name in ("scripts/tests/mutation_budget.py", "scripts/tool_versions.py"):
            (main / name).write_text("from pathlib import Path\nPath('ran').touch()\nraise SystemExit(91)\n")
        subprocess.run(["git", "add", "."], cwd=main, env=env, check=True)
        subprocess.run(["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                        "commit", "-qm", "base"], cwd=main, env=env, check=True)
        subprocess.run(["git", "worktree", "add", "-q", "--detach", linked], cwd=main, env=env, check=True)
        copied = self.root / "copied"
        shutil.copytree(linked, copied)
        for root in (copied, linked, main):
            if root == main:
                subprocess.run(["git", "config", "core.worktree", str(other)], cwd=main, env=env, check=True)
            config = main / ".git/config"
            original = config.read_bytes()
            install = subprocess.run([sys.executable, "-I", CHECKOUT_JUST, "install-hooks"], cwd=root,
                                     env=env, capture_output=True)
            if root != linked:
                self.assertNotEqual(install.returncode, 0)
                self.assertIn(b"worktree differs", install.stderr)
                self.assertEqual(config.read_bytes(), original)
            else:
                self.assertEqual(install.returncode, 0, install.stderr)
            (root / "edit").touch()
            commands = []
            for recipe, hook, gate in (("fast", "pre-commit", "ci_fast.sh"), ("ci", "pre-push", "ci_local.sh")):
                hook_command = ["git", "hook", "run", hook] if root == linked else [root / ".githooks" / hook]
                commands.extend(([sys.executable, "-I", CHECKOUT_JUST, recipe], ["bash", root / "scripts" / gate],
                                 hook_command))
            if root == copied:
                commands.extend((["bash", "scripts/make_patch.sh", "HEAD", "HEAD"],
                                 [sys.executable, "-I", CHECKOUT_JUST, "review"],
                                 [sys.executable, "-I", "scripts/worktree_diff.py", "a" * 40]))
            commands.append([sys.executable, "-I", GUARD, "stop"])
            for command in commands:
                with self.subTest(root=root, command=command):
                    result = subprocess.run(command, cwd=root, env={**env, "CLAUDE_PROJECT_DIR": str(root)},
                                            input=b'{"session_id":"binding"}', capture_output=True)
                    self.assertNotEqual(result.returncode, 0)
                    if root != linked:
                        self.assertFalse(list(root.glob(".review-*")))
                        self.assertIn(b"worktree differs", result.stderr)
                        self.assertFalse((other / "ran").exists())
                        self.assertFalse((root / "ran").exists())
                    else:
                        self.assertTrue((root / "ran").exists(), result.stderr)
                        (root / "ran").unlink()

    def test_checkout_git_symlink_is_refused_before_git_discovery(self):
        source = GUARD.parents[2]
        for name in ("justfile", ".githooks/pre-commit", ".githooks/pre-push", "scripts/worktree_diff.py",
                     "scripts/make_patch.sh", "scripts/ci_fast.sh", "scripts/ci_local.sh"):
            path = self.root / name
            path.parent.mkdir(exist_ok=True)
            shutil.copy2(source / name, path)
        other = self.root / "other"
        env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
        subprocess.run(["git", "init", "-q", "--template=", other], env=env, check=True)
        (self.root / ".git").symlink_to(other / ".git")
        tools = self.root / "bin"
        tools.mkdir()
        git = tools / "git"
        git.write_text("#!/bin/sh\necho called > git-called\nexit 91\n")
        git.chmod(0o755)
        env["PATH"] = str(tools) + os.pathsep + env["PATH"]
        for command in ([sys.executable, "-I", "scripts/worktree_diff.py", "--clean"],
                        [sys.executable, "-I", CHECKOUT_JUST, "judge-bound"],
                        [sys.executable, "-I", CHECKOUT_JUST, "review"],
                        ["bash", "scripts/make_patch.sh", "HEAD", "HEAD", "scripts"],
                        ["bash", "scripts/ci_fast.sh"], ["bash", "scripts/ci_local.sh"],
                        ["bash", ".githooks/pre-commit"], ["bash", ".githooks/pre-push"]):
            with self.subTest(command=command):
                result = subprocess.run(command, cwd=self.root, env=env, capture_output=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(b".git", result.stderr)
                self.assertFalse(list(self.root.rglob("git-called")))
                self.assertFalse(list(self.root.glob(".review-*")))

    def test_fixed_just_recipes_select_checkout(self):
        recipes = ("install-hooks", "fast", "ci", "mutants", "judge", "judge-bound", "review")
        justfile = self.root / "justfile"
        justfile.write_text("\n".join(f'{name}:\n    @echo {name} > receipt\n' for name in recipes))
        receipt = self.root / "receipt"
        env = {**os.environ, "JUST_DRY_RUN": "true", "JUST_JUSTFILE": str(self.root / "other")}
        command = [sys.executable, "-I", CHECKOUT_JUST]
        for recipe in recipes:
            with self.subTest(recipe=recipe):
                subprocess.run([*command, recipe], cwd=self.root, env=env, check=True)
                self.assertEqual(receipt.read_text().strip(), recipe)
                receipt.unlink()
        for args in ([], ["fast", "ci"], ["--list"]):
            self.assertNotEqual(subprocess.run([*command, *args], cwd=self.root, capture_output=True).returncode, 0)
            self.assertFalse(receipt.exists())
        justfile.rename(self.root / "other")
        justfile.symlink_to(self.root / "other")
        self.assertNotEqual(subprocess.run([*command, "fast"], cwd=self.root, env=env, capture_output=True).returncode, 0)
        self.assertFalse(receipt.exists())

    def test_hook_installation_only_writes_selected_repository(self):
        repo = GUARD.parents[2]
        (self.root / "scripts").mkdir()
        shutil.copy2(repo / "scripts/make_patch.sh", self.root / "scripts")
        shutil.copyfile(repo / "justfile", self.root / "justfile")
        shutil.copytree(repo / ".githooks", self.root / ".githooks")
        env = {k: v for k, v in os.environ.items() if not k.startswith(("JUST_", "GIT_"))}
        env.update(GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")
        other = self.root / "other"
        for root in (self.root, other):
            subprocess.run(["git", "init", "-q", "--template=", root], env=env, check=True)
        config = other / ".git/config"
        before = config.read_bytes()
        redirects = {**env, "GIT_DIR": str(other / ".git"), "GIT_WORK_TREE": str(other),
                     "GIT_CONFIG": str(config), "GIT_CONFIG_COUNT": "1",
                     "GIT_CONFIG_KEY_0": "core.hooksPath", "GIT_CONFIG_VALUE_0": "elsewhere"}

        def git(*args):
            return subprocess.run(["git", *args], cwd=self.root, env=env, capture_output=True)

        def install(**overrides):
            return subprocess.run([sys.executable, "-I", CHECKOUT_JUST, "install-hooks"], cwd=self.root,
                                  env={**redirects, **overrides}, capture_output=True)

        local = self.root / ".git/config"
        git("config", "--local", "core.worktree", str(other)).check_returncode()
        original = local.read_bytes()
        refused = install()
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn(b"worktree differs", refused.stderr)
        self.assertEqual(local.read_bytes(), original)
        self.assertEqual(config.read_bytes(), before)
        git("config", "--local", "--unset", "core.worktree").check_returncode()
        git("config", "extensions.worktreeConfig", "true").check_returncode()
        git("config", "--worktree", "core.hooksPath", "other-hooks").check_returncode()
        (self.root / "other-hooks").mkdir()
        refused = install()
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn(b"effective core.hooksPath", refused.stderr)
        self.assertEqual(git("config", "--worktree", "--get", "core.hooksPath").stdout, b"other-hooks\n")
        self.assertEqual(config.read_bytes(), before)

        git("config", "--worktree", "--unset", "core.hooksPath").check_returncode()
        git("config", "--local", "core.hooksPath", "old-hooks").check_returncode()
        included = self.root / "included"
        included.write_text("[core]\n\thooksPath = other-hooks\n")
        git("config", "--local", "include.path", str(included)).check_returncode()
        original = local.read_bytes()
        refused = install()
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn(b"effective core.hooksPath", refused.stderr)
        self.assertEqual(local.read_bytes(), original)
        git("config", "--local", "--unset", "include.path").check_returncode()
        tools = self.root / "bin"
        tools.mkdir()
        probe = tools / "git"
        probe.write_text('#!/bin/sh\n'
                         'if { [ "$DISCOVERY_ONLY" = 0 ] || [ "$1" = rev-parse ]; } && '
                         '[ "$("$REAL_GIT" --git-dir=.git config --local --get core.hooksPath)" = .githooks ]; then\n'
                         '    exec "$REAL_GIT" -c core.hooksPath=other-hooks "$@"\n'
                         'fi\nexec "$REAL_GIT" "$@"\n')
        probe.chmod(0o755)
        for existing in (True, False):
            if not existing:
                git("config", "--local", "--unset", "core.hooksPath").check_returncode()
            original = local.read_bytes()
            refused = install(REAL_GIT=shutil.which("git"), PATH=str(tools) + os.pathsep + env["PATH"],
                              DISCOVERY_ONLY="0" if existing else "1")
            self.assertNotEqual(refused.returncode, 0)
            self.assertIn(b"effective core.hooksPath", refused.stderr)
            self.assertEqual(local.read_bytes(), original)
        install().check_returncode()
        message = self.root / "message"
        message.write_text("wip\n")
        result = git("hook", "run", "commit-msg", "--", str(message))
        self.assertEqual(result.returncode, 1)
        self.assertIn(b"does not say what changed", result.stderr)
        self.assertEqual(config.read_bytes(), before)

    def test_recipe_helpers_are_checked_before_execution(self):
        repo = GUARD.parents[2]
        shutil.copyfile(repo / "justfile", self.root / "justfile")
        scripts = self.root / "scripts"
        scripts.mkdir()
        marker = self.root / "ran"
        recorder = self.root / "record"
        recorder.write_text("#!/bin/sh\ntouch ran\nexit 77\n")
        recorder.chmod(0o755)
        helpers = {
            "review": ("make_patch.sh",),
            "mutants": ("make_patch.sh", "worktree_diff.py"),
            "judge": ("make_patch.sh", "worktree_diff.py"),
            "judge-bound": ("make_patch.sh", "worktree_diff.py"),
        }
        for recipe, names in helpers.items():
            for name in names:
                shutil.copy2(recorder, scripts / name)
            for name in names:
                with self.subTest(recipe=recipe, helper=name):
                    helper = scripts / name
                    helper.unlink()
                    helper.symlink_to(recorder)
                    result = subprocess.run([sys.executable, "-I", CHECKOUT_JUST, recipe], cwd=self.root,
                                            capture_output=True)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(f"scripts/{name}".encode(), result.stderr)
                    self.assertFalse(marker.exists())
                    helper.unlink()
                    shutil.copy2(recorder, helper)
        shutil.copy2(repo / "scripts/make_patch.sh", scripts)
        shutil.copy2(repo / "scripts/worktree_diff.py", scripts / "worktree_diff.py")
        (self.root / ".cargo").mkdir()
        (self.root / ".cargo/config.toml").write_text("[build]\n")
        env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
        subprocess.run(["git", "init", "-q", "--template=", self.root], env=env, check=True)
        for recipe in ("mutants", "judge", "judge-bound"):
            with self.subTest(recipe=recipe, forbidden="Cargo configuration"):
                result = subprocess.run([sys.executable, "-I", CHECKOUT_JUST, recipe], cwd=self.root,
                                        capture_output=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(b".cargo/config.toml", result.stderr)
                self.assertFalse(marker.exists())

    @unittest.skipUnless(shutil.which("just"), "just is required for the vocabulary recipe")
    def test_just_defers_vocabulary_lookup(self):
        command = [shutil.which("just"), "--justfile", self.root / "justfile"]
        env = {k: v for k, v in os.environ.items() if not k.startswith(("JUST_", "GIT_"))}

        def run(*args):
            return subprocess.run([*command, *args], cwd=self.root, env=env, capture_output=True)

        repo = GUARD.parents[2]
        scripts = self.root / "scripts"
        scripts.mkdir()
        shutil.copyfile(repo / "justfile", self.root / "justfile")
        receipt = self.root / "lookup"
        lookup = scripts / "tool_versions.py"
        target = self.root / "version.py"
        target.write_text("from pathlib import Path\nPath('lookup').touch()\nprint('pinned')\n")
        lookup.symlink_to(target)
        vocab = scripts / "check_vocab.sh"
        vocab.write_text('#!/bin/sh\nprintf "%s" "$1" > revision\n')
        vocab.chmod(0o755)
        for recipe, gate in (("fast", "ci_fast.sh"), ("ci", "ci_local.sh")):
            script = scripts / gate
            script.write_text("#!/bin/sh\nexit 0\n")
            script.chmod(0o755)
            run(recipe).check_returncode()
            self.assertFalse(receipt.exists())
        revision = self.root / "revision"
        for state in ("symlink", "missing"):
            with self.subTest(lookup=state):
                if state == "missing":
                    lookup.unlink()
                result = run("vocab")
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(b"scripts/tool_versions.py", result.stderr)
                self.assertFalse(revision.exists())
                run("vocab", "explicit revision").check_returncode()
                self.assertEqual(revision.read_text(), "explicit revision")
                self.assertFalse(receipt.exists())
                revision.unlink()
        target.rename(lookup)
        run("vocab").check_returncode()
        self.assertEqual(revision.read_text(), "pinned")
        self.assertTrue(receipt.exists())

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
