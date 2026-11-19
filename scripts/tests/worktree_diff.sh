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
python3 -I "$root/scripts/worktree_diff.py" "$base" > diff.out
grep -Fxq '+edit' diff.out
git config --unset core.checkStat
git config --unset core.trustctime

echo hidden > "$src/lib.rs"
echo visible > "$src/unrelated.rs"
for flag in assume-unchanged skip-worktree; do
    git update-index "--$flag" -- "$src/lib.rs"
    if python3 -I "$root/scripts/worktree_diff.py" "$base" > diff.out 2> log; then exit 1; fi
    grep -Fq "$flag" log
    [[ ! -s diff.out && $(cat "$src/lib.rs") == hidden && $(cat "$src/unrelated.rs") == visible ]]
    git update-index "--no-$flag" -- "$src/lib.rs"
done
git update-index --assume-unchanged -- notes
python3 -I "$root/scripts/worktree_diff.py" "$base" > diff.out
grep -Fxq '+hidden' diff.out
grep -Fxq '+visible' diff.out
