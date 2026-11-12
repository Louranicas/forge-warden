"""Optional second opinion from Jev (TypeSafe's hosted System One model).

Jev answers typed questions with probabilities. Here it may only *tighten*: an answer can
add a `hold`, and can never remove an item, produce a pass or fail a change on its own.

Rules this module keeps:
- Off unless the base commit's configuration turns it on and TYPESAFE_API_KEY is set.
- The model must be pinned (`jev-1.14.0`, never `jev-latest`), so answers are replayable.
- A hunk from a file deep-diff-forge marks `secret_like` or `hazardous_text` is never sent.
- Answers are cached by hunk content, question set and model: an unchanged hunk costs nothing.
- When the configuration says `required` and no answer can be had, the change holds.

NOT TESTED AGAINST THE LIVE SERVICE. The request and answer shapes follow public write-ups
of `POST /v1/systemone`; the tests use a recorded-shape transport. Run `forge doctor --oracle`
with a key before relying on it. Sending code to Jev uploads that code to a third party.
"""

from __future__ import annotations

import json
import os
import urllib.error
import urllib.request
from pathlib import Path

from .core import Config, read_json, safe, sha256, state_dir, write_json

URL = "https://api.typesafe.ai/v1/systemone"
QUESTION_SET = "forge-stack.questions.v1"
NEVER_SEND = {"secret_like", "hazardous_text"}
MAX_STATE_CHARS = 60_000
QUESTIONS = {
    "guard_removed": "Does `change` remove or weaken an authorization, validation or safety check without replacing it?",
    "test_weakened": "Does `change` make a test weaker: skipping it, removing or loosening an assertion, or asserting something always true?",
    "error_swallowed": "Does `change` cause an error or failure to be ignored or hidden from the caller?",
    "stubbed": "Does `change` leave functionality unimplemented or replaced by a placeholder?",
}
OFF_TASK = "Is `change` unrelated to the task described in `task`?"
WHY = {
    "guard_removed": "Jev judges that a guard was probably removed or weakened.",
    "test_weakened": "Jev judges that a test was probably weakened.",
    "error_swallowed": "Jev judges that an error is probably being hidden.",
    "stubbed": "Jev judges that this is probably a placeholder.",
    "off_task": "Jev judges that this hunk is probably unrelated to the declared task.",
}
PLAYBOOKS = {
    "bug": "Fix a defect: reproduce it with a failing test first.",
    "feature": "Add new behaviour behind new tests.",
    "refactor": "Change structure without changing behaviour; tests are frozen.",
    "review": "Review an existing change and report findings.",
    "investigate": "Explain how or why something works; no edits.",
    "other": "None of the above fits.",
}


