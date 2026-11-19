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
src=crates/x/src
mkdir -p "$src"
echo base > "$src/lib.rs"
echo base > "$src/unrelated.rs"
echo original > notes
git add notes "$src"
git commit -qm base
base="$(git rev-parse HEAD)"
git config core.trustctime false
git config core.checkStat minimal
touch -t 202001010000 "$src/lib.rs"
git add "$src/lib.rs"
touch -r "$src/lib.rs" .git/index
echo edit > "$src/lib.rs"
touch -r .git/index "$src/lib.rs"
python3 -I "$root/scripts/worktree_diff.py" "$base" > "$scratch/diff"
grep -Fxq '+edit' "$scratch/diff"
touch -t 202001020000 .git/index
printf 'edit\n' > "$scratch/replacement"
touch -r "$src/lib.rs" "$scratch/replacement"
mv "$scratch/replacement" "$src/lib.rs"
python3 -I "$root/scripts/worktree_diff.py" "$base" > "$scratch/diff"
grep -Fxq '+edit' "$scratch/diff"
if python3 -I "$root/scripts/worktree_diff.py" --clean; then exit 1; fi
git config --unset core.checkStat
git config --unset core.trustctime

echo hidden > "$src/lib.rs"
echo visible > "$src/unrelated.rs"
for flag in assume-unchanged skip-worktree; do
    git update-index "--$flag" -- "$src/lib.rs"
    if python3 -I "$root/scripts/worktree_diff.py" "$base" > "$scratch/diff" 2> "$scratch/log"; then exit 1; fi
    grep -Fq "$flag" "$scratch/log"
    [[ ! -s "$scratch/diff" && $(cat "$src/lib.rs") == hidden && $(cat "$src/unrelated.rs") == visible ]]
    git update-index "--no-$flag" -- "$src/lib.rs"
done
git update-index --assume-unchanged -- notes
python3 -I "$root/scripts/worktree_diff.py" "$base" > "$scratch/diff"
grep -Fxq '+hidden' "$scratch/diff"
grep -Fxq '+visible' "$scratch/diff"

mkdir -p "$src/parser" crates/x/tests .git/info
echo 'fn helper() {}' > crates/x/tests/helper.rs
printf 'ignored.rs\n' > .git/info/exclude
for path in "$src/parser/new.rs" "$src/parser/ignored.rs"; do
    echo 'pub fn parse() {}' > "$path"
    if python3 -I "$root/scripts/worktree_diff.py" "$base" > "$scratch/diff" 2> "$scratch/log"; then exit 1; fi
    grep -Fq "$path" "$scratch/log"
    [[ ! -s "$scratch/diff" ]]
    git add -f -- "$path"
    python3 -I "$root/scripts/worktree_diff.py" "$base" > "$scratch/diff"
    grep -Fxq "+++ b/$path" "$scratch/diff"
done
