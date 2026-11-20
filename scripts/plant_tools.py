#!/usr/bin/env python3
"""Reviewed plant capabilities and fail-closed runtime/hosted provisioning checks."""
import argparse
import ast
import fnmatch
import hashlib
import json
import os
from pathlib import Path
import re
import runpy
import shlex
import subprocess
import sys
import tomllib

sys.dont_write_bytecode = True

PROBES = (
    ('bash', ('bash', '--version')),
    ('cargo', ('cargo', '--version')),
    ('cargo-clippy', ('cargo', 'clippy', '--version')),
    ('cargo-mutants', ('cargo', 'mutants', '--version')),
    ('coreutils', ('env', '--version')),
    ('git', ('git', '--version')),
    ('just', ('just', '--version')),
    ('python3', ('python3', '-I', '--version')),
    ('rustc', ('rustc', '--version')),
    ('rustup', ('rustup', '--version')),
    ('sh', ('sh', '-c', 'exit 0')),
    # ssh-keygen has no portable --version; an option parse is not an availability proof.
    ('ssh-keygen', ('ssh-keygen', '-Y', 'find-principals', '-f', '/dev/null', '-s', '/dev/null')),
)
PROBE_NAMES = {name for name, _ in PROBES}
HOSTED = {'bash', 'coreutils', 'git', 'python3', 'sh', 'ssh-keygen'}
JUST_CONSUMERS = {
    'scripts/tests/dogfood.sh',
    'scripts/checkout_just.py', 'scripts/ci_fast.sh', 'scripts/ci_local.sh', 'scripts/check_plants.py',
    '.githooks/pre-commit', '.githooks/pre-push',
}
JUST_TESTS = {
    '.claude/hooks/test_guard.py': {
        'Guard.test_gate_launchers_validate_script_paths', 'Guard.test_gate_checkout_identity',
        'Guard.test_checkout_git_symlink_is_refused_before_git_discovery', 'Guard.test_fixed_just_recipes_select_checkout',
        'Guard.test_hook_installation_only_writes_selected_repository', 'Guard.test_recipe_helpers_are_checked_before_execution',
        'Guard.test_just_defers_vocabulary_lookup',
    },
    'scripts/tests/managed_review_bundle.py': {
        'ReviewBundle.test_review_captures_patch_before_optional_ranking',
        'ReviewBundle.test_review_and_patch_refuse_redirected_worktree', 'ReviewBundle.test_copied_review_evidence_is_owner_only',
        'ReviewBundle.test_review_publishes_only_completed_outputs',
    },
}
INDEPENDENT_TESTS = {
    'Guard.test_stop_runs_gate_for_hidden_and_untracked_changes',
    'Guard.test_malformed_events_fail_closed',
    'Guard.test_protected_paths_all_spellings',
    'Guard.test_shell_tokenization_and_rust_writes',
    'Guard.test_read_and_ordinary_edit_positive_controls',
    'Guard.test_alternative_edit_tools_refuse',
    'Guard.test_rust_spacing_and_complete_edit',
    'Guard.test_toml_semantics_and_member_inheritance',
    'Guard.test_owner_protected_manifest_refuses_even_unchanged_content',
    'Guard.test_invalid_edit_shape_refuses',
    'ReviewBundle.test_candidate_cannot_supply_instructions_or_policy',
    'ReviewBundle.test_missing_trusted_policy_never_falls_back',
    'ReviewBundle.test_missing_trusted_contract_refuses',
    'ReviewBundle.test_symlink_contract_refuses',
    'ReviewBundle.test_wrong_head_or_base_refuses',
    'ReviewBundle.test_modified_worktree_instructions_are_ignored',
    'ReviewBundle.test_modified_helper_refuses',
    'ReviewBundle.test_output_inside_candidate_refuses',
    'ReviewBundle.test_candidate_fsmonitor_cannot_execute',
    'ReviewBundle.test_clean_filter',
    'ReviewBundle.test_smudge_filter',
    'ReviewBundle.test_textconv',
    'ReviewBundle.test_external_diff',
    'ReviewBundle.test_candidate_attributes_diff_driver',
    'ReviewBundle.test_hooks_path',
    'ReviewBundle.test_include_path',
    'ReviewBundle.test_include_if',
}


def requirements(cases):
    required = set()
    for case in cases:
        names = case.get('required_tools')
        if (not isinstance(names, list) or not names or any(not isinstance(n, str) for n in names)
                or len(names) != len(set(names)) or set(names) - PROBE_NAMES):
            raise ValueError('invalid required_tools for plant ' + str(case.get('id')))
        if 'cargo' in case and 'cargo' not in names:
            raise ValueError('cargo recipe omits cargo: ' + case['id'])
        if case.get('cargo', [None])[0] == 'clippy' and 'cargo-clippy' not in names:
            raise ValueError('clippy recipe omits cargo-clippy: ' + case['id'])
        if 'command' in case and case['command'][0] not in names:
            raise ValueError('command recipe omits its executable: ' + case['id'])
        required.update(names)
    if not required:
        raise ValueError('plant catalog has no tool requirements')
    return required


