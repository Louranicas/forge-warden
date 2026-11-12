"""Tests for forge-stack. Run: python3 -m unittest discover -s tests -v

Tests marked `needs tools` run the real deep-diff-forge and forge-warden binaries and are
skipped when either is absent, so a skip is "not verified", never a pass.
"""

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

PACK = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(PACK))

from forge_stack import checks, gate, hooks, install, oracle, scope  # noqa: E402
from forge_stack.core import Config, build_patch, find_tool, glob_regex, is_test_path, load_config, matches, safe  # noqa: E402

TOOLS = bool(find_tool("deep-diff-forge", "FORGE_DDF") and find_tool("forge-warden", "FORGE_WARDEN"))
FORGE = [sys.executable, str(PACK / "forge.py")]


def sh(cwd, *args, stdin=None):
    return subprocess.run(args, cwd=cwd, input=stdin, capture_output=True, text=True, check=False)


def repo(files):
    root = Path(tempfile.mkdtemp(prefix="forge-test-")).resolve()
    sh(root, "git", "init", "-q")
    sh(root, "git", "config", "user.email", "t@example.invalid")
    sh(root, "git", "config", "user.name", "t")
    write(root, files)
    sh(root, "git", "add", "-A")
    sh(root, "git", "commit", "-qm", "base")
    return root


def write(root, files):
    for rel, text in files.items():
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)


def review(path, lines, status="modified"):
    """A minimal deep-diff-forge.review.v0 with one hunk; lines are (kind, text)."""
    rows, old, new = [], 0, 0
    for kind, text in lines:
        old += kind in ("removed", "context")
        new += kind in ("added", "context")
        rows.append({"kind": kind, "text": text, "old_line": old if kind != "added" else None,
                     "new_line": new if kind != "removed" else None})
    return {"path": path, "status": status, "hunks": [{"address": "a" * 64, "new_start": 1, "old_start": 1,
                                                       "body_sha256": "b" * 64, "lines": rows}]}


def found(*files):
    return sorted({(i["check"], i["path"]) for i in checks.integrity({"files": list(files)})})


class Globs(unittest.TestCase):
    def test_star_stays_in_a_segment_and_doublestar_crosses(self):
        self.assertTrue(matches("src/a.rs", ["src/*"]))
        self.assertFalse(matches("src/x/a.rs", ["src/*"]))
        self.assertTrue(matches("src/x/y/a.rs", ["src/**"]))
        self.assertTrue(matches("src", ["src/**"]))
        self.assertTrue(matches("a/b/c_test.go", ["**/*_test.*"]))
        self.assertTrue(matches("c_test.go", ["**/*_test.*"]))
        self.assertFalse(matches("srcx/a.rs", ["src/**"]))
        self.assertIsNotNone(glob_regex("a.b").match("a.b"))
        self.assertIsNone(glob_regex("a.b").match("axb"))

    def test_test_paths(self):
        for path in ("tests/t.rs", "a/test_x.py", "x_test.go", "web/a.test.ts", "a/__tests__/b.js", "spec/a_spec.rb"):
            self.assertTrue(is_test_path(path), path)
        for path in ("src/lib.rs", "src/contest.py", "latest/a.py", "attest.rs"):
            self.assertFalse(is_test_path(path), path)

    def test_safe_escapes_controls_and_bidi(self):
        self.assertEqual(safe("a‮b\x1b[31m"), "a\\u{202e}b\\u{1b}[31m")


