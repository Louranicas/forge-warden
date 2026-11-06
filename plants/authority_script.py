#!/usr/bin/env python3
"""Name the authority exit-propagation failure in the maintained shell fixture suite."""
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    # Use the product suite and its existing controlled commands. Add only a diagnostic to its
    # existing exact-exit assertion, in a separate copy; never change the checked-out test source.
    source = (ROOT / "scripts/tests/ci_local.sh").read_text()
    old = '[ "$actual" -eq "$JUDGE_EXIT" ] || { cat "$sandbox/output"; exit 1; }'
    new = ('[ "$actual" -eq "$JUDGE_EXIT" ] || { '
           'printf \'AUTHORITY_JUDGE_EXIT_MISMATCH expected=%s actual=%s\\n\' '
           '"$JUDGE_EXIT" "$actual"; cat "$sandbox/output"; exit 1; }')
    if source.count(old) != 1:
        raise ValueError("authority exact-exit assertion changed; review this fixture adapter")
    with tempfile.TemporaryDirectory(prefix="fw-authority-script-") as directory:
        checkout = Path(directory)
        for name in ("scripts/ci_local.sh", "scripts/judge_mode.py", ".githooks/pre-push"):
            target = checkout / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, target)
        suite = checkout / "scripts/tests/ci_local.sh"
        suite.parent.mkdir(parents=True)
        suite.write_text(source.replace(old, new))
        return subprocess.run(["bash", str(suite)], cwd=checkout, check=False).returncode


if __name__ == "__main__":
    raise SystemExit(main())
