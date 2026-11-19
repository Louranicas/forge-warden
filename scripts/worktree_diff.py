"""Check index visibility before comparing tracked working files."""
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


def worktree_env():
    return {key: value for key, value in os.environ.items()
            if key == "GIT_CONFIG_NOSYSTEM" or not key.startswith("GIT_")}


def run_git(*args, env, **kwargs):
    return subprocess.run(["git", "-c", "core.fsmonitor=false", "-c", "core.ignoreStat=false",
                           *args], env=env, **kwargs)


def git(*args, env, data=None):
    return run_git(*args, env=env, input=data, stdout=subprocess.PIPE, check=True).stdout


def check_index_flags(env, scope):
    for row in git("ls-files", "-v", "-z", "--", scope, env=env).split(b"\0"):
        tag = row[:1]
        if tag.islower() or tag == b"S":
            flag = "assume-unchanged" if tag.islower() else "skip-worktree"
            sys.exit(f"Working-tree comparison unverified. Git marks {os.fsdecode(row[2:])!r} {flag}.")


def mutation_diff(base):
    scope = "crates/*/src/**.rs"
    env = {**worktree_env(), "GIT_NO_REPLACE_OBJECTS": "1"}
    index = git("rev-parse", "--path-format=absolute", "--git-path", "index", env=env)
    with tempfile.TemporaryDirectory() as directory:
        snapshot = Path(directory) / "index"
        shutil.copyfile(os.fsdecode(index[:-1]), snapshot)
        env["GIT_INDEX_FILE"] = str(snapshot)
        check_index_flags(env, scope)
        run_git("-c", "core.quotePath=false", "-c", "diff.renames=true", "diff", "--no-ext-diff",
                "--no-textconv", "--text", "--no-color", "--src-prefix=a/", "--dst-prefix=b/",
                base, "--", scope, env=env, check=True)


if __name__ == "__main__":
    if len(sys.argv) != 2 or not re.fullmatch(r"(?:[0-9a-f]{40}|[0-9a-f]{64})", sys.argv[1]):
        sys.exit("expected a full mutation base object ID")
    mutation_diff(sys.argv[1])
