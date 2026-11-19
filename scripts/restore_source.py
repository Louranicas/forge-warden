"""Restore the source paths changed in each layer of a saved stash."""
import importlib.util
import os
import re
import sys
from pathlib import Path

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("source_stash", Path(__file__).with_name("source_stash.py"))
source_stash = importlib.util.module_from_spec(spec)
spec.loader.exec_module(source_stash)

git = source_stash.git
SOURCES = source_stash.SOURCES


def index_entries(env, *paths):
    entries = {}
    for row in git("ls-files", "--stage", "-z", "--", *paths, env=env).split(b"\0")[:-1]:
        header, name = row.split(b"\t", 1)
        entries[Path(os.fsdecode(name))] = header
    return entries


def refuse(path):
    sys.exit(f"Source changed since the stash was saved. Resolve the overlap at {str(path)!r} before restoring.")


def snapshot(revision, env):
    git("read-tree", revision, env=env)
    changed = git("diff", "--no-ext-diff", "--no-renames", "--name-only", "-z",
                  "--ignore-submodules=none", revision, "--", SOURCES, env=env)
    return index_entries(env, SOURCES), {Path(os.fsdecode(p)) for p in changed.split(b"\0")[:-1]}


def changed_paths(base, target):
    return sorted(path for path in base.keys() | target.keys() if base.get(path) != target.get(path))


def matches_working_file(path, exists, entries, dirty, env):
    if exists != (path in entries) or path in dirty:
        return False
    if not exists or path.is_symlink():
        return True
    mode, blob, _ = entries[path].split()
    expected = (git("cat-file", "blob", blob, env=env) if mode == b"120000"
                else source_stash.checkout_bytes(path, blob, env))
    return path.read_bytes() == expected


def restore_paths(layer, revision, paths, env):
    if paths:
        git("--literal-pathspecs", "restore", "--source=" + revision, "--" + layer,
            "--pathspec-from-file=-", "--pathspec-file-nul", env=env,
            data=b"".join(os.fsencode(path) + b"\0" for path in paths))


def restore(stash, scratch, live):
    baseline = {**live, "GIT_INDEX_FILE": str(scratch / "base")}
    live = {**live, "GIT_NO_REPLACE_OBJECTS": "1"}
    parent = git("rev-parse", "--verify", stash + "^1", env=live).decode().strip()
    saved = {**live, "GIT_INDEX_FILE": str(scratch / "saved")}
    base, base_dirty = snapshot(parent, baseline)
    worktree, worktree_dirty = snapshot(stash, saved)
    git("read-tree", stash + "^2", env=saved)
    staged = index_entries(saved, SOURCES)
    worktree_changes = changed_paths(base, worktree)
    staged_changes = changed_paths(base, staged)
    source_stash.worktree_diff.check_index_flags(live, SOURCES, set(worktree_changes).union(staged_changes))
    worktree_paths = []
    for path in worktree_changes:
        if any(p.is_symlink() or (p.exists() and not p.is_dir()) for p in path.parents):
            refuse(path)
        exists = os.path.lexists(path)
        if exists and not (path.is_symlink() or path.is_file()):
            refuse(path)
        if matches_working_file(path, exists, worktree, worktree_dirty, saved):
            continue
        if not matches_working_file(path, exists, base, base_dirty, baseline):
            refuse(path)
        worktree_paths.append(path)

    current = index_entries(live)
    index_dirs = {directory for path in current for directory in path.parents}
    staged_paths = []
    for path in staged_changes:
        if path in index_dirs or any(p in current for p in path.parents):
            refuse(path)
        if current.get(path) == staged.get(path):
            continue
        if current.get(path) != base.get(path):
            refuse(path)
        staged_paths.append(path)

    restore_paths("staged", stash + "^2", staged_paths, live)
    restore_paths("worktree", stash, worktree_paths, {**baseline, "GIT_NO_REPLACE_OBJECTS": "1"})


if __name__ == "__main__":
    if len(sys.argv) != 2 or not re.fullmatch(r"(?:[0-9a-f]{40}|[0-9a-f]{64})", sys.argv[1]):
        sys.exit("expected a full stash object ID")
    with source_stash.locked_index() as (live, scratch):
        restore(sys.argv[1], scratch, live)
