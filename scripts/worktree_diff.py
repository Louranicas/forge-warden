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
                           "-c", "core.checkStat=default", "-c", "core.trustctime=true",
                           "-c", "submodule.recurse=false",
                           *args], env=env, **kwargs)


def git(*args, env, data=None):
    return run_git(*args, env=env, input=data, stdout=subprocess.PIPE, check=True).stdout


def scan_worktree(*args, env):
    result = run_git("-c", "core.untrackedCache=false", *args, env=env, capture_output=True)
    if result.returncode or result.stderr:
        sys.stderr.buffer.write(result.stderr)
        sys.exit("Working-tree scan unverified.")
    return result.stdout


def clean_worktree(env):
    if scan_worktree("status", "--porcelain=v1", "-z", "--untracked-files=normal",
                     "--ignore-submodules=none", env=env):
        return False

    autocrlf = git("config", "--default", "false", "--get", "core.autocrlf", env=env).strip().lower()
    paths = []
    entries = git("ls-files", "-z", "--format=%(objectmode) %(eolattr)%x09%(path)", env=env)
    for entry in entries.split(b"\0"):
        if entry.startswith((b"100644 ", b"100755 ")):
            metadata, path = entry.split(b"\t", 1)
            eol = metadata.split(b" ", 1)[1]
            if eol != b"-text" and (eol or autocrlf not in (b"false", b"no", b"off", b"0")):
                return False
            paths.append(path + b"\0")
    attrs = git("check-attr", "-z", "--stdin", "--all", env=env, data=b"".join(paths)).split(b"\0")
    filters = set()
    for name, value in zip(attrs[1::3], attrs[2::3]):
        if name == b"ident" and value == b"set":
            return False
        if name == b"working-tree-encoding" and value.lower() not in (b"", b"utf8", b"utf-8"):
            return False
        if name == b"filter":
            filters.add(value)
    if filters:
        keys = set(git("config", "--null", "--name-only", "--list", env=env).split(b"\0"))
        return all(b"filter." + driver + b"." + setting not in keys
                   for driver in filters for setting in (b"clean", b"smudge", b"process", b"required"))
    return True


def check_index_flags(env, scope):
    for row in git("ls-files", "-v", "--stage", "-z", "--", scope, env=env).split(b"\0"):
        if row[2:].startswith(b"160000 "):
            sys.exit("Working-tree comparison unverified. The index contains a submodule in scope.")
        tag = row[:1]
        if tag.islower() or tag == b"S":
            path = Path(os.fsdecode(row.split(b"\t", 1)[1]))
            flag = "assume-unchanged" if tag.islower() else "skip-worktree"
            sys.exit(f"Working-tree comparison unverified. Git marks {str(path)!r} {flag}.")


@contextmanager
def checked_snapshot(scope):
    env = {**worktree_env(), "GIT_NO_REPLACE_OBJECTS": "1", "GIT_OPTIONAL_LOCKS": "0"}
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
        untracked = scan_worktree("ls-files", "--others", "-z", "--", scope, env=env)
        if untracked:
            path = os.fsdecode(untracked.split(b"\0", 1)[0])
            sys.exit(f"Mutation comparison unverified. Untracked Rust source: {path!r}.")
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
