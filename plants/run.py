#!/usr/bin/env python3
"""Re-run named guardrail breaches in a disposable clone; never change the source tree."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import tempfile
import time

HERE = Path(__file__).resolve().parent
LIMIT = 4 * 1024 * 1024


def environment():
    """Neutral Git configuration and offline tools; this is not an execution sandbox."""
    env = {k: v for k, v in os.environ.items() if not k.startswith('GIT_')}
    env.pop('CARGO_TARGET_DIR', None)
    env.update(GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null',
               GIT_NO_REPLACE_OBJECTS='1', CARGO_NET_OFFLINE='true', RUSTUP_AUTO_INSTALL='0')
    return env


def execute(argv, cwd, log, timeout=300):
    """Bound duration and output; a killed or truncated command can never count as caught."""
    started = time.monotonic()
    reason = None
    with log.open('wb') as output:
        try:
            child = subprocess.Popen(argv, cwd=cwd, env=environment(), stdout=output,
                                     stderr=subprocess.STDOUT, start_new_session=True)
        except OSError as error:
            output.write(str(error).encode())
            return {'argv': argv, 'exit': 126, 'error': 'spawn_failed', 'log': log.name,
                    'sha256': None}
        try:
            while child.poll() is None:
                if time.monotonic() - started > timeout:
                    reason = 'timeout'
                elif log.stat().st_size > LIMIT:
                    reason = 'output_limit'
                if reason:
                    os.killpg(child.pid, signal.SIGKILL)
                    break
                time.sleep(0.02)
            code = child.wait()
        finally:
            # Also stop descendants surviving their group leader. This is cleanup, not isolation.
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
    if log.stat().st_size > LIMIT:
        reason = 'output_limit'
    digest = hashlib.sha256(log.read_bytes()).hexdigest() if not reason else None
    return {'argv': argv, 'exit': code, 'error': reason, 'log': log.name, 'sha256': digest}


def classify(baseline, planted, diagnostic):
    if baseline['error'] or baseline['exit'] != 0:
        return 'ERROR_BASELINE'
    if planted['error'] or planted['exit'] < 0:
        return 'ERROR_COMMAND'
    if planted['exit'] == 0:
        return 'ESCAPED'
    return 'CAUGHT' if re.search(diagnostic, planted['text']) else 'ERROR_DIAGNOSTIC'


def edit(root, recipe):
    """Exact once-only edits. Refuse changed anchors and paths outside the private checkout."""
    relative = Path(recipe['path'])
    if relative.is_absolute() or '..' in relative.parts or relative.parts[0] == '.git':
        raise ValueError('unsafe plant path')
    path = root / relative
    if path.resolve() != path.absolute():
        raise ValueError('symlink plant path')
    before = path.read_bytes()
    text = before.decode('utf-8')
    if 'append' in recipe:
        text += recipe['append']
    else:
        anchor = recipe['old']
        if not anchor or text.count(anchor) != 1:
            raise ValueError('plant anchor must occur exactly once')
        text = text.replace(anchor, recipe['new'], 1)
    path.write_text(text, encoding='utf-8')
    return path, before


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, required=True, help='local repository, read only')
    parser.add_argument('--revision', required=True, help='exact full lowercase commit id')
    parser.add_argument('--output', type=Path, required=True, help='new private evidence directory')
    parser.add_argument('--runner', type=Path, help='trusted executable taking CHECKOUT then cargo arguments')
    parser.add_argument('--case', action='append', help='case id; repeat to select; default all runnable')
    args = parser.parse_args()
    if not re.fullmatch('[0-9a-f]{40}', args.revision):
        parser.error('revision must be an exact full lowercase commit id')
    if not args.source.is_dir():
        parser.error('source must be a local repository directory')
    catalog = json.loads((HERE / 'catalog.json').read_text())
    cases = catalog['cases']
    selected = set(args.case or [c['id'] for c in cases])
    if selected - {c['id'] for c in cases}:
        parser.error('unknown case id')
    output = args.output.absolute()
    output.mkdir(mode=0o700, parents=False, exist_ok=False)
    runner = str(args.runner.resolve()) if args.runner else None
    rows = []
    report = {'schema': 'forge-warden.plants.v1', 'revision': args.revision,
              'catalog_sha256': hashlib.sha256((HERE / 'catalog.json').read_bytes()).hexdigest(),
              'register': catalog['register'], 'results': rows,
              'pending_guardrails': catalog['pending_guardrails']}
    with tempfile.TemporaryDirectory(prefix='fw-plants-') as directory:
        root = Path(directory) / 'checkout'
        clone = execute(['git', '-c', 'core.hooksPath=/dev/null', 'clone', '--no-local',
                         '--no-checkout', '--', str(args.source.resolve()), str(root)],
                        output, output / 'clone.log')
        checkout = execute(['git', '-c', 'core.hooksPath=/dev/null', '-C', str(root),
                            'checkout', '--detach', args.revision], output, output / 'checkout.log') if clone['exit'] == 0 and not clone['error'] else clone
        if checkout['exit'] or checkout['error']:
            report['setup_error'] = checkout
        else:
            # Scratch target never shared with another run or mutation worker.
            target = root.parent / 'target'
            for case in cases:
                if case['id'] not in selected:
                    continue
                row = {'id': case['id'], 'guardrail': case['guardrail']}
                rows.append(row)
                command = ([runner, str(root)] if runner else ['cargo']) + case['cargo']
                if not runner:
                    command += ['--target-dir', str(target)]
                baseline = execute(command, root, output / (case['id'] + '-baseline.log'))
                row['baseline'] = baseline
                if baseline['exit'] or baseline['error']:
                    row['status'] = 'ERROR_BASELINE'
                    continue
                snapshots = {root / 'Cargo.lock': (root / 'Cargo.lock').read_bytes()}
                try:
                    path, before = edit(root, case)
                    snapshots[path] = before
                    planted = execute(command, root, output / (case['id'] + '-planted.log'))
                    row['planted'] = planted
                    planted['text'] = (output / planted['log']).read_bytes()[:LIMIT].decode('utf-8', errors='replace')
                    row['status'] = classify(baseline, planted, case['diagnostic'])
                    planted.pop('text')
                except (ValueError, OSError) as error:
                    row.update(status='ERROR_RECIPE', detail=str(error))
                finally:
                    for path, before in snapshots.items():
                        path.write_bytes(before)
                # Do not cache baselines: a broken restore must not contaminate later evidence.
                (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))
    if report.get('setup_error') or any(r['status'] != 'CAUGHT' for r in rows):
        return 1
    # A full survey cannot look green while guardrail recipes remain unimplemented.
    return 2 if not args.case and catalog['pending_guardrails'] else 0


if __name__ == '__main__':
    raise SystemExit(main())
