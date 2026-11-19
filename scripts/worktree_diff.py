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
    env = {key: value for key, value in os.environ.items()
           if key == "GIT_CONFIG_NOSYSTEM" or not key.startswith("GIT_")}
    env.update(GIT_GRAFT_FILE="", GIT_NO_REPLACE_OBJECTS="1")
    return env


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


def base_source_blobs(env, base, scope, indexed):
    blobs = indexed.copy()
    changes = git("diff-index", "--cached", "--raw", "--no-abbrev", "--no-renames", "-z",
                  base, "--", scope, env=env).split(b"\0")
    for metadata, path in zip(changes[::2], changes[1::2]):
        old_mode, _, old_blob, _, _ = metadata[1:].split()
        if old_mode.startswith(b"100"):
            blobs[path] = old_blob
        else:
            blobs.pop(path, None)
    return blobs


def check_conversions(env, scope, base=None):
    entries = git("ls-files", "--abbrev=64", "-z", "--format=%(objectmode) %(objectname) %(eolattr)%x09%(path)",
                  "--", scope, env=env)
    blobs = {}
    regular = {}
    eol_attrs = {}
    for entry in entries.split(b"\0")[:-1]:
        metadata, path = entry.split(b"\t", 1)
        mode, blob, eol = metadata.split(b" ", 2)
        if mode.startswith(b"100"):
            regular[path] = blob
        elif mode == b"120000" and (os.path.islink(path) or not os.path.isfile(path)):
            continue
        blobs[path] = blob
        eol_attrs[path] = eol
    baseline = base_source_blobs(env, base, scope, regular) if base else {}
    if not blobs and not baseline:
        return []
    autocrlf = git("config", "--default", "false", "--get", "core.autocrlf", env=env).strip().lower()
    auto = autocrlf not in (b"false", b"no", b"off", b"0")
    sensitive = {path for path, eol in eol_attrs.items() if eol != b"-text" and (eol or auto)}
    base_options = ["--attr-source=" + base] if base else []
    filters = {}
    checkout_filters = {}
    for paths, source in ((blobs.keys() | baseline.keys(), []), (baseline, base_options)):
        if not paths:
            continue
        attrs = git(*source, "check-attr", "-z", "--stdin", "--all", env=env,
                    data=b"".join(path + b"\0" for path in paths)).split(b"\0")
        if source and auto:
            sensitive.update(paths)
        for path, name, value in zip(attrs[0::3], attrs[1::3], attrs[2::3]):
            if ((name == b"ident" and value == b"set") or
                    (name == b"working-tree-encoding" and value.lower() not in (b"", b"utf8", b"utf-8")) or
                    (source and name in (b"text", b"crlf", b"eol"))):
                sensitive.add(path)
            if name == b"filter":
                filters.setdefault(value, set()).add(path)
                if source:
                    checkout_filters.setdefault(value, set()).add(path)
    unknown_checkout = set()
    options = []
    for driver, paths in filters.items():
        prefix = b"filter." + driver + b"."
        required = git("config", "--type=bool", "--default=false", "--get", prefix + b"required", env=env)
        commands = {setting for setting in (b"clean", b"smudge", b"process")
                    if git("config", "-z", "--default=", "--get", prefix + setting, env=env) != b"\0"}
        if required == b"true\n" or commands:
            sensitive.update(paths)
            if commands & {b"smudge", b"process"}:
                unknown_checkout.update(checkout_filters.get(driver, ()))
            for setting in (b"clean=", b"smudge=", b"process=", b"required=false"):
                options.extend(("-c", prefix + setting))
    if not sensitive:
        return options
    if base is None:
        sys.exit(f"Working-tree comparison unverified. Git may convert {os.fsdecode(min(sensitive))!r}.")
    unstaged = set(git(*options, "diff-files", "--name-only", "-z", "--no-renames", "--no-ext-diff", "--no-textconv",
                       "--", scope, env=env).split(b"\0"))
    for path in sorted(sensitive):
        refusal = f"Working-tree comparison unverified. Git may convert {os.fsdecode(path)!r}."
        if path in unknown_checkout or any(parent.is_symlink() for parent in Path(os.fsdecode(path)).parents):
            sys.exit(refusal)
        original = baseline.get(path)
        checkout = original
        if original is not None:
            content = git(*options, *base_options, "cat-file", "--filters", b"--path=" + path, original, env=env)
            checkout = git("hash-object", "--stdin", env=env, data=content).strip()
        try:
            mode = os.lstat(path).st_mode
        except (FileNotFoundError, NotADirectoryError):
            mode = 0
        if not stat.S_ISREG(mode):
            if checkout != original:
                sys.exit(refusal)
            continue
        raw = git("hash-object", "--no-filters", "--", path, env=env).strip()
        endpoint = git(*options, "hash-object", "--", path, env=env).strip() if path in unstaged else blobs.get(path)
        if checkout == original:
            if raw != endpoint:
                sys.exit(refusal)
        elif raw != checkout or endpoint != original:
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


