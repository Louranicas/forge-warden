#!/usr/bin/env bash
# Build the Claude Project knowledge bundle (docs/claude-project/README.md) in target/claude-project/.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
out=target/claude-project
rm -rf "$out" && mkdir -p "$out"
rev="$(git rev-parse --short HEAD)"
files=(CLAUDE.md README.md CHANGELOG.md docs/POLICY_REFERENCE.md docs/adr/*.md schemas/*.json \
       warden.toml examples/starter.warden.toml crates/forge-warden-core/src/*.rs)
for f in "${files[@]}"; do
  name="$(echo "$f" | tr '/' '_')"
  { printf '<!-- %s @ %s -->\n' "$f" "$rev"; cat "$f"; } > "$out/$name"
done
echo "wrote ${#files[@]} files to $out (commit $rev)"
