#!/usr/bin/env bash
set -euo pipefail
unset "${!GIT_@}"
root="$(cd "$(dirname "$0")/../.." && pwd)"
scratch="$(mktemp -d)"
trap 'rm -rf -- "$scratch"' EXIT
export HOME="$scratch/home" XDG_CONFIG_HOME="$scratch/home/.config" GIT_CONFIG_NOSYSTEM=1
mkdir -p "$XDG_CONFIG_HOME" "$scratch/repo"
cd "$scratch/repo"
git init -q --template=
git config user.name Fixture
git config user.email fixture@example.invalid
mkdir -p scripts crates/x/src
cp "$root"/scripts/{stash_source.sh,restore_source.py} scripts/
src=crates/x/src
printf '*.swp\nignored.rs\n' > .gitignore
for file in lib.rs unrelated.rs removed.rs staged-only.rs; do
    echo base > "$src/$file"
done
mkdir "$src/tree"
echo leaf > "$src/tree/leaf"
ln -s lib.rs "$src/link"
git add .gitignore "$src"
git commit -qm base

refuse_save() {
    local index
    index="$(git write-tree)"
    if bash scripts/stash_source.sh > log 2>&1; then exit 1; fi
    grep -Fq 'Failing-first check unverified' log
    [[ $(git write-tree) == "$index" && -z $(git stash list) ]]
}
for kind in file symlink; do
    mv "$src/tree" "$src/moved"
    if [[ $kind == file ]]; then
        echo saved > "$src/tree"
    else
        ln -s moved "$src/tree"
    fi
    git add "$src"
    refuse_save
    [[ $(cat "$src/moved/leaf") == leaf ]]
    if [[ $kind == file ]]; then
        [[ $(cat "$src/tree") == saved ]]
    else
        [[ $(readlink "$src/tree") == moved ]]
    fi
    git restore --source=HEAD --staged --worktree -- "$src"

    leaf="$src/link"
    if [[ $kind == file ]]; then leaf="$src/lib.rs"; fi
    rm "$leaf"
    mkdir "$leaf"
    echo later > "$leaf/editor.swp"
    refuse_save
    [[ $(cat "$leaf/editor.swp") == later ]]
    mv "$leaf/editor.swp" "$leaf/new.rs"
    git add "$src"
    refuse_save
    [[ $(cat "$leaf/new.rs") == later ]]
    git restore --source=HEAD --staged --worktree -- "$src"
done

echo staged > "$src/staged-only.rs"
git add "$src/staged-only.rs"
git restore --source=HEAD --worktree -- "$src/staged-only.rs"
refuse_save
[[ $(cat "$src/staged-only.rs") == base && $(git show ":$src/staged-only.rs") == staged ]]
git restore --staged -- "$src/staged-only.rs"

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
    if bash scripts/stash_source.sh > log 2>&1; then exit 1; fi
    grep -Fq "$new" log
    rm "$src/$new"
done
stash="$(bash scripts/stash_source.sh | tail -n 1)"
[[ -f "$src/editor.swp" && $(cat "$src/lib.rs") == base ]]

echo later > "$src/staged-only.rs"
echo later > "$src/unrelated.rs"
git add "$src/unrelated.rs"
mkdir "$src/generated"
echo later > "$src/generated/result.rs"
refused() {
    if python3 -I scripts/restore_source.py "$stash"; then exit 1; fi
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
    python3 -I scripts/restore_source.py "$stash"
    [[ $(git show ":$src/lib.rs") == staged && $(cat "$src/lib.rs") == working ]]
    [[ $(git show ":$src/generated") == saved && $(cat "$src/generated") == saved ]]
    [[ $(git show ":$src/unrelated.rs") == later && $(cat "$src/unrelated.rs") == later ]]
    [[ $(git show ":$src/staged-only.rs") == staged && $(cat "$src/staged-only.rs") == later ]]
    [[ ! -e "$src/removed.rs" ]]
    if git ls-files --error-unmatch -- "$src/removed.rs" >/dev/null 2>&1; then exit 1; fi
done