def preflight(required, root, env):
    versions = tomllib.loads((root / 'scripts/tool-versions.toml').read_text())
    unknown = set(required) - PROBE_NAMES
    if unknown:
        raise ValueError('plant preflight: unknown tool ' + min(unknown))
    for name, argv in PROBES:
        if name not in required:
            continue
        try:
            result = subprocess.run(argv, cwd=root, env=env, stdin=subprocess.DEVNULL,
                                    capture_output=True, text=True, timeout=15, check=False)
        except (OSError, subprocess.TimeoutExpired) as error:
            raise ValueError('plant preflight: unavailable tool ' + name) from error
        # OpenSSH deliberately returns 255 for empty signatures; verify the named diagnostic.
        available = result.returncode == 0
        if name == 'ssh-keygen':
            available = result.returncode == 255 and 'signature' in result.stderr.lower()
        if not available:
            raise ValueError('plant preflight: unavailable tool ' + name)
        if name == 'cargo-mutants' and not re.search(
                r'\bcargo-mutants ' + re.escape(versions['cargo_mutants']) + r'(?:\s|$)', result.stdout):
            raise ValueError('plant preflight: cargo-mutants version differs from scripts/tool-versions.toml')
        print('plant preflight: ' + name + ' OK', flush=True)


# Reviewed bounds, not candidate-provided ranges. Evidence/headroom: docs/CI_BOUNDS.md.
# A budget change needs the same protected-code review as a workflow change.
CI_TIMEOUTS = {
    "ci.yml": {"gate": 45, "msrv": 10, "supply-chain": 10, "workflow-audit": 10,
               "mutation": 5, "mutation-plan": 10, "mutation-shards": "${{ matrix.minutes }}",
               "dogfood": 10, "vocabulary": 10},
    "mutation-baseline.yml": {"route": 5, "full-workspace": 5, "mutation-plan": 10,
                              "mutation-shards": "${{ matrix.minutes }}"},
}


# CI-GATE-BUDGET (ledger Q49): the gate job's time grows with the plant catalogue, and at 101
# plants it no longer fitted 20 minutes on a hosted runner. The 45-minute budget above holds, with
# the headroom docs/CI_BOUNDS.md derives, for at most this many catalogue cases. A catalogue that
# outgrows it fails here, in the fast gate, instead of being cancelled on a hosted runner.
GATE_PLANT_CAPACITY = 140


def check_gate_capacity(root):
    """Refuse a catalogue larger than the one the gate job's reviewed budget was derived for."""
    count = len(json.loads((root / 'plants/catalog.json').read_text())['cases'])
    if count > GATE_PLANT_CAPACITY:
        raise ValueError(
            f'plant catalogue has {count} cases; the ci/gate budget was derived for at most '
            f'{GATE_PLANT_CAPACITY}: re-derive it (docs/CI_BOUNDS.md) before adding more')


def workflow_lines(text):
    """Reject multiline quoted/flow scalars; never count their contents as YAML fields."""
    block_indent = None
    for line in text.splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        indent = len(line) - len(line.lstrip(" "))
        if block_indent is not None and indent > block_indent:
            continue  # Contents of an explicitly admitted literal block are data.
        block_indent = None
        match = re.fullmatch(r" *(?:- )?[A-Za-z_][A-Za-z_0-9-]*:(?: (.*))?", line)
        if not match:
            raise ValueError("CI timeout contract: unsupported YAML line shape")
        value = match[1] or ""
        if value.startswith((" ", "\t")):
            raise ValueError("CI timeout contract: extra scalar separation whitespace refused")
        if value == "|" or value in ("|-", "|+") and line.lstrip(" -").startswith("run:"):
            block_indent = indent
        elif value.startswith("'"):
            if not re.fullmatch(r"'(?:[^']|'')*'(?: +#[^\n]*)?", value):
                raise ValueError("CI timeout contract: multiline quoted scalar refused")
        elif value.startswith('"'):
            if not re.fullmatch(r'"(?:[^"\\]|\\.)*"(?: +#[^\n]*)?', value):
                raise ValueError("CI timeout contract: multiline quoted scalar refused")
        elif value.startswith("["):
            if not re.fullmatch(r"\[[a-z][a-z0-9_-]*(?:, *[a-z][a-z0-9_-]*)*\]", value):
                raise ValueError("CI timeout contract: unsupported flow scalar")
        elif value.startswith(("{", "&", "*", "!", ">", "|")):
            raise ValueError("CI timeout contract: unsupported YAML scalar")
        yield line


