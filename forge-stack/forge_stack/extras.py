"""Reading, findings, blast radius, and the corpus of escaped defects."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

from . import gate
from .core import (EXIT_ENV, EXIT_FAIL, EXIT_PASS, PACK_ROOT, Config, EnvError, base_commit,
                   build_patch, find_tool, git, run, safe)

CORPUS = "forge-corpus"
DEFINES = re.compile(r"\b(?:fn|def|func|function|class|struct|enum|trait|interface|type|impl)\s+"
                     r"([A-Za-z_][A-Za-z0-9_]{2,})")


def need_ddf() -> str:
    ddf = find_tool("deep-diff-forge", "FORGE_DDF")
    if not ddf:
        raise EnvError("deep-diff-forge is not installed")
    return ddf


def passthrough(root: Path, base: str | None, flags: list[str]) -> int:
    """Outline or hunks of the session's change, straight from deep-diff-forge."""
    done = run([need_ddf(), "--stdin-patch", *flags], stdin=build_patch(root, base_commit(root, base)))
    sys.stdout.write(done.stdout.decode(errors="replace"))
    sys.stderr.write(done.stderr.decode(errors="replace"))
    return done.returncode


def findings(root: Path, base: str | None, file: str) -> int:
    """Check an agent's findings against the patch. An unanchored finding is refused."""
    done = run([need_ddf(), "findings", "verify", "--findings", file, "--require-findings"],
               stdin=build_patch(root, base_commit(root, base)))
    if done.returncode != 0:
        sys.stderr.write(done.stderr.decode(errors="replace"))
        return done.returncode
    doc = json.loads(done.stdout)
    accepted, refused = doc.get("accepted", []), doc.get("refused", [])
    print(f"forge findings: {len(accepted)} accepted, {len(refused)} refused "
          f"(patch {doc.get('input_sha256', '')[:12]}; claims are anchored, never verified)")
    for entry in accepted:
        line = entry.get("line") or {}
        print(f"  [ok]      {safe(str(entry.get('severity')), 10)} {safe(str(entry.get('path')), 80)}"
              f":{line.get('number', '-')} {safe(str(entry.get('title')), 80)}")
    for entry in refused:
        print(f"  [refused] {safe(str(entry.get('reason')), 30)}: {safe(json.dumps(entry.get('finding', entry)), 140)}")
    if refused:
        print("  Fix or drop each refused finding: it does not point at this change.")
    return EXIT_FAIL if refused else EXIT_PASS


def radius(root: Path, base: str | None, as_json: bool) -> int:
    """Names the change defines or sits inside, and where else the repository uses them.

    A textual search (`git grep -w`), not a call graph: it over-reports common names and
    misses dynamic use. It tells you where to look, not what is safe.
    """
    patch = build_patch(root, base_commit(root, base))
    review, _, error = gate.ddf_json(need_ddf(), patch, "--json")
    if review is None:
        raise EnvError(f"deep-diff-forge refused the patch ({error})")
    names: dict[str, set[str]] = {}
    changed: dict[str, list[tuple[int, int]]] = {}
    for file in review.get("files", []):
        path = file.get("path", "")
        for hunk in file.get("hunks", []):
            start = hunk.get("new_start") or 0
            changed.setdefault(path, []).append((start, start + (hunk.get("new_lines") or 0)))
            texts = [hunk.get("section") or ""] + [l.get("text", "") for l in hunk.get("lines", [])
                                                   if l.get("kind") in ("added", "removed")]
            for text in texts:
                for name in DEFINES.findall(text):
                    names.setdefault(name, set()).add(path)
    report = []
    for name in sorted(names)[:40]:
        done = run(["git", "grep", "-n", "-w", "-F", "-I", "--untracked", "-e", name], cwd=root)
        uses = []
        for row in done.stdout.decode(errors="replace").splitlines():
            parts = row.split(":", 2)
            if len(parts) < 3 or not parts[1].isdigit():
                continue
            path, number = parts[0], int(parts[1])
            if any(lo <= number < hi for lo, hi in changed.get(path, [])):
                continue  # inside the change itself
            uses.append({"path": path, "line": number, "text": safe(parts[2], 110)})
        report.append({"name": name, "changed_in": sorted(names[name]), "uses_elsewhere": len(uses),
                       "uses": uses[:12]})
    if as_json:
        print(json.dumps({"schema": "forge-stack.radius.v0", "names": report}, indent=2))
        return 0
    print(f"forge radius: {len(report)} name(s) touched by the change (textual search, not a call graph)")
    for entry in report:
        print(f"  {entry['name']} — changed in {', '.join(entry['changed_in'])}; "
              f"{entry['uses_elsewhere']} use(s) elsewhere")
        for use in entry["uses"]:
            print(f"      {use['path']}:{use['line']}  {use['text']}")
    return 0


# ---------------------------------------------------------------- corpus

