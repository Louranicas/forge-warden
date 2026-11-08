#!/usr/bin/env python3
"""Harness controls, including unrelated-red and timeout false-positive prevention."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location('plants_runner', Path(__file__).with_name('run.py'))
RUN = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RUN)


class Runner(unittest.TestCase):
    def test_inherited_lint_caps_are_removed(self):
        flags = ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_BUILD_RUSTFLAGS')
        with patch.dict(os.environ, {name: '--cap-lints=allow' for name in flags}):
            actual = RUN.environment()
        self.assertTrue(all(name not in actual for name in flags))

    def test_caller_colour_cannot_change_named_diagnostic(self):
        child = ("import os,sys; "
                 "colour=os.environ.get('CARGO_TERM_COLOR')=='always' or "
                 "any(os.environ.get(k) not in (None, '', '0') for k in ('CLICOLOR_FORCE','FORCE_COLOR')); "
                 "print('\\x1b[1merror\\x1b[0m: named breach' if colour else 'error: named breach'); "
                 "sys.exit(101)")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for inherited in ({'CARGO_TERM_COLOR': 'never'},
                              {'CARGO_TERM_COLOR': 'always', 'CLICOLOR': '1',
                               'CLICOLOR_FORCE': '1', 'FORCE_COLOR': '3', 'TERM': 'xterm-256color'}):
                with self.subTest(inherited=inherited), patch.dict(os.environ, inherited):
                    result = RUN.execute([sys.executable, '-c', child], root, root / 'colour.log')
                    result['text'] = (root / 'colour.log').read_text()
                    self.assertEqual(RUN.classify(dict(exit=0, error=None, text='baseline'),
                                                  result, 'error: named breach'), 'CAUGHT')
                    env = RUN.environment()
                    self.assertEqual(env['CARGO_TERM_COLOR'], 'never')
                    self.assertEqual(env['TERM'], 'dumb')
                    self.assertTrue(env['NO_COLOR'])
                    self.assertNotIn('CLICOLOR_FORCE', env)
                    self.assertNotIn('FORCE_COLOR', env)

    def test_execution_profiles_keep_argv_and_refuse_silent_native_fallback(self):
        root, target = Path('/checkout'), Path('/target')
        cargo = dict(cargo=['test', '--offline'])
        script = dict(command=['python3', '-I', 'fixture.py', 'literal; $(false)'])
        self.assertEqual(RUN.command_for(cargo, root, target, '/cargo-backend'),
                         ['/cargo-backend', '/checkout', 'test', '--offline'])
        self.assertEqual(RUN.command_for(cargo, root, target, exec_runner='/exec-backend'),
                         ['/exec-backend', '/checkout', 'cargo', 'test', '--offline'])
        self.assertEqual(RUN.command_for(script, root, target, exec_runner='/exec-backend'),
                         ['/exec-backend', '/checkout', *script['command']])
        self.assertEqual(RUN.command_for(script, root, target), script['command'])
        with self.assertRaises(ValueError):
            RUN.command_for(script, root, target, cargo_runner='/cargo-only')
        for invalid in ({}, dict(cargo=[], command=[]), dict(command=[]), dict(cargo=[]),
                        dict(cargo=None), dict(cargo='test'), dict(command='python3'),
                        dict(command=['']), dict(command=['python3', None]), dict(command=['x\0y'])):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                RUN.command_for(invalid, root, target)

    def test_named_red_is_required(self):
        green = dict(exit=0, error=None)
        self.assertEqual(RUN.classify(green, dict(exit=101, error=None, text='named violation'), 'named violation'), 'CAUGHT')
        self.assertEqual(RUN.classify(green, dict(exit=101, error=None, text='unrelated compile error'), 'named violation'), 'ERROR_DIAGNOSTIC')
        self.assertEqual(RUN.classify(green, dict(exit=0, error=None, text='named violation'), 'named violation'), 'ESCAPED')
        self.assertEqual(RUN.classify(dict(exit=101, error=None), dict(exit=101, error=None, text='named violation'), 'named violation'), 'ERROR_BASELINE')
        ambiguous = dict(exit=0, error=None, text='named violation already present')
        self.assertEqual(RUN.classify(ambiguous, dict(exit=101, error=None, text='named violation'), 'named violation'), 'ERROR_DIAGNOSTIC_AMBIGUOUS')
        for failure in (dict(exit=-9, error=None, text='named violation'), dict(exit=101, error='output_limit', text='named violation')):
            self.assertEqual(RUN.classify(green, failure, 'named violation'), 'ERROR_COMMAND')

    def test_exact_edits_and_scope_refusals(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'source';source.mkdir()
            (source / 'file').write_text('once\n')
            outside = root / 'outside';outside.write_text('once\n')
            (source / 'alias').symlink_to(outside)
            for path in ('../outside', str(outside), 'alias', '.git/config'):
                with self.subTest(path=path), self.assertRaises(ValueError):
                    RUN.edit(source, dict(path=path, old='once', new='plant'))
                self.assertEqual(outside.read_text(), 'once\n')
            with self.assertRaises(ValueError):
                RUN.edit(source, dict(path='file', old='missing', new='plant'))
            self.assertEqual((source / 'file').read_text(), 'once\n')
            path, original = RUN.edit(source, dict(path='file', old='once', new='planted'))
            self.assertEqual(path.read_text(), 'planted\n')
            self.assertEqual(original, b'once\n')

    def test_commands_timeout_instead_of_counting_red(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            result=RUN.execute([sys.executable, '-c', 'import time; time.sleep(20)'], root, root/'log', timeout=0.05)
            self.assertEqual(result['error'], 'timeout')
            self.assertNotEqual(result['exit'], 0)

    def test_script_profile_runs_real_argv_in_the_disposable_checkout(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'source'; source.mkdir()
            (source / 'Cargo.lock').write_text('fixture lock\n')
            (source / 'fixture').write_text('clean\n')
            literal = 'literal; $(false)'
            (source / 'check.py').write_text(
                'from pathlib import Path\nimport sys\n'
                f'assert sys.argv[1:] == [{literal!r}]\n'
                'bad=Path("fixture").read_text()=="breach\\n"\n'
                'print("named script breach" if bad else "clean script baseline")\n'
                'sys.exit(101 if bad else 0)\n')
            def git(*args):
                return subprocess.check_output(['git', '-c', 'core.hooksPath=/dev/null',
                    '-C', str(source), *args], env=RUN.environment(), stderr=subprocess.PIPE)
            git('init', '-q'); git('add', '--', 'Cargo.lock', 'fixture', 'check.py')
            git('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                '-c', 'commit.gpgsign=false', 'commit', '-qm', 'fixture')
            revision = git('rev-parse', 'HEAD').decode().strip()
            (root / 'run.py').write_bytes(Path(__file__).with_name('run.py').read_bytes())
            (root / 'catalog.json').write_text(json.dumps(dict(register={},
                cases=[dict(id='script', guardrail='G-31', path='fixture', old='clean', new='breach',
                    command=[sys.executable, '-I', 'check.py', literal], diagnostic='named script breach')],
                pending_guardrails=[])))
            backend = root / 'backend'
            backend.write_text('#!' + sys.executable + '\nimport subprocess,sys\n'
                'sys.exit(subprocess.run(sys.argv[2:], cwd=sys.argv[1]).returncode)\n')
            backend.chmod(0o700)
            common = [sys.executable, '-I', str(root / 'run.py'), '--source', str(source),
                      '--revision', revision]
            for name, options in [('native', []), ('exec-runner', ['--exec-runner', str(backend)])]:
                output = root / name
                result = subprocess.run([*common, '--output', str(output), *options], capture_output=True, timeout=10)
                self.assertEqual(result.returncode, 0, result.stderr)
                report = json.loads((output / 'report.json').read_text())
                self.assertEqual(report['execution_profile'], name)
                self.assertEqual(report['results'][0]['status'], 'CAUGHT')
                self.assertIn(literal, report['results'][0]['baseline']['argv'])
            # A cargo-only backend must refuse before any checkout or command, not run scripts natively.
            refused = root / 'refused'
            result = subprocess.run([*common, '--output', str(refused), '--runner', str(backend)], capture_output=True)
            self.assertEqual(result.returncode, 2)
            self.assertFalse(refused.exists())
            self.assertEqual((source / 'fixture').read_text(), 'clean\n')
            self.assertEqual(git('status', '--porcelain'), b'')
            # H3: a deliberately broken restoration must be detected from the actual Git checkout.
            program = root / 'run.py'
            original = program.read_text()
            self.assertEqual(original.count('path.write_bytes(before)'), 1)
            program.write_text(original.replace('path.write_bytes(before)', 'pass  # planted missing restore'))
            output = root / 'broken-restore'
            result = subprocess.run([*common, '--output', str(output), '--case', 'script'], capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 1, result.stderr)
            report = json.loads((output / 'report.json').read_text())
            self.assertEqual(report['results'][0]['status'], 'ERROR_RESTORE')
            self.assertTrue(report['aborted_after_restore_error'])
            self.assertEqual((source / 'fixture').read_text(), 'clean\n')
            self.assertEqual(git('status', '--porcelain'), b'')

    def test_native_cargo_target_precedes_test_arguments(self):
        case = {'cargo': ['test', '-p', 'forge-warden-cli', '--', 'a_test', '--exact']}
        argv = RUN.command_for(case, Path('/checkout'), Path('/isolated-target'))
        self.assertEqual(argv, ['cargo', 'test', '-p', 'forge-warden-cli',
                                '--target-dir', '/isolated-target', '--', 'a_test', '--exact'])

    def test_missing_executable_is_an_error(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            result = RUN.execute([str(root / 'missing')], root, root / 'log')
            self.assertEqual(result['error'], 'spawn_failed')
            self.assertEqual(result['exit'], 126)

    def test_catalog_has_no_hidden_missing_guardrails(self):
        catalog=json.loads(Path(__file__).with_name('catalog.json').read_text())
        runnable={c['guardrail'] for c in catalog['cases']}
        pending={c['id'] for c in catalog['pending_guardrails']}
        self.assertFalse(runnable & pending)
        self.assertEqual(runnable | pending, {f'G-{i:02}' for i in range(1,52)})
        self.assertTrue(all(c['status']=='PENDING' for c in catalog['pending_guardrails']))

    def test_cli_receipt_is_incomplete_and_source_is_unchanged(self):
        # Real Git, synthetic compiler protocol: tests the runner, not Rust enforcement.
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'source'; source.mkdir()
            (source / 'Cargo.lock').write_text('fixture lock\n')
            (source / 'fixture').write_text('clean\n')
            def git(*args):
                return subprocess.check_output(['git', '-c', 'core.hooksPath=/dev/null',
                    '-C', str(source), *args], env=RUN.environment(), stderr=subprocess.PIPE)
            git('init', '-q')
            git('add', '--', 'Cargo.lock', 'fixture')
            git('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                '-c', 'commit.gpgsign=false', 'commit', '-qm', 'fixture')
            revision = git('rev-parse', 'HEAD').decode().strip()
            program = root / 'run.py'
            program.write_bytes(Path(__file__).with_name('run.py').read_bytes())
            (root / 'catalog.json').write_text(json.dumps(dict(register={},
                cases=[dict(id='probe', guardrail='G-01', path='fixture', old='clean',
                    new='breach', cargo=['clippy'], diagnostic='named violation')],
                pending_guardrails=[dict(id='G-02', status='PENDING')])) )
            backend = root / 'backend'
            backend.write_text('#!' + sys.executable + '\nimport sys\nfrom pathlib import Path\n'
                'bad="breach" in (Path(sys.argv[1])/"fixture").read_text()\n'
                'print("named violation" if bad else "baseline")\nsys.exit(101 if bad else 0)\n')
            backend.chmod(0o700)
            common = [sys.executable, '-I', str(program), '--source', str(source),
                '--revision', revision, '--runner', str(backend)]
            for selection, expected, name in [([], 2, 'survey'), (['--case', 'probe'], 0, 'selected')]:
                output = root / name
                result = subprocess.run([*common, '--output', str(output), *selection],
                    capture_output=True, timeout=10)
                self.assertEqual(result.returncode, expected, result.stderr)
                report = json.loads((output / 'report.json').read_text())
                self.assertEqual(report['results'][0]['status'], 'CAUGHT')
                self.assertEqual(report['revision'], revision)
                self.assertEqual(len(report['pending_guardrails']), 1)
            self.assertEqual((source / 'fixture').read_text(), 'clean\n')
            self.assertEqual(git('status', '--porcelain'), b'')


if __name__ == '__main__':
    unittest.main()
