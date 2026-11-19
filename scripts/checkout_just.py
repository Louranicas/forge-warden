import os
from pathlib import Path
import shutil
import sys


if len(sys.argv) != 2 or sys.argv[1] not in {
    "install-hooks", "fast", "ci", "mutants", "judge", "judge-bound", "review",
}:
    sys.exit("expected one fixed checkout recipe")
justfile = Path.cwd() / "justfile"
if justfile.is_symlink() or not justfile.is_file():
    sys.exit("Gate input refused: justfile must be a regular checkout file")
just = shutil.which("just")
if just is None:
    sys.exit("Just is required on PATH to run checkout recipes")
env = {name: value for name, value in os.environ.items() if not name.startswith("JUST_")}
os.execve(just, [just, "--justfile", justfile, "--", sys.argv[1]], env)
