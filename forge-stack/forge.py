#!/usr/bin/env python3
"""forge: the agent-first front door to deep-diff-forge and forge-warden.

  forge gate [--json] [--base REV]        judge the session's change: 0 pass · 7 fail · 8 hold
  forge scope declare|show|widen|clear    say what the task may touch, before editing
  forge outline | forge hunks ADDRESS…    read the change cheaply, then exactly
  forge findings FILE                     anchor-check review findings (JSONL)
  forge radius [--json]                   where else the changed names are used
  forge corpus add|run                    escaped defects, replayed for ever
  forge route "TASK"                      pick a playbook (Jev, when configured)
  forge doctor | forge init [--apply]     check or install the setup
  forge hook session-start|pre-tool|stop  Claude Code hooks (JSON on stdin)

Exit codes: 0 pass/ok · 2 usage · 3 environment · 4 invalid input · 7 fail · 8 hold.
Results go to stdout, diagnostics to stderr. Python 3.11+, standard library only.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

sys.dont_write_bytecode = True  # never leave __pycache__ in the repository being judged
sys.path.insert(0, str(Path(__file__).resolve().parent))

from forge_stack import extras, gate, hooks, install, oracle, scope  # noqa: E402
from forge_stack.core import (EXIT_ENV, EXIT_USAGE, EnvError, base_commit, load_config,  # noqa: E402
                              repo_root)


def parser() -> argparse.ArgumentParser:
    top = argparse.ArgumentParser(prog="forge", description=__doc__,
                                  formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = top.add_subparsers(dest="cmd", required=True)

    def add(name: str, **kwargs) -> argparse.ArgumentParser:
        p = sub.add_parser(name, **kwargs)
        p.add_argument("--base", help="measure the change from this commit instead of the session's start")
        return p

    add("gate").add_argument("--json", action="store_true")
    add("outline")
    hunks = add("hunks")
    hunks.add_argument("addresses", nargs="*")
    hunks.add_argument("--path", action="append", default=[])
    add("findings").add_argument("file")
    add("radius").add_argument("--json", action="store_true")
    add("doctor")
    add("route").add_argument("task")

    sc = sub.add_parser("scope").add_subparsers(dest="action", required=True)
    declare = sc.add_parser("declare")
    declare.add_argument("--task", required=True)
    declare.add_argument("--allow", action="append", required=True)
    declare.add_argument("--tests", choices=["open", "frozen"], default="open")
    widen = sc.add_parser("widen")
    widen.add_argument("--allow", action="append", required=True)
    widen.add_argument("--reason", required=True)
    sc.add_parser("show")
    sc.add_parser("clear")

    co = sub.add_parser("corpus").add_subparsers(dest="action", required=True)
    cadd = co.add_parser("add")
    cadd.add_argument("--id", required=True)
    cadd.add_argument("--origin", required=True)
    cadd.add_argument("--expect", action="append", default=[], metavar="CHECK:PATH")
    cadd.add_argument("--patch")
    cadd.add_argument("--base")
    cadd.add_argument("--known-miss", action="store_true")
    co.add_parser("run").add_argument("--json", action="store_true")

    init = sub.add_parser("init")
    init.add_argument("--apply", action="store_true")
    init.add_argument("--force", action="store_true")
    sub.add_parser("hook").add_argument("name")
    return top


def main(argv: list[str]) -> int:
    args = parser().parse_args(argv)
    if args.cmd == "hook":
        return hooks.main(args.name)
    root = repo_root()
    base = getattr(args, "base", None)
    if args.cmd == "gate":
        result = gate.judge_worktree(root, base)
        print(json.dumps(result, indent=2) if args.json else gate.render(result))
        return result["exit"]
    if args.cmd == "outline":
        return extras.passthrough(root, base, ["--outline"])
    if args.cmd == "hunks":
        flags = ["--hunks"]
        for address in args.addresses:
            flags += ["--address", address]
        for path in args.path:
            flags += ["--path", path]
        return extras.passthrough(root, base, flags)
    if args.cmd == "findings":
        return extras.findings(root, base, args.file)
    if args.cmd == "radius":
        return extras.radius(root, base, args.json)
    if args.cmd == "doctor":
        return extras.doctor(root)
    if args.cmd == "route":
        choice, confidence, note = oracle.route(load_config(root, base_commit(root)), args.task)
        print(json.dumps({"playbook": choice, "confidence": round(confidence, 3), "note": note}))
        return 0
    if args.cmd == "scope":
        if args.action == "declare":
            declared = scope.declare(root, args.task, args.allow, args.tests, base_commit(root))
        elif args.action == "widen":
            declared = scope.widen(root, args.allow, args.reason)
        elif args.action == "clear":
            scope.clear(root)
            declared = None
        else:
            declared = scope.load(root)
        print(json.dumps(declared, indent=2) if declared else "no scope is declared")
        return 0
    if args.cmd == "corpus":
        if args.action == "add":
            return extras.corpus_add(root, args.base, args.id, args.origin, args.expect, args.patch,
                                     args.known_miss)
        return extras.corpus_run(root, args.json)
    if args.cmd == "init":
        return install.init(root, args.apply, args.force)
    return EXIT_USAGE


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except EnvError as error:
        print(f"forge: {error}\nnext: check_environment", file=sys.stderr)
        sys.exit(EXIT_ENV)
    except ValueError as error:
        print(f"forge: {error}\nnext: fix_invocation", file=sys.stderr)
        sys.exit(EXIT_USAGE)
    except BrokenPipeError:
        sys.exit(0)
