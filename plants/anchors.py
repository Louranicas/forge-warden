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

usage: python3 -I plants/anchors.py   (exit 1 names every case that would not apply)
"""
import importlib.util
import json
from pathlib import Path
import sys

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location('plants_runner', HERE / 'run.py')
RUN = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RUN)


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


def main():
    root = HERE.parent
    cases = json.loads((HERE / 'catalog.json').read_text(encoding='utf-8'))['cases']
    found = problems(root, cases)
    for line in found:
        print('plant anchors: ' + line, file=sys.stderr)
    if found:
        return 1
    print(f'plant anchors: all {len(cases)} catalogue recipes apply to this tree')
    return 0


if __name__ == '__main__':
    sys.exit(main())