def http_transport(body: dict, key: str, timeout: float = 5.0) -> dict:
    request = urllib.request.Request(
        URL, data=json.dumps(body).encode(), method="POST",
        headers={"Authorization": f"Bearer {key}", "Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=timeout) as response:  # noqa: S310 (fixed https URL)
        return json.loads(response.read())


def availability(cfg: Config) -> tuple[bool, str]:
    if cfg.oracle_mode == "off":
        return False, "off"
    if not cfg.oracle_model or cfg.oracle_model.endswith(("latest", "preview")):
        return False, "the oracle model must be a pinned version such as jev-1.14.0"
    if not os.environ.get("TYPESAFE_API_KEY"):
        return False, "TYPESAFE_API_KEY is not set"
    return True, f"{cfg.oracle_model}, hold at {cfg.oracle_hold_at}"


def hunk_text(hunk: dict) -> str:
    sign = {"added": "+", "removed": "-"}
    return "\n".join(sign.get(l.get("kind"), " ") + l.get("text", "") for l in hunk.get("lines", []))


def advise(root: Path, cfg: Config, review: dict, rank: dict, task: str | None,
           transport=http_transport) -> tuple[list[dict], list[str]]:
    """Hold items from Jev's answers, and notes about what was skipped or failed."""
    if cfg.oracle_mode == "off":
        return [], []
    ok, reason = availability(cfg)

    def unavailable(why: str) -> tuple[list[dict], list[str]]:
        if cfg.oracle_mode == "required":
            return [{"check": "oracle-unavailable", "severity": "hold", "path": None, "line": None,
                     "side": None, "address": "", "text": "", "source": "oracle",
                     "why": f"The policy requires Jev's advice and it could not be had: {why}.",
                     "fix": "Restore access to Jev, or have a person review this change."}], []
        return [], [f"oracle skipped: {why}"]

    if not ok:
        return unavailable(reason)
    withheld = {f["path"] for f in rank.get("ranked", []) if NEVER_SEND & set(f.get("signals", []))}
    cache_path = state_dir(root) / "oracle-cache.json"
    cache = read_json(cache_path, {})
    questions = {k: {"type": "noul", "instructions": v} for k, v in QUESTIONS.items()}
    if task:
        questions["off_task"] = {"type": "noul", "instructions": OFF_TASK}
    items: list[dict] = []
    notes: list[str] = []
    if withheld:
        notes.append(f"oracle: {len(withheld)} file(s) withheld from Jev (secret or hidden-text signals)")
    for file in review.get("files", []):
        path = file.get("path", "")
        if path in withheld:
            continue
        for hunk in file.get("hunks", []):
            text = hunk_text(hunk)
            if len(text) > MAX_STATE_CHARS:
                notes.append(f"oracle: a hunk of {safe(path)} is too large to send; not advised")
                continue
            key = sha256("\0".join([hunk.get("body_sha256", sha256(text.encode())), QUESTION_SET,
                                    cfg.oracle_model, task or ""]).encode())
            answers = cache.get(key)
            if answers is None:
                body = {"model": cfg.oracle_model, "questions": questions,
                        "state": {"path": path, "change": text, **({"task": task} if task else {})}}
                try:
                    reply = transport(body, os.environ["TYPESAFE_API_KEY"])
                    answers = {name: float(reply["answers"][name]["noul"]) for name in questions}
                except (urllib.error.URLError, OSError, KeyError, TypeError, ValueError) as exc:
                    return unavailable(f"request failed ({type(exc).__name__})")
                if not all(0.0 <= value <= 1.0 for value in answers.values()):
                    return unavailable("an answer was outside 0..1")
                cache[key] = answers
            for name, value in sorted(answers.items()):
                if name in questions and value >= cfg.oracle_hold_at:
                    items.append({"check": f"oracle-{name.replace('_', '-')}", "severity": "hold",
                                  "path": path, "line": hunk.get("new_start") or hunk.get("old_start"),
                                  "side": "new", "address": hunk.get("address", "")[:12],
                                  "text": f"p={value:.2f} ({cfg.oracle_model})", "why": WHY[name],
                                  "fix": "Read the hunk and either fix it or say why Jev is wrong.",
                                  "source": "oracle"})
    write_json(cache_path, cache)
    return items, notes


def route(cfg: Config, task: str, transport=http_transport) -> tuple[str | None, float, str]:
    """Jev's pick of a playbook for a task: (playbook or None, confidence, note)."""
    ok, reason = availability(cfg)
    if not ok:
        return None, 0.0, f"oracle {reason}"
    body = {"model": cfg.oracle_model, "state": {"task": task},
            "questions": {"playbook": {"type": "choice", "criteria": PLAYBOOKS,
                                       "instructions": "Which playbook fits the work asked for in `task`?"}}}
    try:
        answer = transport(body, os.environ["TYPESAFE_API_KEY"])["answers"]["playbook"]
        choice, confidence = str(answer["choice"]), float(answer["confidence"])
    except (urllib.error.URLError, OSError, KeyError, TypeError, ValueError) as exc:
        return None, 0.0, f"oracle request failed ({type(exc).__name__})"
    if choice not in PLAYBOOKS or choice == "other" or confidence < 0.6:
        return None, confidence, "Jev was not confident; choose from the table"
    return choice, confidence, cfg.oracle_model
