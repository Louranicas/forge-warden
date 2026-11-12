"""The gate: one deterministic judgement of a patch.

    deep-diff-forge  senses   (patch truth, ranking, evidence)
    forge-warden     judges   (the repository's policy, read from the base commit)
    integrity/scope  judge    (agent failure modes; the declared task scope)
    Jev              advises  (optional; can only add a hold)

Unknown is never pass: a tool that is missing or refuses the patch holds or fails the result.
"""

from __future__ import annotations

import json
import tempfile
from pathlib import Path

from . import checks, oracle, scope as scope_mod
from .core import (EXIT_FAIL, EXIT_HOLD, EXIT_PASS, Config, EnvError, base_commit, build_patch,
                   find_tool, load_config, load_policy, matches, run, safe, sha256)

SCHEMA = "forge-stack.gate.v0"
MAX_SHOWN = 25
LOCAL_NOTES = {"rank_unverified"}  # expected locally: this script ran the ranker itself


def plain(check: str, severity: str, why: str, fix: str, path: str | None = None,
          source: str = "gate") -> dict:
    return {"check": check, "severity": severity, "path": path, "line": None, "side": None,
            "address": "", "text": "", "why": why, "fix": fix, "source": source}


def ddf_json(ddf: str, patch: bytes, *flags: str) -> tuple[dict | None, bytes, str]:
    """A deep-diff-forge JSON document for the patch: (parsed, its exact bytes, error)."""
    done = run([ddf, "--stdin-patch", *flags], stdin=patch)
    if done.returncode != 0:
        return None, b"", f"exit {done.returncode}: {safe(done.stderr.decode(errors='replace'), 300)}"
    try:
        return json.loads(done.stdout), done.stdout, ""
    except ValueError:
        return None, b"", "output was not JSON"


def warden_items(warden: str, policy: str, patch: bytes, rank_bytes: bytes) -> tuple[list[dict], list[str]]:
    with tempfile.TemporaryDirectory(prefix="forge-gate-") as tmp:
        files = {name: Path(tmp) / name for name in ("warden.toml", "change.patch", "rank.json")}
        files["warden.toml"].write_text(policy)
        files["change.patch"].write_bytes(patch)
        files["rank.json"].write_bytes(rank_bytes)
        done = run([warden, "check", "--policy", str(files["warden.toml"]), "--patch",
                    str(files["change.patch"]), "--advisory", "--rank", str(files["rank.json"]), "--json"])
    if done.returncode not in (0, 7, 8):
        message = safe(done.stderr.decode(errors="replace"), 300)
        return [plain("policy-unjudged", "hold", f"forge-warden could not judge the change ({message}).",
                      "Fix the policy or the environment, then run the gate again.", source="warden")], []
    try:
        verdict = json.loads(done.stdout)
    except ValueError:
        return [plain("policy-unjudged", "hold", "forge-warden's verdict was not JSON.",
                      "Report this as a bug.", source="warden")], []
    items, notes = [], []
    for rule in verdict.get("rules", []):
        if rule.get("outcome") not in ("fail", "hold"):
            continue
        for violation in rule.get("violations", []):
            if violation.get("waived_by"):
                continue
            evidence = (violation.get("evidence") or [{}])[0]
            items.append({"check": rule.get("id", "policy"), "severity": rule["outcome"],
                          "path": violation.get("path"), "line": evidence.get("line"),
                          "side": evidence.get("side"), "address": evidence.get("address", "")[:12],
                          "text": safe(violation.get("message", "")), "why": rule.get("why", ""),
                          "fix": "Change the code to satisfy the rule; the policy itself is not yours to edit.",
                          "source": "warden"})
    for waiver in verdict.get("expired_waivers", []):
        items.append(plain("waiver-expired", "fail", f"A waiver has expired: {safe(json.dumps(waiver), 200)}",
                           "Ask the waiver's owner to renew or remove it.", source="warden"))
    for note in verdict.get("notes", []):
        if note.get("code") in LOCAL_NOTES:
            notes.append("local run: forge-warden's authority mode in CI decides what may merge")
        else:
            items.append(plain(f"note-{note.get('code')}", "hold", safe(note.get("detail", ""), 300),
                               "A person must look at this.", source="warden"))
    return items, notes