class Integrity(unittest.TestCase):
    def test_each_failure_mode_is_caught(self):
        cases = {
            "test-skipped": ("tests/t.rs", "#[ignore]"),
            "test-trivial-assert": ("tests/t.rs", "    assert!(true);"),
            "stub-added": ("src/a.rs", "fn save() -> bool { todo!() }"),
            "marker-added": ("src/a.rs", "// TODO: later"),
            "panic-path-added": ("src/a.rs", "let v = parse(s).unwrap();"),
            "error-swallowed": ("web/a.js", "try { go() } catch (e) {}"),
        }
        for check, (path, text) in cases.items():
            self.assertIn((check, path), found(review(path, [("added", text)])), check)

    def test_python_except_pass_across_two_lines(self):
        hits = found(review("a.py", [("added", "    except Exception:"), ("added", "        pass")]))
        self.assertIn(("error-swallowed", "a.py"), hits)
        handled = found(review("a.py", [("added", "    except Exception:"), ("added", "        raise")]))
        self.assertNotIn(("error-swallowed", "a.py"), handled)

    def test_hard_negatives(self):
        clean = [
            ("src/a.rs", "let total = items.len();"),
            ("src/a.rs", "// TODO(#42): tracked"),
            ("tests/t.rs", "assert_eq!(parse(\"5\"), Ok(5));"),
            ("tests/t.rs", "let v = parse(\"5\").unwrap();"),       # unwrap in a test is fine
            ("src/a.py", "assert True is not None"),
            ("src/a.js", "try { go() } catch (e) { log(e) }"),
        ]
        for path, text in clean:
            self.assertEqual(found(review(path, [("added", text)])), [], text)

    def test_guard_removed_unless_replaced(self):
        gone = review("src/auth.rs", [("removed", "    if !user.is_admin { return false; }"), ("context", "    true")])
        self.assertIn(("guard-removed", "src/auth.rs"), found(gone))
        moved = review("src/auth.rs", [("removed", "    if !user.is_admin { return false; }"),
                                       ("added", "    ensure_admin(user)?;")])
        self.assertNotIn(("guard-removed", "src/auth.rs"), found(moved))

    def test_assertions_removed_and_expectations_changed(self):
        lost = review("tests/t.rs", [("removed", "assert_eq!(a, 1);"), ("removed", "assert_eq!(b, 2);"),
                                     ("added", "assert_eq!(a, 1);")])
        self.assertIn(("assertions-removed", "tests/t.rs"), found(lost))
        swapped = review("tests/t.rs", [("removed", "assert_eq!(cap(500), 100);"), ("added", "assert_eq!(cap(500), 500);")])
        self.assertEqual(found(swapped), [])  # tests alone changing is not flagged
        source = review("src/a.rs", [("added", "let x = 1;")])
        self.assertIn(("expectations-changed", "tests/t.rs"), found(swapped, source))


class Scope(unittest.TestCase):
    def setUp(self):
        self.root = repo({"src/a.rs": "fn a() {}\n"})

    def test_declare_once_then_widen_is_logged(self):
        scope.declare(self.root, "Fix the pagination cursor", ["src/pagination/**"], "open", None)
        with self.assertRaises(ValueError):
            scope.declare(self.root, "A second declaration", ["**"], "open", None)
        widened = scope.widen(self.root, ["src/cursor.rs"], "pagination calls Cursor::next")
        self.assertEqual([w["glob"] for w in widened["widenings"]], ["src/cursor.rs"])
        self.assertTrue(scope.permits(widened, "src/cursor.rs"))
        self.assertFalse(scope.permits(widened, "src/other.rs"))

    def test_bad_declarations_are_refused(self):
        for task, allow in (("short", ["src/**"]), ("A long enough task", ["/etc/**"]), ("A long enough task", ["../x"])):
            with self.assertRaises(ValueError):
                scope.declare(self.root, task, allow, "open", None)

    def test_judge(self):
        declared = scope.declare(self.root, "Refactor the parser module", ["src/parser/**"], "frozen", None)
        items = scope.judge(declared, True, ["src/parser/a.rs", "src/net.rs", "tests/t.rs"])
        self.assertEqual([(i["severity"], i["path"]) for i in items], [("fail", "src/net.rs"), ("fail", "tests/t.rs")])
        self.assertEqual(scope.judge(None, True, ["a"])[0]["severity"], "hold")
        self.assertEqual(scope.judge(None, False, ["a"]), [])
        self.assertEqual(scope.judge(None, True, []), [])
        unbounded = {"allow": ["**"], "widenings": [], "tests": "open", "task": "x"}
        self.assertEqual(scope.judge(unbounded, True, ["a"])[0]["severity"], "hold")


