#!/usr/bin/env python3
"""Read canonical inputs; reject drift in the workflow's declarative copies."""
import collections
import importlib.util
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
    "deep-diff-forge": "warden.toml has no [producer] pin, so a local dogfood run can only be advisory",
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
    _, hosted = local_gate(root)
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


if __name__ == "__main__":
    try:
        if sys.argv[1:] == ["--check"]:
            check(ROOT)
            check_ci_local(ROOT)
        elif sys.argv[1:] == ["--check-local-tools"]:
            check_local_tools(ROOT)
        elif len(sys.argv) == 2:
            print(versions(ROOT)[sys.argv[1]])
        else:
            raise ValueError("usage: tool_versions.py KEY | --check | --check-local-tools")
    except (ValueError, KeyError, OSError) as error:
        sys.exit("tool versions: " + str(error))
