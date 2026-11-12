"""Integrity checks for the ways agents most often write bad code.

These read deep-diff-forge's parsed review (never the raw patch), line by line. They are
lexical: a pattern in a comment or a string can match, and code assembled at run time is
missed. They exist until deep-diff-forge carries these as evidence signals of its own.
Every item names a path and a line so the finding can be checked by reading one hunk.
"""

from __future__ import annotations

import re

from .core import is_test_path, safe

SKIP = re.compile(
    r"#\[ignore\b|@pytest\.mark\.(skip|xfail)|@unittest\.skip|\bpytest\.skip\(|\bt\.Skip(f|Now)?\("
    r"|\b(it|test|describe)\.(skip|todo)\(|\bx(it|describe|test)\(|@Disabled\b|@Ignore\b"
)
TRIVIAL = re.compile(
    r"\bassert!\(\s*true\s*\)|\bassert_eq!\(\s*(true|1)\s*,\s*(true|1)\s*\)|^\s*assert\s+True\s*$"
    r"|\bassert\(\s*true\s*\)|expect\(\s*true\s*\)\.(toBe|toEqual)\(\s*true\s*\)"
    r"|\bassertTrue\(\s*[Tt]rue\s*\)"
)
STUB = re.compile(
    r"\b(todo|unimplemented)!\(|\braise\s+NotImplementedError\b"
    r"|(throw\s+new\s+Error|panic)\(\s*[\"'`]\s*(not\s+implemented|todo|unimplemented)", re.I
)
MARKER = re.compile(r"\b(TODO|FIXME|XXX|HACK)\b(?!\(#?\w+)")
ASSERT = re.compile(r"\bassert\w*!?\s*[\(\s]|\bexpect\(|\.should\b|\brequire\.\w+\(|\bt\.(Error|Fatal)")
EXCEPT = re.compile(r"^\s*except\b.*:\s*(pass\s*)?(#.*)?$")
EMPTY_CATCH = re.compile(r"\bcatch\s*(\([^)]*\))?\s*\{\s*\}")
GO_DROPPED = re.compile(r"^\s*_\s*(,\s*_\s*)?:?=\s*.*\berr\b|^\s*_\s*=\s*err\b")
UNWRAP = re.compile(r"\.unwrap\(\)|\.expect\(")
GUARD = re.compile(
    r"^\s*(if|guard|unless|assert|require|ensure)\b.*"
    r"(auth|admin|permission|privilege|role|token|owner|allowed|forbid|verify|valid|csrf|signature)", re.I
)
GUARD_WORD = re.compile(
    r"auth|admin|permission|privilege|role|token|owner|allowed|forbid|verify|valid|csrf|signature", re.I
)

WHY = {
    "test-skipped": ("fail", "A skipped test checks nothing, so the behaviour it pinned is now unguarded.",
                     "Restore the test and make it pass, or fix the code it exposes."),
    "test-trivial-assert": ("fail", "An assertion that is always true cannot fail, so the test proves nothing.",
                            "Assert the behaviour the test is named for."),
    "assertions-removed": ("hold", "This test file lost more assertions than it gained.",
                           "Keep the removed checks, or say in your summary why each is obsolete."),
    "expectations-changed": ("hold", "Test expectations were rewritten in the same change as the code they test.",
                             "Confirm the new expectation is the intended behaviour, not the bug made official."),
    "stub-added": ("fail", "A placeholder is unfinished work presented as a change.",
                   "Implement it, or remove the function and say what remains."),
    "marker-added": ("hold", "An untracked TODO is a promise nobody owns.",
                     "Finish it now, or reference an issue: TODO(#123)."),
    "error-swallowed": ("hold", "An ignored error turns a failure into silent wrong behaviour.",
                        "Handle the error or return it to the caller."),
    "panic-path-added": ("hold", "unwrap/expect turns a recoverable error into a crash.",
                         "Return a Result, or justify why this cannot fail."),
    "guard-removed": ("hold", "A check that looks like an authorization or validation guard was removed.",
                      "Keep the guard, or point to where the check now happens."),
}


def item(check: str, path: str, line: int | None, side: str, address: str, text: str) -> dict:
    severity, why, fix = WHY[check]
    return {"check": check, "severity": severity, "path": path, "line": line, "side": side,
            "address": address[:12], "text": safe(text), "why": why, "fix": fix, "source": "integrity"}


def integrity(review: dict) -> list[dict]:
    """Every integrity item for a `deep-diff-forge.review.v0` document, in file order."""
    found: list[dict] = []
    source_changed = any(not is_test_path(f.get("path", "")) and f.get("hunks")
                         for f in review.get("files", []))
    for file in review.get("files", []):
        path = file.get("path", "")
        in_test = is_test_path(path)
        rust = path.endswith(".rs")
        lost = gained = 0
        first_lost: tuple | None = None
        for hunk in file.get("hunks", []):
            address = hunk.get("address", "")
            lines = hunk.get("lines", [])
            added = [l for l in lines if l.get("kind") == "added"]
            removed = [l for l in lines if l.get("kind") == "removed"]
            added_guard_words = sum(1 for l in added if GUARD_WORD.search(l.get("text", "")))
            for index, line in enumerate(lines):
                text = line.get("text", "")
                if line.get("kind") == "added":
                    number = line.get("new_line")

                    def hit(check: str) -> None:
                        found.append(item(check, path, number, "new", address, text))

                    if SKIP.search(text):
                        hit("test-skipped")
                    if TRIVIAL.search(text):
                        hit("test-trivial-assert")
                    if STUB.search(text):
                        hit("stub-added")
                    elif MARKER.search(text):
                        hit("marker-added")
                    following = next((l.get("text", "") for l in lines[index + 1:]
                                      if l.get("kind") in ("added", "context")), "")
                    if (EXCEPT.match(text) and ("pass" in text.split(":", 1)[-1]
                                                or following.strip() == "pass")) \
                            or EMPTY_CATCH.search(text) or GO_DROPPED.search(text):
                        hit("error-swallowed")
                    if rust and not in_test and UNWRAP.search(text):
                        hit("panic-path-added")
                    if in_test and ASSERT.search(text) and not TRIVIAL.search(text):
                        gained += 1
                elif line.get("kind") == "removed":
                    if in_test and ASSERT.search(text):
                        lost += 1
                        first_lost = first_lost or (line.get("old_line"), address, text)
            if not in_test:
                guards = [l for l in removed if GUARD.search(l.get("text", ""))]
                if guards and added_guard_words < len(guards):
                    first = guards[0]
                    found.append(item("guard-removed", path, first.get("old_line"), "old", address,
                                      first.get("text", "")))
        if in_test and lost > gained and first_lost:
            found.append(item("assertions-removed", path, first_lost[0], "old", first_lost[1],
                              f"{lost} assertion line(s) removed, {gained} added; first: {first_lost[2]}"))
        elif in_test and lost and first_lost and source_changed and file.get("status") != "added":
            found.append(item("expectations-changed", path, first_lost[0], "old", first_lost[1],
                              first_lost[2]))
    return found