def check_timeouts(root):
    """Admit this repository's literal block-style jobs; unknown YAML shapes refuse."""
    for filename, budgets in CI_TIMEOUTS.items():
        text = (root / ".github/workflows" / filename).read_text()
        roots = []
        jobs = {}
        job = None
        in_jobs = False
        fields = set()
        for line in list(workflow_lines(text)):
            if not line.strip() or line.lstrip().startswith("#"):
                continue
            if "\t" in line[:len(line) - len(line.lstrip())]:
                raise ValueError("CI timeout contract: tab indentation")
            if not line.startswith(" "):
                match = re.fullmatch(r"([a-z][a-z-]*):(?: [A-Za-z0-9_-]+)?", line)
                if not match or match[1] not in {"name", "on", "permissions", "concurrency", "env", "jobs"}:
                    raise ValueError("CI timeout contract: unsupported workflow root shape")
                if match[1] in roots:
                    raise ValueError("CI timeout contract: duplicate workflow key")
                roots.append(match[1])
                in_jobs = line == "jobs:"
                job = None
            elif in_jobs:
                indent = len(line) - len(line.lstrip(" "))
                if indent == 2:
                    match = re.fullmatch(r"  ([a-z][a-z0-9-]*):", line)
                    if not match or match[1] in jobs or match[1] not in budgets:
                        raise ValueError("CI timeout contract: unsupported, duplicate or unreviewed job")
                    job = match[1]
                    jobs[job] = None
                    fields = set()
                elif indent == 4 and job is not None:
                    match = re.fullmatch(r"    ([a-z][a-z-]*):(.*)", line)
                    allowed = {"name", "runs-on", "timeout-minutes", "if", "env", "steps"}
                    allowed.update({"mutation-plan": {"outputs"},
                                    "mutation-shards": {"needs", "strategy"},
                                    "mutation": {"needs"}, "full-workspace": {"needs"}}.get(job, set()))
                    # WP-RUNNER-UP: only the baseline's route job has outputs of its own, and only
                    # the baseline's plan waits on it. ci.yml gains no field.
                    allowed.update({("mutation-baseline.yml", "route"): {"outputs"},
                                    ("mutation-baseline.yml", "mutation-plan"): {"needs"}}.get((filename, job), set()))
                    if not match or match[1] not in allowed:
                        raise ValueError("CI timeout contract: unsupported job field")
                    if match[1] in fields:
                        raise ValueError("CI timeout contract: duplicate job field")
                    fields.add(match[1])
                    if match[1] == "timeout-minutes":
                        if match[2] != " " + str(budgets[job]):
                            raise ValueError("CI timeout contract: unjustified timeout for " + filename + ":" + job)
                        jobs[job] = budgets[job]
                elif job is None or indent < 4:
                    raise ValueError("CI timeout contract: unsupported job indentation")
        if set(jobs) != set(budgets) or any(value is None for value in jobs.values()):
            raise ValueError("CI timeout contract: missing job or timeout in " + filename)


# Exact reviewed orchestration: protects the single required check, every matrix member,
# failure propagation, honest scope/flags, inventory binding and the measured budget formula.
# These pins are not external authority; changing them requires owner review (ADR0006/0009).
MUTATION_CONTRACT = {
    ('ci.yml', 'mutation-plan'): 'ead40464329ae65d88e971c65024ddb0c59685f300545e5b7ff1bfee9ca35ab6',
    ('ci.yml', 'mutation-shards'): 'fb283a1342631e9837b7a77313433a5e5607772011fba36402666667866d491e',
    ('ci.yml', 'mutation'): 'dcbad6a3b73cbc3e2f5c16fc8e307aa18ed1225ca61f94c4c61d5b8e529d8e02',
    # WP-RUNNER-UP v8: plan and shards gain the routed header and the guarded guest bootstrap only.
    ('mutation-baseline.yml', 'mutation-plan'): '388a92e62af3f3f0b1e7b340fea30821d25531c2dc103b37f7020b9de7f1d35f',
    ('mutation-baseline.yml', 'mutation-shards'): 'e9867e6cf81ef1edaa343ae218822bd4070e9d782ad795be6d18d14f8683901a',
    ('mutation-baseline.yml', 'full-workspace'): '952432a0135022009d488de17045a82fc742520493518173f5e02fda1e4c6277',
}
MUTATION_DRIVER_SHA256 = '2f490c3fc3f3946d4d35a80cc9d0d1fca3bafaabadb2ca3386d5bfa0efebbf5f'


def check_mutation_contract(root):
    for (filename, job), digest in MUTATION_CONTRACT.items():
        text = (root / '.github/workflows' / filename).read_text()
        match = re.search(r'^  ' + job + r':\n(.*?)(?=^  [a-z][a-z0-9-]*:|^  # Repository|\Z)',
                          text, re.M | re.S)
        if not match or hashlib.sha256((match[0].rstrip() + '\n').encode()).hexdigest() != digest:
            raise ValueError('mutation orchestration requires review: ' + filename + ':' + job)
    if hashlib.sha256((root / 'scripts/mutation_ci.py').read_bytes()).hexdigest() != MUTATION_DRIVER_SHA256:
        raise ValueError('mutation inventory/budget driver requires review')


