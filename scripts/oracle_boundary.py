#!/usr/bin/env python3
"""Compile an oracle as a standalone target with only Rust's sysroot available."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def main():
    source = sys.stdin.buffer.read(65537)
    if len(source) > 65536:
        raise ValueError("ORACLE_STANDALONE_BOUNDARY: source exceeds 64 KiB")
    # Ignore TMPDIR and all compiler injection variables. No harness, Cargo metadata,
    # --extern, dependency search directory, build script, or test cfg enters this target.
    admitted = {key: value for key, value in os.environ.items()
                if key in {"PATH", "HOME", "RUSTUP_HOME", "RUSTUP_TOOLCHAIN", "CARGO_HOME"}}
    admitted.update(LC_ALL="C", LANG="C")
    with tempfile.TemporaryDirectory(prefix="fw-oracle-", dir="/tmp") as directory:
        root = Path(directory)
        (root / "oracle.rs").write_bytes(source)
        (root / "lib.rs").write_text("mod oracle;\n")
        out = subprocess.run([
            "rustc", "--crate-name", "fw_oracle_boundary", "--crate-type", "lib",
            "--edition=2024", "--emit=metadata", "--cap-lints", "allow",
            str(root / "lib.rs"), "-o", str(root / "boundary.rmeta"),
        ], env=admitted, stdin=subprocess.DEVNULL, capture_output=True, timeout=30, check=False)
        if out.returncode:
            sys.stderr.write("ORACLE_STANDALONE_BOUNDARY: standalone compilation refused\n")
            sys.stderr.buffer.write(out.stderr)
            return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