class Guard(unittest.TestCase):
    def setUp(self):
        self.root = repo({"src/a.rs": "fn a() {}\n", "warden.toml": "x\n"})

    def test_edit_needs_a_scope_and_stays_inside_it(self):
        self.assertEqual(hooks.judge_edit(self.root, "src/a.rs")[0], "deny")
        scope.declare(self.root, "Change the a function", ["src/**"], "open", None)
        self.assertEqual(hooks.judge_edit(self.root, "src/a.rs")[0], "allow")
        self.assertEqual(hooks.judge_edit(self.root, str(self.root / "docs/x.md"))[0], "deny")
        self.assertEqual(hooks.judge_edit(self.root, "tests/new.rs")[0], "allow")
        self.assertEqual(hooks.judge_edit(self.root, "/tmp/elsewhere.txt")[0], "allow")

    def test_protected_and_state_paths(self):
        scope.declare(self.root, "Change everything here", ["**"], "open", None)
        self.assertEqual(hooks.judge_edit(self.root, "warden.toml")[0], "ask")
        self.assertEqual(hooks.judge_edit(self.root, ".claude/settings.json")[0], "ask")
        self.assertEqual(hooks.judge_edit(self.root, ".git/forge/scope.json")[0], "deny")
        self.assertEqual(hooks.judge_edit(self.root, "src/../.git/config")[0], "deny")

    def test_bash(self):
        verdicts = {
            "git commit --no-verify -m x": "deny",
            "git commit -n -m x": "deny",
            "git push --force origin main": "deny",
            "git push -f": "deny",
            "echo '{}' > .git/forge/scope.json": "deny",
            'python3 "$(git rev-parse --show-toplevel)/.claude/forge/forge.py" scope widen --allow x --reason y': "ask",
            "git config core.hooksPath /dev/null": "ask",
            "sed -i s/fail/hold/ warden.toml": "ask",
            "echo x >> .github/workflows/ci.yml": "ask",
            "cargo test 2>&1 | tail -5": "allow",
            "git commit --amend -m 'tidy'": "allow",
            "git push origin feature": "allow",
            "cat warden.toml": "allow",
            'python3 "$(git rev-parse --show-toplevel)/.claude/forge/forge.py" gate 2>&1': "allow",
        }
        for command, want in verdicts.items():
            self.assertEqual(hooks.judge_bash(self.root, command)[0], want, command)


class Configuration(unittest.TestCase):
    def test_config_is_read_from_the_base_not_the_working_tree(self):
        root = repo({"forge-stack.toml": 'scope = "required"\n', "src/a.rs": "x\n"})
        base = sh(root, "git", "rev-parse", "HEAD").stdout.strip()
        write(root, {"forge-stack.toml": 'scope = "advisory"\nprotected = []\n'})
        self.assertEqual(load_config(root, base).scope, "required")

    def test_protected_can_only_grow_and_bad_values_are_problems(self):
        root = repo({"forge-stack.toml": 'protected = ["secrets/**"]\nscope = "off"\nnope = 1\n'})
        cfg = load_config(root, sh(root, "git", "rev-parse", "HEAD").stdout.strip())
        self.assertIn("secrets/**", cfg.protected)
        self.assertIn("warden.toml", cfg.protected)
        self.assertEqual(cfg.scope, "required")
        self.assertEqual(len(cfg.problems), 2)

    def test_no_config_means_strict_defaults(self):
        cfg = load_config(repo({"a": "b\n"}), None)
        self.assertEqual((cfg.scope, cfg.oracle_mode), ("required", "off"))


