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
ANCHOR_SPEC = importlib.util.spec_from_file_location('plants_anchors', Path(__file__).with_name('anchors.py'))
ANCHORS = importlib.util.module_from_spec(ANCHOR_SPEC)
ANCHOR_SPEC.loader.exec_module(ANCHORS)


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

    def test_create_refuses_unsafe_or_existing_paths_by_name(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'source'; source.mkdir(); (source / 'dir').mkdir()
            (source / 'existing').write_text('kept\n')
            (source / 'dangling').symlink_to(root / 'nowhere')
            outside = root / 'outside'; outside.mkdir()
            (source / 'linkdir').symlink_to(outside)
            # N2: a nested .git is refused by name, even where it exists (parent check alone would pass).
            (source / 'nested' / '.git').mkdir(parents=True)
            refusals = {
                '../escape': 'relative and normalised', str(root / 'abs'): 'relative and normalised',
                './dot': 'relative and normalised', 'dir//double': 'relative and normalised',
                'dir/../up': 'relative and normalised', 'dir/': 'relative and normalised',
                '': 'relative and normalised', '.git/hooks/x': 'inside .git',
                'existing': 'already exists', 'dangling': 'already exists',
                'nested/.git/x': 'inside .git',
                'linkdir/x': 'missing or symlink parent', 'missing/x': 'missing or symlink parent',
            }
            before = sorted(p.name for p in source.iterdir())
            for path, reason in refusals.items():
                with self.subTest(path=path), self.assertRaisesRegex(ValueError, reason):
                    RUN.edit(source, dict(path=path, create='breach\n'))
            self.assertEqual(sorted(p.name for p in source.iterdir()), before)
            self.assertEqual(list(outside.iterdir()), [])
            self.assertEqual((source / 'existing').read_text(), 'kept\n')
            self.assertFalse((root / 'nowhere').exists())

    def test_recipe_has_exactly_one_kind(self):
        for invalid in (dict(path='f'), dict(path='f', new='x'), dict(path='f', old='a'),
                        dict(path='f', create='a', new='b'), dict(path='f', old='a', new='b', create='c'),
                        dict(path='f', append='a', create='b'), dict(path='f', create=None),
                        dict(path='f', append=['x']),
                        # N1: a missing or non-string path is a named refusal, not a KeyError later.
                        dict(create='a'), dict(path=None, create='a'), dict(path=['f'], create='a'),
                        # F1: text the write cannot encode is refused before any file exists.
                        dict(path='f', create='\ud800'), dict(path='f', old='a', new='\ud800'),
                        dict(path='f', old='a', new=None),
                        # N5: so is a path the filesystem call could not encode.
                        dict(path='\ud800', create='x')):
            with self.subTest(invalid=invalid), self.assertRaisesRegex(ValueError, 'recipe'):
                RUN.recipe_kind(invalid)
        for valid, kind in ((dict(path='f', old='a', new='b'), 'old'), (dict(path='f', append='a'), 'append'),
                            (dict(path='f', create=''), 'create')):
            self.assertEqual(RUN.recipe_kind(valid), kind)

    def test_create_under_a_symlinked_ancestor_of_the_root_still_works(self):
        # N3: /home -> /var/home is normal here. Only components below the root are judged.
        with tempfile.TemporaryDirectory() as directory:
            real = Path(directory) / 'real'; (real / 'source' / 'dir').mkdir(parents=True)
            alias = Path(directory) / 'alias'; alias.symlink_to(real)
            (real / 'source' / 'linkdir').symlink_to(real / 'source' / 'dir')
            path, before = RUN.edit(alias / 'source', dict(path='dir/new', create='x\n'))
            self.assertEqual((real / 'source' / 'dir' / 'new').read_text(), 'x\n')
            RUN.restore(path, before)
            with self.assertRaisesRegex(ValueError, 'missing or symlink parent'):
                RUN.edit(alias / 'source', dict(path='linkdir/new', create='x\n'))
            self.assertEqual(sorted(p.name for p in (real / 'source' / 'dir').iterdir()), [])

    def test_a_failed_create_write_leaves_no_file(self):
        # F1: the file exists from the open on; a write that fails must not leave it behind.
        class Broken:
            def __init__(self, handle, *args, **kwargs): os.close(handle)
            def __enter__(self): return self
            def __exit__(self, *exc): return False
            def write(self, text): raise OSError('disk full (simulated)')
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)
            with patch.object(RUN.os, 'fdopen', Broken), \
                    self.assertRaisesRegex(ValueError, 'create plant write failed'):
                RUN.edit(source, dict(path='new', create='x\n'))
            self.assertFalse(os.path.lexists(source / 'new'))

    def test_create_then_restore_removes_exactly_that_file(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory); (source / 'dir').mkdir(); (source / 'dir' / 'kept').write_text('k\n')
            path, before = RUN.edit(source, dict(path='dir/.hidden', create='breach\n'))
            self.assertIsNone(before)
            self.assertEqual(path.read_text(), 'breach\n')
            RUN.restore(path, before)
            self.assertFalse(os.path.lexists(path))
            self.assertEqual(sorted(p.name for p in (source / 'dir').iterdir()), ['kept'])
            # Something that cannot be unlinked as a file is a RestoreError (main() aborts on it),
            # never a plain OSError that would read as a recipe error (Keel R1).
            (source / 'dir' / 'blocker').mkdir()
            with self.assertRaisesRegex(RUN.RestoreError, 'could not be removed'):
                RUN.restore(source / 'dir' / 'blocker', None)

    def test_create_case_is_caught_and_restored_even_when_ignored(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'source'; source.mkdir()
            (source / 'Cargo.lock').write_text('fixture lock\n')
            (source / '.gitignore').write_text('ignored.plant\n')
            (source / 'check.py').write_text(
                'from pathlib import Path\nimport sys\n'
                'bad=[p for p in ("new.plant","ignored.plant") if Path(p).exists()]\n'
                'print("named create breach" if bad else "clean create baseline")\n'
                'sys.exit(101 if bad else 0)\n')
            def git(*args):
                return subprocess.check_output(['git', '-c', 'core.hooksPath=/dev/null',
                    '-C', str(source), *args], env=RUN.environment(), stderr=subprocess.PIPE)
            git('init', '-q'); git('add', '--', 'Cargo.lock', '.gitignore', 'check.py')
            git('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                '-c', 'commit.gpgsign=false', 'commit', '-qm', 'fixture')
            revision = git('rev-parse', 'HEAD').decode().strip()
            (root / 'run.py').write_bytes(Path(__file__).with_name('run.py').read_bytes())
            command = [sys.executable, '-I', 'check.py']
            (root / 'catalog.json').write_text(json.dumps(dict(register={}, pending_guardrails=[], cases=[
                dict(id='new', guardrail='G-06', path='new.plant', create='x\n', command=command,
                     diagnostic='named create breach'),
                dict(id='ignored', guardrail='G-06', path='ignored.plant', create='x\n', command=command,
                     diagnostic='named create breach')])))
            common = [sys.executable, '-I', str(root / 'run.py'), '--source', str(source),
                      '--revision', revision]
            result = subprocess.run([*common, '--output', str(root / 'ok')], capture_output=True, timeout=20)
            self.assertEqual(result.returncode, 0, result.stderr)
            report = json.loads((root / 'ok' / 'report.json').read_text())
            self.assertEqual([r['status'] for r in report['results']], ['CAUGHT', 'CAUGHT'])
            # Plant: restore no longer removes the created file. Git status cannot see the ignored
            # one; the existence check must still report ERROR_RESTORE and stop the run.
            program = root / 'run.py'
            original = program.read_text()
            unlink = '        path.unlink(missing_ok=True)\n'
            self.assertEqual(original.count(unlink), 1)
            program.write_text(original.replace(unlink, '        pass  # planted missing unlink\n'))
            result = subprocess.run([*common, '--output', str(root / 'broken'), '--case', 'ignored'],
                                    capture_output=True, timeout=20)
            self.assertEqual(result.returncode, 1, result.stderr)
            report = json.loads((root / 'broken' / 'report.json').read_text())
            self.assertEqual(report['results'][0]['status'], 'ERROR_RESTORE')
            self.assertIn('created plant file still present', report['results'][0]['detail'])
            self.assertTrue(report['aborted_after_restore_error'])
            self.assertEqual(git('status', '--porcelain'), b'')

    def test_no_create_outlives_its_case_even_when_its_write_fails(self):
        # F1 (Keel): a create whose write fails, at an ignored path git status cannot see, must not
        # leave its file for the next case. Pinned through run.py main, case after case.
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'source'; source.mkdir()
            (source / 'Cargo.lock').write_text('fixture lock\n')
            (source / '.gitignore').write_text('*.plant\n')
            (source / 'check.py').write_text(
                'from pathlib import Path\nimport sys\n'
                'bad=sorted(str(p) for p in Path(".").glob("*.plant"))\n'
                'print("named create breach", bad) if bad else print("clean create baseline")\n'
                'sys.exit(101 if bad else 0)\n')
            def git(*args):
                return subprocess.check_output(['git', '-c', 'core.hooksPath=/dev/null',
                    '-C', str(source), *args], env=RUN.environment(), stderr=subprocess.PIPE)
            git('init', '-q'); git('add', '--', 'Cargo.lock', '.gitignore', 'check.py')
            git('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                '-c', 'commit.gpgsign=false', 'commit', '-qm', 'fixture')
            revision = git('rev-parse', 'HEAD').decode().strip()
            program = root / 'run.py'
            original = Path(__file__).with_name('run.py').read_text()
            program.write_text(original)
            command = [sys.executable, '-I', 'check.py']
            (root / 'catalog.json').write_text(json.dumps(dict(register={}, pending_guardrails=[], cases=[
                dict(id='unwritable', guardrail='G-06', path='first.plant', create='\ud800', command=command,
                     diagnostic='named create breach'),
                dict(id='next', guardrail='G-06', path='second.plant', create='x\n', command=command,
                     diagnostic='named create breach')])))
            common = [sys.executable, '-I', str(program), '--source', str(source), '--revision', revision]
            # (1) Refused before any checkout: nothing runs, no output directory.
            result = subprocess.run([*common, '--output', str(root / 'refused')], capture_output=True, timeout=20)
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertIn(b'UTF-8 encodable', result.stderr)
            self.assertFalse((root / 'refused').exists())
            # (2) Plant: the up-front encoding check is removed, so the write itself fails inside
            # create(). The file must be gone before the next case's baseline runs.
            check = "            text.encode('utf-8')\n"
            self.assertEqual(original.count(check), 1)
            program.write_text(original.replace(check, '            pass  # planted: no encoding check\n'))
            result = subprocess.run([*common, '--output', str(root / 'planted')], capture_output=True, timeout=20)
            report = json.loads((root / 'planted' / 'report.json').read_text())
            rows = {r['id']: r for r in report['results']}
            self.assertEqual(rows['unwritable']['status'], 'ERROR_RECIPE', rows['unwritable'])
            self.assertIn('create plant write failed', rows['unwritable']['detail'])
            self.assertEqual(rows['next']['status'], 'CAUGHT', rows['next'])
            self.assertNotIn('first.plant', (root / 'planted' / 'next-planted.log').read_text())
            self.assertNotIn('aborted_after_restore_error', report)
            self.assertEqual(result.returncode, 1)  # one case is an ERROR, never counted as caught
            # (3) Plant both: no encoding check and a cleanup that cannot remove the file. The run
            # must report ERROR_RESTORE and stop, not carry a hidden plant into the next case.
            unlink = '        path.unlink(missing_ok=True)\n'
            self.assertEqual(original.count(unlink), 1)
            program.write_text(original.replace(check, '            pass  # planted: no encoding check\n')
                               .replace(unlink, '        pass  # planted: no unlink\n'))
            result = subprocess.run([*common, '--output', str(root / 'stuck')], capture_output=True, timeout=20)
            report = json.loads((root / 'stuck' / 'report.json').read_text())
            self.assertEqual([r['status'] for r in report['results']], ['ERROR_RESTORE'])
            self.assertTrue(report['aborted_after_restore_error'])
            self.assertEqual(result.returncode, 1)

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
        self.assertEqual(runnable | pending, {f'G-{i:02}' for i in range(1,54)})
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



class Anchors(unittest.TestCase):
    """WP-PLANT-ANCHORS: the fast-gate check that every catalogue recipe still applies."""

    def tree(self):
        root = Path(tempfile.mkdtemp())
        (root / 'src').mkdir()
        (root / 'src/a.rs').write_text('fn once() {}\nfn twice() {}\nfn twice() {}\n')
        return root

    def test_a_recipe_that_applies_has_no_problem(self):
        root = self.tree()
        cases = [dict(id='old', path='src/a.rs', old='fn once()', new='x'),
                 dict(id='append', path='src/a.rs', append='y'),
                 dict(id='create', path='src/new.rs', create='z')]
        self.assertEqual(ANCHORS.problems(root, cases), [])

    def test_each_way_a_recipe_stops_applying_is_named(self):
        root = self.tree()
        (root / 'real').mkdir()
        (root / 'link').symlink_to(root / 'real')
        (root / 'real/b.rs').write_text('fn once() {}\n')
        (root / 'src/alias.rs').symlink_to(root / 'src/a.rs')
        bad = {
            'anchor gone (Q25 and G-14)': dict(path='src/a.rs', old='fn removed()', new='x'),
            'anchor twice': dict(path='src/a.rs', old='fn twice()', new='x'),
            'empty anchor': dict(path='src/a.rs', old='', new='x'),
            'file gone': dict(path='src/gone.rs', old='fn once()', new='x'),
            'append target gone': dict(path='src/gone.rs', append='y'),
            'create path taken': dict(path='src/a.rs', create='z'),
            'create under a missing directory': dict(path='nowhere/new.rs', create='z'),
            'symlinked file': dict(path='src/alias.rs', old='fn once()', new='x'),
            'symlinked parent': dict(path='link/b.rs', old='fn once()', new='x'),
            'absolute path': dict(path=str(root / 'src/a.rs'), old='fn once()', new='x'),
            'climbs out': dict(path='src/../src/a.rs', old='fn once()', new='x'),
            'inside .git': dict(path='.git/config', append='y'),
            'two recipe kinds': dict(path='src/a.rs', old='fn once()', new='x', append='y'),
        }
        for name, case in bad.items():
            with self.subTest(case=name):
                found = ANCHORS.problems(root, [dict(id=name, **case)])
                self.assertEqual(len(found), 1, found)
                self.assertTrue(found[0].startswith(name + ': '), found)

    def test_the_catalogue_applies_to_this_tree(self):
        cases = json.loads(Path(__file__).with_name('catalog.json').read_text())['cases']
        self.assertEqual(ANCHORS.problems(Path(__file__).resolve().parents[1], cases), [])


if __name__ == '__main__':
    unittest.main()