def gate_inventory():
    paths = set()
    reviewed_clippy = (b"clippy.toml", b"crates/forge-warden-core/clippy.toml")
    directories = [b"."]
    while directories:
        directory = directories.pop()
        with os.scandir(directory) as entries:
            for entry in entries:
                if directory == b"./.github/workflows":
                    paths.add(os.path.relpath(entry.path))
                if entry.name == b".git" or (directory == b"." and entry.name in
                                             (b"target", b"mutants.out", b"mutants.out.old")):
                    continue
                if entry.is_symlink():
                    raise ValueError(f"Source symlink: {os.fsdecode(entry.path)!r}.")
                if entry.name in (b"config", b"config.toml") and os.path.basename(directory) == b".cargo":
                    raise ValueError(f"Cargo configuration: {os.fsdecode(entry.path)!r}.")
                if entry.name in (b"clippy.toml", b".clippy.toml"):
                    path = os.path.relpath(entry.path)
                    if path not in reviewed_clippy or not entry.is_file(follow_symlinks=False):
                        raise ValueError(f"Clippy configuration: {os.fsdecode(path)!r}.")
                if entry.name == b"Cargo.toml" and entry.path.startswith(b"./crates/"):
                    paths.add(os.path.relpath(entry.path))
                if entry.is_dir(follow_symlinks=False):
                    directories.append(entry.path)
                elif entry.name.lower().endswith(b".md") and len(entry.name) > 3:
                    paths.add(os.path.relpath(entry.path))
    return paths


@contextmanager
def checked_snapshot(scope):
    env = {**worktree_env(), "GIT_OPTIONAL_LOCKS": "0"}
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
        options = check_conversions(env, RUST_SOURCES, base)
        run_git(*options, "-c", "core.quotePath=false", "-c", "diff.renames=true", "diff", "--no-ext-diff",
                "--no-textconv", "--text", "--no-color", "--src-prefix=a/", "--dst-prefix=b/",
                base, "--", RUST_SOURCES, env=env, check=True)


if __name__ == "__main__":
    if sys.argv[1:] in (["--check-inputs"], ["--clean"]):
        try:
            checkout = Path.cwd()
            mode = (checkout / ".git").lstat().st_mode
            if not (stat.S_ISDIR(mode) or stat.S_ISREG(mode)):
                raise ValueError(".git must be a directory or a regular worktree pointer.")
            resolved = run_git("-C", checkout, "rev-parse", "--show-toplevel", env=worktree_env(), capture_output=True)
            if resolved.returncode or resolved.stdout != os.fsencode(checkout) + b"\n":
                raise ValueError(f"Git worktree differs from the selected checkout {checkout!s}.")
            inventory = gate_inventory()
        except (ValueError, OSError) as error:
            print(f"Gate input refused: {error}", file=sys.stderr)
            sys.exit(2)
        if sys.argv[1] == "--check-inputs":
            sys.exit(0)
        with checked_snapshot(ALL_FILES) as env:
            untracked = inventory.difference(git("ls-files", "-z", env=env).split(b"\0"))
            if untracked:
                sys.exit(f"Untracked gate input: {os.fsdecode(min(untracked))!r}.")
            check_conversions(env, ALL_FILES)
            sys.exit(bool(git("status", "--porcelain=v1", "-z", "--untracked-files=normal",
                              "--ignore-submodules=none", env=env)))
    if len(sys.argv) != 2 or not re.fullmatch(r"(?:[0-9a-f]{40}|[0-9a-f]{64})", sys.argv[1]):
        sys.exit("expected a full mutation base object ID")
    mutation_diff(sys.argv[1])
