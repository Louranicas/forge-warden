#!/usr/bin/env bash
# WP-RUNNER-UP: make a fresh runner-up KVM guest (stock Ubuntu 24.04 cloud image) ready for the
# jobs routed to it. Runs only when runner.environment == 'self-hosted'; hosted images already ship
# these tools. Idempotent. Pins live in scripts/tool-versions.toml (rustup, rustup_sha256).
# - a C toolchain (the linker cargo needs), from the image's own Ubuntu archive;
# - rustup-init at the pinned version, refused unless its sha256 matches the pin, installed with
#   no default toolchain (each job installs its own pinned toolchain next, as on hosted runners).
# Nothing is written into the checkout: the plants gate requires a clean committed subject.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
version=$(python3 -I "$root/scripts/tool_versions.py" rustup)
sha256=$(python3 -I "$root/scripts/tool_versions.py" rustup_sha256)
arch=$(uname -m)
if [ "$arch" != x86_64 ]; then
  echo "guest bootstrap: unsupported architecture $arch" >&2
  exit 1
fi
if ! command -v cc >/dev/null 2>&1; then
  sudo -n apt-get update -q
  sudo -n env DEBIAN_FRONTEND=noninteractive apt-get install -y -q --no-install-recommends build-essential
fi
if ! command -v rustup >/dev/null 2>&1; then
  work=$(mktemp -d)
  curl --proto '=https' --tlsv1.2 -fsS --retry 3 -o "$work/rustup-init" \
    "https://static.rust-lang.org/rustup/archive/$version/x86_64-unknown-linux-gnu/rustup-init"
  echo "$sha256  $work/rustup-init" | sha256sum --check --strict -
  chmod 0755 "$work/rustup-init"
  "$work/rustup-init" -y --no-modify-path --profile minimal --default-toolchain none
  rm -f "$work/rustup-init"
  rmdir "$work"
  echo "$HOME/.cargo/bin" >> "$GITHUB_PATH"
  export PATH="$HOME/.cargo/bin:$PATH"
fi
# Provenance for the run log (no secrets: versions and the image's release only).
grep -E '^(PRETTY_NAME|VERSION_ID)=' /etc/os-release
cc --version | head -n 1
rustup --version 2>/dev/null | head -n 1
python3 --version
git --version
echo "nproc=$(nproc) mem_kb=$(awk '/MemTotal/ {print $2}' /proc/meminfo)"
