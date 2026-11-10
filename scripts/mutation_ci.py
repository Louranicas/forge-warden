#!/usr/bin/env python3
"""Measured mutation sharding; every shard rediscovers the same complete inventory."""
import hashlib
import json
import math
import os
from pathlib import Path
import re
import subprocess
import sys

# Hosted5478daa: baseline24+337s, largest completed mutant1+193s.
# Conservative serial workers avoid assuming unmeasured --jobs2/4 throughput.
MAX_PER_SHARD = 20
MAX_SHARDS = 256
BASELINE_SECONDS = 361
MUTANT_SECONDS = 194
SETUP_SECONDS = 180
MARGIN = 1.5


def matrix(total, digest):
    if type(total) is not int or not 0 <= total <= MAX_PER_SHARD * MAX_SHARDS:
        raise ValueError('mutation count exceeds reviewed matrix capacity (5120); review capacity')
    if not re.fullmatch('[0-9a-f]{64}', digest):
        raise ValueError('invalid inventory digest')
    shards = max(1, math.ceil(total / MAX_PER_SHARD))
    size = math.ceil(total / shards)
    minutes = math.ceil((SETUP_SECONDS + MARGIN * (BASELINE_SECONDS + size * MUTANT_SECONDS)) / 60)
    return {'include': [dict(index=i, shards=shards, total=total, digest=digest, minutes=minutes)
                        for i in range(shards)]}


def aggregate(plan, shards):
    return plan == shards == 'success'


def inventory(scope):
    # A caller's target/config must not redirect or reduce mutation coverage.
    env = dict(os.environ)
    env.pop('CARGO_TARGET_DIR', None)
    command = ['cargo', 'mutants', '--no-config', '--cap-lints', 'true',
               '--workspace', '--no-shuffle', '--jobs', '1', '--cargo-arg=--locked']
    empty_diff = False
    if scope == 'diff':
        base = env.get('BASE_SHA', '')
        if not re.fullmatch('[0-9a-f]{40}', base):
            raise ValueError('BASE_SHA must be the full trusted base SHA')
        diff = subprocess.check_output(['git', 'diff', base + '...HEAD', '--', 'crates/*/src/**.rs'])
        Path('pr.diff').write_bytes(diff)
        empty_diff = not diff
        command += ['--in-diff', 'pr.diff']
    elif scope != 'full':
        raise ValueError('unknown mutation scope')
    # cargo-mutants exits early without JSON for an empty diff. Only that proven-empty
    # byte sequence maps to []; nonempty diffs must produce a valid tool inventory.
    result = b'[]' if empty_diff else subprocess.check_output(command + ['--list', '--json'], env=env)
    mutants = json.loads(result)
    if not isinstance(mutants, list) or any(not isinstance(m, dict) for m in mutants):
        raise ValueError('invalid cargo-mutants inventory')
    canonical = json.dumps(mutants, sort_keys=True, separators=(',', ':')).encode()
    digest = hashlib.sha256(canonical).hexdigest()
    return command, env, len(mutants), digest


def main(args):
    if args == ['aggregate']:
        if not aggregate(os.environ.get('PLAN_RESULT'), os.environ.get('SHARD_RESULT')):
            raise ValueError('mutation planning or a shard failed, was cancelled, skipped, or missing')
        return
    if len(args) != 2 or args[0] not in ('plan', 'run'):
        raise ValueError('usage: mutation_ci.py plan|run diff|full OR aggregate')
    command, env, total, digest = inventory(args[1])
    plan = matrix(total, digest)
    if args[0] == 'plan':
        payload = json.dumps(plan, separators=(',', ':'))
        print(payload)
        with open(os.environ['GITHUB_OUTPUT'], 'a', encoding='utf-8') as output:
            output.write('matrix=' + payload + '\n')
        return
    index = int(os.environ['SHARD_INDEX'])
    if not 0 <= index < len(plan['include']):
        raise ValueError('invalid shard index')
    expected = plan['include'][index]
    supplied = dict(index=index, shards=int(os.environ['SHARD_COUNT']),
                    total=int(os.environ['MUTANT_TOTAL']), digest=os.environ['INVENTORY_DIGEST'],
                    minutes=int(os.environ['SHARD_MINUTES']))
    if supplied != expected:
        raise ValueError('shard inventory, denominator or measured budget differs from plan')
    print(json.dumps(expected), flush=True)
    # Baseline remains enabled on EVERY shard; no warm gate substitutes for copied-tree tests.
    subprocess.run(command + ['--shard', f'{index}/{expected["shards"]}', '-vV'],
                   env=env, check=True)


if __name__ == '__main__':
    try:
        main(sys.argv[1:])
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError) as error:
        sys.exit('mutation CI: ' + str(error))
