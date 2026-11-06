#!/usr/bin/env python3
"""Prove the real mutation command rejects a deliberately untested named branch."""
import json
from pathlib import Path
import subprocess
import tempfile


def main():
    # A zero-site mutation selection alone is not a build-positive control.
    baseline = subprocess.run(['cargo', 'clippy', '--workspace', '--all-targets', '--locked', '--offline'],
                              capture_output=True, text=True, timeout=120)
    if baseline.returncode:
        print('ERROR_MUTATION_BUILD:', baseline.stderr)
        return 1
    with tempfile.TemporaryDirectory(prefix='a4-survivor-') as directory:
        command = ['env', '-u', 'CARGO_TARGET_DIR', 'cargo', 'mutants', '--no-config',
                   '--cap-lints', 'true', '--workspace', '--file', 'crates/forge-warden-core/src/render.rs',
                   '--re', 'planted_uncovered_branch',
                   '--output', directory, '--cargo-arg=--locked', '--cargo-arg=--offline']
        result = subprocess.run(command, capture_output=True, text=True, timeout=180)
        print(result.stdout, end='')
        print(result.stderr, end='')
        path = Path(directory) / 'mutants.out/outcomes.json'
        outcomes = json.loads(path.read_text()) if path.exists() else {}
        counts = {key: outcomes.get(key, 0) for key in
                  ('total_mutants', 'caught', 'missed', 'unviable', 'timeout')}
        print('MUTATION_PLANT_COUNTS:', json.dumps(counts, sort_keys=True))
        missed = Path(directory) / 'mutants.out/missed.txt'
        missed_names = missed.read_text().splitlines() if missed.exists() else []
        if missed.exists():
            print('MISSED_NAMES:', missed.read_text(), end='')
        caught = Path(directory) / 'mutants.out/caught.txt'
        if caught.exists():
            print('CAUGHT_NAMES:', caught.read_text())
        if counts['total_mutants'] and not counts['missed']:
            for log in sorted((Path(directory) / 'mutants.out/log').glob('*.log')):
                print('UNEXPECTED_CAUGHT_LOG:', log.name)
                print(log.read_text(errors='replace')[-5000:])
        if result.returncode == 0 and not any(counts[k] for k in ('missed', 'unviable', 'timeout')):
            return 0
        if (result.returncode == 2 and counts['missed'] > 0
                and len(missed_names) == counts['missed']
                and all('planted_uncovered_branch' in name for name in missed_names)
                and counts['unviable'] == counts['timeout'] == 0):
            print('MISSED_SURVIVOR_REJECTED')
            return 1
        print('ERROR_MUTATION_STATUS: unexpected process/outcome combination', result.returncode)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
