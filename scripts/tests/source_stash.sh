#!/usr/bin/env bash
set -euo pipefail
unset "${!GIT_@}"
root="$(cd "$(dirname "$0")/../.." && pwd)"
scratch="$(mktemp -d)"
trap 'rm -rf -- "$scratch"' EXIT
cd "$scratch"
git init -q --template=
git config user.name Fixture
git config user.email fixture@example.invalid
mkdir -p scripts crates/x/src
cp "$root"/{justfile,Cargo.toml,rust-toolchain.toml} .
cp "$root"/scripts/{restore_source.py,tool_versions.py,tool-versions.toml} scripts/
src=crates/x/src
printf '*.swp\nignored.rs\n' > .gitignore
for file in lib.rs unrelated.rs removed.rs staged-only.rs; do
    echo base > "$src/$file"
done
git add .gitignore "$src"
git commit -qm base

echo staged > "$src/lib.rs"
echo saved > "$src/generated"
mkdir "$src/nested"
echo saved > "$src/nested/file"
echo staged > "$src/staged-only.rs"
rm "$src/removed.rs"
git add "$src"
echo base > "$src/staged-only.rs"
echo working > "$src/lib.rs"
touch "$src/editor.swp"
for new in new.rs ignored.rs; do
    touch "$src/$new"
    if just stash-source > log 2>&1; then exit 1; fi
    grep -Fq "$new" log
    rm "$src/$new"
done
stash="$(just stash-source | tail -n 1)"
[[ -f "$src/editor.swp" && $(cat "$src/lib.rs") == base ]]

echo later > "$src/staged-only.rs"
echo later > "$src/unrelated.rs"
git add "$src/unrelated.rs"
mkdir "$src/generated"
echo later > "$src/generated/result.rs"
refused() {
    if just restore-source "$stash"; then exit 1; fi
    [[ $(cat "$src/lib.rs") == base && $(git show ":$src/lib.rs") == base ]]
}
refused
[[ $(cat "$src/generated/result.rs") == later ]]
git add "$src/generated/result.rs"
rm "$src/generated/result.rs"
rmdir "$src/generated"
refused
[[ $(git show ":$src/generated/result.rs") == later ]]
git restore --staged -- "$src/generated"

echo later > "$src/nested"
refused
[[ $(cat "$src/nested") == later ]]
rm "$src/nested"
git restore --source="$stash" --worktree -- "$src/lib.rs"
git restore --source="$stash^2" --staged -- "$src/removed.rs"
for attempt in 1 2; do
    just restore-source "$stash"
    [[ $(git show ":$src/lib.rs") == staged && $(cat "$src/lib.rs") == working ]]
    [[ $(git show ":$src/generated") == saved && $(cat "$src/generated") == saved ]]
    [[ $(git show ":$src/unrelated.rs") == later && $(cat "$src/unrelated.rs") == later ]]
    [[ $(git show ":$src/staged-only.rs") == staged && $(cat "$src/staged-only.rs") == later ]]
    [[ ! -e "$src/removed.rs" ]]
    if git ls-files --error-unmatch -- "$src/removed.rs" >/dev/null 2>&1; then exit 1; fi
done
