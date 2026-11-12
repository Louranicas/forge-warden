#!/usr/bin/env bash
# forge-warden trusted-dogfood-driver v2
# Run from a clean trusted base checkout. The candidate is Git data, never executable code.
# Usage: dogfood.sh [--hosted-advisory] <base-sha> <head-sha> <candidate-checkout> <new-evidence-directory> [ack.json ack.sig]
# --hosted-advisory, first and written by the caller, asks for advisory mode although the trusted
# policy pins a producer. It is for a hosted runner, which has no owner-enrolled Git allowlist and
# so cannot judge in authority mode; the result cannot PASS. Nothing in the environment selects
# it, and without it a pinned policy means authority mode: a missing allowlist is then exit 3.
set -euo pipefail
hosted_advisory=false
if [ "${1:-}" = --hosted-advisory ]; then
  hosted_advisory=true
  shift
fi
if [ "$#" -ne 4 ] && [ "$#" -ne 6 ]; then
  echo "dogfood: expected [--hosted-advisory] and four arguments, optionally followed by signed acknowledgement and signature" >&2
  exit 2
fi
base="${1:?base sha}"
head="${2:?head sha}"
candidate="${3:?candidate checkout}"
out="${4:?new evidence directory}"
if [[ ! "$base" =~ ^[0-9a-f]{40}$ || ! "$head" =~ ^[0-9a-f]{40}$ ]]; then
  echo 'dogfood: base and head must be full commit IDs' >&2
  exit 2
fi
# The runner and this script must be trusted independently of the candidate workflow.
trusted="$(cd "$(dirname "$0")/.." && pwd -P)"
candidate="$(cd "$candidate" && pwd -P)"
# Nothing about Git comes from the caller's environment: GIT_CONFIG_COUNT/KEY_n/VALUE_n and
# GIT_CONFIG_PARAMETERS carry configuration (diff.noprefix alone makes the judge read
# `scripts/x` as `x`, so no path rule matches), GIT_DIFF_OPTS changes the hunks, GIT_DIR and
# its relatives move the repository. Unset every GIT_ variable, then set the fixed ones.
while IFS= read -r name; do unset "$name"; done < <(compgen -v GIT_)
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null GIT_ATTR_NOSYSTEM=1 GIT_NO_REPLACE_OBJECTS=1
if [ "$trusted" = "$candidate" ] || [ "$(git -C "$trusted" rev-parse HEAD)" != "$base" ]; then
  echo 'dogfood: judge checkout must be the distinct trusted base' >&2
  exit 2
fi
if [ -n "$(git -C "$trusted" status --porcelain --untracked-files=all --ignored)" ]; then
  echo 'dogfood: trusted checkout must be clean, including ignored files' >&2
  exit 2
fi
for commit in "$base" "$head"; do
  test "$(git -C "$candidate" rev-parse --verify "$commit^{commit}")" = "$commit"
