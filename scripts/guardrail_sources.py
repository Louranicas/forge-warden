"""Fresh rustc dep-info admission for first-party explicit lint attributes.

This is deliberately a restricted dep-info language: unsupported path spellings
fail closed. It does not claim to inspect generated macro expansions.
"""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile


def require(condition, message):
    if not condition:
        raise ValueError(message)


def dependencies(text):
    """Consume every rule; refuse escapes/metacharacters rather than guess Make."""
    result = set()
    for line in text.splitlines():
        if not line:
            continue
        # rustc may append environment comments; they are not source rules.
        if line.startswith("# env-dep:"):
            continue
        require(re.fullmatch(r"[A-Za-z0-9_./+-]+:?(?: [A-Za-z0-9_./+-]+)*", line)
                is not None and line.count(":") == 1,
                "unsupported rustc dep-info syntax: " + repr(line))
        target, inputs = line.split(":", 1)
        require(bool(target), "empty dep-info target")
        result.update(inputs.split())
    require(result, "empty compiler dep-info")
    return result


def admitted_path(root, spelling):
    """Check every traversed component before resolving '..' or reading bytes."""
    raw = Path(spelling)
    path = raw if raw.is_absolute() else root / raw
    cursor = Path(path.anchor)
    for part in path.parts[1:]:
        cursor = cursor / part
        require(not cursor.is_symlink(), "symlinked compiler input: " + spelling)
    canonical = path.resolve(strict=True)
    require(canonical.is_relative_to(root), "out-of-root compiler input: " + spelling)
    require(canonical.is_file(), "non-file compiler input: " + spelling)
    return canonical.relative_to(root).as_posix()


def admit(root, records, data_inputs):
    sources = set()
    seen_data = set()
    for spelling, test in records:
        name = admitted_path(root, spelling)
        if name.endswith(".rs"):
            parts = Path(name).parts
            ordinary = len(parts) >= 4 and parts[0] == "crates" and parts[2] in ("src", "tests")
            helper = name.startswith("fixtures/adversarial/") and test
            require(ordinary or helper, "source outside admitted Rust roots: " + name)
            require(not (ordinary and parts[2] == "tests" and not test),
                    "test source compiled in production: " + name)
            sources.add(name)
        else:
            # include! need not use .rs. Non-Rust inputs therefore need exact
            # reviewed byte hashes, never an extension-based silent exemption.
            require(name in data_inputs, "unreviewed non-Rust compiler input: " + name)
            require(hashlib.sha256((root / name).read_bytes()).hexdigest() == data_inputs[name],
                    "reviewed compiler data changed: " + name)
            seen_data.add(name)
    require(seen_data == set(data_inputs), "stale compiler data ledger")
    walked = set()
    for member in (root / "crates").iterdir():
        for directory in (member / "src", member / "tests"):
            if directory.exists():
                for path in directory.rglob("*.rs"):
                    walked.add(admitted_path(root, str(path)))
    require(walked == {name for name in sources if name.startswith("crates/")},
            "compiler/source walk disagreement (inactive or uncompiled Rust source)")
    require(sources, "no first-party compiled sources")
    return sorted(sources)


def discover(root, data_inputs):
    root = root.resolve(strict=True)
    env = os.environ.copy()
    # Fresh artifacts, with no inherited wrapper or dependency path rewriting.
    for key in ("CARGO_TARGET_DIR", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER",
                "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"):
        env.pop(key, None)
    def cargo(args):
        run = subprocess.run(["cargo", *args], cwd=root, env=env, text=True,
                             capture_output=True, check=False)
        require(run.returncode == 0, "source discovery cargo failed: " + run.stderr[-4000:])
        return run.stdout
    metadata = json.loads(cargo(["metadata", "--no-deps", "--locked", "--offline", "--format-version=1"]))
    members = set(metadata["workspace_members"])
    with tempfile.TemporaryDirectory(prefix="fw-dep-info-") as target:
        messages = cargo(["check", "--workspace", "--all-targets", "--locked", "--offline",
                          "--message-format=json", "--target-dir", target])
        records = set()
        observed = set()
        for line in messages.splitlines():
            message = json.loads(line)
            if message.get("reason") != "compiler-artifact" or message["package_id"] not in members:
                continue
            require(not message["fresh"], "cached first-party artifact in fresh discovery")
            require("custom-build" not in message["target"]["kind"], "first-party build scripts unsupported")
            observed.add(message["package_id"])
            candidates = set()
            for filename in message["filenames"]:
                artifact = Path(filename)
                stem = artifact.stem
                for candidate in (artifact.with_suffix(".d"), artifact.with_name(stem.removeprefix("lib") + ".d")):
                    if candidate.is_file():
                        candidates.add(candidate)
            require(len(candidates) == 1, "ambiguous or missing first-party dep-info")
            depfile = candidates.pop()
            require(depfile.resolve().is_relative_to(Path(target).resolve()), "dep-info outside fresh target")
            for name in dependencies(depfile.read_text()):
                records.add((name, message["profile"]["test"]))
        require(observed == members, "workspace member missing compiler evidence")
        return admit(root, records, data_inputs)