def corpus_add(root: Path, base: str | None, case_id: str, origin: str, expect: list[str],
               patch_file: str | None, known_miss: bool) -> int:
    if not re.fullmatch(r"[a-z0-9][a-z0-9-]{2,60}", case_id):
        raise ValueError("--id must be kebab-case, 3 to 61 characters")
    if len(origin.strip()) < 12:
        raise ValueError("--origin must say where this defect came from (12+ characters)")
    pairs = []
    for entry in expect:
        check, _, path = entry.partition(":")
        if not check or not path:
            raise ValueError(f"--expect `{entry}` must be CHECK:PATH")
        pairs.append({"check": check, "path": path})
    if not pairs:
        raise ValueError("at least one --expect CHECK:PATH is required")
    patch = Path(patch_file).read_bytes() if patch_file else build_patch(root, base_commit(root, base))
    if not patch.strip():
        raise ValueError("the patch is empty")
    case = root / CORPUS / case_id
    if case.exists():
        raise ValueError(f"{CORPUS}/{case_id} already exists")
    case.mkdir(parents=True)
    (case / "change.patch").write_bytes(patch)
    (case / "case.json").write_text(json.dumps(
        {"schema": "forge-stack.case.v0", "id": case_id, "origin": origin.strip(), "expect": pairs,
         "known_miss": known_miss}, indent=2) + "\n")
    print(f"added {CORPUS}/{case_id} ({len(patch)} bytes, {len(pairs)} expectation(s)). Run: forge corpus run")
    return 0


def corpus_run(root: Path, as_json: bool) -> int:
    """Replay every case. A defect the gate once missed must stay caught."""
    policy = (PACK_ROOT / "defaults" / "warden.toml").read_text()
    cfg = Config(scope="advisory")
    rows = []
    for case_file in sorted((root / CORPUS).glob("*/case.json")):
        case = json.loads(case_file.read_text())
        result = gate.judge((case_file.parent / "change.patch").read_bytes(), policy, cfg, scope=None, root=None)
        seen = {(i["check"], i["path"]) for i in result["items"]}
        missing = [e for e in case["expect"] if (e["check"], e["path"]) not in seen]
        if not missing:
            status = "STALE" if case.get("known_miss") else "CAUGHT"
        else:
            status = "KNOWN" if case.get("known_miss") else "ESCAPED"
        rows.append({"id": case["id"], "status": status, "missing": missing, "verdict": result["verdict"]})
    counts = {s: sum(1 for r in rows if r["status"] == s) for s in ("CAUGHT", "KNOWN", "ESCAPED", "STALE")}
    if as_json:
        print(json.dumps({"schema": "forge-stack.corpus.v0", "counts": counts, "cases": rows}, indent=2))
    else:
        print(f"forge corpus: {len(rows)} case(s): {counts['CAUGHT']} caught, {counts['KNOWN']} declared miss, "
              f"{counts['ESCAPED']} escaped, {counts['STALE']} stale")
        for row in rows:
            extra = "" if not row["missing"] else " missing " + ", ".join(
                f"{m['check']}:{m['path']}" for m in row["missing"])
            if row["status"] == "STALE":
                extra = " now caught: remove known_miss from its case.json"
            print(f"  {row['status']:8} {row['id']}{extra}")
    if not rows:
        return EXIT_PASS
    return EXIT_FAIL if counts["ESCAPED"] or counts["STALE"] else EXIT_PASS


def doctor(root: Path) -> int:
    from . import oracle, scope as scope_mod
    from .core import load_config, load_policy
    base = base_commit(root)
    cfg = load_config(root, base)
    _, policy_source = load_policy(root, base, cfg)
    ok = True
    for name, env in (("deep-diff-forge", "FORGE_DDF"), ("forge-warden", "FORGE_WARDEN")):
        tool = find_tool(name, env)
        version = run([tool, "--version"]).stdout.decode().strip() if tool else "NOT INSTALLED"
        ok &= bool(tool)
        print(f"  {'ok ' if tool else 'BAD'} {name}: {version}")
    print(f"  ok  git: {git(root, '--version').decode().strip()}")
    print(f"  --  base: {base[:12] if base else 'none'}")
    print(f"  --  configuration: {cfg.source}" + (f" ({'; '.join(cfg.problems)})" if cfg.problems else ""))
    print(f"  --  policy: {policy_source}")
    scope = scope_mod.load(root)
    print(f"  --  scope ({cfg.scope}): " + (", ".join(scope_mod.allowed(scope)) if scope else "not declared"))
    print(f"  --  oracle (Jev): {oracle.availability(cfg)[1]}")
    hooked = "forge.py" in (root / ".claude" / "settings.json").read_text() if (root / ".claude" / "settings.json").is_file() else False
    print(f"  {'ok ' if hooked else 'BAD'} hooks: {'installed' if hooked else 'not installed (run: forge init --apply)'}")
    return EXIT_PASS if ok and hooked and not cfg.problems else EXIT_ENV