class Oracle(unittest.TestCase):
    """Recorded-shape transport: these prove the tighten-only logic, not the live service."""

    def setUp(self):
        self.root = repo({"a": "b\n"})
        self.review = {"files": [review("src/a.rs", [("added", "x")]), review("src/key.rs", [("added", "k")])]}
        self.rank = {"ranked": [{"path": "src/a.rs", "signals": []}, {"path": "src/key.rs", "signals": ["secret_like"]}]}
        self.cfg = Config(oracle_mode="advisory", oracle_model="jev-1.14.0", oracle_hold_at=0.7)
        os.environ["TYPESAFE_API_KEY"] = "test-key"
        self.sent = []

    def tearDown(self):
        os.environ.pop("TYPESAFE_API_KEY", None)

    def transport(self, value):
        def send(body, key):
            self.sent.append(body)
            return {"model": body["model"], "answers": {q: {"type": "noul", "noul": value} for q in body["questions"]}}
        return send

    def test_high_probability_holds_and_secret_files_are_never_sent(self):
        items, notes = oracle.advise(self.root, self.cfg, self.review, self.rank, None, self.transport(0.9))
        self.assertEqual({i["severity"] for i in items}, {"hold"})
        self.assertEqual({i["path"] for i in items}, {"src/a.rs"})
        self.assertEqual([b["state"]["path"] for b in self.sent], ["src/a.rs"])
        self.assertTrue(any("withheld" in n for n in notes))

    def test_low_probability_adds_nothing_and_answers_are_cached(self):
        self.assertEqual(oracle.advise(self.root, self.cfg, self.review, self.rank, None, self.transport(0.1))[0], [])
        oracle.advise(self.root, self.cfg, self.review, self.rank, None, self.transport(0.1))
        self.assertEqual(len(self.sent), 1)

    def test_unavailable_is_a_note_when_advisory_and_a_hold_when_required(self):
        def broken(body, key):
            raise OSError("no network")
        items, notes = oracle.advise(self.root, self.cfg, self.review, self.rank, None, broken)
        self.assertEqual((items, len(notes)), ([], 1))
        self.cfg.oracle_mode = "required"
        items, _ = oracle.advise(self.root, self.cfg, self.review, self.rank, None, broken)
        self.assertEqual([i["check"] for i in items], ["oracle-unavailable"])

    def test_unpinned_model_and_malformed_answers_are_refused(self):
        self.cfg.oracle_model = "jev-latest"
        self.assertFalse(oracle.availability(self.cfg)[0])
        self.cfg.oracle_model, self.cfg.oracle_mode = "jev-1.14.0", "required"
        items, _ = oracle.advise(self.root, self.cfg, self.review, self.rank, None, self.transport(7))
        self.assertEqual([i["check"] for i in items], ["oracle-unavailable"])

    def test_off_sends_nothing(self):
        self.cfg.oracle_mode = "off"
        self.assertEqual(oracle.advise(self.root, self.cfg, self.review, self.rank, None, self.transport(1.0)), ([], []))
        self.assertEqual(self.sent, [])


class Bundled(unittest.TestCase):
    def test_the_bundled_default_policy_is_forge_wardens_starter(self):
        starter = PACK.parent / "examples" / "starter.warden.toml"
        if not starter.is_file():
            self.skipTest("not inside the forge-warden repository")
        self.assertEqual((PACK / "defaults" / "warden.toml").read_text(), starter.read_text())


