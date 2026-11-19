"""Check index visibility before comparing tracked working files."""
import os
import re
import shutil
import subprocess
import sys
import tempfile
from contextlib import contextmanager
from pathlib import Path


def worktree_env():
    return {key: value for key, value in os.environ.items()
            if key == "GIT_CONFIG_NOSYSTEM" or not key.startswith("GIT_")}


def run_git(*args, env, **kwargs):
    return subprocess.run(["git", "-c", "core.fsmonitor=false", "-c", "core.ignoreStat=false",
                           *args], env=env, **kwargs)


def git(*args, env, data=None):
    return run_git(*args, env=env, input=data, stdout=subprocess.PIPE, check=True).stdout


def clean_worktree(env):
    result = run_git("-c", "core.untrackedCache=false", "status", "--porcelain=v1", "-z",
                     "--untracked-files=normal", "--ignore-submodules=none", env=env, capture_output=True)
    if result.returncode or result.stderr:
        sys.stderr.buffer.write(result.stderr)
        sys.exit("Working-tree scan unverified.")
    if result.stdout:
        return False

    autocrlf = git("config", "--default", "false", "--get", "core.autocrlf", env=env).strip().lower()
    if autocrlf not in (b"false", b"no", b"off", b"0"):
        return False
    paths = []
    for entry in git("ls-files", "--stage", "-z", env=env).split(b"\0"):
        if entry.startswith(b"160000 "):
            return False
        if entry.startswith((b"100644 ", b"100755 ")):
            paths.append(entry.split(b"\t", 1)[1] + b"\0")
    attrs = git("check-attr", "-z", "--stdin", "--all", env=env, data=b"".join(paths)).split(b"\0")
    conversion = {b"text", b"crlf", b"eol", b"filter", b"ident", b"working-tree-encoding"}
    return all(name not in conversion or (name == b"text" and value == b"unset")
               for name, value in zip(attrs[1::3], attrs[2::3]))


def check_index_flags(env, scope):
    for row in git("ls-files", "-v", "-z", "--", scope, env=env).split(b"\0"):
        tag = row[:1]
        if tag.islower() or tag == b"S":
            path = Path(os.fsdecode(row[2:]))
            flag = "assume-unchanged" if tag.islower() else "skip-worktree"
            sys.exit(f"Working-tree comparison unverified. Git marks {str(path)!r} {flag}.")


@contextmanager
def checked_snapshot(scope):
    env = {**worktree_env(), "GIT_NO_REPLACE_OBJECTS": "1"}
    index = Path(os.fsdecode(git("rev-parse", "--path-format=absolute", "--git-path", "index", env=env)[:-1]))
    scratch_parent = Path(tempfile.gettempdir())
    if scratch_parent.is_relative_to(Path.cwd()):
        scratch_parent = index.parent
    with tempfile.TemporaryDirectory(dir=scratch_parent) as directory:
        snapshot = Path(directory) / "index"
        with index.open("rb") as source:
            stat = os.fstat(source.fileno())
            with snapshot.open("wb") as target:
                shutil.copyfileobj(source, target)
            os.utime(snapshot, ns=(stat.st_atime_ns, stat.st_mtime_ns))
        env["GIT_INDEX_FILE"] = str(snapshot)
        check_index_flags(env, scope)
        yield env


def mutation_diff(base):
    scope = "crates/*/src/**.rs"
    with checked_snapshot(scope) as env:
        run_git("-c", "core.quotePath=false", "-c", "diff.renames=true", "diff", "--no-ext-diff",
                "--no-textconv", "--text", "--no-color", "--src-prefix=a/", "--dst-prefix=b/",
                base, "--", scope, env=env, check=True)


if __name__ == "__main__":
    if sys.argv[1:] == ["--clean"]:
        with checked_snapshot(":(top,glob)**") as env:
            sys.exit(0 if clean_worktree(env) else 1)
    if len(sys.argv) != 2 or not re.fullmatch(r"(?:[0-9a-f]{40}|[0-9a-f]{64})", sys.argv[1]):
        sys.exit("expected a full mutation base object ID")
    mutation_diff(sys.argv[1])
