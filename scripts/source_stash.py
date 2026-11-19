"""Save and restore crate source changes under the Git index lock."""
import importlib.util
import os
import shutil
import signal
import sys
import tempfile
import uuid
from contextlib import contextmanager
from pathlib import Path

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("worktree_diff", Path(__file__).with_name("worktree_diff.py"))
worktree_diff = importlib.util.module_from_spec(spec)
spec.loader.exec_module(worktree_diff)

git = worktree_diff.git

SOURCE_GLOB = "crates/*/src/**"
SOURCES = ":(glob)" + SOURCE_GLOB
CANCEL_SIGNALS = (signal.SIGHUP, signal.SIGINT, signal.SIGTERM)


def cancel(signum, _frame):
    signal.pthread_sigmask(signal.SIG_BLOCK, CANCEL_SIGNALS)
    raise SystemExit(128 + signum)


@contextmanager
def locked_index():
    for sig in CANCEL_SIGNALS:
        signal.signal(sig, cancel)
    env = worktree_diff.worktree_env()
    index = Path(os.fsdecode(git("rev-parse", "--path-format=absolute", "--git-path", "index", env=env)[:-1]))
    lock = index.with_name(index.name + ".lock")
    previous = signal.pthread_sigmask(signal.SIG_BLOCK, CANCEL_SIGNALS)
    try:
        try:
            lock.touch(exist_ok=False)
        except FileExistsError:
            sys.exit("Could not lock the Git index for source recovery.")
        try:
            with tempfile.TemporaryDirectory(dir=index.parent) as directory:
                scratch = Path(directory)
                try:
                    signal.pthread_sigmask(signal.SIG_SETMASK, previous)
                    pending = scratch / "index"
                    shutil.copy2(index, pending)
                    yield {**env, "GIT_INDEX_FILE": str(pending)}, scratch
                    os.replace(pending, index)
                finally:
                    signal.pthread_sigmask(signal.SIG_BLOCK, CANCEL_SIGNALS)
        finally:
            lock.unlink()
    finally:
        signal.pthread_sigmask(signal.SIG_SETMASK, previous)


def check_source(env):
    worktree_diff.check_index_flags(env, SOURCES)
    untracked = worktree_diff.scan_worktree("ls-files", "--others", "--exclude-standard", "--", SOURCES, env=env)
    ignored = worktree_diff.scan_worktree("ls-files", "--others", "--ignored", "--exclude-standard", "--",
                                          ":(glob)crates/*/src/**/*.rs", env=env)
    if untracked or ignored:
        sys.exit("Failing-first check unverified. Stage the new source before saving:\n"
                 + os.fsdecode(untracked + ignored).rstrip("\n"))
    status_rows = git("status", "--porcelain=v1", "-z", "--no-renames", "--untracked-files=no",
                      "--", "crates", env=env)
    entries = {Path(os.fsdecode(row[3:])): row[:2] for row in status_rows.split(b"\0") if row}
    staged_source = False
    working_files = []
    for path, status in entries.items():
        if len(path.parts) < 4 or path.parts[2] != "src":
            continue
        if status[:1] != b" ":
            staged_source = True
        if status == b"AD":
            sys.exit("Failing-first check unverified. Git stash cannot preserve a newly staged file missing from the working tree.")
        blocked_parent = any(parent in entries or parent.is_symlink() or
                             (parent.exists() and not parent.is_dir()) for parent in path.parents)
        if blocked_parent or (not path.is_symlink() and path.is_dir()):
            sys.exit(f"Failing-first check unverified. Source changes between a file and a directory at {str(path)!r}.")
        if not path.is_symlink() and path.is_file():
            working_files.append(path)
    if staged_source and not git(
            "diff", "--no-ext-diff", "--no-textconv", "--name-only", "-z", "HEAD", "--", SOURCES, env=env):
        sys.exit("Failing-first check unverified. Source edits are staged, but the working source matches HEAD.")
    return working_files


def checkout_bytes(path, blob, env):
    return git("cat-file", "--filters", "--path=" + str(path), blob, env=env)


def check_round_trip(paths, env, scratch):
    objects = scratch / "objects"
    objects.mkdir()
    repository_objects = os.fsdecode(git("rev-parse", "--path-format=absolute", "--git-path", "objects", env=env)[:-1])
    alternate = repository_objects.replace("\\", "\\\\").replace('"', '\\"')
    env = {**env, "GIT_OBJECT_DIRECTORY": str(objects), "GIT_ALTERNATE_OBJECT_DIRECTORIES": f'"{alternate}"'}
    for path in paths:
        data = path.read_bytes()
        blob = git("hash-object", "-w", "--path", str(path), "--stdin", data=data, env=env).strip()
        if checkout_bytes(path, blob, env) != data:
            sys.exit(f"Failing-first check unverified. Git would change the working bytes of {str(path)!r}.")


def save(live, scratch):
    working_files = check_source(live)
    private_index = scratch / "source"
    shutil.copy2(live["GIT_INDEX_FILE"], private_index)
    private = {**live, "GIT_INDEX_FILE": str(private_index)}
    git("reset", "-q", "HEAD", "--", ":(exclude,glob)" + SOURCE_GLOB, env=private)
    check_round_trip(working_files, private, scratch)
    marker = "forge-warden-source-" + str(uuid.uuid4())
    previous = signal.pthread_sigmask(signal.SIG_BLOCK, CANCEL_SIGNALS)
    try:
        result = worktree_diff.run_git("-c", "apply.whitespace=nowarn", "stash", "push", "--message",
                                       marker, "--", SOURCES, env=private,
                                       preexec_fn=lambda: signal.pthread_sigmask(signal.SIG_SETMASK, previous))
        stored = git("stash", "list", "--fixed-strings", "--grep-reflog=" + marker,
                     "--format=%H", env=live).decode().strip()
        if stored:
            print(stored, flush=True)
    finally:
        signal.pthread_sigmask(signal.SIG_SETMASK, previous)
    if not stored:
        print("Failing-first check unverified. No crate source stash was recorded.", file=sys.stderr)
        sys.exit(result.returncode or 1)
    if result.returncode:
        sys.exit(result.returncode)
    git("reset", "-q", stored + "^1", "--", SOURCES, env=live)


if __name__ == "__main__":
    with locked_index() as (live, scratch):
        save(live, scratch)