def workflow_document(text):
    import yaml

    class WorkflowLoader(yaml.BaseLoader):
        def construct_mapping(self, node, deep=False):
            mapping = super().construct_mapping(node, deep=deep)
            if len(mapping) != len(node.value):
                raise ValueError("duplicate workflow mapping key")
            return mapping

    try:
        workflow = yaml.load(text, Loader=WorkflowLoader)
        jobs = workflow['jobs']
        if not isinstance(jobs, dict) or not all(
                isinstance(job['steps'], list) and all(isinstance(step, dict) for step in job['steps'])
                for job in jobs.values()):
            raise ValueError('invalid workflow steps')
        return workflow
    except (yaml.YAMLError, KeyError, TypeError) as error:
        raise ValueError('invalid workflow jobs') from error


def workflow_jobs(text):
    return workflow_document(text)['jobs']


def installed_tool(step, values):
    uses = step.get('uses', '')
    if not isinstance(uses, str):
        raise ValueError('workflow step uses must be an action reference')
    if uses.partition('@')[0] != 'taiki-e/install-action':
        return None
    if uses != 'taiki-e/install-action@' + values['install_action']:
        raise ValueError('CI install-action pin drift')
    inputs = step.get('with')
    if not isinstance(inputs, dict) or not isinstance(inputs.get('tool'), str):
        raise ValueError('CI install-action needs a tool')
    return inputs['tool']


def plant_needs_just(arguments, cases, unknown):
    parser = argparse.ArgumentParser(add_help=False, exit_on_error=False)
    runner = runpy.run_path(str(Path(__file__).resolve().parents[1] / 'plants/run.py'))
    runner['add_arguments'](parser, required_inputs=False)
    parser.add_argument('-h', '--help', action='store_true')
    try:
        options, extra = parser.parse_known_args([arg if arg is not None else unknown for arg in arguments or ()])
    except (argparse.ArgumentError, SystemExit) as error:
        raise ValueError('cannot resolve plant runner options before Just installation') from error
    if options.help:
        return False
    all_possible = not options.case or any(unknown in arg for arg in [*options.case, *extra])
    return any('just' in case['required_tools'] and (all_possible or case['id'] in options.case)
               and (options.runner is None or 'cargo' in case) for case in cases)


def unittest_consumer(arguments, cwd, fixture, unknown):
    unresolved = fixture or 'unittest'
    if arguments is None or any(arg is None or unknown in arg for arg in arguments):
        return unresolved
    parser = argparse.ArgumentParser(add_help=False, exit_on_error=False)
    parser.add_argument('-k', action='append', default=[])
    parser.add_argument('-h', '--help', action='store_true')
    parser.add_argument('-v', '-q', '-f', '-b', '-c', '--verbose', '--quiet', '--failfast',
                        '--buffer', '--catch', '--locals', action='store_true')
    parser.add_argument('--durations')
    parser.add_argument('names', nargs='*')
    try:
        options, extra = parser.parse_known_intermixed_args(arguments)
    except (argparse.ArgumentError, SystemExit) as error:
        raise ValueError('cannot resolve unittest selection before Just installation') from error
    if options.help:
        return None
    if extra or 'discover' in options.names or (not fixture and not options.names):
        return unresolved
    patterns = [pattern if '*' in pattern else '*' + pattern + '*' for pattern in options.k]
    for name in options.names or ['']:
        path, module = fixture, '__main__'
        if path is None:
            if name.endswith('.py'):
                module = Path(os.path.normpath(name)).with_suffix('').as_posix().replace('/', '.')
                path, name = os.path.normpath(cwd / name), ''
            else:
                parts = name.split('.')
                for size in range(1, len(parts) + 1):
                    path = (cwd / Path(*parts[:size])).as_posix() + '.py'
                    if path in JUST_TESTS:
                        module, name = '.'.join(parts[:size]), '.'.join(parts[size:])
                        break
            if path not in JUST_TESTS:
                return unresolved
        dependent = JUST_TESTS[path]
        suite = next(iter(dependent)).partition('.')[0]
        if name and name != suite:
            if name.partition('.')[0] != suite or name not in INDEPENDENT_TESTS:
                return path
        elif not patterns or any(fnmatch.fnmatchcase(module + '.' + method, pattern)
                                 for method in dependent for pattern in patterns):
            return path
    return None


READ_ONLY_PROGRAMS = {'wc', 'head', 'tail', 'cut', 'tr', 'basename', 'dirname', 'readlink', 'stat'}


def check_checkout_write(path, reads, workspace):
    for read in reads:
        if path is None or read.is_relative_to(path):
            raise ValueError('cannot resolve modified gate script ' + str(read.relative_to(workspace)))


