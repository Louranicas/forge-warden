#!/usr/bin/env python3
"""Read canonical inputs; reject drift in the workflow's declarative copies."""
import collections
import importlib.util
import os
import re
import subprocess
import sys
import tomllib
from pathlib import Path

sys.dont_write_bytecode = True

ROOT = Path(__file__).resolve().parents[1]


def versions(root):
    values = tomllib.loads((root / "scripts/tool-versions.toml").read_text())
    values["rust"] = tomllib.loads((root / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    values["msrv"] = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["package"]["rust-version"]
    if not re.fullmatch(r"[0-9a-f]{40}", values["ddf_rev"]):
        raise ValueError("invalid pinned DDF revision")
    if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", values["ddf_release"]):
        raise ValueError("invalid pinned DDF release tag")
    if not re.fullmatch(r"[0-9a-f]{64}", values["ddf_archive_sha256"]):
        raise ValueError("invalid DDF release archive sha256")
    for name in ("rust", "zizmor", "cargo_mutants", "cargo_deny", "rustup"):
        if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", values[name]):
            raise ValueError("invalid exact tool version: " + name)
    if not re.fullmatch(r"[0-9a-f]{64}", values["rustup_sha256"]):
        raise ValueError("invalid rustup-init sha256")
    return values


def check(root):
    values = versions(root)
    workflow = (root / ".github/workflows/ci.yml").read_text()
    pins = re.findall(r"tool: (zizmor|cargo-mutants|cargo-deny)@([^\s]+)", workflow)
    expected = {(name.replace("_", "-"), values[name]) for name in ("zizmor", "cargo_mutants", "cargo_deny")}
    if collections.Counter(pins) != collections.Counter([*expected, *[('cargo-mutants', values['cargo_mutants'])] * 2]):
        raise ValueError("CI tool pins drift from scripts/tool-versions.toml")
    if not re.fullmatch(r"[0-9a-f]{40}", values['install_action']):
        raise ValueError("invalid install-action revision")
    for file in ("ci.yml", "mutation-baseline.yml"):
        actions = re.findall(r"uses: taiki-e/install-action@([^\s]+)",
                             (root / ".github/workflows" / file).read_text())
        if not actions or any(action != values['install_action'] for action in actions):
            raise ValueError("CI install-action pin drift")
    spec = importlib.util.spec_from_file_location("plant_tools", root / "scripts/plant_tools.py")
    capabilities = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(capabilities)
    capabilities.check_ci(root)
    baseline = (root / ".github/workflows/mutation-baseline.yml").read_text()
    if re.findall(r"tool: (cargo-mutants)@([^\s]+)", baseline) != [("cargo-mutants", values["cargo_mutants"])] * 2:
        raise ValueError("baseline mutation tool pin drifts from scripts/tool-versions.toml")
    baseline_compilers = re.findall(r"(?:toolchain install |rustup default )([0-9.]+)", baseline)
    if baseline_compilers != [values["rust"], values["rust"]] * 2:
        raise ValueError("baseline Rust pin drifts from rust-toolchain.toml")
    allowed = {values["rust"], values["msrv"] if values["msrv"].count(".") == 2 else values["msrv"] + ".0"}
    compilers = re.findall(r"(?:toolchain install |rustup default |cargo \+|RUSTUP_TOOLCHAIN: )([0-9.]+)", workflow)
    if not compilers or set(compilers) != allowed:
        raise ValueError("CI Rust pins drift from rust-toolchain.toml/Cargo.toml")
    revisions = re.findall(r"DDF_REV: ([0-9a-f]+)", workflow)
    if revisions != [values["ddf_rev"]]:
        raise ValueError("CI DDF revision drift")
    return values


# G-18. How each required local-gate tool reports its version, through the same `cargo <sub>`
# dispatch the gate step uses, and which pin it must equal.
LOCAL_PROBES = {
    "cargo-deny": (["cargo", "deny", "--version"], "cargo_deny"),
    "cargo-mutants": (["cargo", "mutants", "--version"], "cargo_mutants"),
}
# Why a step may be hosted-only. A name outside this table cannot be declared hosted-only.
HOSTED_ONLY_REASONS = {
    "zizmor": "the trusted offline image has no zizmor (G-47)",
    "deep-diff-forge": "the dogfood gate fetches the pinned release archive; a local run needs that binary on PATH",
}


def local_gate(root):
    table = tomllib.loads((root / "scripts/tool-versions.toml").read_text()).get("local_gate")
    if not isinstance(table, dict) or set(table) != {"required", "hosted_only"}:
        raise ValueError("[local_gate] must hold exactly `required` and `hosted_only` (G-18)")
    required, hosted = table["required"], table["hosted_only"]
    for name, names in (("required", required), ("hosted_only", hosted)):
        if not isinstance(names, list) or not all(isinstance(n, str) for n in names) or len(set(names)) != len(names):
            raise ValueError(f"[local_gate] {name} must be a list of distinct names (G-18)")
    if set(required) & set(hosted):
        raise ValueError("[local_gate] a tool cannot be both required and hosted-only (G-18)")
    if not set(required) <= set(LOCAL_PROBES):
        missing = sorted(set(required) - set(LOCAL_PROBES))
        raise ValueError("[local_gate] required tool has no version probe: " + ", ".join(missing))
    if not set(hosted) <= set(HOSTED_ONLY_REASONS):
        unknown = sorted(set(hosted) - set(HOSTED_ONLY_REASONS))
        raise ValueError("[local_gate] hosted-only tool has no recorded reason: " + ", ".join(unknown))
    return required, hosted


# The tools whose steps below are required lines; [local_gate] required must keep them (Keel A6).
STEP_TOOLS = ("cargo-deny", "cargo-mutants")
# G-18: each required tool's step, as one unconditional line of fixed form. A guard, a `|| true` or a
# moved command changes the line and is refused here; scripts/tests/ci_local.sh proves the lines run.
REQUIRED_STEP_LINES = (
    "step deny; cargo deny check",
    "step mutation",
    'if [ -s target/pr.diff ]; then env -u CARGO_TARGET_DIR cargo mutants --no-config --cap-lints true '
    '--in-diff target/pr.diff --workspace; else echo "no source changes to mutate"; fi',
)


def check_ci_local(root):
    """ci_local probes only hosted-only tools, runs the local-tools step, and never claims a pass it skipped."""
    required, hosted = local_gate(root)
    missing = sorted(set(STEP_TOOLS) - set(required))
    if missing:
        raise ValueError("a tool whose step ci_local requires is not in [local_gate] required (G-18): "
                         + ", ".join(missing))
    text = (root / "scripts/ci_local.sh").read_text()
    probed = re.findall(r"command -v ([A-Za-z0-9_.+-]+)", text)
    stray = sorted(set(probed) - set(hosted))
    if stray:
        raise ValueError("ci_local probes a tool that is not hosted-only, so it could skip it (G-18): "
                         + ", ".join(stray))
    if re.search(r"""echo\s+["']?skip""", text):
        raise ValueError("ci_local echoes a skip instead of refusing or naming a hosted-only step (G-18)")
    if text.count("step local-tools; python3 -I scripts/tool_versions.py --check-local-tools\n") != 1:
        raise ValueError("ci_local must run the local-tools step exactly once (G-18)")
    lines = text.splitlines()
    for required in REQUIRED_STEP_LINES:
        if lines.count(required) != 1:
            raise ValueError(f"required step must appear exactly once as an unconditional line (G-18): {required}")
    if "all available gates passed" in text:
        raise ValueError("ci_local must not claim that all available gates passed (G-18)")


def check_local_tools(root, env=None, timeout=15):
    """Each required tool answers its probe with exactly `<name> <pinned version>`."""
    values = versions(root)
    required, _ = local_gate(root)
    for name in required:
        argv, key = LOCAL_PROBES[name]
        try:
            result = subprocess.run(argv, cwd=root, env=env, stdin=subprocess.DEVNULL,
                                    capture_output=True, text=True, timeout=timeout, check=False)
        except (OSError, subprocess.TimeoutExpired) as error:
            raise ValueError(f"required tool unavailable: {name} (G-18)") from error
        lines = result.stdout.splitlines()
        if result.returncode != 0 or not lines:
            raise ValueError(f"required tool missing: {name}; install {values[key]} (G-18)")
        if lines[0].strip() != f"{name} {values[key]}" or len(lines) != 1:
            raise ValueError(f"required tool {name} reports {lines[0].strip()!r}, pinned {values[key]} (G-18)")


# G-16 (R-08). Every machine-readable Rust toolchain version in the repository names the pinned
# compiler (rust-toolchain.toml) or the MSRV (Cargo.toml), and lives in one of these reviewed files.
# A new site, or a site that drifts, fails here; a toolchain bump is one reviewed diff of all of them.
TOOLCHAIN_SITES = (
    ".github/workflows/ci.yml",
    ".github/workflows/mutation-baseline.yml",
    "CONTRIBUTING.md",
    "Cargo.toml",
    "plants/msrv_boundary.py",
    "rust-toolchain.toml",
)
# Dated history keeps the versions it recorded.
TOOLCHAIN_HISTORY = ("CHANGELOG.md",)
# Each form captures whatever names the toolchain, numeric or not, so a channel name (nightly, beta,
# stable) in any of them is drift too (Keel F4). The quoted argv form stays numeric-only.
# Whitespace is flexible in every form (Keel B4f, B4g): two spaces after cargo, or none around an
# equals sign, still make a site.
TOOLCHAIN_FORM = re.compile(
    r"""(?:toolchain\s+install\s+|toolchain\s+add\s+|rustup\s+install\s+|rustup\s+default\s+"""
    r"""|rustup\s+run\s+|rustup\s+update\s+|rustup\s+override\s+set\s+|cargo\s+\+"""
    r"""|--toolchain[\s=]+["']?|RUSTUP_TOOLCHAIN\s*[:=]\s*["']?|rust-toolchain@"""
    r"""|(?<![\w-])toolchain\s*[:=]\s*["']?|rust-version\s*=\s*"|channel\s*=\s*")"""
    r"""([A-Za-z0-9][A-Za-z0-9._-]*)"""
    r"""|["']\+([0-9]+\.[0-9]+(?:\.[0-9]+)?)\b""")
SKIPPED_DIRS = {".git", "target", "mutants.out", "mutants.out.old"}


def canonical_toolchains(values):
    msrv = values["msrv"]
    return {values["rust"], msrv, msrv if msrv.count(".") == 2 else msrv + ".0"}


def check_toolchain_sites(root):
    """Every toolchain version written in one of these forms is canonical and sits in a listed file.

    Covered, and nothing more (a list of spellings cannot be complete; Keel NOTE a99f6064):
    rustup default|run|install|update|override set|toolchain install|toolchain add <v>; cargo +<v>;
    --toolchain <v>; RUSTUP_TOOLCHAIN <v> (= or :, quoted or not); a toolchain key (: or =);
    rust-version = "<v>"; channel = "<v>"; an action ref rust-toolchain@<v>; a quoted argv '+<v>'.
    Prose ("Rust 1.99.1") is not scanned.
    """
    allowed = canonical_toolchains(versions(root))
    found = set()
    # A symlinked file is scanned through its link (Keel B7): skipping it let drift hide behind one.
    for path in toolchain_candidates(root):
        if path.is_file():
            scan_toolchain_file(root, path, allowed, found)
    if found != set(TOOLCHAIN_SITES):
        extra, missing = sorted(found - set(TOOLCHAIN_SITES)), sorted(set(TOOLCHAIN_SITES) - found)
        raise ValueError(f"toolchain sites differ from TOOLCHAIN_SITES (G-16): unlisted {extra}, absent {missing}")


def toolchain_candidates(root):
    """The files to scan: Git's tracked files when `root` is a work tree (so an untracked vendor/ or
    registry copy is not a site, Keel F5), else every file below `root` (a git-less export such as
    fw-verify's holds only tracked files, and walking everything is the stricter fallback)."""
    try:
        listed = subprocess.run(["git", "-C", str(root), "ls-files", "-z"], stdin=subprocess.DEVNULL,
                                capture_output=True, timeout=60, check=False)
    except (OSError, subprocess.TimeoutExpired):
        listed = None
    if listed is not None and listed.returncode == 0 and listed.stdout:
        names = sorted(n for n in listed.stdout.decode("utf-8", "surrogateescape").split("\0") if n)
        return [root / n for n in names if not SKIPPED_DIRS & set(Path(n).parts)]
    out = []
    for directory, subdirectories, files in os.walk(root):
        subdirectories[:] = sorted(d for d in subdirectories if d not in SKIPPED_DIRS)
        out.extend(Path(directory) / f for f in sorted(files))
    return out


def scan_toolchain_file(root, path, allowed, found):
    name = path.relative_to(root).as_posix()
    if name in TOOLCHAIN_HISTORY:
        return
    try:
        text = path.read_text(encoding="utf-8")
    except (UnicodeDecodeError, OSError):
        return
    for number, line in enumerate(text.splitlines(), 1):
        for match in TOOLCHAIN_FORM.finditer(line):
            found.add(name)
            value = match.group(1) or match.group(2)
            if value not in allowed:
                raise ValueError(f"{name}:{number}: toolchain {value} drifts from "
                                 f"rust-toolchain.toml/Cargo.toml (G-16)")


# Variables that make cargo build with a compiler other than the rustc the probe asks (Keel F3).
COMPILER_OVERRIDES = ("RUSTC", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "CARGO_BUILD_RUSTC",
                      "CARGO_BUILD_RUSTC_WRAPPER", "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER")
CONFIG_COMPILER_KEYS = ("rustc", "rustc-wrapper", "rustc-workspace-wrapper")


def cargo_configs(root, env):
    """Cargo config files that apply to a build in `root`: every `.cargo/config[.toml]` from `root` up,
    and the one in CARGO_HOME (default ~/.cargo)."""
    out, here = [], Path(root).resolve()
    for directory in (here, *here.parents):
        out.extend(directory / ".cargo" / n for n in ("config", "config.toml"))
    home = env.get("CARGO_HOME") or (str(Path(env["HOME"]) / ".cargo") if env.get("HOME") else None)
    if home:
        out.extend(Path(home) / n for n in ("config", "config.toml"))
    return [p for p in dict.fromkeys(out) if p.is_file()]


def config_compiler_keys(path):
    """The [build] compiler keys a cargo config sets; an unreadable config is refused, not skipped."""
    try:
        build = tomllib.loads(path.read_text()).get("build", {})
    except (OSError, ValueError) as error:
        raise ValueError(f"{path}: cargo config cannot be read: {error} (G-16)") from error
    return [k for k in CONFIG_COMPILER_KEYS if isinstance(build, dict) and k in build]


def check_toolchain(root, env=None, timeout=15):
    """The compiler that runs the local gate is the pinned one, chosen by rust-toolchain.toml alone."""
    env = dict(os.environ if env is None else env)
    pinned = versions(root)["rust"]
    for name in COMPILER_OVERRIDES:
        # Set at all, even empty, is refused: the gate does not depend on how cargo reads "".
        if name in env:
            raise ValueError(f"{name} is set, so cargo would not build with the probed rustc (G-16)")
    for config in cargo_configs(root, env):
        for key in config_compiler_keys(config):
            raise ValueError(f"{config}: build.{key} overrides the compiler for the gate (G-16)")
    if env.get("RUSTUP_TOOLCHAIN", pinned) != pinned:
        raise ValueError(f"RUSTUP_TOOLCHAIN={env['RUSTUP_TOOLCHAIN']} overrides the pinned {pinned} "
                         "for the gate (G-16)")
    # A missing pinned toolchain must refuse here, offline, rather than be downloaded.
    env["RUSTUP_AUTO_INSTALL"] = "0"
    # Whole stable versions only: `1.99.1-nightly` or `-beta` with the same number is refused (Keel F2).
    for argv, form in ((["rustc", "-Vv"], r"release: ([0-9]+\.[0-9]+\.[0-9]+)$"),
                       (["cargo", "-V"], r"cargo ([0-9]+\.[0-9]+\.[0-9]+) \(")):
        try:
            result = subprocess.run(argv, cwd=root, env=env, stdin=subprocess.DEVNULL,
                                    capture_output=True, text=True, timeout=timeout, check=False)
        except (OSError, subprocess.TimeoutExpired) as error:
            raise ValueError(f"{argv[0]} is unavailable for the gate (G-16)") from error
        found = re.findall(r"^" + form, result.stdout, re.MULTILINE)
        if result.returncode != 0 or len(found) != 1:
            raise ValueError(f"{argv[0]} did not report one version; is toolchain {pinned} installed? (G-16)")
        if found[0] != pinned:
            raise ValueError(f"{argv[0]} is {found[0]}, but rust-toolchain.toml pins {pinned} (G-16)")


if __name__ == "__main__":
    try:
        if sys.argv[1:] == ["--check"]:
            check(ROOT)
            check_ci_local(ROOT)
            check_toolchain_sites(ROOT)
        elif sys.argv[1:] == ["--check-local-tools"]:
            check_local_tools(ROOT)
            check_toolchain(ROOT)
        elif sys.argv[1:] == ["--check-toolchain"]:
            check_toolchain(ROOT)
        elif len(sys.argv) == 2:
            print(versions(ROOT)[sys.argv[1]])
        else:
            raise ValueError("usage: tool_versions.py KEY | --check | --check-local-tools | --check-toolchain")
    except (ValueError, KeyError, OSError) as error:
        sys.exit("tool versions: " + str(error))
