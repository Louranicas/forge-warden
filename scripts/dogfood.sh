#!/usr/bin/env bash
# forge-warden trusted-dogfood-driver v2
# Run from a clean trusted base checkout. The candidate is Git data, never executable code.
# Usage: dogfood.sh [--hosted-advisory] [--ranker-archive <file>] <base-sha> <head-sha> <candidate-checkout> <new-evidence-directory> [ack.json ack.sig]
# --hosted-advisory, written by the caller before the commit ids, asks for advisory mode although
# the trusted policy pins a producer. It is for a hosted runner, which has no owner-enrolled Git
# allowlist and so cannot judge in authority mode; the result cannot PASS. Nothing in the
# environment selects it, and without it a pinned policy means authority mode: a missing allowlist
# is then exit 3.
# --ranker-archive names a local copy of the pinned deep-diff-forge release archive (this team keeps
# one under ~/.cache/fw-ranker/); it is copied into the evidence directory and that copy must have
# the pinned sha256. Only the hosted runner downloads the archive: a run without --ranker-archive
# must say --hosted-advisory, so a local run never reaches the network (ledger Q67).
set -euo pipefail
hosted_advisory=false
ranker_archive=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --hosted-advisory) hosted_advisory=true; shift ;;
    --ranker-archive)
      if [ "$#" -lt 2 ] || [ -z "$2" ]; then
        echo 'dogfood: --ranker-archive needs a file' >&2
        exit 2
      fi
      ranker_archive="$2"
      shift 2 ;;
    *) break ;;
  esac
done
if [ -z "$ranker_archive" ] && [ "$hosted_advisory" = false ]; then
  echo 'dogfood: a local run takes --ranker-archive <file>, the pinned release archive (kept under ~/.cache/fw-ranker/); only --hosted-advisory downloads it' >&2
  exit 2
fi
if [ -n "$ranker_archive" ] && [ ! -f "$ranker_archive" ]; then
  echo "dogfood: --ranker-archive $ranker_archive is not a file" >&2
  exit 2
fi
# Absolute now: the script changes directory before it copies the archive.
if [ -n "$ranker_archive" ]; then
  ranker_archive="$(cd "$(dirname -- "$ranker_archive")" && pwd -P)/$(basename -- "$ranker_archive")"
fi
if [ "$#" -ne 4 ] && [ "$#" -ne 6 ]; then
  echo "dogfood: expected [--hosted-advisory] [--ranker-archive <file>] and four arguments, optionally followed by signed acknowledgement and signature" >&2
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
while [[ "$candidate" != / && ! -e "$candidate/.git" && ! -L "$candidate/.git" ]]; do
  candidate="$(dirname -- "$candidate")"
done
# Nothing about Git comes from the caller's environment: GIT_CONFIG_COUNT/KEY_n/VALUE_n and
# GIT_CONFIG_PARAMETERS carry configuration (diff.noprefix alone makes the judge read
# `scripts/x` as `x`, so no path rule matches), GIT_DIFF_OPTS changes the hunks, GIT_DIR and
# its relatives move the repository. Unset every GIT_ variable, then set the fixed ones.
while IFS= read -r name; do unset "$name"; done < <(compgen -v GIT_)
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null GIT_ATTR_NOSYSTEM=1 GIT_NO_REPLACE_OBJECTS=1
if [[ -L "$trusted/scripts" || -L "$trusted/scripts/make_patch.sh" || ! -f "$trusted/scripts/make_patch.sh" ]]; then
  echo "dogfood: invalid trusted patch-maker" >&2
  exit 2
fi
for checkout in "$trusted" "$candidate"; do
  (cd -- "$checkout" && bash "$trusted/scripts/make_patch.sh" --check-worktree)
done
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
# The patch is made by the trusted checkout's patch-maker: the diff runs in a new repository that
# only borrows the candidate's objects, so nothing in the candidate's .git/config, in the
# caller's environment or in the directory named for the candidate can shape it, and attributes
# come from the empty tree (scripts/make_patch.sh says why for each).
bash "$trusted/scripts/make_patch.sh" "$base" "$head" "$candidate" > "$out/change.patch"
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
if [ -n "$ranker_archive" ]; then
  # The copy, not the named file, is checked and used: the bytes checked are the bytes run.
  cp -- "$ranker_archive" "$out/ranker/$ddf_name.tar.gz"
else
  curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' --tlsv1.2 --max-time 300 \
    --output "$out/ranker/$ddf_name.tar.gz" \
    "https://github.com/Louranicas/deep-diff-forge/releases/download/$ddf_release/$ddf_name.tar.gz"
fi
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
  # Bound mode (ADR 0012): the judge makes the diff itself from the candidate's objects and refuses a
  # patch that is not byte-equal to it, so the verdict's claims are verified, not asserted.
  candidate_git="$(git -C "$candidate" rev-parse --path-format=absolute --git-common-dir)"
  judge_mode=(--ranker "$out/ranker/bin/deep-diff-forge" --repo "$candidate_git")
else
  echo "dogfood: explicit advisory mode ($advisory_reason); this result cannot PASS" >&2
  "$out/ranker/bin/deep-diff-forge" --stdin-patch --rank --json < "$out/change.patch" > "$out/rank.json"
  judge_mode=(--advisory --rank "$out/rank.json")
fi
printf 'base=%s\nhead=%s\nddf_rev=%s\nddf_release=%s\nddf_archive_sha256=%s\n' \
  "$base" "$head" "$ddf_rev" "$ddf_release" "$ddf_archive_sha256" > "$out/provenance.txt"
if [ -n "$ranker_archive" ]; then printf 'ranker_archive=local\n' >> "$out/provenance.txt"; else printf 'ranker_archive=downloaded\n' >> "$out/provenance.txt"; fi
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
# A verdict judged in authority mode carries verified bound claims for this base and head, whatever its
# outcome: `verified` is JSON true (not a string, not false) and `base`/`head` are the ids passed. One that
# does not was not judged in bound mode on this change, and is refused here rather than trusted (exit 4).
if [ "$mode" = authority ] && { [ "$code" -eq 0 ] || [ "$code" -eq 7 ] || [ "$code" -eq 8 ]; }; then
  if ! python3 -I -c 'import json, sys
claims = json.load(open(sys.argv[1], encoding="utf-8")).get("seals", {}).get("claims", {})
bound = isinstance(claims, dict) and claims.get("verified") is True
sys.exit(0 if bound and claims.get("base") == sys.argv[2] and claims.get("head") == sys.argv[3] else 1)' \
      "$out/verdict.json" "$base" "$head"; then
    echo 'dogfood: the authority verdict has no verified bound claims for this base and head (seals.claims); not judged in bound mode' >&2
    exit 4
  fi
fi
if [ "$mode" = authority ] && [ "$code" -eq 8 ] && [ "$#" -eq 6 ]; then
  # Key lookup is inside this trusted checkout; receipt data never chooses a key path.
  if python3 -I "$trusted/scripts/hold_ack.py" verify --base "$base" --head "$head" \
      --verdict "$out/verdict.json" --ack "$5" --signature "$6" > "$out/acknowledgement.json"; then
    echo 'dogfood: signed HOLD acknowledgement accepted; raw verdict remains HOLD'
    exit 0
  fi
  rm -f -- "$out/acknowledgement.json"
  echo 'dogfood: HOLD acknowledgement refused' >&2
fi
exit "$code"
