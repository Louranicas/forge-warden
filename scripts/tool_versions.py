#!/usr/bin/env python3
"""Read canonical inputs; reject drift in the workflow's declarative copies."""
import collections
import importlib.util
import re
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


if __name__ == "__main__":
    try:
        if sys.argv[1:] == ["--check"]:
            check(ROOT)
        elif len(sys.argv) == 2:
            print(versions(ROOT)[sys.argv[1]])
        else:
            raise ValueError("usage: tool_versions.py KEY | --check")
    except (ValueError, KeyError, OSError) as error:
        sys.exit("tool versions: " + str(error))
