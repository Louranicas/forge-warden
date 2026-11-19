"""Restore the source paths changed in each layer of a saved stash."""
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path


SOURCES = ":(glob)crates/*/src/**"


def git(*args, env, data=None):
    return subprocess.run(["git", *args], env=env, input=data, stdout=subprocess.PIPE,
                          check=True).stdout


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


def restore_paths(layer, revision, paths, env):
    if paths:
        git("--literal-pathspecs", "restore", "--source=" + revision, "--" + layer,
            "--pathspec-from-file=-", "--pathspec-file-nul", env=env,
            data=b"".join(os.fsencode(path) + b"\0" for path in paths))


def restore(stash, scratch):
    if not re.fullmatch(r"(?:[0-9a-f]{40}|[0-9a-f]{64})", stash):
        sys.exit("expected a full stash object ID")
    local = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    raw = {**local, "GIT_NO_REPLACE_OBJECTS": "1"}
    parent = git("rev-parse", "--verify", stash + "^1", env=raw).decode().strip()
    baseline = {**local, "GIT_INDEX_FILE": str(scratch / "base")}
    saved = {**raw, "GIT_INDEX_FILE": str(scratch / "saved")}
    base, base_dirty = snapshot(parent, baseline)
    worktree, worktree_dirty = snapshot(stash, saved)
    git("read-tree", stash + "^2", env=saved)
    staged = index_entries(saved, SOURCES)
    current = index_entries(raw)
    index_dirs = {directory for path in current for directory in path.parents}

    worktree_paths = []
    for path in changed_paths(base, worktree):
        if any(p.is_symlink() or (p.exists() and not p.is_dir()) for p in path.parents):
            refuse(path)
        exists = os.path.lexists(path)
        if exists and not (path.is_symlink() or path.is_file()):
            refuse(path)
        if exists == (path in worktree) and path not in worktree_dirty:
            continue
        if exists != (path in base) or path in base_dirty:
            refuse(path)
        worktree_paths.append(path)

    staged_paths = []
    for path in changed_paths(base, staged):
        if path in index_dirs or any(p in current for p in path.parents):
            refuse(path)
        if current.get(path) == staged.get(path):
            continue
        if current.get(path) != base.get(path):
            refuse(path)
        staged_paths.append(path)

    restore_paths("worktree", stash, worktree_paths, {**baseline, "GIT_NO_REPLACE_OBJECTS": "1"})
    restore_paths("staged", stash + "^2", staged_paths, raw)


if __name__ == "__main__":
    with tempfile.TemporaryDirectory() as directory:
        restore(sys.argv[1], Path(directory))