done
if [[ "$out" != /* || -e "$out" || -L "$out" ]]; then
  echo 'dogfood: evidence directory must be a new absolute path' >&2
  exit 2
fi
umask 077
mkdir -- "$out"
out="$(cd "$out" && pwd -P)"
case "$out/" in
  "$trusted/"*|"$candidate/"*) echo 'dogfood: evidence must be outside both checkouts' >&2; exit 2 ;;
esac
# Diff endpoints are immutable commits; no external diff or textconv can execute candidate config.
# Attributes come from the empty tree, not the candidate's work tree: a candidate .gitattributes
# (at any depth) saying `-diff` or `binary` would otherwise turn added text into a binary patch the
# judge cannot read. --attr-source needs Git 2.40 or later; an older Git refuses the option.
empty_tree="$(git hash-object -t tree /dev/null)"
# The candidate repository's own configuration is not read at all. The diff runs in a new, empty
# repository that borrows the candidate's objects, so no setting in the candidate's .git/config
# (diff.noprefix, diff.relative, diff.context, color.ui, core.quotePath, ...) and no directory the
# caller happened to name can shape the patch. The settings that matter most are also fixed on the
# command line: the judge strips exactly one leading component from each name, reads no colour
# codes, and expects paths from the top of the repository.
git init --quiet --bare "$out/diff.git"
git -C "$candidate" rev-parse --path-format=absolute --git-path objects > "$out/diff.git/objects/info/alternates"
git --git-dir="$out/diff.git" --attr-source="$empty_tree" -c core.attributesFile=/dev/null \
  -c core.quotePath=true diff --no-ext-diff --no-textconv --no-color --no-relative --binary \
  --no-renames --src-prefix=a/ --dst-prefix=b/ "$base...$head" > "$out/change.patch"
# Source, policy, lockfile and build configuration all come from the trusted checkout.
cd "$trusted"
cp -- warden.toml "$out/policy.toml"
export CARGO_TARGET_DIR="$out/target"
cargo build --release --locked --bin forge-warden
# The producer pin is part of this trusted script, not a PR environment variable. The ranker is
# the released binary for the pinned tag, fetched as the release archive and refused unless its
# bytes are the pinned ones. It is not built here: a build's bytes depend on the build host's
# cargo home, linker and libc, so no digest of a build can be pinned for every runner.
ddf_rev="$(python3 -I "$trusted/scripts/tool_versions.py" ddf_rev)"
ddf_release="$(python3 -I "$trusted/scripts/tool_versions.py" ddf_release)"
ddf_archive_sha256="$(python3 -I "$trusted/scripts/tool_versions.py" ddf_archive_sha256)"
ddf_name="deep-diff-forge-$ddf_release-x86_64-unknown-linux-gnu"
mkdir -- "$out/ranker" "$out/ranker/bin"
curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 --max-time 300 \
  --output "$out/ranker/$ddf_name.tar.gz" \
  "https://github.com/Louranicas/deep-diff-forge/releases/download/$ddf_release/$ddf_name.tar.gz"
printf '%s  %s\n' "$ddf_archive_sha256" "$out/ranker/$ddf_name.tar.gz" | sha256sum --check --strict --status || {
  echo 'dogfood: the release archive is not the pinned one' >&2
  exit 3
}
tar --extract --gzip --no-same-owner --file "$out/ranker/$ddf_name.tar.gz" \
  --directory "$out/ranker" --strip-components=1 "$ddf_name/deep-diff-forge"
mv -- "$out/ranker/deep-diff-forge" "$out/ranker/bin/deep-diff-forge"
mode="$(python3 -I "$trusted/scripts/judge_mode.py" "$out/policy.toml")"
advisory_reason='trusted policy has no producer pin'
if [ "$hosted_advisory" = true ]; then
  mode=advisory
  advisory_reason='--hosted-advisory: a hosted runner has no owner-enrolled Git allowlist'
fi
if [ "$mode" = authority ]; then
  judge_mode=(--ranker "$out/ranker/bin/deep-diff-forge")
else
  echo "dogfood: explicit advisory mode ($advisory_reason); this result cannot PASS" >&2
  "$out/ranker/bin/deep-diff-forge" --stdin-patch --rank --json < "$out/change.patch" > "$out/rank.json"
  judge_mode=(--advisory --rank "$out/rank.json")
fi
printf 'base=%s\nhead=%s\nddf_rev=%s\nddf_release=%s\nddf_archive_sha256=%s\n' \
  "$base" "$head" "$ddf_rev" "$ddf_release" "$ddf_archive_sha256" > "$out/provenance.txt"
sha256sum "$out/target/release/forge-warden" "$out/ranker/bin/deep-diff-forge" \
  "$out/change.patch" "$out/policy.toml" >> "$out/provenance.txt"
printf 'mode=%s\n' "$mode" >> "$out/provenance.txt"
if [ "$mode" = advisory ]; then printf 'advisory_reason=%s\n' "$advisory_reason" >> "$out/provenance.txt"; fi
if [ "$mode" = advisory ]; then sha256sum "$out/rank.json" >> "$out/provenance.txt"; fi
# Preserve the raw verdict; a signed acknowledgement is a separate admission decision.
set +e
"$out/target/release/forge-warden" check --policy "$out/policy.toml" \
  "${judge_mode[@]}" --patch "$out/change.patch" --base "$base" --head "$head" --json > "$out/verdict.json"
code=$?
set -e
if [ "$mode" = authority ] && [ "$code" -eq 8 ] && [ "$#" -eq 6 ]; then
  # Key lookup is inside this trusted checkout; receipt data never chooses a key path.
  if python3 -I "$trusted/scripts/hold_ack.py" verify --base "$base" --head "$head" \
      --verdict "$out/verdict.json" --ack "$5" --signature "$6" > "$out/acknowledgement.json"; then
    echo 'dogfood: signed HOLD acknowledgement accepted; raw verdict remains HOLD'
    exit 0
  fi
  echo 'dogfood: HOLD acknowledgement refused' >&2
fi
exit "$code"
