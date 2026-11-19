#!/usr/bin/env python3
"""Owned gate contracts: exact tool inputs, complete plant receipts and merge subjects."""
import importlib.util
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
sys.dont_write_bytecode = True
import tempfile
import textwrap
import unittest
from unittest import mock
from unittest.mock import patch

import yaml

ROOT = Path(__file__).resolve().parents[2]


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


TOOLS = load("tool_versions")
PLANTS = load("check_plants")
CAPS = load("plant_tools")
COMMANDS = load("script_commands")

# Independent reviewed expectations, not values imported from the validator.
TIMEOUTS = {
    "ci.yml": {"gate": 45, "msrv": 10, "supply-chain": 10,
               "workflow-audit": 10, "mutation": 5, "mutation-plan": 10,
               "mutation-shards": "${{ matrix.minutes }}", "dogfood": 10,
               "vocabulary": 10},
    # Provisional budget; hosted full-workspace baseline is not yet available.
    "mutation-baseline.yml": {"full-workspace": 5, "mutation-plan": 10,
                              "mutation-shards": "${{ matrix.minutes }}"},
}


require_bytecode_guard = load("python_imports").require_bytecode_guard


class Contracts(unittest.TestCase):
    def test_runpy_module_bytecode_effect_is_guarded(self):
        for guarded in (False, True):
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                (root / 'probe.py').write_text('value = 1\n')
                source = ('import sys\nsys.dont_write_bytecode = True\n' if guarded else '')
                source += 'import runpy\nrunpy.run_module("probe")\n'
                # Ignore inherited PYTHONDONTWRITEBYTECODE; keep cwd importable for the control.
                subprocess.run([sys.executable, '-E', '-s', '-c', source], cwd=root, check=True)
                self.assertEqual(bool(list(root.rglob('*.pyc'))), not guarded)
                if guarded:
                    require_bytecode_guard(source, {'probe'})
                else:
                    with self.assertRaisesRegex(ValueError, 'PY_BYTECODE_GUARD'):
                        require_bytecode_guard(source, {'probe'})

    def test_all_python_repo_importers_disable_bytecode_before_import(self):
        # Walking this superset also works in git-less exports and covers untracked entrypoints.
        paths = sorted(p for directory in ('scripts', 'plants', '.claude/hooks')
                       for p in (ROOT / directory).rglob('*.py'))
        self.assertTrue(paths)
        local_modules = {'scripts', 'plants', *(p.stem for p in paths)}
        for path in paths:
            with self.subTest(path=str(path.relative_to(ROOT))):
                require_bytecode_guard(path.read_text(), local_modules)

    def test_bytecode_guard_removal_and_import_classes_are_caught(self):
        guard = 'sys.dont_write_bytecode = True\n'
        source = (ROOT / 'scripts/tests/mutation_budget.py').read_text()
        self.assertIn(guard, source)
        with self.assertRaisesRegex(ValueError, 'PY_BYTECODE_GUARD'):
            require_bytecode_guard(source.replace(guard, '', 1), {'scripts', 'plants'})
        for action in ('importlib.util.spec_from_file_location("x", "x.py")\n',
                       'load("x", "x.py")\n', 'sys.path.insert(0, "scripts")\n',
                       'import scripts.mutation_ci\n', 'from plants import run\n',
                       'from mutation_ci import matrix\n', 'from sys import path as search_path\n',
                       'getattr(il.util, "spec_from_file_location")("x", "x.py")\n',
                       'from runpy import run_module as run\nrun("scripts.mutation_ci")\n'):
            prefix = 'import sys\nimport importlib as il\nfrom importlib.util import spec_from_file_location as load\n'
            require_bytecode_guard(prefix + guard + action, {'scripts', 'plants', 'mutation_ci'})
            for bad in (prefix + action, prefix + action + guard,
                        prefix + 'if False:\n    ' + guard + action,
                        prefix + guard + 'sys.dont_write_bytecode = False\n' + action):
                with self.subTest(action=action, source=bad), self.assertRaisesRegex(ValueError, 'PY_BYTECODE_GUARD'):
                    require_bytecode_guard(bad, {'scripts', 'plants', 'mutation_ci'})

    def test_mutation_budget_subprocess_leaves_no_checkout_artifacts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ('scripts/tests/mutation_budget.py', 'scripts/mutation_ci.py'):
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / name, path)
            before = sorted(str(p.relative_to(root)) for p in root.rglob('*'))
            result = subprocess.run([sys.executable, '-I', str(root / 'scripts/tests/mutation_budget.py')],
                                    cwd=root, capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(sorted(str(p.relative_to(root)) for p in root.rglob('*')), before)

    def test_dirty_subject_refusal_names_paths_before_plant_preflight(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ('scripts/check_plants.py', 'scripts/plant_tools.py', 'plants/run.py'):
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / name, path)
            # Deliberately unusable catalog: dirt must be diagnosed before preflight.
            (root / 'plants/catalog.json').write_text('not JSON')
            def git(*args):
                return subprocess.run(['git', '-c', 'core.hooksPath=/dev/null',
                                       '-c', 'user.name=fixture', '-c', 'user.email=fixture@invalid',
                                       *args], cwd=root, capture_output=True, check=True)
            git('init', '-q')
            git('add', '.')
            git('commit', '-qm', 'fixture')
            (root / 'untracked file').write_text('new')
            (root / 'staged file').write_text('new')
            git('add', 'staged file')
            (root / 'plants/run.py').write_text((root / 'plants/run.py').read_text() + '\n# dirty\n')
            result = subprocess.run([sys.executable, '-I', str(root / 'scripts/check_plants.py')],
                                    cwd=root, capture_output=True, text=True, check=False)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('CI_DIRTY_TREE', result.stderr)
            for name in ('untracked file', 'staged file', 'plants/run.py'):
                self.assertIn(name, result.stderr)
            self.assertFalse((root / 'target').exists())

    def test_clean_tree_entrypoint_cannot_precede_commands_or_repeat(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.copy_ci_fixture(root)
            path = root / '.github/workflows/ci.yml'
            original = path.read_text()
            entry = '      - run: python3 -I scripts/check_plants.py\n'
            moved = original.replace(entry, '', 1).replace(
                '      - run: cargo doc --workspace --no-deps --locked\n',
                entry + '      - run: cargo doc --workspace --no-deps --locked\n', 1)
            for text in (moved, original.replace(entry, entry * 2, 1),
                         original.replace(entry, entry + '      - run: touch leftover\n', 1)):
                path.write_text(text)
                with self.assertRaisesRegex(ValueError, 'clean-tree'):
                    CAPS.check_ci(root)

    def copy_ci_fixture(self, root):
        for file in ("plants/catalog.json", "plants/run.py", "scripts/tool-versions.toml", "scripts/mutation_ci.py",
                     "scripts/plant_tools.py", "scripts/script_commands.py", "rust-toolchain.toml", "Cargo.toml",
                     ".github/workflows/ci.yml", ".github/workflows/mutation-baseline.yml"):
            (root / file).parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / file, root / file)
        # Normalize only job-level timeouts in the owned block-style schema.
        # This keeps negative plants isolated even before source workflows change.
        for workflow, jobs in TIMEOUTS.items():
            path = root / ".github/workflows" / workflow
            text = path.read_text()
            for job, minutes in jobs.items():
                pattern = (r"^  " + re.escape(job)
                           + r":\n(.*?)(?=^  [A-Za-z_][\w-]*:\n|\Z)")
                match = re.search(pattern, text, re.M | re.S)
                self.assertIsNotNone(match, (workflow, job))
                body = re.sub(r"^    timeout-minutes:[^\n]*\n", "",
                              match[1], flags=re.M)
                text = (text[:match.start(1)] + f"    timeout-minutes: {minutes}\n"
                        + body + text[match.end(1):])
            path.write_text(text)

    def test_reviewed_job_timeouts_are_accepted(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.copy_ci_fixture(root)
            CAPS.check_ci(root)

    def test_every_job_timeout_is_required_literal_unique_and_reviewed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.copy_ci_fixture(root)
            for workflow, jobs in TIMEOUTS.items():
                path = root / ".github/workflows" / workflow
                original = path.read_text()
                for job, minutes in jobs.items():
                    header = f"  {job}:\n"
                    timeout = f"    timeout-minutes: {minutes}\n"
                    anchor = header + timeout
                    self.assertEqual(original.count(anchor), 1)
                    cases = {
                        "missing": "",
                        "zero": "    timeout-minutes: 0\n",
                        "huge": "    timeout-minutes: 999999\n",
                        "expression": "    timeout-minutes: ${{ 20 }}\n",
                        "duplicate": timeout + timeout,
                        "duplicate-conflicting": timeout + "    timeout-minutes: 0\n",
                        "unreviewed-positive": f"    timeout-minutes: {minutes + 1 if isinstance(minutes, int) else 111}\n",
                    }
                    for case, replacement in cases.items():
                        with self.subTest(workflow=workflow, job=job, case=case):
                            path.write_text(original)
                            CAPS.check_ci(root)  # Positive control for every plant.
                            path.write_text(original.replace(anchor, header + replacement, 1))
                            try:
                                with self.assertRaises(ValueError):
                                    CAPS.check_ci(root)
                            finally:
                                path.write_text(original)

    def test_timeout_text_in_multiline_quoted_names_is_not_a_field(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.copy_ci_fixture(root)
            for workflow, jobs in TIMEOUTS.items():
                path = root / ".github/workflows" / workflow
                original = path.read_text()
                for job, minutes in jobs.items():
                    pattern = rf"(^  {re.escape(job)}:\n)(.*?)(?=^  [a-z][a-z0-9-]*:\n|\Z)"
                    match = re.search(pattern, original, re.M | re.S)
                    self.assertIsNotNone(match)
                    for quote, separation in ((q, sep) for q in ("'", '"') for sep in (" ", "  ", " \t")):
                        with self.subTest(workflow=workflow, job=job, quote=quote, separation=separation):
                            body = match[2].replace(f"    timeout-minutes: {minutes}\n", "")
                            body = re.sub(r"^    name: [^\n]*\n",
                                f"    name:{separation}{quote}label\n    timeout-minutes: {minutes}\n      tail{quote}\n",
                                body, count=1, flags=re.M)
                            path.write_text(original[:match.start(2)] + body + original[match.end(2):])
                            try:
                                with self.assertRaisesRegex(ValueError, "quoted scalar|separation whitespace"):
                                    CAPS.check_ci(root)
                            finally:
                                path.write_text(original)

    def test_shard_and_aggregate_weakening_is_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            self.copy_ci_fixture(root)
            for workflow in ("ci.yml", "mutation-baseline.yml"):
                path=root / ".github/workflows" / workflow
                original=path.read_text()
                plants=[("fail-fast: false","fail-fast: true"),
                        ("needs: [mutation-plan, mutation-shards]","needs: [mutation-plan]"),
                        ("if: always()","if: success()"),
                        ("${{ matrix.minutes }}","${{ 110 }}"),
                        ("        run: python3 -I scripts/mutation_ci.py aggregate", "        run: true"),
                        ("          SHARD_COUNT: ${{ matrix.shards }}", "          SHARD_COUNT: 16"),
                        ("      - name: Verify inventory", "      - if: false\n        name: Verify inventory")]
                for old,new in plants:
                    self.assertIn(old,original)
                    with self.subTest(workflow=workflow,plant=old):
                        path.write_text(original.replace(old,new))
                        with self.assertRaises(ValueError): CAPS.check_ci(root)
                        path.write_text(original)
            path=root / ".github/workflows/ci.yml"
            original=path.read_text()
            path.write_text(original.replace("  gate:\n", "  gate:\n    strategy:\n      fail-fast: false\n",1))
            with self.assertRaises(ValueError): CAPS.check_ci(root)
            path.write_text(original)
            driver=root / "scripts/mutation_ci.py"
            driver.write_text(driver.read_text().replace("MAX_PER_SHARD = 20","MAX_PER_SHARD = 200"))
            with self.assertRaises(ValueError): CAPS.check_ci(root)

    def test_version_copies_match_canonical_inputs(self):
        TOOLS.check(ROOT)

    def test_each_tool_or_compiler_drift_is_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.copy_ci_fixture(root)
            workflow = root / ".github/workflows/ci.yml"
            original = workflow.read_text()
            versions = TOOLS.versions(ROOT)
            pins = [name.replace("_", "-") + "@" + versions[name]
                    for name in ("zizmor", "cargo_mutants", "cargo_deny")]
            for pin in [*pins, versions["rust"], versions["ddf_rev"]]:
                with self.subTest(pin=pin):
                    self.assertIn(pin, original)
                    workflow.write_text(original.replace(pin, "9.9.9"))
                    with self.assertRaises(ValueError):
                        TOOLS.check(root)
            workflow.write_text(original)
            baseline = root / ".github/workflows/mutation-baseline.yml"
            baseline_original = baseline.read_text()
            for pin in ("cargo-mutants@" + versions["cargo_mutants"], versions["rust"]):
                with self.subTest(baseline_pin=pin):
                    baseline.write_text(baseline_original.replace(pin, "9.9.9"))
                    with self.assertRaises(ValueError):
                        TOOLS.check(root)
            baseline.write_text(baseline_original)

    def test_catalog_tools_are_provisioned_in_gate(self):
        CAPS.check_ci(ROOT)

    def test_gate_tool_install_cannot_be_removed_moved_or_revised(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.copy_ci_fixture(root)
            path = root / ".github/workflows/ci.yml"
            original = path.read_text()
            steps = CAPS.workflow_jobs(original)['gate']['steps']
            node = yaml.compose(original, Loader=yaml.BaseLoader)
            for field in ('jobs', 'gate', 'steps'):
                node = next(value for key, value in node.value if key.value == field)
            start = node.start_mark.index - node.start_mark.column
            end = node.end_mark.index - node.end_mark.column
            versions = TOOLS.versions(ROOT)
            action = 'taiki-e/install-action@' + versions['install_action']
            plant = {'run': 'python3 -I scripts/check_plants.py'}
            compiler = next(step for step in steps if '--component rustfmt,clippy' in step.get('run', ''))

            cases = json.loads((root / 'plants/catalog.json').read_text())['cases']
            plant_run = 'python3 -I "$GITHUB_WORKSPACE/plants/run.py" --source .. --revision "$GITHUB_SHA" --output "$RUNNER_TEMP/plants"'

            def consumer_path(step):
                return CAPS.just_consumer(step, vars(COMMANDS), cases)

            def step_yaml(step):
                if isinstance(step, str):
                    return step
                if '\n' in step.get('run', ''):
                    return ('- working-directory: ' + step['working-directory'] + '\n  run: |\n'
                            + textwrap.indent(step['run'], '    '))
                return yaml.safe_dump([step], sort_keys=False, width=1000)

            def write_steps(blocks):
                body = ''.join(step_yaml(block) for block in blocks)
                path.write_text(original[:start] + textwrap.indent(body, ' ' * node.start_mark.column) + original[end:])

            for name in ('just', 'cargo-mutants'):
                tool = name + '@' + versions[name.replace('-', '_')]
                install = {'uses': action, 'with': {'tool': tool}}
                index = steps.index(install)
                before, after = steps[:index], steps[index + 1:]
                quoted = f"- uses: '{action}'\n  with:\n    tool: \"{tool}\"\n"
                write_steps([*before, quoted, *after])
                TOOLS.check(root)
                changes = {
                    'missing': [],
                    'action': [dict(install, uses='another/install@' + versions['install_action'])],
                    'revision': [dict(install, uses='taiki-e/install-action@' + '0' * 40)],
                    'tool': [dict(install, **{'with': {'tool': name + '@0.0.0'}})],
                    'conditional': [dict(install, **{'if': 'false'})],
                    'ignored failure': [dict(install, **{'continue-on-error': True})],
                    'environment': [dict(install, env={'PATH': '/missing'})],
                    'timeout': [dict(install, **{'timeout-minutes': 1})],
                }
                for change, replacement in changes.items():
                    with self.subTest(tool=name, change=change):
                        write_steps([*before, *replacement, *after])
                        with self.assertRaises(ValueError):
                            CAPS.check_ci(root)
                remaining = [*before, *after]
                last = remaining.index(plant)
                first = last
                if name == 'just':
                    first = next(i for i, step in enumerate(remaining) if consumer_path(step))
                write_steps([*remaining[:first], install, *remaining[first:]])
                CAPS.check_ci(root)
                for position in range(first + 1, last + 2):
                    with self.subTest(tool=name, after=remaining[position - 1]):
                        write_steps([*remaining[:position], install, *remaining[position:]])
                        with self.assertRaises(ValueError):
                            CAPS.check_ci(root)
                if name == 'just':
                    for directory, run, expected in (
                        ('scripts/tests', 'just --version', 'just'),
                        ('.', 'env -C scripts/tests bash dogfood.sh', 'scripts/tests/dogfood.sh'),
                        ('scripts', 'env --chdir=tests bash dogfood.sh', 'scripts/tests/dogfood.sh'),
                        ('.', 'env -Cscripts env -Ctests python3 -I managed_review_bundle.py',
                         'scripts/tests/managed_review_bundle.py'),
                        ('scripts/tests', 'env -iC ../../.claude/hooks python3 test_guard.py Guard',
                         '.claude/hooks/test_guard.py'),
                        ('scripts', 'env --chdir tests bash -c "bash dogfood.sh"', 'scripts/tests/dogfood.sh'),
                        ('scripts', "bash '${{ github.workspace }}/scripts/tests/dogfood.sh'", 'scripts/tests/dogfood.sh'),
                        ('${{ github.workspace }}/scripts', 'env -C tests python3 managed_review_bundle.py',
                         'scripts/tests/managed_review_bundle.py'),
                        ('scripts', 'env -C "${{ github.workspace }}/.claude/hooks" python3 guard.py stop',
                         '.claude/hooks/guard.py'),
                        ('.', 'env -C plants python3 run.py --case candidate-judge-build', 'plants/run.py'),
                        ('.', 'env -C scripts/tests python3 -c "import subprocess; subprocess.run([\'bash\', \'dogfood.sh\'])"',
                         'scripts/tests/dogfood.sh'),
                        ('scripts', 'python3 -c "import subprocess; subprocess.run([\'bash\', \'dogfood.sh\'], cwd=\'tests\')"',
                         'scripts/tests/dogfood.sh'),
                        ('.', 'env -C scripts/tests python3 - <<PY\nimport subprocess\nsubprocess.run(["bash", "dogfood.sh"])\nPY\n',
                         'scripts/tests/dogfood.sh'),
                        ('scripts', 'python3 -I tests/managed_review_bundle.py ReviewBundle.test_review_publishes_only_completed_outputs',
                         'scripts/tests/managed_review_bundle.py'),
                        ('scripts', 'python3 -I ../.claude/hooks/test_guard.py Guard.test_shell_tokenization_and_rust_writes'
                                    ' Guard.test_stop_runs_gate_for_hidden_and_untracked_changes', '.claude/hooks/test_guard.py'),
                        ('scripts/tests', 'bash dogfood.sh', 'scripts/tests/dogfood.sh'),
                        ('scripts', 'bash ./tests/../tests/dogfood.sh', 'scripts/tests/dogfood.sh'),
                        ('scripts/tests', 'python3 -I ../../.claude/hooks/test_guard.py', '.claude/hooks/test_guard.py'),
                        ('scripts/tests', 'python3 -I ./managed_review_bundle.py', 'scripts/tests/managed_review_bundle.py'),
                        ('scripts/..', 'python3 -I ./scripts/checkout_just.py fast', 'scripts/checkout_just.py'),
                        ('scripts/tests', 'env bash ../ci_fast.sh', 'scripts/ci_fast.sh'),
                        ('scripts', './ci_local.sh', 'scripts/ci_local.sh'),
                        ('.', './.githooks/pre-commit', '.githooks/pre-commit'),
                        ('.', 'bash .githooks/pre-push', '.githooks/pre-push'),
                        ('.claude/hooks', 'python3 guard.py stop', '.claude/hooks/guard.py'),
                        ('plants', plant_run, 'plants/run.py'),
                        ('plants', plant_run + ' --case candidate-judge-build', 'plants/run.py'),
                        ('plants', plant_run + ' --case=core-read --case candidate-attributes-honoured', 'plants/run.py'),
                        ('plants', plant_run + ' --case "$CASE_ID"', 'plants/run.py'),
                        ('scripts', 'bash "$GITHUB_WORKSPACE/scripts/tests/dogfood.sh"', 'scripts/tests/dogfood.sh'),
                        ('scripts', 'python3 -I "${GITHUB_WORKSPACE}/.claude/hooks/test_guard.py"', '.claude/hooks/test_guard.py'),
                        ('scripts', 'python3 -I $GITHUB_WORKSPACE/scripts/tests/managed_review_bundle.py',
                         'scripts/tests/managed_review_bundle.py'),
                        ('scripts', 'python3 -I -- "${GITHUB_WORKSPACE}"/scripts/checkout_just.py fast',
                         'scripts/checkout_just.py'),
                        ('scripts', 'env bash -- "${GITHUB_WORKSPACE}/scripts/ci_fast.sh"', 'scripts/ci_fast.sh'),
                        ('scripts', '"$GITHUB_WORKSPACE/scripts/ci_local.sh"', 'scripts/ci_local.sh'),
                        ('scripts', 'python3 -I "$GITHUB_WORKSPACE/.claude/hooks/guard.py" stop', '.claude/hooks/guard.py'),
                        ('scripts', 'python3 ../.claude/hooks/guard.py pre && python3 ../.claude/hooks/guard.py stop',
                         '.claude/hooks/guard.py'),
                    ):
                        with self.subTest(directory=directory, first_consumer=run):
                            consumer = {'working-directory': directory, 'run': run}
                            later = [step for step in after if consumer_path(step) != expected]
                            write_steps([*before, install, consumer, *later])
                            CAPS.check_ci(root)
                            write_steps([*before, consumer, install, *later])
                            with self.assertRaises(ValueError) as refused:
                                CAPS.check_ci(root)
                            self.assertEqual(str(refused.exception), 'gate must install Just before running ' + expected)
                    for case_id in ('hook-shell-spelling', 'hook-malformed-open', 'hook-unsupported-tool-open',
                                    'candidate-review-policy'):
                        case = next(case for case in cases if case['id'] == case_id)
                        with self.subTest(catalog_selection=case_id):
                            suite = {'run': shlex.join(case['command'][:-1])}
                            selected = {'run': shlex.join(case['command'])}
                            write_steps([*before, selected, install, *after])
                            CAPS.check_ci(root)
                            write_steps([*before, suite, install, *after])
                            with self.assertRaisesRegex(ValueError, 'install Just before'):
                                CAPS.check_ci(root)
                    for run in ('env -C "$UNKNOWN_DIR" bash dogfood.sh', 'env --chdir=../../.. python3 run.py',
                                'bash "$UNKNOWN_SCRIPT"'):
                        with self.subTest(unresolved_location=run):
                            write_steps([*before, {'run': run}, install, *after])
                            with self.assertRaisesRegex(ValueError, 'cannot resolve|outside the workspace'):
                                CAPS.check_ci(root)
                    for run in ('bash dogfood.sh', 'python3 -I ../.claude/hooks/guard.py pre',
                                'python3 -I "${GITHUB_WORKSPACE}/.claude/hooks/guard.py" post',
                                'env -C tests true && bash dogfood.sh',
                                'python3 -c "print(\'$GITHUB_WORKSPACE\')"',
                                'python3 -c"print(\'${GITHUB_WORKSPACE}\')"',
                                'python3 - <<< "print(\'$GITHUB_WORKSPACE\')"',
                                "python3 - <<PY\nprint('${{ github.workspace }}', '$GITHUB_WORKSPACE')\nPY\n",
                                "python3 - <<'PY'\nprint('$GITHUB_WORKSPACE')\nPY\n",
                                "bash '$GITHUB_WORKSPACE/scripts/tests/dogfood.sh'",
                                plant_run + ' --case core-read', plant_run + ' --case=core-read --case unchecked-add',
                                plant_run + ' --case=core-read --output="$RUNNER_TEMP/other"',
                                plant_run + ' --runner ./cargo-backend --case core-read',
                                plant_run + ' --help'):
                        with self.subTest(independent=run):
                            write_steps([*before, {'working-directory': 'scripts', 'run': run}, install, *after])
                            CAPS.check_ci(root)
                    independent = remaining[:first]
                    write_steps([*independent, install, plant, *remaining[last + 1:]])
                    CAPS.check_ci(root)
                    write_steps([*independent, plant, install, *remaining[last + 1:]])
                    with self.assertRaises(ValueError):
                        CAPS.check_ci(root)
            for target in (compiler, plant):
                write_steps([dict(step, **{'if': 'false'}) if step == target else step for step in steps])
                with self.assertRaises(ValueError):
                    CAPS.check_ci(root)
            path.write_text(original.replace('  gate:\n', '  gate:\n    if: false\n', 1))
            with self.assertRaises(ValueError):
                CAPS.check_ci(root)

    def test_catalog_tool_declarations_are_closed_and_match_direct_commands(self):
        for case in ({'id': 'missing'}, {'id': 'empty', 'required_tools': []},
                     {'id': 'unknown', 'required_tools': ['unknown']},
                     {'id': 'duplicate', 'required_tools': ['cargo','cargo']},
                     {'id': 'wrong', 'cargo': ['clippy'], 'required_tools': ['cargo']},
                     {'id': 'command', 'command': ['bash','x.sh'], 'required_tools': ['python3']}):
            with self.subTest(case=case), self.assertRaises(ValueError):
                CAPS.requirements([case])

    def test_missing_mutants_is_a_named_preflight_refusal(self):
        with tempfile.TemporaryDirectory() as directory:
            tool = Path(directory) / "cargo"
            tool.write_text("#!/bin/sh\necho 'no such command: mutants' >&2\nexit 101\n")
            tool.chmod(0o700)
            with self.assertRaisesRegex(ValueError, "plant preflight: unavailable tool cargo-mutants"):
                CAPS.preflight({'cargo-mutants'}, ROOT, {'PATH': directory})

    def test_g35_missing_just_stops_before_baseline_or_mutation(self):
        catalog = json.loads((ROOT / 'plants/catalog.json').read_text())
        cases = [case for case in catalog['cases'] if case['guardrail'] == 'G-35'
                 and case.get('command') == ['bash', 'scripts/tests/dogfood.sh']]
        self.assertTrue(cases)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'scripts').symlink_to(ROOT / 'scripts')
            (root / 'plants').mkdir()
            (root / 'plants/run.py').symlink_to(ROOT / 'plants/run.py')
            tools = root / 'bin'
            tools.mkdir()
            for name in ('bash', 'cargo', 'env', 'python3', 'rustc', 'rustup', 'sh'):
                (tools / name).symlink_to('/bin/true')
            (tools / 'git').symlink_to(shutil.which('git'))
            env = {key: value for key, value in os.environ.items() if not key.startswith('GIT_')}
            env.update(GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null')
            subprocess.run(['git', 'init', '-q', '--template=', str(root)], env=env, check=True)
            for case in cases:
                with self.subTest(plant=case['id']):
                    (root / 'plants/catalog.json').write_text(json.dumps({'cases': [case]}))
                    subprocess.run(['git', 'add', '.'], cwd=root, env=env, check=True)
                    subprocess.run(['git', '-c', 'user.name=fixture',
                                    '-c', 'user.email=fixture@invalid', 'commit', '-qm', case['id']],
                                   cwd=root, env=env, check=True)
                    with patch.object(PLANTS, 'ROOT', root), patch.dict(os.environ, PATH=str(tools)):
                        with self.assertRaisesRegex(ValueError, 'plant preflight: unavailable tool just$'):
                            PLANTS.run()
                    self.assertFalse((root / 'target').exists())

    def test_receipt_requires_exact_subject_complete_unique_cases_and_caught(self):
        good = {"revision": "subject", "results": [{"id": "one", "status": "CAUGHT"}, {"id": "two", "status": "CAUGHT"}], "pending_guardrails": [{"id": "G-99", "status": "PENDING"}]}
        PLANTS.validate_report(good, "subject", ["one", "two"])
        for field, value in (("revision", "other"), ("results", []),
                             ("results", [good["results"][0]] * 2),
                             ("results", [{"id": "one", "status": "ESCAPED"}, good["results"][1]])):
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                PLANTS.validate_report(dict(good, **{field: value}), "subject", ["one", "two"])

    def test_merge_queue_jobs_bind_both_subjects(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        for job, end in (("mutation-plan", "dogfood"), ("dogfood", "vocabulary")):
            body = workflow.split("\n  " + job + ":\n", 1)[1].split("\n  " + end + ":\n", 1)[0]
            self.assertIn("if: github.event_name == 'pull_request' || github.event_name == 'merge_group'", body)
            self.assertIn("github.event.pull_request.base.sha || github.event.merge_group.base_sha", body)
            if job == "dogfood":
                self.assertIn("github.event.pull_request.head.sha || github.event.merge_group.head_sha", body)

    def test_required_perimeter_is_owned(self):
        paths = set((ROOT / "protected-paths.txt").read_text().splitlines())
        self.assertTrue({".cargo/**", "crates/*/tests/**", "fixtures/**", "plants/**"} <= paths)



class GateCapacity(unittest.TestCase):
    """CI-GATE-BUDGET (ledger Q49): the gate budget was derived for a bounded catalogue."""

    def test_the_catalogue_fits_the_budget_and_one_case_too_many_is_refused(self):
        CAPS.check_gate_capacity(ROOT)
        capacity = CAPS.GATE_PLANT_CAPACITY
        # The documented rule, restated here on purpose: seconds = 525 + 8.7 per plant, 1.5x headroom.
        self.assertLessEqual(1.5 * (525 + 8.7 * capacity), TIMEOUTS["ci.yml"]["gate"] * 60)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "plants").mkdir()
            for count, refused in ((capacity, False), (capacity + 1, True)):
                (root / "plants/catalog.json").write_text(json.dumps({"cases": [{}] * count}))
                if refused:
                    with self.assertRaisesRegex(ValueError, "re-derive it"):
                        CAPS.check_gate_capacity(root)
                else:
                    CAPS.check_gate_capacity(root)



class GateImageCommands(unittest.TestCase):
    """Gap G9 (Mission 7): a script that calls a command the offline gate image lacks is refused.

    Train 14's first gate went red on `cmp` in scripts/tests/dogfood.sh; every lane's toolbox
    has it, the gate image does not. Each case below hides an undeclared command in one shell
    or Python shape the reader must see, and must be named once, on its line."""

    def problems(self, files):
        with tempfile.TemporaryDirectory(prefix="script-commands-") as directory:
            root = Path(directory)
            for name, text in files.items():
                (root / name).parent.mkdir(parents=True, exist_ok=True)
                (root / name).write_text(text)
            env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "GIT_CONFIG_NOSYSTEM": "1",
                   "GIT_CONFIG_GLOBAL": "/dev/null", "HOME": directory}
            subprocess.run(["git", "init", "-q", directory], check=True, env=env, stdin=subprocess.DEVNULL)
            subprocess.run(["git", "-C", directory, "add", "-A"], check=True, env=env, stdin=subprocess.DEVNULL)
            return COMMANDS.problems(root)

    def test_every_tracked_script_calls_only_declared_commands(self):
        self.assertEqual(COMMANDS.problems(ROOT), [])

    def test_a_source_export_is_read_like_the_checkout(self):
        # Train A3's hosted-parity export phase has no .git: the check must read the files there.
        with tempfile.TemporaryDirectory(prefix="script-commands-export-") as directory:
            copy = Path(directory)
            for name in COMMANDS.tracked(ROOT):
                source, target = ROOT / name, copy / name
                target.parent.mkdir(parents=True, exist_ok=True)
                if source.is_symlink():
                    target.symlink_to(os.readlink(source))
                elif source.is_file():
                    shutil.copyfile(source, target)
            self.assertFalse((copy / ".git").exists())
            self.assertEqual(sorted(COMMANDS.tracked(copy)), sorted(COMMANDS.tracked(ROOT)))
            self.assertEqual(COMMANDS.problems(copy), COMMANDS.problems(ROOT))

    def test_an_export_inside_another_repository_is_read(self):
        # Git answers for the enclosing repository there; the export's own files must still be read.
        with tempfile.TemporaryDirectory(prefix="script-commands-outer-") as directory:
            env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "GIT_CONFIG_NOSYSTEM": "1",
                   "GIT_CONFIG_GLOBAL": "/dev/null", "HOME": directory}
            subprocess.run(["git", "init", "-q", directory], check=True, env=env, stdin=subprocess.DEVNULL)
            export = Path(directory) / "export"
            export.mkdir()
            (export / "s.sh").write_text("#!/usr/bin/env bash\ncmp a b\n")
            found = COMMANDS.problems(export)
        self.assertEqual(len(found), 1, found)
        self.assertTrue(found[0].startswith("s.sh:2: calls `cmp`"), found)

    def test_the_walk_skips_a_git_directory(self):
        # A `.git` Git does not accept (no HEAD) is walked around, not read as tracked files.
        with tempfile.TemporaryDirectory(prefix="script-commands-dotgit-") as directory:
            hooks = Path(directory) / ".git" / "hooks"
            hooks.mkdir(parents=True)
            (hooks / "x.sh").write_text("#!/usr/bin/env bash\ncmp a b\n")
            (Path(directory) / "ok.sh").write_text("#!/usr/bin/env bash\ntrue\n")
            self.assertEqual(COMMANDS.tracked(Path(directory)), ["ok.sh"])
            self.assertEqual(COMMANDS.problems(Path(directory)), [])

    def test_the_walk_skips_build_output_only_at_the_root(self):
        # Git can track `x/target/f` (Q66): only the root's build output is skipped.
        with tempfile.TemporaryDirectory(prefix="script-commands-target-") as directory:
            root = Path(directory)
            for name in ("target/a.sh", "mutants.out/b.sh", "x/target/c.sh", "x/mutants.out/d.sh",
                         "x/.git/e.sh", "ok.sh"):
                (root / name).parent.mkdir(parents=True, exist_ok=True)
                (root / name).write_text("#!/usr/bin/env bash\ntrue\n")
            self.assertEqual(COMMANDS.tracked(root), ["ok.sh", "x/mutants.out/d.sh", "x/target/c.sh"])

    def test_inherited_git_variables_do_not_choose_another_index(self):
        # A decoy GIT_DIR / GIT_INDEX_FILE / GIT_WORK_TREE in the caller's environment (Q66).
        with tempfile.TemporaryDirectory(prefix="script-commands-gitenv-") as directory:
            env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "GIT_CONFIG_NOSYSTEM": "1",
                   "GIT_CONFIG_GLOBAL": "/dev/null", "HOME": directory}
            real, decoy = Path(directory) / "real", Path(directory) / "decoy"
            for repo, name in ((real, "a.sh"), (decoy, "b.sh")):
                subprocess.run(["git", "init", "-q", str(repo)], check=True, env=env, stdin=subprocess.DEVNULL)
                (repo / name).write_text("#!/usr/bin/env bash\ntrue\n")
                subprocess.run(["git", "-C", str(repo), "add", name], check=True, env=env, stdin=subprocess.DEVNULL)
            (real / "untracked.sh").write_text("#!/usr/bin/env bash\ntrue\n")  # Git's answer, not the walk's
            decoys = {"GIT_DIR": str(decoy / ".git"), "GIT_WORK_TREE": str(decoy),
                      "GIT_INDEX_FILE": str(decoy / ".git" / "index"),
                      "GIT_CONFIG_COUNT": "1", "GIT_CONFIG_KEY_0": "core.worktree", "GIT_CONFIG_VALUE_0": str(decoy)}
            with mock.patch.dict(os.environ, decoys):
                self.assertEqual(COMMANDS.tracked(real), ["a.sh"])
            with mock.patch.dict(os.environ, {"GIT_INDEX_FILE": str(decoy / ".git" / "index")}):
                self.assertEqual(COMMANDS.tracked(real), ["a.sh"])

    def test_a_tree_without_git_is_read_not_skipped(self):
        with tempfile.TemporaryDirectory(prefix="script-commands-nogit-") as directory:
            (Path(directory) / "s.sh").write_text("#!/usr/bin/env bash\ncmp a b\n")
            found = COMMANDS.problems(Path(directory))
        self.assertEqual(len(found), 1, found)
        self.assertTrue(found[0].startswith("s.sh:2: calls `cmp`"), found)

    def test_the_declared_set_has_none_of_the_commands_the_image_lacks(self):
        absent = {"cmp", "diff", "jq", "xxd", "file", "apt-get", "zizmor", "deep-diff-forge"}
        self.assertEqual(COMMANDS.IMAGE & absent, set())

    def test_an_undeclared_command_is_named_in_each_shape(self):
        shapes = {
            "plain (train 14)": "cmp a b\n",
            "after a pipe": "printf x | jq .\n",
            "after &&": "true && xxd a\n",
            "in an if": "if file a; then :; fi\n",
            "in a substitution": 'x="$(diff a b)"\n',
            "in backquotes": "x=`diff a b`\n",
            "in a process substitution": "while read -r l; do :; done < <(diff a b)\n",
            "after env and assignments": "env -u HOME A=1 \"$b=2\" diff a b\n",
            "after an absolute env": "/usr/bin/env -i PATH=/usr/bin diff a b\n",
            "in bash -c": "bash -c 'diff \"$1\" b' _ a\n",
            "in a trap": "trap 'diff a b' EXIT\n",
            "in a function": "f() {\n  diff a b\n}\n",
            "in a case arm": "case \"$1\" in\n x) diff a b ;;\nesac\n",
            "in a stub here-document": "cat > stub <<'S'\n#!/bin/sh\ndiff a b\nS\n",
            "in a python here-document": "python3 -I - <<'P'\nimport subprocess\nsubprocess.run(['diff', 'a'])\nP\n",
            "after a line continuation": "true \\\n  && diff a b\n",
            # M7-G9 round 2 (Bastion's probe d4c5b000): commands that run their operand.
            "after xargs": "printf '%s\\n' a | xargs -n 1 cmp b\n",
            "after find -exec": "find . -name x -exec cmp {} b \\;\n",
            "after a second find -execdir": "find . -exec true \\; -execdir diff {} b +\n",
            "after tar -I": "tar -I xxd -cf x.tar a\n",
            "in python3 -c": "python3 -I -c 'import subprocess; subprocess.run([\"cmp\", \"a\"])'\n",
            "in env -S": "env -S 'cmp a b'\n",
            "after exec -a NAME": "exec -a name cmp a b\n",
            "after env -C DIR": "env -C /tmp cmp a b\n",
            "after timeout -s SIGNAL": "timeout -s KILL 5 cmp a b\n",
            "after nohup": "nohup cmp a b\n",
            "after coproc": "coproc cmp a b\n",
            "behind a function of the same name": "cmp() { command cmp \"$@\"; }\ncmp a b\n",
            "in a function keyword body": "function f {\n  diff a b\n}\nf\n",
            # Round 3 (Bastion's probe2): options and subcommands that run a program.
            "after rustup's run and its toolchain": "rustup run \"$toolchain\" cmp a b\n",
            "in tar --to-command": "tar -xf a.tar --to-command='cmp - b'\n",
            "in tar --checkpoint-action=exec": "tar -cf a.tar --checkpoint-action=exec='xxd a' a\n",
            "in sort --compress-program": "sort --compress-program=xxd a\n",
            # Round 4 (Bastion's probe3): attached, clustered and abbreviated spellings.
            "in tar -Iprogram": "tar -Ixxd -cf a.tar a\n",
            "in tar -cIprogram": "tar -cIxxd -f a.tar a\n",
            "in tar -Fprogram": "tar -Fxxd -cf a.tar a\n",
            "in tar --use-compress-prog=": "tar --use-compress-prog=xxd -cf a.tar a\n",
            "in tar --to-comm=": "tar -xf a.tar --to-comm='cmp - b'\n",
            "in tar --checkpoint-action exec=": "tar --checkpoint-action 'exec=cmp a b' -cf a.tar a\n",
            "in sort --compress=": "sort --compress=xxd a\n",
            # Q65 / probe4: here-strings, descriptor numbers, clusters on prefixes, -c with -o.
            "in a here-string to bash": "bash <<< 'cmp a b'\n",
            "in a here-string before sh": "<<< 'cmp a b' sh\n",
            "in a here-string to python3": "python3 <<< \"import os; os.system('cmp a b')\"\n",
            "after 2>/dev/null before the command": "2>/dev/null cmp a b\n",
            "after xargs -0rn 1": "printf a | xargs -0rn 1 cmp b\n",
            "after env -iu NAME": "env -iu X cmp a b\n",
            "in bash -co pipefail": "bash -co pipefail 'cmp a b'\n",
        }
        for name, text in shapes.items():
            with self.subTest(shape=name):
                found = self.problems({"s.sh": "#!/usr/bin/env bash\n" + text})
                self.assertEqual(len(found), 1, found)
                self.assertRegex(found[0], r"^s\.sh:\d+: calls `(cmp|jq|xxd|file|diff)`, which the gate image")
                line = int(found[0].split(":")[1])
                self.assertIn(found[0].split("`")[1], ("#!/usr/bin/env bash\n" + text).splitlines()[line - 1])

    def test_python_and_restricted_calls_are_named(self):
        found = self.problems({"t.py": "import subprocess\nsubprocess.run(['jq', '.'])\n"})
        self.assertEqual(found, ["t.py:2: calls `jq`, which the gate image does not declare "
                                 "(scripts/script_commands.py)"])
        found = self.problems({"scripts/x.sh": "apt-get install -y cmp\n"})
        self.assertEqual(found, ["scripts/x.sh:1: calls `apt-get`, which only "
                                 "scripts/guest_bootstrap.sh may call"])

    def test_python_calls_are_read_as_the_command_lines_they_run(self):
        calls = {
            "shell=True pipe": "subprocess.run('cat a | cmp - b', shell=True)",
            "shell=True and": "subprocess.run('true && xxd a', shell=True)",
            "shell=True f-string": "subprocess.run(f'cmp {name} b', shell=True)",
            "bash -c in a list": "subprocess.run(['bash', '-c', 'cmp a b'])",
            "env in a list": "subprocess.run(['env', 'cmp', 'a', 'b'])",
            "list plus a variable": "subprocess.run(['cmp'] + rest)",
            "os.system": "os.system('true && xxd a')",
            "os.popen": "os.popen('jq . a')",
            "getoutput": "subprocess.getoutput('cmp a b')",
            "getstatusoutput": "subprocess.getstatusoutput('xxd a')",
            "posix_spawnp": "os.posix_spawnp('cmp', ['cmp', 'a', 'b'], {})",
            "spawnlp": "os.spawnlp(os.P_WAIT, 'cmp', 'cmp', 'a', 'b')",
            "spawnvp": "os.spawnvp(os.P_WAIT, 'cmp', ['cmp', 'a'])",
            "execlp": "os.execlp('cmp', 'cmp', 'a', 'b')",
            "create_subprocess_exec": "asyncio.create_subprocess_exec('cmp', 'a', 'b')",
            "create_subprocess_shell": "asyncio.create_subprocess_shell('cmp a b')",
            "pty.spawn": "pty.spawn(['cmp', 'a', 'b'])",
            "str.split": "subprocess.run('cmp a b'.split())",
            "shlex.split": "subprocess.run(shlex.split('cmp a b'))",
            "a conditional argv": "subprocess.run(['cmp', 'a'] if flag else ['true'])",
            "shell=1": "subprocess.run('true && cmp a b', shell=1)",
            "shell=flag": "subprocess.run('true && cmp a b', shell=flag)",
        }
        for name, call in calls.items():
            with self.subTest(call=name):
                found = self.problems({"t.py": "import asyncio, os, pty, shlex, subprocess\n" + call + "\n"})
                self.assertEqual(len(found), 1, found)
                self.assertRegex(found[0], r"^t\.py:2: calls `(cmp|jq|xxd)`, which the gate image")

    def test_an_aliased_process_function_is_read(self):
        found = self.problems({"t.py": "from subprocess import run as sh\nsh(['cmp', 'a', 'b'])\n"})
        self.assertEqual(len(found), 1, found)
        self.assertTrue(found[0].startswith("t.py:2: calls `cmp`"), found)

    def test_a_file_a_call_runs_is_read_in_that_language(self):
        """Round 3 (Bastion): a tracked file run or sourced by a literal path is read in the
        language it runs in, even with no extension and no `#!` line; else it is named."""
        cases = {
            "./FILE, no #! line": ({"s.sh": "./tool\n", "tool": "cmp a b\n"},
                                   ["tool (run as shell):1: calls `cmp`"]),
            "bash FILE": ({"s.sh": "bash -o pipefail scripts/helper x\n", "scripts/helper": "cmp a b\n"},
                          ["scripts/helper (run as shell):1: calls `cmp`"]),
            "python3 FILE": ({"s.sh": "python3 -I tools/x a\n",
                              "tools/x": "import subprocess\nsubprocess.run(['cmp'])\n"},
                             ["tools/x (run as python):2: calls `cmp`"]),
            "a sourced library and its function": ({"s.sh": ". lib/common\nf\n", "lib/common": "f() { cmp a b; }\n"},
                                                   ["lib/common (run as shell):1: calls `cmp`"]),
            "a .sh file with a python #!": ({"a.sh": "#!/usr/bin/env python3\nimport subprocess\nsubprocess.run(['cmp'])\n"},
                                            ["a.sh:3: calls `cmp`"]),
            "./FILE with another interpreter": ({"s.sh": "./tool\n", "tool": "#!/usr/bin/perl\nprint 1;\n"},
                                                ["tool: not read: line 1"]),
            # Round 4 (Bastion's probe3): option groups, --, and standard input.
            "bash -euo pipefail FILE": ({"s.sh": "bash -euo pipefail scripts/helper\n", "scripts/helper": "cmp a b\n"},
                                        ["scripts/helper (run as shell):1: calls `cmp`"]),
            "bash -eO extglob FILE": ({"s.sh": "bash -eO extglob scripts/helper\n", "scripts/helper": "cmp a b\n"},
                                      ["scripts/helper (run as shell):1: calls `cmp`"]),
            "bash -- FILE": ({"s.sh": "bash -- scripts/helper\n", "scripts/helper": "cmp a b\n"},
                             ["scripts/helper (run as shell):1: calls `cmp`"]),
            "bash -- FILE named like an option": ({"s.sh": "bash -- -helper\n", "-helper": "cmp a b\n"},
                                                  ["-helper (run as shell):1: calls `cmp`"]),
            "python3 -- FILE named like an option": ({"s.sh": "python3 -- -tool\n", "-tool": "import os\nos.system('cmp a b')\n"},
                                                     ["-tool (run as python):2: calls `cmp`"]),
            ". -- FILE": ({"s.sh": ". -- scripts/helper\n", "scripts/helper": "cmp a b\n"},
                          ["scripts/helper (run as shell):1: calls `cmp`"]),
            "bash < FILE": ({"s.sh": "bash < scripts/helper\n", "scripts/helper": "cmp a b\n"},
                            ["scripts/helper (run as shell):1: calls `cmp`"]),
            "sh -s < FILE": ({"s.sh": "sh -s < scripts/helper\n", "scripts/helper": "cmp a b\n"},
                             ["scripts/helper (run as shell):1: calls `cmp`"]),
            "python3 -uW ignore FILE": ({"s.sh": "python3 -uW ignore tools/x\n",
                                         "tools/x": "import os\nos.system('cmp a b')\n"},
                                        ["tools/x (run as python):2: calls `cmp`"]),
            "python3 --check-hash-based-pycs always FILE": (
                {"s.sh": "python3 --check-hash-based-pycs always tools/x\n", "tools/x": "import os\nos.system('cmp a b')\n"},
                ["tools/x (run as python):2: calls `cmp`"]),
            "python3 - < FILE": ({"s.sh": "python3 - < tools/x\n", "tools/x": "import os\nos.system('cmp a b')\n"},
                                 ["tools/x (run as python):2: calls `cmp`"]),
            "a run of a file ONLY_IN lets call more": (
                {"s.sh": "bash scripts/guest_bootstrap.sh\n", "scripts/guest_bootstrap.sh": "apt-get install -y x\n"},
                ["s.sh:1: runs `scripts/guest_bootstrap.sh`, which alone may call apt-get, cc, sudo"]),
            "bash -eoO pipefail extglob FILE": ({"s.sh": "bash -eoO pipefail extglob scripts/helper\n", "scripts/helper": "cmp a b\n"},
                                                ["scripts/helper (run as shell):1: calls `cmp`"]),
            "bash -o pipefail FILE, a clean file": ({"s.sh": "bash -o pipefail scripts/ok\n", "scripts/ok": "true\n"}, []),
            # Q65: standard input redirected before the command word.
            "< FILE bash": ({"s.sh": "< scripts/helper bash\n", "scripts/helper": "cmp a b\n"},
                            ["scripts/helper (run as shell):1: calls `cmp`"]),
            "<FILE sh": ({"s.sh": "<scripts/helper sh\n", "scripts/helper": "cmp a b\n"},
                         ["scripts/helper (run as shell):1: calls `cmp`"]),
            "shell=True text with no newline, < FILE": ({"t.py": "import subprocess\nsubprocess.run('bash < scripts/helper', shell=True)\n",
                                                          "scripts/helper": "cmp a b\n"},
                                                         ["scripts/helper (run as shell):1: calls `cmp`"]),
            "0< FILE bash": ({"s.sh": "0< scripts/helper bash\n", "scripts/helper": "cmp a b\n"},
                             ["scripts/helper (run as shell):1: calls `cmp`"]),
            "bash 0< FILE": ({"s.sh": "bash 0< scripts/helper\n", "scripts/helper": "cmp a b\n"},
                             ["scripts/helper (run as shell):1: calls `cmp`"]),
            "< FILE env python3": ({"s.sh": "< tools/x env python3\n", "tools/x": "import os\nos.system('cmp a b')\n"},
                                   ["tools/x (run as python):2: calls `cmp`"]),
            # Bastion CHANGES on 2df4ed6f: `--` with no file after it, and `<>` on standard input.
            "bash -- < FILE": ({"s.sh": "bash -- < scripts/helper\n", "scripts/helper": "cmp a b\n"},
                               ["scripts/helper (run as shell):1: calls `cmp`"]),
            "python3 -- < FILE": ({"s.sh": "python3 -- < tools/x\n", "tools/x": "import os\nos.system('cmp a b')\n"},
                                  ["tools/x (run as python):2: calls `cmp`"]),
            "bash <>FILE": ({"s.sh": "bash <>scripts/helper\n", "scripts/helper": "cmp a b\n"},
                            ["scripts/helper (run as shell):1: calls `cmp`"]),
            "bash 0<> FILE": ({"s.sh": "bash 0<> scripts/helper\n", "scripts/helper": "cmp a b\n"},
                              ["scripts/helper (run as shell):1: calls `cmp`"]),
            "bash -- FILE < DATA": ({"s.sh": "bash -- scripts/ok < data.txt\n", "scripts/ok": "true\n",
                                     "data.txt": "cmp a b\n"}, []),
            "bash 2< DATA, no script file": ({"s.sh": "bash 2< data.txt\n", "data.txt": "cmp a b\n"}, []),
            "bash 1<> DATA": ({"s.sh": "bash scripts/ok 1<> data.txt\n", "scripts/ok": "true\n", "data.txt": "cmp a b\n"}, []),
            # Standard input is data when the program got a script: not read.
            "python3 FILE < DATA": ({"s.sh": "python3 tools/ok.py < data.json\n", "tools/ok.py": "x = 1\n",
                                     "data.json": "{not python\n"}, []),
            "< DATA python3 FILE": ({"s.sh": "< data.json python3 tools/ok.py\n", "tools/ok.py": "x = 1\n",
                                     "data.json": "{not python\n"}, []),
            "bash -c TEXT < DATA": ({"s.sh": "bash -c 'true' < data.txt\n", "data.txt": "cmp a b\n"}, []),
            "2< DATA bash FILE": ({"s.sh": "bash scripts/ok 2< data.txt\n", "scripts/ok": "true\n", "data.txt": "cmp a b\n"}, []),
        }
        for name, (files, expected) in cases.items():
            with self.subTest(case=name):
                found = self.problems(files)
                self.assertEqual(len(found), len(expected), found)
                for line, prefix in zip(found, expected):
                    self.assertTrue(line.startswith(prefix), (line, prefix))

    def test_the_routing_pin_allows_one_dynamic_call(self):
        name = "scripts/tests/runner_routing.py"
        one = "import subprocess\nsubprocess.run(['bash', '-c', text])\n"
        self.assertEqual(self.problems({name: one}), [])
        found = self.problems({name: one + "subprocess.run(['bash', '-c', other])\n"})
        self.assertEqual(len(found), 1, found)
        self.assertTrue(found[0].startswith(name + ":3: calls `\"$python_expression\"`"), found)

    def test_a_script_is_known_by_its_first_line(self):
        cases = {
            "env -S bash, no extension": ("#!/usr/bin/env -S bash -eu\ncmp a b\n", ["tool:2: calls `cmp`"]),
            "dash, which the image lacks": ("#!/bin/dash\ntrue\n", ["tool:1: calls `dash`"]),
            "python, no extension": ("#!/usr/bin/env python3\nimport subprocess\nsubprocess.run(['cmp'])\n",
                                     ["tool:3: calls `cmp`"]),
            "another interpreter": ("#!/usr/bin/perl\nprint 1;\n", ["tool: not read: line 1"]),
            "a Rust inner attribute": ("#![forbid(unsafe_code)]\n", []),
        }
        for name, (text, expected) in cases.items():
            with self.subTest(case=name):
                found = self.problems({"tool": text})
                self.assertEqual(len(found), len(expected), found)
                for line, prefix in zip(found, expected):
                    self.assertTrue(line.startswith(prefix), (line, prefix))

    def test_what_the_reader_cannot_resolve_is_refused_not_skipped(self):
        cases = {
            "eval": ("eval \"$cmd\"\n", "not read: line 1: eval"),
            "unterminated quote": ("echo 'x\n", "not read: line 1: unterminated single quote"),
            "unterminated here-document": ("cat <<'E'\nx\n", "not terminated"),
            "unpinned dynamic word": ('"$tool" --version\n', 'calls `"$tool"`'),
            "dynamic bash -c text": ('bash -c "$script"\n', 'calls `"$script"`'),
            "dynamic python3 -c text": ('python3 -c "$code"\n', 'calls `"$code"`'),
            "aliases": ("shopt -s expand_aliases\n", "not read: line 1: `shopt expand_aliases`"),
            "a path out of the repository": ("../tool.sh\n", "calls `../tool.sh`"),
            "a declared name outside the system directories": ("/opt/x/bin/git status\n", "calls `/opt/x/bin/git`"),
        }
        for name, (text, expected) in cases.items():
            with self.subTest(case=name):
                found = self.problems({"s.sh": text})
                self.assertEqual(len(found), 1, found)
                self.assertIn(expected, found[0])

    def test_words_that_are_not_commands_are_not_named(self):
        text = (
            "# cmp in a comment\n"
            "echo 'diff' \"jq\" diff\n"
            "command -v cmp >/dev/null || echo none\n"
            "for tool in cmp diff; do echo \"$tool\"; done\n"
            "case \"$x\" in\n cmp|diff) echo match ;;\n *) : ;;\nesac\n"
            "files=(cmp diff\n  jq)\n"
            "cat > notes <<'D'\ncmp and diff are data here\nD\n"
            "[[ $x == diff && $y == cmp ]]\n"
            "grep -q cmp a 2>/dev/null\n"
            "scripts/tool.sh run\n"
            "./.githooks/x\n"
            "find . -name cmp -print\n"
            "tar -cf diff.tar a\n"
            "python3 tool.py -c diff\n"
            "nohup true\n"
            "/usr/bin/git status\n"
            "coproc worker { true; }\n"
            "function cmp { :; }\ncmp a b\n"
        )
        self.assertEqual(self.problems({"s.sh": text, "scripts/tool.sh": "true\n", ".githooks/x": "true\n"}), [])


if __name__ == "__main__":
    unittest.main()