def judge(patch: bytes, policy: str, cfg: Config, *, scope: dict | None, root: Path | None,
          policy_source: str = "", transport=None) -> dict:
    """Judge patch bytes. Pure but for the two tools it runs (and Jev, when configured)."""
    result = {"schema": SCHEMA, "patch_sha256": sha256(patch), "policy_sha256": sha256(policy.encode()),
              "policy_source": policy_source, "config_source": cfg.source, "files": [],
              "scope": scope, "items": [], "notes": []}
    items: list[dict] = result["items"]
    for problem in cfg.problems:
        items.append(plain("config-invalid", "hold", problem, "A person must fix the configuration."))
    if not patch.strip():
        result["notes"].append("no changes since the base")
        return finish(result)
    ddf, warden = find_tool("deep-diff-forge", "FORGE_DDF"), find_tool("forge-warden", "FORGE_WARDEN")
    if not ddf:
        items.append(plain("tool-missing", "hold", "deep-diff-forge is not installed, so the change is unjudged.",
                           "Install it: cargo install --git https://github.com/Louranicas/deep-diff-forge deep-diff-forge-cli --locked"))
        return finish(result)
    rank, rank_bytes, error = ddf_json(ddf, patch, "--rank", "--json")
    review, _, error2 = ddf_json(ddf, patch, "--json")
    if rank is None or review is None:
        items.append(plain("patch-refused", "fail", f"deep-diff-forge refused the patch ({error or error2}).",
                           "The change does not form a valid patch; fix the working tree.", source="ddf"))
        return finish(result)
    ranked = rank.get("ranked", [])
    result["files"] = [{"path": f.get("path"), "status": f.get("status"), "score": f.get("score"),
                        "signals": f.get("signals", [])} for f in ranked]
    paths = [f.get("path", "") for f in ranked]
    if warden:
        found, notes = warden_items(warden, policy, patch, rank_bytes)
        items += found
        result["notes"] += notes
    else:
        items.append(plain("tool-missing", "hold", "forge-warden is not installed, so the policy is unjudged.",
                           "Install it: cargo install --git https://github.com/Louranicas/forge-warden forge-warden-cli --locked"))
    items += checks.integrity(review)
    items += scope_mod.judge(scope, cfg.scope == "required", paths)
    for path in paths:
        if matches(path, cfg.protected):
            items.append(plain("protected-path", "hold", "This file decides what the other checks mean.",
                               "A person must approve changes to it.", path=path))
    if root is not None:
        kwargs = {"transport": transport} if transport else {}
        found, notes = oracle.advise(root, cfg, review, rank, (scope or {}).get("task"), **kwargs)
        items += [i for i in found if i["severity"] == "hold"]  # tighten only, by construction
        result["notes"] += notes
    return finish(result)


def finish(result: dict) -> dict:
    severities = {i["severity"] for i in result["items"]}
    result["verdict"] = "fail" if "fail" in severities else "hold" if "hold" in severities else "pass"
    result["exit"] = {"pass": EXIT_PASS, "hold": EXIT_HOLD, "fail": EXIT_FAIL}[result["verdict"]]
    order = {"fail": 0, "hold": 1}
    unique = {(i["check"], i["severity"], i["path"], i["line"], i["text"], i["why"]): i for i in result["items"]}
    result["items"] = list(unique.values())
    result["items"].sort(key=lambda i: (order.get(i["severity"], 2), i["path"] or "", i["line"] or 0, i["check"]))
    return result


def judge_worktree(root: Path, base_override: str | None = None, transport=None) -> dict:
    base = base_commit(root, base_override)
    cfg = load_config(root, base)
    policy, source = load_policy(root, base, cfg)
    patch = build_patch(root, base)
    result = judge(patch, policy, cfg, scope=scope_mod.load(root), root=root, policy_source=source,
                   transport=transport)
    result["base"] = base
    return result


def render(result: dict) -> str:
    verdict = result["verdict"].upper()
    nxt = {"pass": "proceed", "hold": "review_evidence", "fail": "change_input"}[result["verdict"]]
    fails = sum(1 for i in result["items"] if i["severity"] == "fail")
    holds = sum(1 for i in result["items"] if i["severity"] == "hold")
    lines = [f"forge gate: {verdict} (exit {result['exit']}, next: {nxt})",
             f"  patch {result['patch_sha256'][:12]}, {len(result['files'])} file(s), "
             f"{fails} fail, {holds} hold; policy: {result['policy_source'] or 'n/a'}"]
    scope = result.get("scope")
    if scope:
        lines.append(f"  scope: {safe(scope['task'], 80)} | allow {', '.join(scope['allow'])}"
                     f" | tests {scope.get('tests', 'open')}")
        for widening in scope.get("widenings", []):
            lines.append(f"    widened: {widening['glob']} ({safe(widening['reason'], 80)})")
    shown = result["items"][:MAX_SHOWN]
    for item in shown:
        where = safe(item["path"] or "(change)", 100)
        if item["line"]:
            where += f":{item['line']}" + (" (old side)" if item["side"] == "old" else "")
        hunk = f" [hunk {item['address']}]" if item["address"] else ""
        lines.append(f"  [{item['severity'].upper():4}] {item['check']} — {where}{hunk}")
        if item["text"]:
            lines.append(f"         {item['text']}")
        lines.append(f"         why: {item['why']}")
        lines.append(f"         fix: {item['fix']}")
    if len(result["items"]) > len(shown):
        lines.append(f"  … and {len(result['items']) - len(shown)} more item(s): forge gate --json lists all")
    for note in dict.fromkeys(result["notes"]):
        lines.append(f"  [note] {note}")
    if result["verdict"] == "hold":
        lines.append("  A hold is a person's decision. Report it; do not work around it.")
    return "\n".join(lines)


__all__ = ["judge", "judge_worktree", "render", "EnvError"]
