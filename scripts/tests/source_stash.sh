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
printf '        fn baseline() {}\n' > "$src/indented.rs"
echo original > notes
echo original > obsolete
git add .gitignore notes obsolete "$src"
git commit -qm base

refuse_save() {
    local index
    index="$(git write-tree)"
    if bash scripts/stash_source.sh > log 2>&1; then exit 1; fi
    grep -Fq 'Failing-first check unverified' log
    [[ $(git write-tree) == "$index" && -z $(git stash list) ]]
}
refuse_save
echo draft > notes
refuse_save
git add notes
refuse_save
[[ $(git show :notes) == draft && $(cat notes) == draft ]]
git restore --source=HEAD --staged --worktree -- notes

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
echo draft > advisory
echo staged > notes
git rm -q obsolete
git add advisory notes
echo working > notes
stash="$(bash scripts/stash_source.sh | tail -n 1)"
for snapshot in "$stash" "$stash^2"; do
    git diff --exit-code "$stash^1" "$snapshot" -- ':(exclude,glob)crates/*/src/**'
done
[[ $(git show :advisory) == draft && $(git show :notes) == staged && $(cat notes) == working ]]
if git ls-files --error-unmatch obsolete >/dev/null 2>&1; then exit 1; fi
git diff --cached --exit-code HEAD -- "$src"
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

git restore --source=HEAD --staged --worktree -- "$src"
git config core.whitespace indent-with-non-tab
for whitespace in fix error; do
    git config apply.whitespace "$whitespace"
    printf '        fn staged() {}\n' > "$src/indented.rs"
    git add "$src/indented.rs"
    printf '        fn working() {}\n' > "$src/indented.rs"
    stash="$(bash scripts/stash_source.sh | tail -n 1)"
    git diff --exit-code HEAD -- "$src/indented.rs"
    git diff --cached --exit-code HEAD -- "$src/indented.rs"
    python3 -I scripts/restore_source.py "$stash"
    [[ $(git show ":$src/indented.rs") == '        fn staged() {}' ]]
    [[ $(cat "$src/indented.rs") == '        fn working() {}' ]]
done

(
    git() {
        command git "$@" || return $?
        if [[ " $* " == *' stash push '* ]]; then return 17; fi
    }
    export -f git
    result=0
    bash scripts/stash_source.sh > log || result=$?
    [[ $result == 17 ]]
)
[[ $(tail -n 1 log) == "$(git rev-parse refs/stash)" ]]
python3 -I scripts/restore_source.py "$(tail -n 1 log)"
[[ $(git show ":$src/indented.rs") == '        fn staged() {}' ]]
[[ $(cat "$src/indented.rs") == '        fn working() {}' ]]

lock="$(git rev-parse --git-path index).lock"
printf 'busy\n' > "$lock"
if bash scripts/stash_source.sh > log 2>&1; then exit 1; fi
if python3 -I scripts/restore_source.py "$stash" > log 2>&1; then exit 1; fi
[[ $(cat "$lock") == busy && $(git show ":$src/indented.rs") == '        fn staged() {}' ]]
rm "$lock"

mkdir "$scratch/bin"
export FIXTURE_PATH="$PATH" RACE_LOG="$scratch/race" RACE_PATH="$src/indented.rs"
export RACE_BLOB
RACE_BLOB="$(printf 'concurrent selection\n' | git hash-object -w --stdin)"
cat > "$scratch/bin/git" <<'SH'
#!/usr/bin/env bash
export PATH="$FIXTURE_PATH"
case " $* " in
    *' stash push '*|*' restore '*' --staged '*)
        if env -u GIT_INDEX_FILE git update-index --cacheinfo "100644,$RACE_BLOB,$RACE_PATH"; then
            exit 99
        fi
        echo blocked >> "$RACE_LOG"
        ;;
esac
exec git "$@"
SH
chmod +x "$scratch/bin/git"
stash="$(PATH="$scratch/bin:$PATH" bash scripts/stash_source.sh | tail -n 1)"
[[ $(cat "$RACE_LOG") == blocked ]]
rm "$RACE_LOG"
PATH="$scratch/bin:$PATH" python3 -I scripts/restore_source.py "$stash"
[[ $(cat "$RACE_LOG") == blocked ]]
[[ $(git show ":$RACE_PATH") == '        fn staged() {}' ]]
git update-index --cacheinfo "100644,$RACE_BLOB,$RACE_PATH"
[[ $(git show ":$RACE_PATH") == 'concurrent selection' ]]