class Install(unittest.TestCase):
    def test_settings_merge_keeps_existing_hooks_and_is_idempotent(self):
        existing = json.dumps({"permissions": {"deny": ["Bash(rm *)"]},
                               "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "mine"}]}]}})
        once = install.merged_settings(existing)
        doc = json.loads(once)
        self.assertEqual(doc["permissions"], {"deny": ["Bash(rm *)"]})
        self.assertEqual(len(doc["hooks"]["Stop"]), 2)
        self.assertEqual(install.merged_settings(once), once)

    def test_init_is_a_dry_run_unless_applied_and_never_overwrites(self):
        root = repo({"warden.toml": "mine\n"})
        self.assertEqual(install.init(root, apply=False, force=False), 0)
        self.assertFalse((root / ".claude").exists())
        self.assertEqual(install.init(root, apply=True, force=False), 0)
        self.assertTrue((root / ".claude/forge/forge.py").is_file())
        self.assertTrue((root / ".claude/skills/forge-mode/SKILL.md").is_file())
        self.assertEqual((root / "warden.toml").read_text(), "mine\n")
        (root / ".claude/forge/forge.py").write_text("tampered\n")
        self.assertEqual(install.init(root, apply=True, force=False), 4)
        self.assertEqual((root / ".claude/forge/forge.py").read_text(), "tampered\n")


@unittest.skipUnless(TOOLS, "needs tools: deep-diff-forge and forge-warden")
class EndToEnd(unittest.TestCase):
    BASE = {
        "src/lib.rs": "pub fn cap(v: u32) -> u32 {\n    if v > 100 { return 100; }\n    v\n}\n",
        "tests/t.rs": "#[test]\nfn caps() { assert_eq!(cap(500), 100); }\n",
    }

    def forge(self, root, *args, stdin=None):
        return sh(root, *FORGE, *args, stdin=stdin)

    def hook(self, root, name, **payload):
        return self.forge(root, "hook", name, stdin=json.dumps({"cwd": str(root), "session_id": "s1", **payload}))

    def test_a_good_change_passes_and_a_gutted_test_fails(self):
        root = repo(self.BASE)
        self.hook(root, "session-start")
        self.forge(root, "scope", "declare", "--task", "Raise the cap to two hundred", "--allow", "src/**")
        write(root, {"src/lib.rs": self.BASE["src/lib.rs"].replace("100", "200"),
                     "tests/t.rs": self.BASE["tests/t.rs"] + "#[test]\nfn caps_at_200() { assert_eq!(cap(500), 200); }\n"})
        good = self.forge(root, "gate", "--json")
        doc = json.loads(good.stdout)
        self.assertEqual([i for i in doc["items"] if i["severity"] == "fail"], [], good.stdout)
        write(root, {"tests/t.rs": "#[test]\n#[ignore]\nfn caps() { assert!(true); }\n"})
        bad = self.forge(root, "gate", "--json")
        self.assertEqual(bad.returncode, 7)
        self.assertLessEqual({"test-skipped", "test-trivial-assert"}, {i["check"] for i in json.loads(bad.stdout)["items"]})

    def test_untracked_and_committed_work_are_both_judged(self):
        root = repo(self.BASE)
        self.hook(root, "session-start")
        write(root, {"src/new.rs": "pub fn n() { todo!() }\n"})
        self.assertIn("stub-added", self.forge(root, "gate").stdout)
        sh(root, "git", "add", "-A")
        sh(root, "git", "commit", "-qm", "wip")
        self.assertIn("stub-added", self.forge(root, "gate").stdout)  # committing does not hide it

    def test_editing_the_policy_cannot_loosen_the_gate(self):
        root = repo({**self.BASE, "warden.toml": (PACK / "defaults/warden.toml").read_text()})
        self.hook(root, "session-start")
        write(root, {"warden.toml": 'schema = "forge-warden.policy.v0"\n[[rule]]\nid = "anything-goes"\n'
                                    'why = "nothing is checked at all"\nkind = "max_files"\nseverity = "hold"\nlimit = 9999\n',
                     "web/k.ts": 'const k = "AKIAABCDEFGHIJKLMNOP";\n'})
        result = json.loads(self.forge(root, "gate", "--json").stdout)
        checks_hit = {(i["check"], i["path"]) for i in result["items"]}
        self.assertIn(("no-secrets", "web/k.ts"), checks_hit)
        self.assertIn(("protected-path", "warden.toml"), checks_hit)

    def test_out_of_scope_change_fails(self):
        root = repo(self.BASE)
        self.hook(root, "session-start")
        self.forge(root, "scope", "declare", "--task", "Document the cap function", "--allow", "docs/**")
        write(root, {"src/lib.rs": self.BASE["src/lib.rs"] + "// touched\n"})
        result = json.loads(self.forge(root, "gate", "--json").stdout)
        self.assertIn(("scope", "src/lib.rs", "fail"), {(i["check"], i["path"], i["severity"]) for i in result["items"]})

    def test_stop_hook_blocks_a_failing_change_once_per_patch(self):
        root = repo(self.BASE)
        self.hook(root, "session-start")
        write(root, {"src/lib.rs": "pub fn cap(v: u32) -> u32 { todo!() }\n"})
        first = self.hook(root, "stop")
        self.assertEqual(first.returncode, 2)
        self.assertIn("stub-added", first.stderr)
        second = self.hook(root, "stop")           # nothing changed: release, but red
        self.assertEqual(second.returncode, 0)
        self.assertIn("FAIL", json.loads(second.stdout)["systemMessage"])
        write(root, {"src/lib.rs": "pub fn cap(v: u32) -> u32 { unimplemented!() }\n"})
        self.assertEqual(self.hook(root, "stop").returncode, 2)   # a new failing patch blocks again

    def test_pre_tool_hook_end_to_end(self):
        root = repo(self.BASE)
        self.hook(root, "session-start")
        edit = {"tool_name": "Edit", "tool_input": {"file_path": str(root / "src/lib.rs")}}
        self.assertEqual(self.hook(root, "pre-tool", **edit).returncode, 2)
        self.forge(root, "scope", "declare", "--task", "Raise the cap to two hundred", "--allow", "src/**")
        self.assertEqual(self.hook(root, "pre-tool", **edit).returncode, 0)
        asked = self.hook(root, "pre-tool", tool_name="Write", tool_input={"file_path": "warden.toml"})
        self.assertEqual(json.loads(asked.stdout)["hookSpecificOutput"]["permissionDecision"], "ask")
        # a new session forgets the old scope
        self.forge(root, "hook", "session-start", stdin=json.dumps({"cwd": str(root), "session_id": "s2"}))
        self.assertEqual(self.hook(root, "pre-tool", **edit).returncode, 2)

    def test_findings_refuse_unanchored_claims(self):
        root = repo(self.BASE)
        self.hook(root, "session-start")
        write(root, {"src/lib.rs": self.BASE["src/lib.rs"].replace("100", "200")})
        findings = Path(tempfile.mkdtemp()) / "f.jsonl"
        ok = {"agent": "t", "severity": "high", "title": "cap raised", "body": "b", "path": "src/lib.rs", "line": 2}
        findings.write_text(json.dumps(ok) + "\n")
        self.assertEqual(self.forge(root, "findings", str(findings)).returncode, 0)
        findings.write_text(json.dumps(ok) + "\n" + json.dumps({**ok, "title": "ghost", "path": "nope.rs"}) + "\n")
        refused = self.forge(root, "findings", str(findings))
        self.assertEqual(refused.returncode, 7)
        self.assertIn("unknown_path", refused.stdout)

    def test_corpus_catches_escapes_and_flags_stale_declared_misses(self):
        root = repo(self.BASE)
        self.hook(root, "session-start")
        write(root, {"src/lib.rs": "pub fn cap(v: u32) -> u32 { todo!() }\n"})
        add = ["corpus", "add", "--origin", "a stub shipped as a finished change"]
        self.assertEqual(self.forge(root, *add, "--id", "stub-shipped", "--expect", "stub-added:src/lib.rs").returncode, 0)
        self.assertIn("CAUGHT", self.forge(root, "corpus", "run").stdout)
        patch = build_patch(root, sh(root, "git", "rev-parse", "HEAD").stdout.strip())
        saved = Path(tempfile.mkdtemp()) / "p.patch"
        saved.write_bytes((root / "forge-corpus/stub-shipped/change.patch").read_bytes())
        self.assertTrue(patch)
        self.forge(root, *add, "--id", "never-caught", "--expect", "no-such-check:src/lib.rs", "--patch", str(saved))
        escaped = self.forge(root, "corpus", "run")
        self.assertEqual(escaped.returncode, 7)
        self.assertIn("ESCAPED  never-caught", escaped.stdout)
        self.forge(root, *add, "--id", "stale-miss", "--expect", "stub-added:src/lib.rs", "--patch", str(saved), "--known-miss")
        self.assertIn("STALE    stale-miss", self.forge(root, "corpus", "run").stdout)

    def test_radius_lists_other_uses(self):
        root = repo({**self.BASE, "src/use.rs": "fn go() -> u32 { cap(7) }\n"})
        self.hook(root, "session-start")
        write(root, {"src/lib.rs": self.BASE["src/lib.rs"].replace("pub fn cap(v: u32)", "pub fn cap(v: u32, hi: u32)")})
        names = {n["name"]: n for n in json.loads(self.forge(root, "radius", "--json").stdout)["names"]}
        self.assertIn("src/use.rs", {u["path"] for u in names["cap"]["uses"]})

    def test_missing_tool_is_never_a_pass(self):
        root = repo(self.BASE)
        write(root, {"src/lib.rs": "pub fn cap(v: u32) -> u32 { v }\n"})
        env = {**os.environ, "FORGE_WARDEN": "/nonexistent/forge-warden"}
        done = subprocess.run([*FORGE, "gate", "--json"], cwd=root, capture_output=True, text=True, env=env, check=False)
        self.assertEqual(done.returncode, 8)
        self.assertIn("tool-missing", done.stdout)


if __name__ == "__main__":
    unittest.main()
