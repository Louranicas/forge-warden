#!/usr/bin/env python3
"""Owned gate contracts: exact tool inputs, complete plant receipts and merge subjects."""
import hashlib
import importlib.util
from contextlib import redirect_stderr, redirect_stdout
from io import StringIO
import json
import os
from pathlib import Path
import re
import runpy
import shlex
import shutil
import subprocess
import sys
sys.dont_write_bytecode = True
import tempfile
import textwrap
from types import ModuleType
import unittest
from unittest import mock
from unittest.mock import patch

import yaml

ROOT = Path(__file__).resolve().parents[2]


def load(name, root=ROOT):
    spec = importlib.util.spec_from_file_location(name, root / "scripts" / (name + ".py"))
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


def gate_step_fixture(path, original):
    steps = CAPS.workflow_jobs(original)['gate']['steps']
    node = yaml.compose(original, Loader=yaml.BaseLoader)
    for field in ('jobs', 'gate', 'steps'):
        node = next(value for key, value in node.value if key.value == field)
    start = node.start_mark.index - node.start_mark.column
    end = node.end_mark.index - node.end_mark.column
    offsets = [item.start_mark.index - item.start_mark.column for item in node.value] + [end]
    original_blocks = {id(step): original[left:right] for step, left, right in
                       zip(steps, offsets, offsets[1:])}

    class LiteralRun(str):
        pass

    class StepDumper(yaml.SafeDumper):
        def determine_block_hints(self, text):
            hints = super().determine_block_hints(text)
            if text.startswith('\n') and not text.lstrip('\n').startswith(' '):
                return hints.lstrip('123456789')
            return hints

    StepDumper.add_representer(LiteralRun, lambda dumper, run:
                               dumper.represent_scalar('tag:yaml.org,2002:str', run, style='|'))
    StepDumper.add_representer(str, lambda dumper, value:
                               dumper.represent_scalar('tag:yaml.org,2002:str', value,
                                                       style='"' if '\n' in value else None))

    def step_yaml(step):
        if isinstance(step, str):
            return step
        if '\n' in step.get('run', ''):
            step = dict(step, run=LiteralRun(step['run']))
        return yaml.dump([step], Dumper=StepDumper, sort_keys=False, width=float('inf')).removesuffix('...\n')

    def write_steps(blocks):
        indent = ' ' * node.start_mark.column
        body = ''.join(original_blocks[id(block)] if id(block) in original_blocks else
                       textwrap.indent(step_yaml(block), indent) for block in blocks)
        path.write_text(original[:start] + body + original[end:])

    return steps, offsets, write_steps


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
            steps, _, write_steps = gate_step_fixture(path, path.read_text())
            plant = {'run': 'python3 -I scripts/check_plants.py'}
            index = steps.index(plant)
            quoted = '- run: "python3 -I scripts/check_plants.py"\n'
            write_steps([*steps[:index], quoted, *steps[index + 1:]])
            self.assertEqual(CAPS.workflow_jobs(path.read_text())['gate']['steps'], steps)
            CAPS.check_ci(root)

            steps, _, write_steps = gate_step_fixture(path, path.read_text())
            before, entry, after = steps[:index], steps[index], steps[index + 1:]
            previous = max(i for i, step in enumerate(before) if 'run' in step)
            for change, expected in (
                ('move', [*before[:previous], entry, *before[previous:], *after]),
                ('duplicate', [*before, entry, entry, *after]),
                ('trailing command', [*before, entry, {'run': 'touch leftover'}, *after]),
            ):
                with self.subTest(change=change):
                    write_steps(expected)
                    decoded = CAPS.workflow_jobs(path.read_text())['gate']['steps']
                    self.assertNotEqual(decoded, steps)
                    self.assertEqual(decoded, expected)
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
                    for name in ("zizmor", "cargo_deny")]
            for pin in [*pins, versions["rust"], versions["ddf_rev"]]:
                with self.subTest(pin=pin):
                    self.assertIn(pin, original)
                    workflow.write_text(original.replace(pin, "9.9.9"))
                    with self.assertRaises(ValueError):
                        TOOLS.check(root)
            workflow.write_text(original)
            mutants = "cargo-mutants@" + versions["cargo_mutants"]
            workflow.write_text(original.replace(mutants, "cargo-mutants@9.9.9"))
            with self.assertRaisesRegex(ValueError, "mutation orchestration requires review"):
                TOOLS.check(root)
            workflow.write_text(original)
            drifted = dict(versions)
            drifted["cargo_mutants"] = "9.9.9"
            for workflow_name in ("main", "baseline"):
                with self.subTest(decoded_pin_workflow=workflow_name):
                    with self.assertRaisesRegex(ValueError, "CI tool pins drift from scripts/tool-versions.toml"):
                        TOOLS.check_ci_tool_pins(root, drifted, workflows=(workflow_name,))
            baseline = root / ".github/workflows/mutation-baseline.yml"
            baseline_original = baseline.read_text()
            for pin in (versions["rust"],):
                with self.subTest(baseline_pin=pin):
                    baseline.write_text(baseline_original.replace(pin, "9.9.9"))
                    with self.assertRaises(ValueError):
                        TOOLS.check(root)
            baseline.write_text(baseline_original)

    def test_catalog_tools_are_provisioned_in_gate(self):
        CAPS.check_ci(ROOT)

    def test_gate_tool_install_cannot_be_removed_moved_or_revised(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / """O'Brien's "gate" checkout"""
            self.copy_ci_fixture(root)
            check_ci = load('plant_tools', root).check_ci
            path = root / ".github/workflows/ci.yml"
            original = path.read_text()
            steps, offsets, write_steps = gate_step_fixture(path, original)
            versions = TOOLS.versions(ROOT)
            action = 'taiki-e/install-action@' + versions['install_action']
            plant = {'run': 'python3 -I scripts/check_plants.py'}
            compiler = next(step for step in steps if '--component rustfmt,clippy' in step.get('run', ''))

            cases = json.loads((root / 'plants/catalog.json').read_text())['cases']
            plant_run = 'python3 -I "$GITHUB_WORKSPACE/plants/run.py" --source .. --revision "$GITHUB_SHA" --output "$RUNNER_TEMP/plants"'

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
                            check_ci(root)
                remaining = [*before, *after]
                dogfood = 'bash scripts/tests/dogfood.sh'
                remaining.insert(index if name == 'just' else remaining.index(plant), {'run': dogfood})
                first = index if name == 'just' else remaining.index(plant)
                for position in range(first, remaining.index(plant) + 2):
                    with self.subTest(tool=name, position=position):
                        moved = remaining.copy()
                        moved.insert(position, steps[index])
                        write_steps(moved)
                        decoded = CAPS.workflow_jobs(path.read_text())['gate']['steps']
                        self.assertEqual(decoded.pop(position), install)
                        self.assertEqual(decoded, remaining)
                        if position == first:
                            check_ci(root)
                        else:
                            with self.assertRaises(ValueError):
                                check_ci(root)
                if name == 'just':
                    for step in ({'run': '\n' + dogfood + '\n'},
                                 {'run': dogfood + '\n'},
                                 {'run': 'true\n' + dogfood},
                                 {'run': dogfood + '\n\n'},
                                 {'run': "bash '${{ github.workspace }}/scripts/tests/dogfood.sh'\n",
                                  'name': 'Run dogfood\nfixture', 'if': 'success()', 'env': {'MODE': 'local\nfixture\n'},
                                  'shell': 'bash', 'working-directory': 'scripts'},
                                 {'run': "import subprocess\nsubprocess.check_call(['bash', 'dogfood.sh'])",
                                  'shell': 'python', 'working-directory': 'scripts/tests'},
                                 {'run': "import subprocess\nsubprocess.run(args=['bash', 'dogfood.sh'], cwd='scripts/tests')",
                                  'shell': 'python'},
                                 {'run': "from subprocess import Popen\nPopen(['bash', 'dogfood.sh'], -1, None, None, "
                                         "None, None, None, True, False, 'scripts/tests')",
                                  'shell': 'python'}):
                        with self.subTest(step=step):
                            write_steps([*before, install, step, *after])
                            decoded = CAPS.workflow_jobs(path.read_text())['gate']['steps']
                            self.assertEqual(decoded[len(before) + 1], step)
                            check_ci(root)
                            write_steps([*before, step, install, *after])
                            with self.assertRaisesRegex(ValueError, 'install Just before running scripts/tests/dogfood.sh'):
                                check_ci(root)
                    for shell in ('python', 'python3 {0}', 'python3 -I {0}'):
                        with self.subTest(independent_shell=shell):
                            write_steps([*before, {'shell': shell, 'run': 'print("ready")'}, install, *after])
                            check_ci(root)
                    linux = ("import sys\nif sys.platform == 'linux':\n"
                             "    from subprocess import run as launch\n"
                             "else:\n    def launch(*args): return args\n"
                             "launch(['just', '--version'])")
                    for source in (linux, "import pty as terminal; terminal.spawn(['just', '--version'])",
                                   "from subprocess import run as launch; launch(['just', '--version'])",
                                   "from subprocess import *; run(['just', '--version'])",
                                   "import asyncio.subprocess as proc; import asyncio; "
                                   "asyncio.run(proc.create_subprocess_exec(program='just'))",
                                   "from asyncio.subprocess import *; import asyncio; "
                                   "asyncio.run(create_subprocess_shell(cmd='just --version'))"):
                        with self.subTest(process_alias=source):
                            step = {'shell': 'python', 'run': source}
                            write_steps([*before, install, step, *after])
                            check_ci(root)
                            write_steps([*before, step, install, *after])
                            with self.assertRaisesRegex(ValueError, 'install Just before running just'):
                                check_ci(root)
                    local_run = ("def run(**overrides): return overrides\n"
                                 "run(**{'args': ['just']})")
                    write_steps([*before, {'shell': 'python', 'run': local_run}, install, *after])
                    check_ci(root)
                    local_branches = linux.replace("from subprocess import run as launch", "def launch(*args): return args")
                    write_steps([*before, {'shell': 'python', 'run': local_branches}, install, *after])
                    check_ci(root)
                    for shell in ('pwsh', 'python -m unittest {0}'):
                        with self.subTest(unsupported_shell=shell):
                            step = {'shell': shell, 'run': 'true'}
                            write_steps([*before, install, step, *after])
                            self.assertTrue(path.read_bytes().endswith(original[offsets[index + 1]:].encode()))
                            check_ci(root)
                            write_steps([*before, step, install, *after])
                            with self.assertRaisesRegex(ValueError, 'unsupported gate shell'):
                                check_ci(root)
                    unpacked = "import subprocess; subprocess.run(['bash', 'dogfood.sh'], **{'cwd': 'scripts/tests'})"
                    for step in ({'shell': 'python', 'run': unpacked},
                                 {'shell': 'python', 'run': "import pty as terminal; terminal.spawn(**{'argv': ['just']})"},
                                 {'run': f'python3 -c "{unpacked}"'},
                                 {'run': f'python3 -c"{unpacked}"'},
                                 {'run': f'python3 - <<< "{unpacked}"'},
                                 {'run': f"python3 - <<'PY'\n{unpacked}\nPY\n"},
                                 {'shell': 'python', 'run': "from subprocess import run; run(**{'args': ['just']})"}):
                        with self.subTest(unpacked=step):
                            write_steps([*before, step, install, *after])
                            with self.assertRaisesRegex(ValueError, 'unpacked Python process keywords'):
                                check_ci(root)
                    python_source = "import subprocess; subprocess.run(['bash', '$GITHUB_WORKSPACE/scripts/tests/dogfood.sh'])"
                    for directory, run, expected in (
                        ('scripts', f'python3 -c "{python_source}"', 'scripts/tests/dogfood.sh'),
                        ('scripts', f'python3 -c"{python_source}"', 'scripts/tests/dogfood.sh'),
                        ('scripts', f'python3 - <<< "{python_source}"', 'scripts/tests/dogfood.sh'),
                        ('scripts', f'python3 - <<PY\n{python_source}\nPY\n', 'scripts/tests/dogfood.sh'),
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
                                    ' Guard.test_fixed_just_recipes_select_checkout', '.claude/hooks/test_guard.py'),
                        ('scripts/tests', 'python3 managed_review_bundle.py ReviewBundle.test_review_publishes_only_completed_outputs -k isolation',
                         'scripts/tests/managed_review_bundle.py'),
                        ('.claude/hooks', 'python3 -m unittest test_guard.Guard.test_fixed_just_recipes_select_checkout -k unrelated',
                         '.claude/hooks/test_guard.py'),
                        ('scripts/tests', 'python3 -m unittest managed_review_bundle.ReviewBundle',
                         'scripts/tests/managed_review_bundle.py'),
                        ('scripts/tests', 'python3 -munittest managed_review_bundle.ReviewBundle.test_candidate_cannot_supply_instructions_or_policy'
                                         ' managed_review_bundle.ReviewBundle.test_review_publishes_only_completed_outputs -kisolation',
                         'scripts/tests/managed_review_bundle.py'),
                        ('.', 'python3 -m unittest scripts/tests/managed_review_bundle.py', 'scripts/tests/managed_review_bundle.py'),
                        ('scripts/tests', 'python3 -m unittest managed_review_bundle.ReviewBundle.unknown_method',
                         'scripts/tests/managed_review_bundle.py'),
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
                            write_steps([*before, install, consumer, *after])
                            check_ci(root)
                            write_steps([*before, consumer, install, *after])
                            with self.assertRaises(ValueError) as refused:
                                check_ci(root)
                            self.assertEqual(str(refused.exception), 'gate must install Just before running ' + expected)
                    for case_id in ('hook-shell-spelling', 'hook-malformed-open', 'hook-unsupported-tool-open',
                                    'candidate-review-policy'):
                        case = next(case for case in cases if case['id'] == case_id)
                        with self.subTest(catalog_selection=case_id):
                            suite = {'run': shlex.join(case['command'][:-1])}
                            selected = {'run': shlex.join(case['command'])}
                            write_steps([*before, selected, install, *after])
                            check_ci(root)
                            write_steps([*before, suite, install, *after])
                            with self.assertRaisesRegex(ValueError, 'install Just before'):
                                check_ci(root)
                    for run in ('env -C "$UNKNOWN_DIR" bash dogfood.sh', 'env --chdir=../../.. python3 run.py',
                                'bash "$UNKNOWN_SCRIPT"'):
                        with self.subTest(unresolved_location=run):
                            write_steps([*before, {'run': run}, install, *after])
                            with self.assertRaisesRegex(ValueError, 'cannot resolve|outside the workspace'):
                                check_ci(root)
                    for run in ('bash dogfood.sh', 'python3 -I ../.claude/hooks/guard.py pre',
                                'python3 -I "${GITHUB_WORKSPACE}/.claude/hooks/guard.py" post',
                                'env -C tests python3 -m unittest managed_review_bundle.ReviewBundle.test_candidate_cannot_supply_instructions_or_policy -k other',
                                'env -C ../.claude/hooks python3 -munittest test_guard.Guard.test_shell_tokenization_and_rust_writes',
                                'python3 ../.claude/hooks/test_guard.py Guard.test_shell_tokenization_and_rust_writes'
                                ' Guard.test_stop_runs_gate_for_hidden_and_untracked_changes',
                                'env -C ../.claude/hooks python3 -m unittest test_guard.Guard -k stop_runs_gate_for_hidden',
                                'env -C tests python3 -m unittest managed_review_bundle.ReviewBundle -k candidate_cannot',
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
                            check_ci(root)
            for target in (compiler, plant):
                write_steps([dict(step, **{'if': 'false'}) if step == target else step for step in steps])
                with self.assertRaises(ValueError):
                    check_ci(root)
            path.write_text(original.replace('  gate:\n', '  gate:\n    if: false\n', 1))
            with self.assertRaises(ValueError):
                check_ci(root)

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

    def test_unreviewed_merge_worker_cannot_run(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / ".github/workflows/ci.yml"
            path.parent.mkdir(parents=True)
            workflow = yaml.load((ROOT / ".github/workflows/ci.yml").read_bytes(), Loader=yaml.BaseLoader)
            marker = root / "ran"
            for step in workflow["jobs"]["dogfood"]["steps"]:
                if "run" in step:
                    step["run"] = f": > {shlex.quote(str(marker))}\n" + step["run"]
            path.write_text(yaml.safe_dump(workflow))
            with patch.object(subprocess, "run") as launch, self.assertRaisesRegex(ValueError, "WORKFLOW_SHA256"):
                self.check_merge_subjects(root)
            launch.assert_not_called()
            self.assertFalse(marker.exists())

    def test_merge_queue_jobs_bind_both_subjects(self):
        self.check_merge_subjects(ROOT)

    def check_merge_subjects(self, root):
        routing = runpy.run_path(str(ROOT / "scripts/tests/runner_routing.py"))
        jobs = CAPS.workflow_jobs(routing["reviewed_workflow"](routing["CI"], root))
        evaluate = routing["evaluate"]
        events = ("pull_request", "merge_group", "push")
        contexts = [{"github.event_name": event, "github.sha": "e" * 40,
                     "github.event.pull_request.base.sha": "a" * 40 if event == "pull_request" else "",
                     "github.event.pull_request.head.sha": "b" * 40 if event == "pull_request" else "",
                     "github.event.merge_group.base_sha": "c" * 40 if event == "merge_group" else "",
                     "github.event.merge_group.head_sha": "d" * 40 if event == "merge_group" else ""}
                    for event in events]
        bases, heads = ["a" * 40, "c" * 40], ["b" * 40, "d" * 40]
        def invocation(command, environment, driver_version):
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                driver = root / "trusted/scripts/dogfood.sh"
                driver.parent.mkdir(parents=True)
                record = """printf '%s\\0' "$@" "$BASE_SHA" "${HEAD_SHA-}" >> "$RECORD"\n"""
                driver.write_text(f"# forge-warden trusted-dogfood-driver {driver_version}\n"
                                  'set -- driver "$@"\n' + record)
                python = root / "python3"
                python.write_text('#!/bin/sh\nset -- python3 "$@"\n' + record)
                python.chmod(0o700)
                for tool in ("bash", "grep"):
                    (root / tool).symlink_to(shutil.which(tool))
                receipt = root / "argv"
                env = dict(environment, RECORD=str(receipt), PATH=directory,
                           GITHUB_WORKSPACE="/workspace", RUNNER_TEMP="/runner-temp")
                subprocess.run(["/bin/bash", "-eo", "pipefail"], cwd=root, env=env,
                               input=command, text=True, check=True, timeout=10)
                return receipt.read_bytes().decode().split("\0")

        def check_field(mapping, key, read, expected, wrong):
            self.assertEqual(read(mapping), expected)
            with mock.patch.dict(mapping, {key: wrong}):
                self.assertNotEqual(read(mapping), expected)

        for name in ("mutation-plan", "mutation-shards", "dogfood"):
            job = jobs[name]
            steps = job["steps"]
            with self.subTest(job=name):
                if name == "mutation-shards":
                    check_field(job, "needs", lambda job: job["needs"], ["mutation-plan"], [])
                    check_field(job, "if", lambda job: job.get("if", "success()").removeprefix("${{")
                                .removesuffix("}}").strip() in ("true", "success()"), True, "false")
                else:
                    def active(job):
                        condition = job.get("if", "true")
                        if not condition.startswith("${{"):
                            condition = "${{ " + condition + " }}"
                        return [evaluate(condition, context) for context in contexts]
                    check_field(job, "if", active, [True, True, False], "false")

                refs = ({"trusted": bases, "candidate": heads} if name == "dogfood" else
                        {".": [context["github.sha"] for context in contexts[:2]]})
                checkouts = [step.get("with", {}) for step in steps
                             if step.get("uses", "").partition("@")[0] == "actions/checkout"]
                self.assertCountEqual([inputs.get("path", ".") for inputs in checkouts], refs.keys())
                fields = [(inputs, "ref", refs[inputs.get("path", ".")]) for inputs in checkouts]
                if name != "mutation-shards":
                    def is_worker(step):
                        if "run" not in step:
                            return False
                        if name == "mutation-plan":
                            matrix = job["outputs"]["matrix"].removeprefix("${{").removesuffix("}}").strip()
                            return "id" in step and matrix == f"steps.{step['id']}.outputs.matrix"
                        return any(word == "(sh)trusted/scripts/dogfood.sh"
                                   for _, word, _ in COMMANDS.shell_calls(step["run"]))

                    worker, = filter(is_worker, steps)
                    env = {"BASE_SHA": bases, **({"HEAD_SHA": heads} if name == "dogfood" else {})}
                    for i, context in enumerate(contexts[:2]):
                        environment = {key: evaluate(worker["env"][key], context) for key in env}
                        for version in (("v1", "v2") if name == "dogfood" else ("v2",)):
                            if name == "dogfood":
                                mode = ["--hosted-advisory"] if version == "v2" else []
                                expected = ["driver", *mode, bases[i], heads[i], "/workspace/candidate",
                                            "/runner-temp/warden-evidence", bases[i], heads[i], ""]
                                wrong = shlex.join(["bash", "trusted/scripts/dogfood.sh", *mode]) + (
                                    ' "$HEAD_SHA" "$BASE_SHA" "$GITHUB_WORKSPACE/candidate" "$RUNNER_TEMP/warden-evidence"')
                            else:
                                expected = ["python3", "-I", "scripts/mutation_ci.py", "plan", "diff", bases[i], "", ""]
                                wrong = shlex.join(["python3", "-I", "scripts/mutation_ci.py", "plan", "full"])
                            self.assertEqual(invocation(worker["run"], environment, version), expected)
                            self.assertNotEqual(invocation(wrong, environment, version), expected)
                    self.assertEqual(worker["env"].keys() & {"BASE_SHA", "HEAD_SHA"}, env.keys())
                    fields.extend((worker["env"], key, expected) for key, expected in env.items())
                for mapping, key, expected in fields:
                    with self.subTest(field=key, path=mapping.get("path")):
                        def revisions(fields):
                            expression = fields.get("ref", "${{ github.sha }}") if key == "ref" else fields[key]
                            return [evaluate(expression, context) for context in contexts[:2]]
                        check_field(mapping, key, revisions, expected, "${{ 'wrong-revision' }}")

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
        }
        for name, call in calls.items():
            with self.subTest(call=name):
                found = self.problems({"t.py": "import asyncio, os, pty, shlex, subprocess\n" + call + "\n"})
                self.assertEqual(len(found), 1, found)
                self.assertRegex(found[0], r"^t\.py:2: calls `(cmp|jq|xxd)`, which the gate image")

    def test_an_aliased_process_function_is_read(self):
        for source in ("from subprocess import run as sh\nsh(['cmp', 'a', 'b'])",
                       "import subprocess as process\nprocess.run(['cmp', 'a', 'b'])",
                       "import pty as terminal\nterminal.spawn(['cmp', 'a', 'b'])",
                       "from pty import spawn as launch\nlaunch(argv=['cmp', 'a', 'b'])"):
            with self.subTest(source=source):
                found = self.problems({"t.py": source})
                self.assertEqual(len(found), 1, found)
                self.assertTrue(found[0].startswith("t.py:2: calls `cmp`"), found)

    def test_local_run_helpers_do_not_mask_imported_process_calls(self):
        source = textwrap.dedent("""\
            from subprocess import run
            class Helper:
                def run(self, **overrides): return overrides
                def use(self): self.run(**{'args': ['cmp']})
            def local():
                def run(**overrides): return overrides
                run(**{'args': ['cmp']})
            local()
            Helper().use()
            run(['cmp', 'a', 'b'])
        """)
        found = self.problems({"t.py": source})
        self.assertEqual(found, ["t.py:10: calls `cmp`, which the gate image does not declare (scripts/script_commands.py)"])

    def test_exported_process_commands_in_tracked_and_nested_python(self):
        for source in (
            "import asyncio.subprocess; asyncio.run(asyncio.subprocess.create_subprocess_exec(program='cmp'))",
            "import asyncio.subprocess as proc; import asyncio; asyncio.run(proc.create_subprocess_shell(cmd='cmp a b'))",
            "from asyncio.subprocess import create_subprocess_exec as launch; import asyncio; asyncio.run(launch('cmp'))",
            "from asyncio import *; run(create_subprocess_exec(program='cmp'))",
            "from asyncio.subprocess import *; import asyncio; asyncio.run(create_subprocess_shell('cmp a b'))",
            "from subprocess import *; run(['cmp'])",
            "from os import *; system('cmp a b')",
            "from pty import *; spawn(['cmp'])",
        ):
            with self.subTest(source=source):
                found = self.problems({"t.py": source, "nested.sh": 'python3 -c ' + shlex.quote(source)})
                self.assertCountEqual(found, [path + ":1: calls `cmp`, which the gate image does not declare "
                                              "(scripts/script_commands.py)" for path in ("t.py", "nested.sh")])

    def test_conditional_imports_retain_reachable_process_calls(self):
        for bindings in (
            "if linux:\n    from subprocess import run as launch\nelse:\n    def launch(*args): pass",
            "if linux:\n    def launch(*args): pass\nelse:\n    from subprocess import run as launch",
            "from subprocess import run as launch\nif linux:\n    launch = print",
            "import subprocess\nlaunch = subprocess.run if linux else print",
            "try:\n    from subprocess import run as launch\nexcept ImportError:\n    launch = print",
            "from subprocess import run as launch\nlinux or (launch := print)",
            "from subprocess import run as launch\nfor item in []:\n    launch = print",
            "match linux:\n    case True:\n        from subprocess import run as launch\n    case _:\n        launch = print",
        ):
            with self.subTest(bindings=bindings):
                source = "import sys\nlinux = sys.platform == 'linux'\n" + bindings + "\nlaunch(['cmp'])"
                found = self.problems({"t.py": source})
                self.assertEqual(found, [f"t.py:{len(source.splitlines())}: calls `cmp`, which the gate image "
                                         "does not declare (scripts/script_commands.py)"])

    def test_unpacked_process_keywords_refuse_tracked_scripts(self):
        unpacked = "from subprocess import run; run(**{'args': ['cmp', 'a', 'b']})"
        found = self.problems({
            "unpacked.py": unpacked,
            "terminal.py": "import pty as terminal; terminal.spawn(**{'argv': ['cmp', 'a', 'b']})",
            "nested.sh": f'python3 -c "{unpacked}"\n',
            "explicit.py": "from subprocess import run; run(args=['cmp', 'a', 'b'])",
        })
        self.assertCountEqual(found, [
            "unpacked.py: not read: cannot resolve unpacked Python process keywords",
            "terminal.py: not read: cannot resolve unpacked Python process keywords",
            "nested.sh: not read: cannot resolve unpacked Python process keywords",
            "explicit.py:1: calls `cmp`, which the gate image does not declare (scripts/script_commands.py)",
        ])

    def test_helpers_use_bindings_at_the_call(self):
        for owner in ("subprocess.run", "print"):
            for helper in (
                "def cleanup(): launch(['just'])",
                "def inner(): launch(['just'])\ndef cleanup(): inner()",
            ):
                with self.subTest(owner=owner, helper=helper):
                    source = (f"import subprocess\nlaunch = {owner}\n{helper}\ntry:\n    open('config')\n"
                              "    launch = print\nfinally:\n    cleanup()")
                    expected = "just" if owner == "subprocess.run" else None
                    self.assertEqual(CAPS.just_consumer({"run": source, "shell": "python"}, vars(COMMANDS), []), expected)
                    found = self.problems({"t.py": source.replace("'just'", "'cmp'")})
                    self.assertEqual(len(found), int(expected is not None), found)
                    if expected:
                        self.assertRegex(found[0], r"^t\.py:\d+: calls `cmp`")
        local_call = ("from subprocess import run as launch\ndef cleanup(): launch(['just'])\n"
                      "launch = print\ncleanup()")
        self.assertIsNone(CAPS.just_consumer({"run": local_call, "shell": "python"}, vars(COMMANDS), []))

    def test_helper_callable_arguments_before_install(self):
        helper = "from subprocess import run\ndef invoke(launch): launch(['just'])\n"
        for call, expected in (("invoke(run)", "just"), ("invoke(launch=run)", "just"),
                               ("invoke(print)", None), ("invoke(len)", None), ("invoke(print); invoke(run)", "just")):
            with self.subTest(call=call):
                self.assertEqual(CAPS.just_consumer({"shell": "python", "run": helper + call},
                                                    vars(COMMANDS), []), expected)
        method = "class Cleanup:\n    def finish(self): invoke(run)\nCleanup().finish()"
        self.assertEqual(CAPS.just_consumer({"shell": "python", "run": helper + method}, vars(COMMANDS), []), "just")
        for call in ("invoke(unknown)", "invoke(*arguments)", "invoke(**options)"):
            with self.subTest(call=call), self.assertRaisesRegex(ValueError, "Python callable"):
                CAPS.just_consumer({"shell": "python", "run": helper + call}, vars(COMMANDS), [])

    def test_only_invoked_methods_contribute_process_commands(self):
        setup = textwrap.dedent("""\
            from subprocess import run
            class Cleanup:
                def __init__(self, launch):
                    self.launch = launch
                def run(self):
                    run(['just'])
                def forward(self, program):
                    self.launch([program])
                def finish(self, program):
                    self.forward(program)
            """)
        for call, expected in (
            ("Cleanup(run)", None),
            ("item = Cleanup(print); bound = item.run", None),
            ("Cleanup(print).run()", "just"),
            ("bound = Cleanup(print).run; bound()", "just"),
            ("Cleanup(run).finish(program='just')", "just"),
            ("Cleanup(print).finish('just')", None),
            ("def outer(run): Cleanup(print).run()\nouter(print)", "just"),
            ("class Local:\n    def run(self):\n        run = print\n        run(['just'])\nLocal().run()", None),
            ("class Chain:\n    def run(self):\n        if again: self.run()\n        run(['just'])\nChain().run()", "just"),
            ("class Start:\n    def __init__(self): Cleanup(print).run()\nStart()", "just"),
        ):
            with self.subTest(call=call):
                self.assertEqual(CAPS.just_consumer({"shell": "python", "run": setup + call},
                                                    vars(COMMANDS), []), expected)
        nested = ("def outer(run):\n    class Inner:\n        def run(self): run(['just'])\n"
                  "    Inner().run()\n")
        for launcher, expected in (("print", None), ("run", "just")):
            source = setup + nested + f"outer({launcher})"
            with self.subTest(launcher=launcher):
                self.assertEqual(CAPS.just_consumer({"shell": "python", "run": source}, vars(COMMANDS), []), expected)

    def test_wrapped_launchers_resolve_at_invocation(self):
        setup = "import functools, subprocess\nfrom functools import partial as wrap\n"
        for call, expected in (
            ("launch = functools.partial(subprocess.run, check=True)\nlaunch(['just'])", "just"),
            ("wrap(subprocess.run, ['probe'], executable='just')()", "just"),
            ("wrap(subprocess.run, args=['just'])()", "just"),
            ("wrap(subprocess.run, ['probe'], executable='just')(executable='true')", None),
            ("def factory(): return subprocess.run\nlaunch = factory()\nlaunch(['just'])", "just"),
            ("def factory(launch): return launch\nfactory(subprocess.run)(['just'])", "just"),
            ("wrap(print, end='')('just')", None),
            ("def factory(): return print\nfactory()(['just'])", None),
            ("wrap(subprocess.run, ['just'])", None),
        ):
            source = setup + call
            with self.subTest(call=call):
                self.assertEqual(CAPS.just_consumer({"shell": "python", "run": source}, vars(COMMANDS), []), expected)
                tracked = source.replace("'just'", "'cmp'")
                found = self.problems({"t.py": tracked, "nested.sh": "python3 -c " + shlex.quote(tracked)})
                self.assertEqual(len(found), 2 if expected else 0, found)
                for issue in found:
                    self.assertIn("calls `cmp`", issue)
        for call in ("def factory(): return lookup()\nfactory()(['just'])",
                     "wrap(unknown)(['just'])", "wrap(subprocess.run)(arguments)",
                     "wrap(subprocess.run, ['probe'], executable=choice)()"):
            with self.subTest(call=call), self.assertRaisesRegex(ValueError, "cannot resolve"):
                CAPS.just_consumer({"shell": "python", "run": setup + call}, vars(COMMANDS), [])

    def test_partial_arguments_keep_creation_scope_through_forwarders(self):
        setup = "from functools import partial\nfrom subprocess import run\nprogram = 'just'\n"
        for bind in ("partial(run, [program])", "partial(run, args=[program])",
                     "partial(run, ['probe'], executable=program)", "partial(launch, program)",
                     "partial(launch, tool=program)", "partial(launch, tool='true')"):
            call = "check(tool='just')" if bind.endswith("tool='true')") else "check()"
            source = (setup + "def launch(tool): run([tool])\ncheck = " + bind +
                      f"\ndef start(program): {call}\nstart('true')")
            with self.subTest(bind=bind):
                self.assertEqual(CAPS.just_consumer({"shell": "python", "run": source}, vars(COMMANDS), []), "just")
                tracked = source.replace("'just'", "'cmp'")
                found = self.problems({"t.py": tracked, "nested.sh": "python3 -c " + shlex.quote(tracked)})
                self.assertEqual(len(found), 2, found)
                self.assertTrue(all("calls `cmp`" in issue for issue in found), found)
        for bind in ("partial(run, [program])", "partial(run, args=['bash', '-c', program])",
                     "partial(run, executable=program)", "partial(launch, program)"):
            source = ("from functools import partial\nfrom subprocess import run\nprogram = read_config()\n"
                      "def launch(tool): run([tool])\ncheck = " + bind +
                      "\ndef start(program): check()\nstart('true')")
            with self.subTest(unknown=bind):
                with self.assertRaisesRegex(ValueError, "cannot resolve"):
                    CAPS.just_consumer({"shell": "python", "run": source}, vars(COMMANDS), [])
                self.assertIn("cannot resolve", self.problems({"t.py": source})[0])
        pure = "import functools, shlex\nfunctools.partial(shlex.split, posix=True)('ready')"
        self.assertIsNone(CAPS.just_consumer({"shell": "python", "run": pure}, vars(COMMANDS), []))
        self.assertEqual(self.problems({"t.py": pure}), [])

    def test_unittest_main_dispatches_only_test_cases_and_lifecycle(self):
        setup = textwrap.dedent("""\
            from unittest import TestCase as Case, main as discover
            from subprocess import run
            class Suite(Case):
                @classmethod
                def setUpClass(cls): run(['cmp', 'class setup'])
                def setUp(self): self.launch = run
                def test_probe(self): self.launch(['cmp', 'test'])
                def tearDown(self): run(['cmp', 'teardown'])
                @classmethod
                def tearDownClass(cls): run(['cmp', 'class teardown'])
                def unused(self): run(['xxd'])
            class Ordinary:
                def test_probe(self): run(['xxd'])
            def entry(): discover()
            """)
        for invoke in ("", "if False: entry()", "entry()"):
            with self.subTest(invoke=invoke):
                found = self.problems({"suite.py": setup + invoke})
                expected = [5, 7, 8, 10] if invoke == "entry()" else []
                self.assertCountEqual(found, [f"suite.py:{line}: calls `cmp`, which the gate image does not declare "
                                         "(scripts/script_commands.py)" for line in expected])
                source = (setup + invoke).replace("'cmp'", "'just'")
                self.assertEqual(CAPS.just_consumer({"shell": "python", "run": source}, vars(COMMANDS), []),
                                 "just" if expected else None)

    def test_instance_field_overwrites_and_joins(self):
        setup = ("from subprocess import run\nclass Task:\n    def set(self, fn): self.launch = fn\n"
                 "    def fire(self): self.launch(['just'])\na = Task()\nb = a\na.set(run)\n")
        for changes, expected in (
            ("b.launch = print", None), ("b.set(print)", None),
            ("if flag: b.set(print)", "just"), ("b.set(print)\nif flag: a.launch = run", "just"),
            ("if flag: b.launch = print\nelse: a.set(print)", None),
            ("for i in items: b.launch = print", "just"),
            ("try:\n    read_config()\n    b.set(print)\nexcept OSError: pass", "just"),
        ):
            with self.subTest(changes=changes):
                source = setup + changes + "\na.fire()"
                self.assertEqual(CAPS.just_consumer({"shell": "python", "run": source}, vars(COMMANDS), []), expected)

    def test_inherited_dispatch_resolves_declared_local_implementations(self):
        setup = ("from subprocess import run\nclass Base:\n    def prepare(self): run(['just'])\n"
                 "class Child(Base): pass\nclass Leaf(Child): pass\n")
        init = setup.replace("def prepare(self)", "def __init__(self)")
        unknown = "from elsewhere import Base\nclass Child(Base): pass\n"
        for source, expected in ((setup + "Leaf()", None), (unknown, None),
                                 (setup + "Leaf().prepare()", "just"), (init + "Leaf()", "just"),
                                 (init + "class Quiet(Leaf):\n    def __init__(self): pass\nQuiet()", None)):
            with self.subTest(source=source):
                self.assertEqual(CAPS.just_consumer({"shell": "python", "run": source}, vars(COMMANDS), []), expected)
                found = self.problems({"t.py": source.replace("'just'", "'cmp'")})
                self.assertEqual(len(found), int(expected is not None), found)
                if expected:
                    self.assertIn("calls `cmp`", found[0])
        for source in (unknown + "Child()", unknown + "Child.prepare(None)",
                       setup + "class Derived(Child):\n    def prepare(self): super().prepare()\nDerived().prepare()"):
            with self.subTest(source=source):
                with self.assertRaisesRegex(ValueError, "cannot resolve"):
                    CAPS.just_consumer({"shell": "python", "run": source}, vars(COMMANDS), [])
                self.assertIn("cannot resolve", self.problems({"t.py": source})[0])

    def test_unresolved_shell_mode_audits_argv_and_shell_payload(self):
        setup = "from functools import partial\nfrom subprocess import Popen\nmode = read_flag()\n"
        for form in ("Popen(ARGV, shell=mode)", "Popen(ARGV, -1, None, None, None, None, None, True, mode)",
                     "partial(Popen, ARGV, shell=mode)()",
                     "partial(Popen, ARGV, -1, None, None, None, None, None, True, mode)()"):
            for argv in (["bash", "-c", "just --version"], ["python3", "-c", "import os; os.system('just')"],
                         ["true && just"]):
                source = setup + form.replace("ARGV", repr(argv))
                with self.subTest(form=form, argv=argv):
                    self.assertEqual(CAPS.just_consumer({"shell": "python", "run": source}, vars(COMMANDS), []), "just")
                    tracked = source.replace("just", "cmp")
                    found = self.problems({"t.py": tracked, "nested.sh": "python3 -c " + shlex.quote(tracked)})
                    self.assertEqual(sum("calls `cmp`" in problem for problem in found), 2, found)
            source = setup + form.replace("ARGV", "['bash', '-c', unknown_payload]")
            with self.subTest(form=form, unresolved=True):
                with self.assertRaisesRegex(ValueError, "cannot resolve"):
                    CAPS.just_consumer({"shell": "python", "run": source}, vars(COMMANDS), [])
                self.assertIn("cannot resolve", self.problems({"t.py": source})[0])

    def test_conditional_unittest_decorators_preserve_possible_commands(self):
        prefix = ("from unittest import TestCase, main, skipIf, skipUnless\nfrom shutil import which\n"
                  "from subprocess import run\nclass Case(TestCase):\n    def test_control(self): pass\n")
        for method in ("test_probe", "setUp", "tearDown", "setUpClass", "tearDownClass"):
            descriptor = "    @classmethod\n" if method.endswith("Class") else ""
            for decorator, enabled in (("skipIf(which('just'), 'dependency')", True),
                                       ("skipUnless(which('just'), 'dependency')", True),
                                       ("skipUnless(condition=False, reason='off')", False),
                                       ("skipIf(True, 'off')", False), ("skipIf(False, 'on')", True)):
                with self.subTest(method=method, decorator=decorator):
                    source = prefix + descriptor + f"    @{decorator}\n    def {method}(self): run(['just'])\nmain()"
                    self.assertEqual(CAPS.just_consumer({"shell": "python", "run": source}, vars(COMMANDS), []),
                                     "just" if enabled else None)
                    found = self.problems({"t.py": source.replace("run(['just'])", "run(['cmp'])")})
                    self.assertEqual(len(found), int(enabled), found)
                    if enabled:
                        self.assertIn("calls `cmp`", found[0])

    def test_finalizers_update_outgoing_fields(self):
        for action in (
            "try:\n    return\nfinally:\n    self.launch = last",
            "try:\n    raise ValueError\nexcept ValueError:\n    return\nfinally:\n    self.launch = last",
            "try:\n    try:\n        return\n    finally:\n        self.launch = first\nfinally:\n    self.launch = last",
            "try:\n    raise ValueError\nfinally:\n    self.launch = last",
        ):
            for last, expected in (("run", "just"), ("print", None)):
                source = (f"from subprocess import run\nlast = {last}\nfirst = print\nclass State:\n    def update(self):\n"
                          + textwrap.indent(action, "        ") + "\nx = State()\nx.launch = first\n"
                          "try:\n    x.update()\nexcept ValueError: pass\nx.launch(['just'])")
                with self.subTest(action=action, last=last):
                    self.assertEqual(CAPS.just_consumer({"shell": "python", "run": source}, vars(COMMANDS), []), expected)
                    found = self.problems({"t.py": source.replace("'just'", "'cmp'")})
                    self.assertEqual(len(found), int(expected is not None), found)
                    if expected:
                        self.assertIn("calls `cmp`", found[0])
        overwritten = ("from subprocess import run\nclass Task:\n    def update(self):\n"
                       "        try: return\n        finally: self.launch = print\n"
                       "x = Task()\nx.launch = run\nx.update()\nx.launch(['just'])")
        self.assertIsNone(CAPS.just_consumer({"shell": "python", "run": overwritten}, vars(COMMANDS), []))
        for transfer in ("continue", "break"):
            source = ("from subprocess import run\nlaunch = print\nfor item in (1, 2):\n"
                      f"    try:\n        {transfer}\n    finally:\n        launch = run\n    launch = print\nlaunch(['just'])")
            with self.subTest(transfer=transfer):
                self.assertEqual(CAPS.just_consumer({"shell": "python", "run": source}, vars(COMMANDS), []), "just")

        carried = ("from subprocess import run\nlaunch = print\nfor item in (1, 2):\n    launch(['just'])\n"
                   "    try: continue\n    finally: launch = run")
        self.assertEqual(CAPS.just_consumer({"shell": "python", "run": carried}, vars(COMMANDS), []), "just")

    def test_interpreter_override_reads_the_effective_payload(self):
        prefix = "import asyncio, subprocess\nfrom functools import partial\nmode = bool(1)\n"
        for executable, payload in (("python3", "import subprocess; subprocess.run(['just'])"),
                                    ("/bin/sh", "just --version"), ("python3", "print('just')")):
            for invocation in (
                "subprocess.Popen([PAYLOAD], shell=True, executable=EXE)",
                "subprocess.run([PAYLOAD], shell=mode, executable=EXE)",
                "partial(subprocess.Popen, [PAYLOAD], executable=EXE)(shell=True)",
                "asyncio.run(asyncio.create_subprocess_shell(PAYLOAD, executable=EXE))",
            ):
                source = prefix + invocation.replace("PAYLOAD", repr(payload)).replace("EXE", repr(executable))
                expected = None if payload.startswith("print") else "just"
                with self.subTest(source=source):
                    self.assertEqual(CAPS.just_consumer({"shell": "python", "run": source}, vars(COMMANDS), []), expected)
                    tracked = source.replace('just', 'cmp')
                    found = self.problems({"t.py": tracked, "nested.sh": "python3 -c " + shlex.quote(tracked)})
                    self.assertEqual(sum("calls `cmp`" in issue for issue in found), 2 if expected else 0, found)
        for executable, payload in (("'other'", "'text'"), ("'python3'", "dynamic")):
            with self.subTest(executable=executable, payload=payload):
                source = prefix + f"subprocess.run({payload}, shell=True, executable={executable})"
                with self.assertRaisesRegex(ValueError, "cannot resolve"):
                    CAPS.just_consumer({"shell": "python", "run": source}, vars(COMMANDS), [])
                self.assertIn("cannot resolve", self.problems({"t.py": source})[0])

    def recorded_commands(self, source):
        module = ModuleType('__main__')
        with patch.object(subprocess, 'run') as launch, patch.dict(sys.modules, {'__main__': module}), \
                patch.object(sys, 'argv', ['fixture']), redirect_stdout(StringIO()), redirect_stderr(StringIO()):
            try:
                exec(source, module.__dict__)
            except SystemExit as exc:
                self.assertEqual(exc.code, 0)
        return [call.args[0] for call in launch.call_args_list]

    async_manager = ("class Manager:\n    async def __aenter__(self): return self\n"
                     "    async def __aexit__(self, *exc): return True\nmanager = Manager()\n")

    def test_loop_back_edges_see_finalized_bindings(self):
        for loop in ("for i in (1, 2):", "async for i in items:", "while next(active, False):"):
            for transfer in ("", "continue"):
                for launcher, expected in (("run", "just"), ("print", None)):
                    body = ("launch = print\n" + loop + "\n    launch(['just'])\n    try: pass\n"
                            f"    finally: launch = {launcher}\n    {transfer}")
                    source = "from subprocess import run\nactive = iter((True, True))\n"
                    if loop.startswith("async"):
                        source += "import asyncio\nasync def check(items):\n" + textwrap.indent(body, "    ")
                        source += "\nasync def finite():\n    yield 1\n    yield 2\nasyncio.run(check(finite()))"
                    else:
                        source += body
                    with self.subTest(loop=loop, transfer=transfer, launcher=launcher):
                        self.assertEqual(self.recorded_commands(source), [['just']] if expected else [])
                        self.assertEqual(CAPS.just_consumer({"shell": "python", "run": source}, vars(COMMANDS), []), expected)
                        found = self.problems({"t.py": source.replace("'just'", "'cmp'")})
                        self.assertEqual(bool(found), bool(expected), found)
                        self.assertTrue(all("calls `cmp`" in issue for issue in found), found)

    def test_discovery_constructs_cases_before_skips(self):
        imports = "from unittest import TestCase, main, skipIf\nfrom subprocess import run\n"
        prefix = ("def identity(cls): return cls\nclass Mixin: pass\nclass Parent(TestCase):\n"
                  "    def __init__(self, method):\n        run([method])\n        super().__init__(method)\n")
        for decorator in ("", "@skipIf(True, 'skip')\n", "@identity\n"):
            source = (imports + prefix + decorator + "class Suite(Mixin, Parent):\n"
                      "    @skipIf(True, 'skip')\n    def test_constructor(self): run(['xxd'])\nmain()")
            with self.subTest(decorator=decorator):
                found = self.problems({"suite.py": source})
                self.assertEqual(len(found), 1, found)
                self.assertIn("calls `test_constructor`", found[0])
                workflow = source.replace("run([method])", "run(['just'])")
                self.assertEqual(CAPS.just_consumer({"shell": "python", "run": workflow}, vars(COMMANDS), []), "just")
        for body, expected in (("run(['just'])", "just"), ("print('just')", None)):
            source = (imports + "class Mixin: pass\ndef identity(cls): return cls\n@identity\n"
                      f"class Suite(Mixin, TestCase):\n    def test_it(self): {body}\nmain()")
            self.assertEqual(CAPS.just_consumer({"shell": "python", "run": source}, vars(COMMANDS), []), expected)
        for header in ("@unknown\nclass Suite(TestCase):", "class Suite(external, TestCase):"):
            source = imports + header + "\n    def test_it(self): pass\nmain()"
            with self.subTest(header=header), self.assertRaisesRegex(ValueError, "cannot resolve"):
                CAPS.just_consumer({"shell": "python", "run": source}, vars(COMMANDS), [])

    def test_context_manager_suppression_preserves_continuation(self):
        prefix = "from subprocess import run\nfrom contextlib import suppress\nimport asyncio, unittest\n" + self.async_manager
        for source in (
            "with suppress(ValueError): raise ValueError\nrun(['just'])",
            "class Suite(unittest.TestCase):\n    def test_it(self):\n        with self.assertRaises(ValueError):"
            " raise ValueError\n        run(['just'])\nunittest.main()",
            "async def check(manager):\n    async with manager: raise ValueError\n    run(['just'])\nasyncio.run(check(manager))",
        ):
            for launcher, expected in (("run", "just"), ("print", None)):
                script = prefix + source.replace("run(['just'])", f"{launcher}(['just'])")
                with self.subTest(script=script):
                    self.assertEqual(self.recorded_commands(script), [['just']] if expected else [])
                    self.assertEqual(CAPS.just_consumer({"shell": "python", "run": script}, vars(COMMANDS), []), expected)
                    found = self.problems({"t.py": script.replace("'just'", "'cmp'")})
                    self.assertEqual(bool(found), bool(expected), found)
                    self.assertTrue(all("calls `cmp`" in issue for issue in found), found)
        source = prefix + "def finished():\n    with suppress(ValueError): return\n    run(['just'])\nfinished()"
        self.assertIsNone(CAPS.just_consumer({"shell": "python", "run": source}, vars(COMMANDS), []))

    def test_implicit_exceptions_can_resume_after_context_manager(self):
        for context, declaration, invoke in (
            ("with suppress(Exception)", "def probe():", "probe()"),
            ("async with manager", "async def probe(manager):", "asyncio.run(probe(manager))"),
        ):
            for operation in ("return cached_result", "assert False\nreturn", "a, b = (1,)\nreturn",
                              "return cache.value", "return lookup()", "return 1", "return result"):
                source = ("import asyncio\nfrom contextlib import suppress\nfrom subprocess import run\n" + self.async_manager
                          + declaration + "\n    result = 1\n    " + context + ":\n" + textwrap.indent(operation, "        ")
                          + "\n    run(['just'])\n" + invoke)
                with self.subTest(context=context, operation=operation):
                    expected = operation not in ("return 1", "return result")
                    self.assertEqual(self.recorded_commands(source), [['just']] if expected else [])
                    self.assertEqual(CAPS.just_consumer({"shell": "python", "run": source}, vars(COMMANDS), []),
                                     "just" if expected else None)
                    found = self.problems({"t.py": source.replace("'just'", "'cmp'")})
                    self.assertEqual(len(found), int(expected), found)
                    if expected:
                        self.assertIn("calls `cmp`", found[0])

    def test_unittest_refuses_unresolved_handlers(self):
        for method in ("__init__", "setUp", "tearDown", "setUpClass", "tearDownClass", "test_probe", "runTest"):
            base = "class Fixture:\n    " + method + " = shared\n"
            body = "pass" if method in ("test_probe", "runTest") else "def test_probe(self): pass"
            source = ("from support import shared\nfrom unittest import TestCase, main\n" + base
                      + f"class Case(Fixture, TestCase):\n    {body}\nmain()")
            with self.subTest(method=method):
                with self.assertRaisesRegex(ValueError, "cannot resolve"):
                    CAPS.just_consumer({"shell": "python", "run": source}, vars(COMMANDS), [])
                self.assertIn("cannot resolve", self.problems({"suite.py": source})[0])

    def test_unittest_uses_runtest_for_noncallable_test_members(self):
        for member, expected in (("test_data = 'fixture'", "just"), ("test_data = []", "just"),
                                 ("def test_data(self): pass", None)):
            source = ("from subprocess import run\nfrom unittest import TestCase, main\nclass Fixture:\n    " + member
                      + "\nclass Case(Fixture, TestCase):\n    def runTest(self): run(['just'])\nmain()")
            with self.subTest(member=member):
                self.assertEqual(CAPS.just_consumer({"shell": "python", "run": source}, vars(COMMANDS), []), expected)
                found = self.problems({"suite.py": source.replace("'just'", "'cmp'")})
                self.assertEqual(len(found), int(bool(expected)), found)
                if expected:
                    self.assertIn("calls `cmp`", found[0])

    def test_loop_instances_converge_without_losing_field_changes(self):
        prefix = "from subprocess import run\nclass Task:\n    def noop(self): pass\nprevious = Task()\n"
        for loop in ("for i in (1, 2):", "while next(active, False):"):
            for body, expected in (("previous.noop()\nprevious = Task()", None),
                                   ("previous.launch(['just'])\nprevious = Task()\nprevious.launch = print", None),
                                   ("previous.launch(['just'])\nprevious = Task()\nprevious.launch = run", "just")):
                source = prefix + "active = iter((True, True))\nprevious.launch = print\n" + loop + "\n" + textwrap.indent(body, "    ")
                with self.subTest(loop=loop, body=body):
                    self.assertEqual(self.recorded_commands(source), [['just']] if expected else [])
                    self.assertEqual(CAPS.just_consumer({"shell": "python", "run": source}, vars(COMMANDS), []), expected)
                    found = self.problems({"loop.py": source.replace("'just'", "'cmp'")})
                    self.assertEqual(len(found), int(bool(expected)), found)
                    if expected:
                        self.assertIn("calls `cmp`", found[0])
        source = prefix + "previous.launch = print\nfor i in (1,):\n    previous.launch(['just'])\n    previous.launch = run"
        self.assertIsNone(CAPS.just_consumer({"shell": "python", "run": source}, vars(COMMANDS), []))

    def test_literal_loop_targets_and_duplicate_keys(self):
        for loop, expected in (
            ("for launch in (print, run): launch(['just'])", [['just']]),
            ("launchers = (print, run)\nfor launch in launchers: launch(['just'])", [['just']]),
            ("launchers = [('just', (run,))]\nitems = launchers\nrun = print\n"
             "for label, [launch] in items: launch([label])", [['just']]),
            ("items = [print, run]\nfirst, *rest = items\nfor launch in rest: launch(['just'])", [['just']]),
            ("for launch in {print, run}: launch(['just'])", [['just']]),
            ("for [label, (launch,)] in [('just', (run,))]: launch([label])", [['just']]),
            ("for i in {'setup', 'setup'}:\n    launch(['just'])\n    launch = run", []),
            ("for i in {0: 1, False: 2}:\n    launch(['just'])\n    launch = run", []),
        ):
            source = "from subprocess import run\nlaunch = print\n" + loop
            with self.subTest(loop=loop):
                self.assertEqual(self.recorded_commands(source), expected)
                self.assertEqual(CAPS.just_consumer({"run": source, "shell": "python"}, vars(COMMANDS), []),
                                 'just' if expected else None)
                tracked = source.replace("'just'", "'cmp'")
                found = self.problems({'t.py': tracked, 'nested.sh': 'python3 -c ' + shlex.quote(tracked)})
                self.assertEqual(sum('calls `cmp`' in issue for issue in found), 2 if expected else 0, found)
        source = "launchers = [unknown()]\nfor launch in launchers: launch(['just'])"
        with self.assertRaisesRegex(ValueError, 'cannot resolve'):
            CAPS.just_consumer({"run": source, "shell": "python"}, vars(COMMANDS), [])
        self.assertIn('cannot resolve', self.problems({'t.py': source})[0])

    def test_unittest_runs_aliases_in_module_name_order(self):
        source = textwrap.dedent("""\
            import unittest
            from subprocess import run
            class State: pass
            state = State()
            state.launch = print
            class ZUse(unittest.TestCase):
                def test_use(self): state.launch(['just'])
            class MSetup(unittest.TestCase):
                def test_setup(self): state.launch = run
            AUse = ZUse
            ZZAgain = ZUse
            unittest.main()
            """)
        self.assertEqual(self.recorded_commands(source), [['just'], ['just']])
        self.assertEqual(CAPS.just_consumer({"run": source, "shell": "python"}, vars(COMMANDS), []), 'just')
        self.assertIn('calls `cmp`', self.problems({'suite.py': source.replace("'just'", "'cmp'")})[0])

    def test_context_lifecycle_commands_and_entry_result(self):
        for prefix, keyword in (('', 'with'), ('async ', 'async with')):
            method = '__a' if prefix else '__'
            source = ("from subprocess import run\nimport asyncio\nclass Manager:\n"
                      f"    {prefix}def {method}enter__(self):\n        run(['cmp'])\n        return run\n"
                      f"    {prefix}def {method}exit__(self, *exc): run(['diff'])\n"
                      f"{prefix}def probe():\n    {keyword} Manager() as launch:\n        launch(['xxd'])\n        return\n"
                      + ("asyncio.run(probe())" if prefix else "probe()"))
            with self.subTest(keyword=keyword):
                self.assertEqual(self.recorded_commands(source), [['cmp'], ['xxd'], ['diff']])
                for command in ('cmp', 'xxd', 'diff'):
                    script = source.replace(repr(command), "'just'")
                    self.assertEqual(CAPS.just_consumer({"run": script, "shell": "python"}, vars(COMMANDS), []), 'just')
                found = self.problems({'manager.py': source})
                self.assertEqual(len(found), 3, found)
        source = ("from external import acquire\nclass Manager:\n    __enter__ = acquire\n"
                  "    def __exit__(self, *exc): pass\nwith Manager(): pass")
        with self.assertRaisesRegex(ValueError, 'cannot resolve'):
            CAPS.just_consumer({"run": source, "shell": "python"}, vars(COMMANDS), [])
        self.assertIn('cannot resolve', self.problems({'t.py': source})[0])

    def test_discovery_resolves_imported_suite_bindings(self):
        for path, cls in (('.claude/hooks/test_guard.py', 'Guard'), ('scripts/tests/managed_review_bundle.py', 'ReviewBundle')):
            fixture = Path(path)
            source = f'from {fixture.stem} import {cls} as Alias\nimport unittest\nunittest.main()'
            for step in ({'shell': 'python', 'run': source}, {'run': 'python3 -c ' + shlex.quote(source)}):
                with self.subTest(path=path, step=step):
                    self.assertEqual(CAPS.just_consumer(dict(step, **{'working-directory': str(fixture.parent)}),
                                                        vars(COMMANDS), []), path)
        suite = "from unittest import TestCase\nfrom subprocess import run\nclass Case(TestCase):\n    def test_command(self): run(['cmp'])"
        source = "from exposed import Case as Alias\nimport unittest\nunittest.main()"
        found = self.problems({'support.py': suite, 'bridge.py': 'import support\nCase = support.Case',
                               'exposed.py': 'import bridge\nCase = bridge.Case',
                               'entry.py': source, 'embedded.sh': 'python3 -c ' + shlex.quote(source)})
        self.assertEqual(len(found), 2, found)
        self.assertTrue(all(issue.startswith('support.py:4: calls `cmp`') for issue in found), found)
        for other in ('from absent import Case', 'from exposed import Case', 'import exposed\nCase = exposed.Case'):
            refused = self.problems({'entry.py': source, 'exposed.py': other})
            self.assertTrue(any('cannot resolve' in issue for issue in refused), refused)

    def test_package_initializers_run_before_suite_imports(self):
        files = {
            'pkg/__init__.py': "from subprocess import run\nrun(['cmp'])",
            'pkg/data/__init__.py': "from pkg import run\nrun(['diff'])",
            'pkg/data/suite.py': 'from unittest import TestCase\nclass Case(TestCase): pass',
            'entry.py': 'from pkg.data.suite import Case\nimport unittest\nunittest.main()',
        }
        found = self.problems(files)
        self.assertTrue(any(issue.startswith('pkg/__init__.py:2: calls `cmp`') for issue in found), found)
        self.assertTrue(any(issue.startswith('pkg/data/__init__.py:2: calls `diff`') for issue in found), found)
        self.assertFalse(any('not read' in issue for issue in found), found)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name, body in files.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(body.replace("'diff'", "'just'").replace("'cmp'", "'just'"))
            for statement in ('from pkg.data.suite import Case', 'import pkg.data.suite',
                              'import pkg', 'from pkg import run'):
                self.assertEqual(CAPS.just_consumer({'shell': 'python', 'run': statement}, vars(COMMANDS), [], root), 'just')
            (root / 'pkg/data/__init__.py').unlink()
            with self.assertRaisesRegex(ValueError, 'cannot resolve'):
                CAPS.just_consumer({'shell': 'python', 'run': 'import pkg.data.suite'}, vars(COMMANDS), [], root)

    def test_loop_reads_mutated_list_elements(self):
        for setup, update in (('[print]', 'alias.append(run)'), ('[print, print]', 'alias[1] = run'),
                              ('[print]', 'add(run)'), ('[print]', 'alias.extend([run])')):
            source = ("from subprocess import run\nitems = " + setup + "\nalias = items\nadd = alias.append\n"
                      "for launch in items:\n    launch(['just'])\n    if launch is print: " + update)
            with self.subTest(update=update):
                self.assertEqual(self.recorded_commands(source), [['just']])
                self.assertEqual(CAPS.just_consumer({'shell': 'python', 'run': source}, vars(COMMANDS), []), 'just')
                found = self.problems({'loop.py': source.replace("'just'", "'cmp'")})
                self.assertTrue(any('calls `cmp`' in issue for issue in found), found)

        source = ("from subprocess import run\nitems = [('first', print)]\nfor name, launch in items:\n"
                  "    launch(['just'])\n    if name == 'first': items.append(('next', run))")
        self.assertEqual(self.recorded_commands(source), [['just']])
        self.assertEqual(CAPS.just_consumer({'shell': 'python', 'run': source}, vars(COMMANDS), []), 'just')

        cleared = "from subprocess import run\nitems = [run]\nitems.clear()\nfor launch in items: launch(['just'])"
        self.assertEqual(self.recorded_commands(cleared), [])
        self.assertIsNone(CAPS.just_consumer({'shell': 'python', 'run': cleared}, vars(COMMANDS), []))

        replaced = ("from subprocess import run\nitems = [print]\nitems.clear()\nitems.append(run)\n"
                    "for launch in items: launch(['just'])")
        self.assertEqual(self.recorded_commands(replaced), [['just']])
        self.assertEqual(CAPS.just_consumer({'shell': 'python', 'run': replaced}, vars(COMMANDS), []), 'just')

        conditional = ("import os\nfrom subprocess import run\nitems = [run]\n"
                       "if os.getenv('FORGE_WARDEN_TEST_CLEAR'):\n    items.clear()\n"
                       "for launch in items: launch(['just'])")
        self.assertEqual(self.recorded_commands(conditional), [['just']])
        self.assertEqual(CAPS.just_consumer({'shell': 'python', 'run': conditional}, vars(COMMANDS), []), 'just')

    def test_stored_argv_at_process_calls(self):
        for argv in ("['cmp']", "('cmp',)"):
            base = "from subprocess import run\nclass Task: argv = " + argv + "\n"
            for value in ('Task.argv', 'Task().argv'):
                for argument in (value, 'args=' + value):
                    source = base + 'run(' + argument + ')'
                    with self.subTest(source=source):
                        found = self.problems({'argv.py': source, 'inline.sh': 'python3 -c ' + shlex.quote(source)})
                        self.assertEqual(sum('calls `cmp`' in issue for issue in found), 2, found)
                        self.assertIsNone(CAPS.just_consumer({'shell': 'python', 'run': source.replace('cmp', 'git')},
                                                            vars(COMMANDS), []))
        for argv in ("[unknown()]", "['git']\nTask.argv[0] = unknown()"):
            source = "from subprocess import run\nclass Task: argv = " + argv + "\nrun(Task.argv)"
            self.assertIn('cannot resolve', self.problems({'argv.py': source})[0])

    def test_unresolved_executable_refuses_every_process_form(self):
        imports = "import asyncio, os, pty, shlex, subprocess, sys\n"
        for call in ("subprocess.run([head])", "subprocess.Popen(args=[head])",
                     "subprocess.run(['git'], executable=head)", "asyncio.create_subprocess_exec(head)",
                     "pty.spawn([head])", "os.execvp(head, ['arg0'])", "os.spawnlp(0, head, 'arg0')"):
            source = imports + call.replace('head', "f'{os.getenv(\"PROGRAM\")}'")
            with self.subTest(call=call):
                issues = self.problems({'entry.py': source, 'inline.sh': 'python3 -c ' + shlex.quote(source)})
                self.assertEqual(len(issues), 2, issues)
                self.assertTrue(all('cannot resolve Python process executable' in issue for issue in issues), issues)
        for head in ("'git' + shlex.quote(sys.argv[1])", "sys.argv[1]", "f'{sys.argv[1]}'"):
            source = imports + f"command = [{head}]\nsubprocess.run(command)"
            self.assertIn('cannot resolve Python process executable', self.problems({'entry.py': source})[0])
        source = imports + "name = 'cmp'\nsubprocess.run([f'{name}'])"
        self.assertIn('calls `cmp`', self.problems({'entry.py': source})[0])

    def test_fixture_context_keeps_the_verified_inode(self):
        producer = load('golden_output')
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / 'fixture'
            manifest = Path(directory) / 'receipt.json'
            data = b'\x7fELFFORGE_WARDEN_TEST_POLICY_ROOT'
            binary.write_bytes(data)
            inputs = {'source': '0' * 64}
            subject = 'source-sha256:' + hashlib.sha256(json.dumps(inputs, separators=(',', ':')).encode()).hexdigest()
            receipt = dict(schema='forge-warden.fixture-cli.v1', artifact_sha256=hashlib.sha256(data).hexdigest(),
                           cfg=['fw_fixture_cli', 'test'], inputs=inputs, subject=subject)
            manifest.write_text(json.dumps(receipt))
            with producer.verified_fixture(binary, manifest, subject) as (fd, named, home, actual):
                descriptor = Path(f'/proc/self/fd/{fd}')
                self.assertEqual(named.parent, Path(home))
                self.assertEqual(named.readlink(), descriptor)
                self.assertEqual(named.name, 'forge-warden')
                self.assertTrue(binary.samefile(named))
                binary.unlink()
                binary.write_bytes(b'replacement')
                self.assertEqual(named.read_bytes(), data)
                self.assertEqual(actual, receipt)
            self.assertFalse(named.is_symlink())
            with self.assertRaises(OSError):
                os.fstat(fd)
            with self.assertRaisesRegex(ValueError, 'artifact digest mismatch'):
                with producer.verified_fixture(binary, manifest, subject):
                    self.fail('opened a replacement artifact')

    def test_only_open_fixture_executables_are_admitted(self):
        filename = 'scripts/golden_output.py'
        producer = (ROOT / filename).read_text()
        self.assertEqual(self.problems({filename: producer}), [])
        context = '\nwith verified_fixture(binary, manifest, subject) as (fd, named, home, receipt):\n'
        for suffix in ("subprocess.run(['forge-warden'])", "subprocess.run([args.binary])",
                       "subprocess.run([f'/proc/self/fd/{fd}x'], pass_fds=(fd,))",
                       "subprocess.run([f'/proc/self/fd/{fd}'])",
                       "subprocess.run([f'{home}/forge-warden'], executable=f'/proc/self/fd/{fd}', pass_fds=(fd,))",
                       "command = [f'/proc/self/fd/{fd}']\nsubprocess.run(command, pass_fds=(fd,))"):
            with self.subTest(suffix=suffix):
                issues = self.problems({filename: producer + context + '    ' + suffix})
                self.assertEqual(len(issues), 1, issues)
                self.assertRegex(issues[0], r'cannot resolve|calls `forge-warden`')

    def test_context_decorators_wrap_each_call(self):
        for prefix, decorator in (('', 'contextmanager'), ('async ', 'asynccontextmanager')):
            source = (f"import asyncio\nfrom contextlib import {decorator}\nfrom subprocess import run\n"
                      f"@{decorator}\n{prefix}def manager():\n    run(['cmp'])\n    try: yield\n"
                      "    finally: run(['diff'])\n@manager()\n" + prefix + "def work(): run(['xxd'])\n"
                      + ("asyncio.run(work())" if prefix else "work()"))
            self.assertEqual(self.recorded_commands(source), [['cmp'], ['xxd'], ['diff']])
            for command in ('cmp', 'xxd', 'diff'):
                self.assertEqual(CAPS.just_consumer({'shell': 'python', 'run': source.replace(repr(command), "'just'")},
                                                    vars(COMMANDS), []), 'just')
            self.assertEqual(len(self.problems({'manager.py': source})), 3)

    def test_generator_managers_suspend_around_the_body(self):
        for prefix, decorator, keyword in (('', 'contextmanager', 'with'), ('async ', 'asynccontextmanager', 'async with')):
            for transfer in ('return', 'raise ValueError'):
                source = (f"import asyncio\nfrom contextlib import {decorator}, suppress\nfrom subprocess import run\n"
                          "class State: pass\nstate = State()\nstate.launch = print\n"
                          f"@{decorator}\n{prefix}def manager(launch):\n    state.launch = launch\n"
                          "    try:\n        yield launch\n    finally:\n        launch(['diff'])\n        state.launch = print\n"
                          f"{prefix}def check():\n    {keyword} manager(run) as invoke:\n"
                          f"        state.launch(['cmp'])\n        invoke(['xxd'])\n        {transfer}\n"
                          "with suppress(ValueError): " + ("asyncio.run(check())" if prefix else "check()")
                          + "\nstate.launch(['late'])")
                with self.subTest(decorator=decorator, transfer=transfer):
                    self.assertEqual(self.recorded_commands(source), [['cmp'], ['xxd'], ['diff']])
                    for command in ('cmp', 'xxd', 'diff'):
                        script = source.replace(repr(command), "'just'")
                        self.assertEqual(CAPS.just_consumer({'shell': 'python', 'run': script}, vars(COMMANDS), []), 'just')
                    found = self.problems({'suite.py': source})
                    for command in ('cmp', 'xxd', 'diff'):
                        self.assertTrue(any(f'calls `{command}`' in issue for issue in found), found)
        source = textwrap.dedent("""\
            from contextlib import contextmanager
            from subprocess import run
            class State:
                @contextmanager
                def active(self):
                    self.launch = run
                    yield self.launch
                    self.launch = print
            state = State()
            with state.active() as launch:
                launch(['cmp'])
            state.launch(['just'])
            """)
        self.assertEqual(self.recorded_commands(source), [['cmp']])
        self.assertIsNone(CAPS.just_consumer({'shell': 'python', 'run': source}, vars(COMMANDS), []))
        found = self.problems({'suite.py': source})
        self.assertEqual(len(found), 1, found)
        self.assertIn('calls `cmp`', found[0])
        for body in ('pass', 'yield 1\n    yield 2', 'if flag: yield 1'):
            source = f'from contextlib import contextmanager\n@contextmanager\ndef manager():\n    {body}\nwith manager(): pass'
            with self.assertRaisesRegex(ValueError, 'yield'):
                CAPS.just_consumer({'shell': 'python', 'run': source}, vars(COMMANDS), [])

    def test_standard_contexts_preserve_process_classification(self):
        prefix = "from contextlib import redirect_stdout, redirect_stderr, closing\nfrom io import StringIO\nfrom subprocess import Popen\n"
        for statement, expected in (("with Popen(['git', '--version']) as child: child.wait()", None),
                                    ("with Popen(['just']) as child: child.wait()", 'just'),
                                    ("with closing(StringIO()) as out, redirect_stdout(out), redirect_stderr(out): print('just')", None)):
            source = prefix + statement
            self.assertEqual(CAPS.just_consumer({'shell': 'python', 'run': source}, vars(COMMANDS), []), expected)
        for expr in ('StringIO()', 'closing(StringIO())', 'redirect_stdout(StringIO())', 'redirect_stderr(StringIO())',
                     "open('/dev/null')", "Popen(['git', '--version'])"):
            source = prefix + "with " + expr + ": raise ValueError\nPopen(['just'])"
            self.assertIsNone(CAPS.just_consumer({'shell': 'python', 'run': source}, vars(COMMANDS), []))
            self.assertEqual(self.problems({'manager.py': source.replace("'just'", "'cmp'")}), [])
        source = prefix + "class Item:\n    def close(self): Popen(['just'])\nwith closing(Item()): pass"
        with self.assertRaisesRegex(ValueError, 'context manager'):
            CAPS.just_consumer({'shell': 'python', 'run': source}, vars(COMMANDS), [])
        self.assertIn('context manager', self.problems({'close.py': source})[0])

    def test_expansion_at_positional_cwd_is_refused(self):
        source = ("from subprocess import run\nrun(['bash', 'dogfood.sh'], -1, None, None, None, None, "
                  "None, True, False, *options)")
        with self.assertRaisesRegex(ValueError, 'starred Python process arguments'):
            CAPS.just_consumer({"run": source, "shell": "python"}, vars(COMMANDS), [])

    def test_python_ordering_resolves_process_arguments_at_the_call(self):
        def consumer(source):
            return CAPS.just_consumer({"shell": "python", "run": "from subprocess import run\n" + source},
                                      vars(COMMANDS), [])

        for source in ("print(len('ready'))", "def size(value): return len(value.strip())\nprint(size(' ready '))",
                       "from pathlib import Path\nprint(Path.cwd())"):
            with self.subTest(source=source):
                self.assertIsNone(consumer(source))

        for setup, argument in (
            ("def split(arg): return ['just', arg]", "split('fmt')"),
            ("def command(): return ['just', 'fmt']", "command()"),
            ("import shlex as words", """words.split("'just' fmt")"""),
            ("from shlex import split as words", """words("'just' fmt")"""),
            ("from shlex import split\ndef split(_): return 'just'", "[split('true')]"),
        ):
            with self.subTest(setup=setup):
                source = setup + "\nrun(" + argument + ")"
                self.assertEqual(consumer(source), "just")
                found = self.problems({"t.py": "from subprocess import run\n" + source.replace("just", "cmp")})
                self.assertEqual(len(found), 1, found)
                self.assertIn("calls `cmp`", found[0])

        for setup, argument in (("def split(arg): return lookup(arg)", "split('true')"),
                                ("import shlex", "shlex.split('true', comments=flag)"),
                                ("import shlex as words\ndef split(_): return ['just']\nwords.split = split",
                                 "words.split('true')"),
                                ("from other import split", "split('true')")):
            with self.subTest(setup=setup), self.assertRaisesRegex(ValueError, "Python process command"):
                consumer(setup + "\nrun(" + argument + ")")

    def test_fixed_executable_with_variable_arguments(self):
        for program in ("git", "cmp"):
            source = f"import subprocess\ndef git(*args): return subprocess.run(['{program}', *args])\ngit('status')"
            for path, script in (("wrapper.py", source), ("embedded.sh", "python3 - <<'PY'\n" + source + "\nPY")):
                with self.subTest(program=program, path=path):
                    problems = self.problems({path: script})
                    if program == "git":
                        self.assertEqual(problems, [])
                    else:
                        self.assertEqual(len(problems), 1)
                        self.assertIn("calls `cmp`", problems[0])
            self.assertIsNone(CAPS.just_consumer({"shell": "python", "run": source}, vars(COMMANDS), []))
        for source, expected in (
            ("subprocess.run(['bash', 'other.sh', *args])", None),
            ("subprocess.run(['bash', 'scripts/tests/dogfood.sh', *args])", "scripts/tests/dogfood.sh"),
            ("subprocess.run(['python3', '.claude/hooks/guard.py', *args, 'stop'])", ".claude/hooks/guard.py"),
            ("subprocess.run(args, executable='git')", None),
        ):
            step = {"shell": "python", "run": "import subprocess; " + source}
            with self.subTest(source=source):
                self.assertEqual(CAPS.just_consumer(step, vars(COMMANDS), []), expected)
        path = "scripts/worktree_diff.py"
        with patch.object(COMMANDS, "DECLARED", COMMANDS.DECLARED - {"git"}):
            problems = self.problems({path: (ROOT / path).read_text()})
            self.assertTrue(any("calls `git`" in problem for problem in problems), problems)

    def test_starred_process_arguments_preserve_script_operands(self):
        for template in (
            "os.execvp('bash', [*ARGS])",
            "os.spawnvp(os.P_WAIT, 'bash', [*ARGS])",
            "os.posix_spawnp('bash', [*ARGS], {})",
            "os.execlp('bash', *ARGS)",
            "os.spawnlp(os.P_WAIT, 'bash', *ARGS)",
            "subprocess.run([*ARGS])",
            "subprocess.run(['env', *ARGS])",
            "subprocess.run(['ignored', *ARGS], executable='env')",
            "subprocess.run(*[ARGS])",
            "asyncio.run(asyncio.create_subprocess_exec(*ARGS))",
        ):
            for vector, expected in (("['bash', 'scripts/tests/dogfood.sh']", "scripts/tests/dogfood.sh"),
                                     ("['bash', 'unrelated.sh']", None)):
                with self.subTest(template=template, vector=vector):
                    source = "import asyncio, os, subprocess; " + template.replace("ARGS", vector)
                    self.assertEqual(CAPS.just_consumer({"run": source, "shell": "python"}, vars(COMMANDS), []), expected)
            unknown = template.replace("*[ARGS]", "*dynamic").replace("ARGS", "dynamic")
            source = "import asyncio, os, subprocess; " + unknown
            with self.subTest(unknown=unknown):
                with self.assertRaisesRegex(ValueError, "starred Python process arguments"):
                    CAPS.just_consumer({"run": source, "shell": "python"}, vars(COMMANDS), [])
                self.assertEqual(self.problems({"t.py": source}),
                                 ["t.py: not read: cannot resolve starred Python process arguments"])

    def test_effective_process_executable(self):
        for wrapper in ("subprocess.Popen", "subprocess.run", "subprocess.call", "subprocess.check_call",
                        "subprocess.check_output", "asyncio.create_subprocess_exec", "asyncio.create_subprocess_shell"):
            args = ("'probe', '--version'" if wrapper.endswith("_exec") else
                    "'true'" if wrapper.endswith("_shell") else "['probe', '--version']")
            for executable, expected in (("'just'", "just"), ("'true'", None)):
                with self.subTest(wrapper=wrapper, executable=executable):
                    source = f"import asyncio, subprocess; {wrapper}({args}, executable={executable})"
                    if wrapper.endswith("_shell"):
                        with self.assertRaisesRegex(ValueError, "interpreter"):
                            CAPS.just_consumer({"run": source, "shell": "python"}, vars(COMMANDS), [])
                    else:
                        self.assertEqual(CAPS.just_consumer({"run": source, "shell": "python"}, vars(COMMANDS), []), expected)
            for keyword, message in (("executable=dynamic", "process executable"),
                                     ("**{'executable': 'just'}", "unpacked Python process keywords")):
                with self.subTest(wrapper=wrapper, keyword=keyword):
                    source = f"import asyncio, subprocess; {wrapper}({args}, {keyword})"
                    with self.assertRaisesRegex(ValueError, message):
                        CAPS.just_consumer({"run": source, "shell": "python"}, vars(COMMANDS), [])
        for call in ("subprocess.run(['probe', 'scripts/tests/dogfood.sh'], executable='/bin/bash')",
                     "subprocess.Popen(['probe', 'scripts/tests/dogfood.sh'], -1, '/bin/bash')",
                     "asyncio.create_subprocess_exec('probe', 'scripts/tests/dogfood.sh', executable='bash')",
                     "subprocess.run('bash scripts/tests/dogfood.sh', shell=True, executable='bash')"):
            source = "import asyncio, subprocess; " + call
            self.assertEqual(CAPS.just_consumer({"run": source, "shell": "python"}, vars(COMMANDS), []),
                             "scripts/tests/dogfood.sh")
        found = self.problems({"t.py": "import subprocess; subprocess.run(['probe'], executable='cmp')"})
        self.assertEqual(found, ["t.py:1: calls `cmp`, which the gate image does not declare (scripts/script_commands.py)"])

    def test_callable_expression_values(self):
        for value in ("None or target", "True and target", "(alias := target)",
                      "(alias := module) and alias.run", "(module := module.run)"):
            for assigned in (False, True):
                for process in (True, False):
                    with self.subTest(value=value, assigned=assigned, process=process):
                        owner = "subprocess" if process else "builtins"
                        expression = value
                        source = ("import subprocess, builtins\n"
                                  f"module = {owner}\ntarget = module.run\n")
                        if assigned:
                            source += "launch = " + expression + "\n"
                            expression = "launch"
                        source += f"({expression})(['just'])"
                        if not process:
                            source = source.replace(".run", ".print")
                        consumer = CAPS.just_consumer({"shell": "python", "run": source}, vars(COMMANDS), [])
                        self.assertEqual(consumer, "just" if process else None)

    def test_finally_sees_interrupted_bindings_without_leaking_them(self):
        cases = (
            "cleanup = run\ntry:\n    open('config')\n    cleanup = print",
            "try:\n    if enabled:\n        cleanup = run\n        open('config')\n        cleanup = print",
            "try:\n    pass\nexcept ValueError:\n    pass\nelse:\n    cleanup = run\n    open('config')\n    cleanup = print",
            "try:\n    open('config')\nexcept OSError:\n    cleanup = run\n    open('fallback')\n    cleanup = print",
            "try:\n    open('config')\nexcept* OSError:\n    cleanup = run\n    raise",
        )
        for protected in cases:
            with self.subTest(protected=protected):
                source = ("from subprocess import run\nenabled = True\ncleanup = print\n" + protected +
                          "\nfinally:\n    cleanup(['cmp'])\ncleanup(['xxd'])")
                found = self.problems({"t.py": source})
                self.assertEqual(found, [f"t.py:{len(source.splitlines()) - 1}: calls `cmp`, which the gate image "
                                         "does not declare (scripts/script_commands.py)"])
        for final, expected in (("cleanup = print", None), ("cleanup = run", "just")):
            source = ("from subprocess import run\ncleanup = run\ntry:\n    open('config')\nfinally:\n    "
                      + final + "\ncleanup(['just'])")
            self.assertEqual(CAPS.just_consumer({"shell": "python", "run": source}, vars(COMMANDS), []), expected)

    def test_named_os_process_program_and_argv(self):
        for api in ("execvp", "execvpe", "execve", "spawnv", "spawnve", "spawnvp", "spawnvpe"):
            file, args = ("path", "argv") if api == "execve" else ("file", "args")
            mode = "os.P_WAIT, " if api.startswith("spawn") else ""
            env = ", env={}" if api.endswith("e") else ""
            for program, argv, expected in (
                (f"{file}='just'", "['just', '--version']", "just"),
                ("'bash'", "['bash', 'scripts/tests/dogfood.sh']", "scripts/tests/dogfood.sh"),
                (f"{file}='bash'", "['bash', 'scripts/tests/dogfood.sh']", "scripts/tests/dogfood.sh"),
                (f"{file}='true'", "['true']", None),
            ):
                with self.subTest(api=api, program=program, argv=argv):
                    source = f"import os; os.{api}({mode}{program}, {args}={argv}{env})"
                    step = {"shell": "python", "run": source}
                    self.assertEqual(CAPS.just_consumer(step, vars(COMMANDS), []), expected)
            with self.subTest(api=api, dynamic=True):
                source = f"import os; os.{api}({mode}{file}=unknown, {args}=['just']{env})"
                with self.assertRaisesRegex(ValueError, "cannot resolve Python process command"):
                    CAPS.just_consumer({"shell": "python", "run": source}, vars(COMMANDS), [])
                source = f"import os; os.{api}({mode}{file}='bash', {args}=unknown{env})"
                with self.assertRaisesRegex(ValueError, "cannot resolve gate script operand"):
                    CAPS.just_consumer({"shell": "python", "run": source}, vars(COMMANDS), [])

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