class GateShell:
    def __init__(self, options, interpreter, functions=None):
        self.lastpipe, self.exported = options
        self.interpreter = interpreter
        self.functions = {} if functions is None else functions

    def child_options(self, word):
        startup = getattr(word, 'lastpipe', None)
        enabled, exported = self.lastpipe and self.exported, self.exported
        if startup is not None:
            if startup.environment is not None:
                enabled, exported = startup.environment
            if startup.option is not None:
                enabled = startup.option
        return enabled, exported


def just_consumer(step, commands, cases, root=None, inherited_env=(), checkout_writes=None, pending_reads=None):
    lexer = commands['WorkflowLexer']
    pythonpath = bashopts = commands['UNSET_PATH']
    for environment in (*inherited_env, step.get('env', {})):
        pythonpath = environment.get('PYTHONPATH', pythonpath) if isinstance(environment, dict) else None
        bashopts = environment.get('BASHOPTS', bashopts) if isinstance(environment, dict) else None
    directory = step.get('working-directory', '.')
    if not isinstance(directory, str):
        raise ValueError('cannot resolve gate working-directory before Just installation')
    directory = lexer.location(lexer.workspace, directory)
    run = step.get('run', '')
    shell = step.get('shell', 'bash')
    if not isinstance(run, str) or not isinstance(shell, str):
        raise ValueError('gate run and shell must be strings before Just installation')
    program, _, arguments_format = shell.partition(' ')
    if shell in ('sh', 'bash') or (program in commands['SHELLS'] and arguments_format == '{0}'):
        reader = commands['shell_calls']
    elif shell == 'python' or (commands['PYTHONS'].fullmatch(program) and arguments_format in ('{0}', '-I {0}')):
        reader = commands['python_calls']
    else:
        raise ValueError('unsupported gate shell before Just installation')
    root = root or lexer.workspace
    files = None
    writes = set() if checkout_writes is None else checkout_writes

    if pending_reads is None:
        pending_reads = set()
    read_scopes = []
    deferred_reads = {}
    invocations = 0

    def require_original(path):
        for reads in read_scopes:
            reads.add(path)
        if None in writes or any(path.is_relative_to(changed) for changed in writes):
            raise ValueError('cannot resolve modified gate script ' + str(path.relative_to(lexer.workspace)))

    def record_write(path, joined):
        if path is not None:
            path = Path(os.path.normpath(path))
            if not path.is_relative_to(lexer.workspace):
                return
            file = root / path.relative_to(lexer.workspace)
            if (None in writes or any(path.is_relative_to(changed) for changed in writes)
                    or file.resolve() != file or file.is_file() and file.stat().st_nlink > 1):
                path = None
        outstanding = (reads for (kind, owner), reads in deferred_reads.items()
                       if kind != 'pending-child' or owner not in joined)
        for reads in (pending_reads, *outstanding, *read_scopes):
            check_checkout_write(path, reads, lexer.workspace)
        writes.add(path)

    def inspect(calls, active, shell=None, conditional=False, joined=frozenset(), options=(False, False), interpreter=None):
        nonlocal files, invocations
        owns_shell = shell is None
        if owns_shell:
            shell = GateShell(options, interpreter)
        frames = []
        for _, word, external, arguments, cwd in commands['resolve_module_calls'](calls, lexer):
            if isinstance(word, commands['ShellDefinition']):
                shell.functions[word] = None if conditional else word
                continue
            if isinstance(word, commands['CheckoutScope']):
                if word == 'bashopts-assignment':
                    if shell.interpreter in ('sh', 'dash', None):
                        shell.lastpipe = word.enabled or (conditional or shell.interpreter != 'dash') and shell.lastpipe
                        if word.autoexport:
                            shell.exported = True
                elif word == 'joined-child':
                    deferred_reads.pop(('pending-child', word.owner), None)
                elif word == 'wait':
                    if not conditional and 'wait' not in shell.functions:
                        deferred_reads.pop(('background', shell), None)
                elif word == 'end':
                    finished = shell
                    kind, shell, conditional, joined = frames.pop()
                    if finished is not shell:
                        deferred_reads.pop(('trap', finished), None)
                    if kind in ('begin', 'trap', 'background', 'process-substitution', 'pending-child'):
                        reads = read_scopes.pop()
                        if kind != 'begin':
                            owner = getattr(kind, 'owner', None) or shell
                            deferred_reads.setdefault((kind, owner), set()).update(reads)
                else:
                    frames.append((word, shell, conditional, joined))
                    if word == 'after-join':
                        joined |= word.owners
                    if word in ('begin', 'trap', 'background', 'process-substitution', 'pending-child'):
                        read_scopes.append(set())
                    if word == 'pipeline-stage' and shell.interpreter != 'dash' and shell.lastpipe:
                        raise ValueError('cannot resolve Bash lastpipe ownership')
                    if word in ('child', 'pending-child'):
                        shell = GateShell(shell.child_options(word), getattr(word, 'interpreter', None))
                    elif word in ('background', 'process-substitution', 'subshell', 'pipeline-stage'):
                        shell = GateShell((shell.lastpipe, shell.exported), shell.interpreter, shell.functions.copy())
                    conditional |= word in ('conditional', 'begin', 'trap')
                continue
            if word.startswith('()'):
                continue
            if isinstance(word, commands['CheckoutRead']):
                require_original(Path(word))
                continue
            if isinstance(word, commands['CheckoutWrite']):
                record_write(Path(word) if word else None, joined)
                continue
            if word.startswith('(module)'):
                consumer = unittest_consumer(arguments, cwd.relative_to(lexer.workspace), None, lexer.unknown)
                if consumer:
                    return consumer
                continue
            marker = next((prefix for prefix in commands['RUNS'] if word.startswith(prefix)), '')
            target = Path(word.removeprefix(marker))
            if not external and not marker and word in shell.functions:
                definition = shell.functions[word]
                invocation = ('function', word)
                invocations += 1
                if definition is None or invocation in active or len(active) >= 12 or invocations > 128:
                    raise ValueError('cannot resolve conditional or recursive gate function ' + word)
                consumer = inspect(definition.calls(word, lexer, cwd, shell.functions), (*active, invocation), shell, conditional, joined)
                if consumer:
                    return consumer
                continue
            if word == 'shopt' and shell.interpreter != 'dash':
                args = arguments or ()
                unknown = arguments is None or any(arg is None or lexer.unknown in arg for arg in args)
                enables = any(arg and arg.startswith('-') and 's' in arg for arg in args)
                if unknown or enables and 'lastpipe' in args:
                    shell.lastpipe = True
                elif args == ['-u', 'lastpipe'] and not conditional:
                    shell.lastpipe = False
                continue
            if target.name == 'just':
                return 'just'
            program = word if '/' not in word else target.name
            if word in ('export', 'declare', 'typeset', 'local', 'readonly') and arguments:
                export = True if word == 'export' else None
                flags = set()
                operands = arguments
                for arg in arguments:
                    if arg is None or lexer.unknown in arg.split('=', 1)[0]:
                        raise ValueError('cannot resolve shell function export')
                    if not arg.startswith(('-', '+')):
                        break
                    operands = operands[1:]
                    if arg == '--':
                        break
                    if arg.startswith('-'):
                        flags.update(arg[1:])
                    if 'x' in arg[1:]:
                        export = arg.startswith('-')
                    if word == 'export' and 'n' in arg[1:]:
                        export = arg.startswith('+')
                if 'f' not in flags and 'p' not in flags:
                    for operand in operands:
                        key, equals, value = (operand or lexer.unknown).partition('=')
                        uncertain = lexer.unknown in key
                        if not uncertain and key != 'BASHOPTS':
                            continue
                        if (equals or uncertain) and shell.interpreter in ('sh', 'dash', None):
                            enabled = uncertain or lexer.unknown in value or commands['may_enable_lastpipe'](value)
                            shell.lastpipe = enabled or (conditional or shell.interpreter != 'dash') and shell.lastpipe
                            if getattr(word, 'shell_state', (None, None, False))[2] is not False:
                                shell.exported = True
                        if export is not None:
                            shell.exported = export or (conditional or uncertain) and shell.exported
                if operands and 'f' in flags and (word == 'export' or 'x' in flags and 'p' not in flags):
                    raise ValueError('cannot resolve shell function export')
            if (word == 'unset' and not external and arguments and 'BASHOPTS' in arguments
                    and shell.interpreter == 'dash' and not conditional and '-f' not in arguments):
                shell.lastpipe = shell.exported = False
            if word == 'unset' and not external and shell.functions and arguments:
                options = [arg for arg in arguments if arg and arg.startswith('-')]
                if not any('v' in flag for flag in options):
                    unknown = any(arg is None or lexer.unknown in arg for arg in arguments)
                    for name in list(shell.functions):
                        if unknown or name in arguments:
                            if conditional or unknown or '-f' not in options:
                                shell.functions[name] = None
                            else:
                                del shell.functions[name]
            if not marker and ('/' not in word or word.startswith(commands['STANDARD_BIN'])):
                if program in ('cp', 'mv') and arguments and len(arguments) >= 2 and all(
                        arg and Path(arg).is_absolute() for arg in arguments):
                    for arg in arguments if program == 'mv' else arguments[-1:]:
                        record_write(Path(arg), joined)
                elif (program not in (commands['PREFIXES'] - {'xargs'}) | commands['SHELLS'] | READ_ONLY_PROGRAMS and
                      not commands['PYTHONS'].fullmatch(program) and
                      program not in (':', '.', 'source', 'true', 'false', 'echo', 'printf', 'cat', 'export', 'unset',
                                      'set', 'declare', 'typeset', 'local', 'readonly', 'test', '[', '[[', 'pwd', 'trap', 'wait') and
                      (program, arguments) != ('git', ['--version'])):
                    record_write(None, joined)
                continue
            path = Path(os.path.normpath(cwd / target))
            if not path.is_relative_to(lexer.workspace):
                if marker:
                    raise ValueError('cannot resolve gate script outside the workspace before Just installation')
                record_write(None, joined)
                continue
            require_original(path)
            path = path.relative_to(lexer.workspace).as_posix()
            if path in JUST_TESTS:
                if unittest_consumer(arguments, cwd.relative_to(lexer.workspace), path, lexer.unknown):
                    return path
                continue
            if path == '.claude/hooks/guard.py':
                if arguments is None or (len(arguments) == 1 and
                        (arguments[0] in ('stop', None) or lexer.unknown in arguments[0])):
                    return path
                continue
            if path == 'plants/run.py':
                if plant_needs_just(arguments, cases, lexer.unknown):
                    return path
                continue
            if path in JUST_CONSUMERS:
                return path
            if marker == '(module-file)':
                continue
            file = root / path
            if file.is_symlink() or file.resolve() != file:
                raise ValueError('cannot resolve linked gate script before Just installation')
            if not file.is_file() or path in active or len(active) >= 12:
                raise ValueError('cannot resolve unsafe or recursive gate script ' + path)
            if files is None:
                files = set(commands['tracked'](root))
            if path not in files:
                raise ValueError('cannot resolve untracked gate script ' + path)
            options = shell.child_options(word)
            env_path = getattr(word, 'pythonpath', pythonpath)
            ignore_env = getattr(word, 'ignore_env', False)
            stdin = getattr(word, 'stdin', ())
            if not marker:
                kind, bang = commands['classify'](root, path)
                if bang and kind not in ('python', 'shell'):
                    raise ValueError('cannot resolve gate script interpreter ' + path)
                if bang:
                    if ('shebang', path) in active:
                        raise ValueError('cannot resolve recursive gate script interpreter ' + path)
                    with file.open(encoding='utf-8') as first:
                        interpreter = first.readline().strip()[2:]
                    nested = commands['shell_calls'](interpreter + ' ' + shlex.quote(str(lexer.workspace / path)),
                                                     lexer=lexer, directory=cwd, pythonpath=env_path, stdin=stdin)
                    consumer = inspect(nested, (*active, ('shebang', path)), joined=joined, options=options)
                    if consumer:
                        return consumer
                    continue
            body = file.read_text(encoding='utf-8')
            if marker in ('(py)', '(isolated-py)', '(cwdpath-py)'):
                origin = cwd if getattr(word, 'cwd_imports', False) else (lexer.workspace / path).parent
                nested = commands['python_calls'](body, lexer=lexer, directory=cwd, filename=path, import_directory=origin,
                                                  pythonpath=env_path, ignore_env=ignore_env, isolated=marker == '(isolated-py)', stdin=stdin)
            else:
                nested = commands['shell_calls'](body, lexer=lexer, directory=cwd, pythonpath=env_path,
                                                 shell_state=getattr(word, 'shell_state', None), stdin=stdin)
            consumer = inspect(nested, (*active, path), shell if marker == '(src)' else None, conditional, joined,
                               options, getattr(word, 'interpreter', None))
            if consumer:
                return consumer

        if owns_shell:
            deferred_reads.pop(('trap', shell), None)

    source = commands['IMPORT_SOURCE']
    token = source.set((root, directory, None, JUST_TESTS))
    audit = commands['AUDIT_CHECKOUT']
    audit_token = audit.set(writes)
    try:
        options = ({'isolated': arguments_format == '-I {0}', 'temporary_script': True}
                   if reader is commands['python_calls'] else {
                       'runner_environment': all(isinstance(env, dict) and 'GITHUB_ENV' not in env
                                                 for env in (*inherited_env, step.get('env', {})))})
        calls = reader(run, lexer=lexer, directory=directory, pythonpath=pythonpath, **options)
        consumer = inspect(calls, (), options=(commands['may_enable_lastpipe'](bashopts), bashopts is not commands['UNSET_PATH']),
                           interpreter=program)
        for (kind, _), reads in deferred_reads.items():
            if kind in ('background', 'process-substitution', 'pending-child'):
                pending_reads.update(reads)
        return consumer
    finally:
        audit.reset(audit_token)
        source.reset(token)


