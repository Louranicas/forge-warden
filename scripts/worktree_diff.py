"""Check index visibility before comparing tracked working files."""
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
from contextlib import contextmanager
from pathlib import Path


ALL_FILES = ":(top,glob)**"
RUST_SOURCES = ":(top,glob)crates/*/src/**/*.rs"
CARGO_INPUTS = (RUST_SOURCES, ":(top,glob)crates/*/build.rs",
                *(":(top,glob)crates/*/" + target + "/**/*.rs" for target in ("tests", "examples", "benches")))


def worktree_env():
    return {key: value for key, value in os.environ.items()
            if key == "GIT_CONFIG_NOSYSTEM" or not key.startswith("GIT_")}


def run_git(*args, env, **kwargs):
    return subprocess.run(["git", "-c", "core.fsmonitor=false", "-c", "core.ignoreStat=false",
                           "-c", "core.checkStat=default", "-c", "core.trustctime=true",
                           "-c", "core.fileMode=true", "-c", "submodule.recurse=false",
                           *args], env=env, **kwargs)


def git(*args, env, data=None):
    result = run_git("-c", "core.untrackedCache=false", *args, env=env, input=data, capture_output=True)
    if result.returncode or result.stderr:
        sys.stderr.buffer.write(result.stderr)
        sys.exit("Working-tree scan unverified.")
    return result.stdout


def check_conversions(env, scope):
    entries = git("ls-files", "--abbrev=64", "-z", "--format=%(objectmode) %(objectname) %(eolattr)%x09%(path)",
                  "--", scope, env=env)
    if not entries:
        return []
    autocrlf = git("config", "--default", "false", "--get", "core.autocrlf", env=env).strip().lower()
    blobs = {}
    sensitive = set()
    for entry in entries.rstrip(b"\0").split(b"\0"):
        metadata, path = entry.split(b"\t", 1)
        mode, blob, eol = metadata.split(b" ", 2)
        if mode == b"120000" and (os.path.islink(path) or not os.path.isfile(path)):
            continue
        blobs[path] = blob
        if eol != b"-text" and (eol or autocrlf not in (b"false", b"no", b"off", b"0")):
            sensitive.add(path)
    attrs = git("check-attr", "-z", "--stdin", "--all", env=env,
                data=b"".join(path + b"\0" for path in blobs)).split(b"\0")
    filters = {}
    for path, name, value in zip(attrs[0::3], attrs[1::3], attrs[2::3]):
        if ((name == b"ident" and value == b"set") or
                (name == b"working-tree-encoding" and value.lower() not in (b"", b"utf8", b"utf-8"))):
            sensitive.add(path)
        if name == b"filter":
            filters.setdefault(value, set()).add(path)
    external = set()
    options = []
    for driver, paths in filters.items():
        prefix = b"filter." + driver + b"."
        required = git("config", "--type=bool", "--default=false", "--get", prefix + b"required", env=env)
        if required == b"true\n" or any(
                git("config", "-z", "--default=", "--get", prefix + setting, env=env) != b"\0"
                for setting in (b"clean", b"smudge", b"process")):
            external.update(paths)
            for setting in (b"clean=", b"smudge=", b"process=", b"required=false"):
                options.extend(("-c", prefix + setting))
    candidates = sensitive | external
    if not candidates:
        return []
    if scope == ALL_FILES:
        sys.exit(f"Working-tree comparison unverified. Git may convert {os.fsdecode(min(candidates))!r}.")
    unstaged = set(git(*options, "diff-files", "--name-only", "-z", "--no-renames", "--no-ext-diff", "--no-textconv",
                       "--", scope, env=env).split(b"\0"))
    for path in sorted(candidates):
        refusal = f"Working-tree comparison unverified. Git may convert {os.fsdecode(path)!r}."
        if any(parent.is_symlink() for parent in Path(os.fsdecode(path)).parents):
            sys.exit(refusal)
        try:
            mode = os.lstat(path).st_mode
        except (FileNotFoundError, NotADirectoryError):
            continue
        if not stat.S_ISREG(mode):
            continue
        raw = git("hash-object", "--no-filters", "--", path, env=env).strip()
        if path in external and raw != blobs[path]:
            sys.exit(refusal)
        endpoint = git(*options, "hash-object", "--", path, env=env).strip() if path in unstaged else blobs[path]
        if raw != endpoint:
            sys.exit(refusal)
    return options


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
        untracked_scope = (RUST_SOURCES,) if scope == RUST_SOURCES else CARGO_INPUTS
        untracked = git("ls-files", "--others", "-z", "--", *untracked_scope, env=env)
        if untracked:
            path = os.fsdecode(untracked.split(b"\0", 1)[0])
            sys.exit(f"Working-tree comparison unverified. Untracked Rust source: {path!r}.")
        yield env


def mutation_diff(base):
    with checked_snapshot(RUST_SOURCES) as env:
        options = check_conversions(env, RUST_SOURCES)
        run_git(*options, "-c", "core.quotePath=false", "-c", "diff.renames=true", "diff", "--no-ext-diff",
                "--no-textconv", "--text", "--no-color", "--src-prefix=a/", "--dst-prefix=b/",
                base, "--", RUST_SOURCES, env=env, check=True)


if __name__ == "__main__":
    if sys.argv[1:] == ["--clean"]:
        with checked_snapshot(ALL_FILES) as env:
            check_conversions(env, ALL_FILES)
            sys.exit(bool(git("status", "--porcelain=v1", "-z", "--untracked-files=normal",
                              "--ignore-submodules=none", env=env)))
    if len(sys.argv) != 2 or not re.fullmatch(r"(?:[0-9a-f]{40}|[0-9a-f]{64})", sys.argv[1]):
        sys.exit("expected a full mutation base object ID")
    mutation_diff(sys.argv[1])
