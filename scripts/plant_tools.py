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


def check_ci(root):
    """Conservative check of this repository's owned workflow shape, not a general YAML parser."""
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
    before = body.split('      - run: python3 -I scripts/check_plants.py', 1)
    if len(before) != 2:
        raise ValueError('plant CI entrypoint missing')
    body = before[0]
    available = set(HOSTED)
    if re.search(r'rustup toolchain install [0-9.]+ --profile minimal --component rustfmt,clippy', body):
        available.update(('rustup', 'rustc', 'cargo', 'cargo-clippy'))
    installs = re.findall(r'      - uses: taiki-e/install-action@([0-9a-f]{40})[^\n]*\n        with:\n          tool: ([^\s]+)', body)
    values = tomllib.loads((root / 'scripts/tool-versions.toml').read_text())
    for action, tool in installs:
        if action != values['install_action']:
            raise ValueError('plant CI install-action pin drift')
        if tool == 'cargo-mutants@' + values['cargo_mutants']:
            available.add('cargo-mutants')
    if required - available:
        raise ValueError('plant CI gate lacks required tools: ' + ', '.join(sorted(required - available)))
    return required