def may_write_runner_environment(step, commands):
    def python_writes(source):
        for statement in ast.parse(source).body:
            if isinstance(statement, ast.Pass) or isinstance(statement, ast.Expr) and isinstance(statement.value, ast.Constant):
                continue
            if isinstance(statement, ast.Expr) and isinstance(call := statement.value, ast.Call):
                if (isinstance(call.func, ast.Name) and call.func.id == 'print' and not call.keywords and
                        all(isinstance(arg, ast.Constant) for arg in call.args)):
                    continue
            return True
        return False

    shell = step.get('shell', 'bash').split()[0]
    if commands['PYTHONS'].fullmatch(shell):
        return python_writes(step['run'])
    tokens = commands['WorkflowLexer'](step['run'].strip(), 1).tokens()
    if any(token[0] != 'word' or not token[1].literal for token in tokens):
        return True
    words = [word.value for _, word in tokens]
    if not words or words[0] in READ_ONLY_PROGRAMS | {':', 'true', 'false', 'echo'} or words == ['git', '--version']:
        return False
    state = words[0]
    if commands['PYTHONS'].fullmatch(state):
        for word in words[1:]:
            if state == 'next-python':
                return python_writes(word)
            state, events = commands['operand_word'](state, word, word)
            for kind, value in events:
                if kind == 'python':
                    return python_writes(value)
            if state is None:
                break
    return True


