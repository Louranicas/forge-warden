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
mkdir -p .git/info
git config filter.local.required true
git config filter.local.clean false
for attr in 'text eol=lf' ident working-tree-encoding=ISO-8859-1 filter=local; do
    printf '*.rs %s\n' "$attr" > .git/info/attributes
    python3 -I "$root/scripts/worktree_diff.py" "$base" > "$scratch/diff"
    [[ ! -s "$scratch/diff" ]]
done
printf '*.rs text eol=lf\n' > .git/info/attributes
echo visible > "$src/lib.rs"
python3 -I "$root/scripts/worktree_diff.py" "$base" > "$scratch/diff"
grep -Fxq '+visible' "$scratch/diff"
echo base > "$src/lib.rs"
rm .git/info/attributes
git config --unset filter.local.clean
git config --unset filter.local.required
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

printf 'id.rs ident\n' > .git/info/attributes
echo 'pub fn id() -> String { "$Id$".to_owned() }' > "$src/id.rs"
git add -- "$src/id.rs"
git commit -qm 'add identifier'
base="$(git rev-parse HEAD)"
printf 'notes ident\nid.rs filter=local\n' > .git/info/attributes
git config filter.local.required false
git config filter.local.clean ''
python3 -I "$root/scripts/worktree_diff.py" "$base" > "$scratch/diff"
grep -Fxq '+hidden' "$scratch/diff"
printf 'id.rs ident\n' > .git/info/attributes
echo 'pub fn id() -> String { "$Id: edited $".to_owned() }' > "$src/id.rs"
git add -- "$src/id.rs"
git diff --quiet "$base" -- "$src/id.rs"
index_before="$(git hash-object --no-filters .git/index)"
bytes_before="$(git hash-object --no-filters "$src/id.rs")"
if python3 -I "$root/scripts/worktree_diff.py" "$base" > "$scratch/diff" 2> "$scratch/log"; then exit 1; fi
grep -Fq "$src/id.rs" "$scratch/log"
[[ ! -s "$scratch/diff" && $(git hash-object --no-filters .git/index) == "$index_before" ]]
[[ $(git hash-object --no-filters "$src/id.rs") == "$bytes_before" ]]
