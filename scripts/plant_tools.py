#!/usr/bin/env python3
"""Reviewed plant capabilities and fail-closed runtime/hosted provisioning checks."""
import json
import re
import subprocess
import tomllib

PROBES = {
    'bash': ['bash', '--version'],
    'cargo': ['cargo', '--version'],
    'cargo-clippy': ['cargo', 'clippy', '--version'],
    'cargo-mutants': ['cargo', 'mutants', '--version'],
    'coreutils': ['env', '--version'],
    'git': ['git', '--version'],
    'python3': ['python3', '--version'],
    'rustc': ['rustc', '--version'],
    'rustup': ['rustup', '--version'],
    'sh': ['sh', '-c', 'exit 0'],
    # ssh-keygen has no portable --version; an option parse is not an availability proof.
    'ssh-keygen': ['ssh-keygen', '-Y', 'find-principals', '-f', '/dev/null', '-s', '/dev/null'],
}
HOSTED = {'bash', 'coreutils', 'git', 'python3', 'sh', 'ssh-keygen'}


def requirements(cases):
    required = set()
    for case in cases:
        names = case.get('required_tools')
        if (not isinstance(names, list) or not names or any(not isinstance(n, str) for n in names)
                or len(names) != len(set(names)) or set(names) - PROBES.keys()):
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
    for name in sorted(required):
        if name not in PROBES:
            raise ValueError('plant preflight: unknown tool ' + name)
        try:
            result = subprocess.run(PROBES[name], cwd=root, env=env, stdin=subprocess.DEVNULL,
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
    "ci.yml": {"gate": 20, "msrv": 10, "supply-chain": 10, "workflow-audit": 10,
               "mutation": 60, "dogfood": 10, "vocabulary": 10},
    "mutation-baseline.yml": {"full-workspace": 120},
}


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
        if value == "|":
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
        for line in workflow_lines(text):
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
                    if not match or match[1] not in {"name", "runs-on", "timeout-minutes", "if", "env", "steps"}:
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


def check_ci(root):
    """Conservative check of this repository's owned workflow shape, not a general YAML parser."""
    check_timeouts(root)
    cases = json.loads((root / 'plants/catalog.json').read_text())['cases']
    required = requirements(cases)
    workflow = (root / '.github/workflows/ci.yml').read_text()
    match = re.search(r'^  gate:\n(.*?)(?=^  [A-Za-z_][\w-]*:\n|\Z)', workflow, re.M | re.S)
    if not match:
        raise ValueError('plant CI gate job missing')
    body = match[1]
    # The preflight checks these hosted-image assumptions at runtime, too.
    if '    runs-on: ubuntu-latest\n' not in body:
        raise ValueError('plant CI tool provisioning needs review for a new runner image')
    steps = re.split(r'(?=^      - )', body, flags=re.M)[1:]
    available = set(HOSTED)
    values = tomllib.loads((root / 'scripts/tool-versions.toml').read_text())
    found_entrypoint = False
    for raw in steps:
        step = raw.rstrip() + '\n'
        if 'scripts/check_plants.py' in step:
            if step != '      - run: python3 -I scripts/check_plants.py\n':
                raise ValueError('plant CI entrypoint must be unconditional with reviewed fields')
            found_entrypoint = True
            break
        if 'rustup toolchain install' in step and '--component' in step:
            compiler = re.fullmatch(
                r'      - run: rustup toolchain install ([0-9.]+) --profile minimal '
                r'--component rustfmt,clippy && rustup default \1\n', step)
            if not compiler:
                raise ValueError('plant CI compiler install must be unconditional with reviewed fields')
            available.update(('rustup', 'rustc', 'cargo', 'cargo-clippy'))
        if 'taiki-e/install-action' in step:
            install = re.fullmatch(
                r'      - uses: taiki-e/install-action@([0-9a-f]{40})(?: #[^\n]*)?\n'
                r'        with:\n          tool: (cargo-mutants@[^\s]+)\n', step)
            if not install:
                raise ValueError('plant CI tool install must be unconditional with reviewed fields')
            action, tool = install.groups()
            if action != values['install_action']:
                raise ValueError('plant CI install-action pin drift')
            if tool == 'cargo-mutants@' + values['cargo_mutants']:
                available.add('cargo-mutants')
    if not found_entrypoint:
        raise ValueError('plant CI entrypoint missing')
    if required - available:
        raise ValueError('plant CI gate lacks required tools: ' + ', '.join(sorted(required - available)))
    return required
