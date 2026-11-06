#!/usr/bin/env bash
# Fail unless forge-warden's signal vocabulary (crates/forge-warden-core/src/vocab.rs) equals the
# labels deep-diff-forge defines at commit $1. Usage:
#   scripts/check_vocab.sh <deep-diff-forge commit> [path to a deep-diff-forge git checkout]
#
# What is read is the named commit's tree (`git archive <commit>`), never a checkout's working tree,
# and the commit must resolve, so the output attests exactly what was compared (R-09).
# Labels are extracted per signal enum (RiskSignal in risk.rs, EvidenceSignal in evidence.rs): every
# enum variant must have a parsed label arm, so a label the extractor cannot read fails the check
# instead of silently dropping out of the comparison (R-16).
set -euo pipefail
rev="${1:?usage: check_vocab.sh <deep-diff-forge commit> [checkout]}"
# The forge-warden tree this script belongs to, found from the script itself: it need not be a Git
# checkout (mutation runs and release tarballs are not), and the caller's cwd does not matter.
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

if [ -n "${2:-}" ]; then
  repo="$2"
else
  repo="$tmp/repo"
  git init -q "$repo"
  git -C "$repo" fetch -q --depth 1 https://github.com/Louranicas/deep-diff-forge "$rev"
fi
if ! commit="$(git -C "$repo" rev-parse --verify --quiet "${rev}^{commit}")"; then
  echo "check_vocab: deep-diff-forge commit ${rev} does not exist in ${repo}" >&2
  exit 1
fi
mkdir "$tmp/tree"
git -C "$repo" archive "$commit" crates/deep-diff-forge-graph/src | tar -x -C "$tmp/tree"

python3 -I - "$tmp/tree/crates/deep-diff-forge-graph/src" "$root/crates/forge-warden-core/src/vocab.rs" "$commit" <<'PY'
import re
import sys

src, vocab, commit = sys.argv[1], sys.argv[2], sys.argv[3]

def block(text, start):
    """The text between the first `{` at or after `start` and its matching `}`."""
    open_at = text.index("{", start)
    depth = 0
    for i in range(open_at, len(text)):
        depth += {"{": 1, "}": -1}.get(text[i], 0)
        if depth == 0:
            return text[open_at + 1:i]
    raise SystemExit("check_vocab: unbalanced braces")

def strip_comments(text):
    return re.sub(r"//[^\n]*", "", text)

CHECKED = {"risk.rs": "RiskSignal", "evidence.rs": "EvidenceSignal"}
labels, problems = set(), []
for file, enum in CHECKED.items():
    text = strip_comments(open(f"{src}/{file}").read())
    enum_at = re.search(rf"pub enum {enum}\s*\{{", text)
    impl_at = re.search(rf"impl {enum}\s*\{{", text)
    if not enum_at or not impl_at:
        problems.append(f"{file}: enum or impl {enum} not found")
        continue
    variants = {}
    for item in block(text, enum_at.start()).split(","):
        item = re.sub(r"#\[[^\]]*\]", "", item).strip()
        m = re.match(r"([A-Z][A-Za-z0-9]*)\s*(\((\w+)\))?$", item)
        if item and not m:
            problems.append(f"{file}: unreadable {enum} variant {item[:40]!r}")
        elif m:
            variants[m.group(1)] = m.group(3)
    impl = block(text, impl_at.start())
    label_at = re.search(r"fn label\s*\(", impl)
    if not label_at:
        problems.append(f"{file}: {enum}::label not found")
        continue
    body = " ".join(block(impl, label_at.start()).split())
    match_at = re.search(r"match self\s*\{", body)
    if not match_at:
        problems.append(f"{file}: {enum}::label has no `match self`")
        continue
    arms_text = block(body, match_at.start())
    # Each arm's COMPLETE expression runs to the next arm head (or the end of the match). Only
    # `"literal"`, `{ "literal" }` or, for a tuple variant, `binding.label()` is accepted; any
    # tail (`.split_at(5).1`, `concat!`, `+`, a method call) is unsupported and fails closed.
    heads = list(re.finditer(r"Self::([A-Z][A-Za-z0-9]*)\s*(?:\(\s*(\w+)\s*\))?\s*=>", arms_text))
    arms = {}
    for n, head in enumerate(heads):
        end = heads[n + 1].start() if n + 1 < len(heads) else len(arms_text)
        expr = arms_text[head.end():end].strip().rstrip(",").strip()
        arms[head.group(1)] = (head.group(2), expr)
    for variant, payload in variants.items():
        binding, expr = arms.get(variant, (None, None))
        if expr is None:
            problems.append(f"{file}: {enum}::{variant} has no label arm")
        elif payload:
            # Only a delegation to another checked enum's own labels is accepted.
            if not (payload in CHECKED.values() and binding and expr == f"{binding}.label()"):
                problems.append(f"{file}: {enum}::{variant}({payload}) has no checked label: {expr!r}")
        else:
            literal = re.fullmatch(r'"([a-z0-9_]+)"|\{\s*"([a-z0-9_]+)"\s*\}', expr)
            if literal:
                labels.add(literal.group(1) or literal.group(2))
            else:
                problems.append(f"{file}: {enum}::{variant} label is not a plain string literal: {expr!r}")
    for extra in sorted(set(arms) - set(variants)):
        problems.append(f"{file}: {enum}::label has an arm for unknown variant {extra}")
if problems:
    print(f"check_vocab: label extraction incomplete at {commit}:", file=sys.stderr)
    for p in problems:
        print(f"  {p}", file=sys.stderr)
    sys.exit(1)

vocab_text = open(vocab).read()
const = vocab_text[vocab_text.index("pub const KNOWN_SIGNALS"):]
ours = set(re.findall(r'"([a-z0-9_]+)"', const[: const.index("];")]))
if labels != ours:
    print(f"vocabulary drift against deep-diff-forge {commit}:", file=sys.stderr)
    for label in sorted(labels - ours):
        print(f"  < {label} (deep-diff-forge only)", file=sys.stderr)
    for label in sorted(ours - labels):
        print(f"  > {label} (forge-warden only)", file=sys.stderr)
    print("update KNOWN_SIGNALS, schemas/policy.v0.schema.json and CHANGELOG.md together", file=sys.stderr)
    sys.exit(1)
print(f"vocabulary matches deep-diff-forge {commit} ({len(ours)} signals; every variant of RiskSignal and EvidenceSignal labelled)")
PY
