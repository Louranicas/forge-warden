#!/usr/bin/env python3
"""Every catalogue recipe still applies to this tree (WP-PLANT-ANCHORS).

A source refactor can delete the line a plant edits. Q25 did that to G-14's anchor, and only the
integrator gate saw it, an hour into a train run. This check is text only (no checkout, no cargo),
so it belongs in the fast gate. Against the working tree, each case must be one that `run.py`
could apply:
- an `old` anchor occurs exactly once in its file;
- an `append` target exists;
- a `create` path does not exist yet, and its parent is a real directory.
In every case the path is relative, stays out of `.git`, and its file and parents are not symlinks.
The recipe shape is `run.py`'s own `recipe_kind`, so there is one definition of a valid recipe.

It also derives every number in the plants README's count sentence from the catalogue and the gate
capacity (ledger Q53: a clean merge once kept "109" for a catalogue of 110). The sentence has one
pinned shape; a README without it, or with it twice, is refused rather than skipped.

usage: python3 -I plants/anchors.py   (exit 1 names every case that would not apply)
"""
import importlib.util
import json
from pathlib import Path
import re
import sys

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location('plants_runner', HERE / 'run.py')
RUN = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RUN)
CAPS_SPEC = importlib.util.spec_from_file_location('plant_tools', HERE.parent / 'scripts/plant_tools.py')
CAPS = importlib.util.module_from_spec(CAPS_SPEC)
CAPS_SPEC.loader.exec_module(CAPS)

# The one allowed shape of the README's count sentence (whitespace collapsed). Every number in it is
# a capture that readme_problems compares with a value derived from the catalogue or plant_tools.
COUNT_SENTENCE = re.compile(
    r'This revision implements (?P<plants>\d+) named plants over (?P<rows>\d+) guardrail rows '
    r'\(the gate admits at most (?P<capacity>\d+): `GATE_PLANT_CAPACITY` in `scripts/plant_tools\.py`\); '
    r'Q3-07/Q3-10 strengthen G-45/G-24, and (?P<z3>\d+) `z3-\*` cases exercise G-45\. '
    r'Of the remaining register rows, (?P<pending>\d+) are explicitly PENDING'
    r'(?: \((?P<pending_ids>G-\d+(?:, G-\d+)*)\))? and (?P<retired>\d+) are RETIRED '
    r'\((?P<retired_ids>G-\d+(?:, G-\d+)*)\)')


def problems(root, cases):
    """Why each case would not apply to the tree at `root` (empty when every case applies)."""
    found = []
    for case in cases:
        name = case.get('id', '?')
        try:
            kind = RUN.recipe_kind(case)
        except ValueError as error:
            found.append(f'{name}: {error}')
            continue
        raw = case['path']
        relative = Path(raw)
        if (not raw or relative.is_absolute() or relative.as_posix() != raw
                or any(part in ('', '.', '..') for part in raw.split('/')) or '.git' in relative.parts):
            found.append(f'{name}: plant path {raw!r} must be relative, normalised and outside .git')
            continue
        # As in run.py's create: each directory below the root must be real; the root itself may
        # sit under a symlinked ancestor (/home -> /var/home).
        directory, real = root, True
        for part in relative.parts[:-1]:
            directory = directory / part
            if directory.is_symlink() or not directory.is_dir():
                real = False
                break
        if not real:
            found.append(f'{name}: {raw} has a missing or symlink parent')
            continue
        path = root / relative
        if kind == 'create':
            if path.exists() or path.is_symlink():
                found.append(f'{name}: create path {raw} already exists')
            continue
        if path.is_symlink() or not path.is_file():
            found.append(f'{name}: {raw} is not a regular file')
            continue
        if kind == 'old':
            count = path.read_text(encoding='utf-8').count(case['old']) if case['old'] else 0
            if count != 1:
                found.append(f'{name}: anchor occurs {count} times in {raw}, not exactly once')
    return found


def readme_problems(readme, catalog, capacity):
    """Each number of the README's count sentence that the catalogue or capacity contradicts."""
    text = ' '.join(readme.split())
    matches = list(COUNT_SENTENCE.finditer(text))
    if len(matches) != 1:
        return [f'README: the count sentence occurs {len(matches)} times, not exactly once '
                '(its shape is COUNT_SENTENCE in plants/anchors.py)']
    said = matches[0].groupdict()
    cases, pending = catalog['cases'], catalog['pending_guardrails']
    retired = catalog['retired_guardrails']
    z3 = [case for case in cases if case['id'].startswith('z3-')]
    derived = {
        'plants': str(len(cases)),
        'rows': str(len({case['guardrail'] for case in cases})),
        'capacity': str(capacity),
        'z3': str(len(z3)),
        'pending': str(len(pending)),
        'pending_ids': ', '.join(row['id'] for row in pending),
        'retired': str(len(retired)),
        'retired_ids': ', '.join(row['id'] for row in retired),
    }
    found = [f'README: {key} says {said[key] or ""!r}, the catalogue gives {value!r}'
             for key, value in derived.items() if (said[key] or '') != value]
    if any(row.get('status') != 'RETIRED' or not row.get('decision') for row in retired):
        found.append('README: a retired register row has no RETIRED status or no decision')
    if {row['id'] for row in retired} & {case['guardrail'] for case in cases}:
        found.append('README: a retired register row still has a runnable case')
    if any(case['guardrail'] != 'G-45' for case in z3):
        found.append('README: a z3-* case does not exercise G-45')
    return found


def main():
    root = HERE.parent
    catalog = json.loads((HERE / 'catalog.json').read_text(encoding='utf-8'))
    cases = catalog['cases']
    found = problems(root, cases)
    found += readme_problems((HERE / 'README.md').read_text(encoding='utf-8'), catalog,
                             CAPS.GATE_PLANT_CAPACITY)
    for line in found:
        print('plant anchors: ' + line, file=sys.stderr)
    if found:
        return 1
    print(f'plant anchors: all {len(cases)} catalogue recipes apply to this tree, '
          'and the README count sentence matches the catalogue')
    return 0


if __name__ == '__main__':
    sys.exit(main())