def check_ci(root):
    """Conservative check of this repository's owned workflow shape, not a general YAML parser."""
    check_timeouts(root)
    check_gate_capacity(root)
    check_mutation_contract(root)
    cases = json.loads((root / 'plants/catalog.json').read_text())['cases']
    required = requirements(cases)
    workflow = workflow_document((root / '.github/workflows/ci.yml').read_text())
    gate = workflow['jobs']['gate']
    if gate.get('runs-on') != 'ubuntu-latest' or 'if' in gate:
        raise ValueError('plant CI requires the unconditional hosted gate')
    steps = gate['steps']
    command = 'python3 -I scripts/check_plants.py'
    entrypoints = [i for i, step in enumerate(steps) if 'scripts/check_plants.py' in step.get('run', '')]
    if len(entrypoints) != 1:
        raise ValueError('plant CI clean-tree entrypoint must occur exactly once')
    entry = entrypoints[0]
    if steps[entry] != {'run': command}:
        raise ValueError('plant CI entrypoint must be unconditional with reviewed fields')
    for step in steps[entry + 1:]:
        if (not re.fullmatch(r'actions/upload-artifact@[0-9a-f]{40}', step.get('uses', ''))
                or step != {'if': 'always()', 'uses': step['uses'],
                            'with': {'name': 'guardrail-plants', 'path': 'target/plants-*'}}):
            raise ValueError('plant CI clean-tree entrypoint must follow every gate command')
    available = set(HOSTED)
    values = tomllib.loads((root / 'scripts/tool-versions.toml').read_text())
    commands = runpy.run_path(str(Path(__file__).with_name('script_commands.py')))
    runner_env = {}
    checkout_writes = set()
    pending_reads = set()
    inherited = workflow.get('env', {}), gate.get('env', {}), runner_env
    for step in steps[:entry]:
        run = step.get('run', '')
        if 'just' not in available:
            step_writes = checkout_writes.copy()
            consumer = just_consumer(step, commands, cases, root, inherited, step_writes, pending_reads)
            if consumer:
                raise ValueError('gate must install Just before running ' + consumer)
        if 'rustup toolchain install' in run and '--component' in run:
            if (step != {'run': run} or not re.fullmatch(
                    r'rustup toolchain install ([0-9.]+) --profile minimal '
                    r'--component rustfmt,clippy && rustup default \1', run)):
                raise ValueError('plant CI compiler install must be unconditional with reviewed fields')
            available.update(('rustup', 'rustc', 'cargo', 'cargo-clippy'))
        elif run and 'just' not in available:
            checkout_writes = step_writes
            if may_write_runner_environment(step, commands):
                runner_env.update({'PYTHONPATH': None, 'BASHOPTS': None})
        tool = installed_tool(step, values)
        if tool is not None:
            name = tool.partition('@')[0]
            if (name not in ('cargo-mutants', 'just') or tool != name + '@' + values[name.replace('-', '_')]
                    or step != {'uses': step['uses'], 'with': {'tool': tool}}):
                raise ValueError('plant CI tool install must be unconditional with reviewed fields')
            available.add(name)
        elif 'just' not in available and 'uses' in step:
            if step['uses'] == 'actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1':
                inputs = step.get('with', {})
                if (set(step) <= {'name', 'uses', 'with'} and isinstance(inputs, dict)
                        and inputs in ({}, {'persist-credentials': 'false'})):
                    continue
                path = inputs.get('path', '.') if isinstance(inputs, dict) else None
                if (set(step) <= {'name', 'uses', 'with', 'if'} and isinstance(path, str)
                        and re.fullmatch(r'[a-zA-Z0-9_./-]+', path) and '..' not in Path(path).parts
                        and not Path(path).is_absolute()):
                    target = commands['WorkflowLexer'].workspace / path
                    if (root / path).resolve() == root / path and not any(
                            changed is None or target.is_relative_to(changed) for changed in checkout_writes):
                        check_checkout_write(target, pending_reads, commands['WorkflowLexer'].workspace)
                        checkout_writes.add(target)
                        continue
            check_checkout_write(None, pending_reads, commands['WorkflowLexer'].workspace)
            checkout_writes.add(None)
            runner_env.update({'PYTHONPATH': None, 'BASHOPTS': None})
    if required - available:
        raise ValueError('plant CI gate lacks required tools: ' + ', '.join(sorted(required - available)))
    return required
