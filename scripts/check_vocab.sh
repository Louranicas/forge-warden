#!/usr/bin/env bash
# Fail if forge-warden's signal vocabulary (crates/forge-warden-core/src/vocab.rs) differs from the
# labels deep-diff-forge defines at commit $1 (risk.rs and evidence.rs). Usage:
#   scripts/check_vocab.sh <deep-diff-forge commit>  [path to an existing deep-diff-forge checkout]
set -euo pipefail
rev="${1:?usage: check_vocab.sh <deep-diff-forge commit> [checkout]}"
root="$(git rev-parse --show-toplevel)"
if [ -n "${2:-}" ]; then
  ddf="$2"
else
  ddf="$(mktemp -d)"
  trap 'rm -rf "$ddf"' EXIT
  git -C "$ddf" init -q
  git -C "$ddf" fetch -q --depth 1 https://github.com/Louranicas/deep-diff-forge "$rev"
  git -C "$ddf" checkout -q FETCH_HEAD
fi
graph="$ddf/crates/deep-diff-forge-graph/src"
# Labels are the string literals in `fn label` match arms: `Self::Variant => "label",`.
upstream="$(grep -hoE 'Self::[A-Za-z]+ => "[a-z_]+"' "$graph/risk.rs" "$graph/evidence.rs" \
  | sed -E 's/.*"([a-z_]+)"/\1/' | grep -vE '^(new|old)$' | sort -u)"
ours="$(sed -n '/pub const KNOWN_SIGNALS/,/^];/p' "$root/crates/forge-warden-core/src/vocab.rs" \
  | grep -oE '"[a-z_]+"' | tr -d '"' | sort -u)"
if [ "$upstream" != "$ours" ]; then
  echo "vocabulary drift against deep-diff-forge $rev:" >&2
  diff <(echo "$upstream") <(echo "$ours") >&2 || true
  echo "update KNOWN_SIGNALS, schemas/policy.v0.schema.json and CHANGELOG.md together" >&2
  exit 1
fi
echo "vocabulary matches deep-diff-forge $rev ($(echo "$ours" | wc -l) signals)"
