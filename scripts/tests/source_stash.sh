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
cp "$root"/scripts/{source_stash.py,restore_source.py,worktree_diff.py} scripts/
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
    if python3 -I scripts/source_stash.py > log 2>&1; then exit 1; fi
    grep -Fq "${1:-Failing-first check unverified}" log
    [[ $(git write-tree) == "$index" && -z $(git stash list) ]]
}
same_bytes() {
    python3 -I -c 'import filecmp, sys; sys.exit(not filecmp.cmp(sys.argv[1], sys.argv[2], shallow=False))' "$1" "$2"
}
base="$(git rev-parse HEAD)"
git config core.trustctime false
git config core.checkStat minimal
touch -t 202001010000 "$src/lib.rs"
git add "$src/lib.rs"
touch -r "$src/lib.rs" .git/index
echo edit > "$src/lib.rs"
touch -r .git/index "$src/lib.rs"
python3 -I scripts/worktree_diff.py "$base" > diff.out
grep -Fxq '+edit' diff.out
stash="$(python3 -I scripts/source_stash.py | tail -n 1)"
[[ $(cat "$src/lib.rs") == base ]]
python3 -I scripts/restore_source.py "$stash"
[[ $(cat "$src/lib.rs") == edit && $(git show ":$src/lib.rs") == base ]]
git restore -- "$src/lib.rs"
git stash drop -q
git config --unset core.checkStat
git config --unset core.trustctime

refuse_save
echo draft > notes
refuse_save
git add notes
refuse_save
[[ $(git show :notes) == draft && $(cat notes) == draft ]]
git restore --source=HEAD --staged --worktree -- notes

mkdir "$src/restricted"
echo new > "$src/restricted/new.rs"
chmod 000 "$src/restricted"
if [[ ! -r "$src/restricted" ]]; then
    echo visible > "$src/lib.rs"
    refuse_save 'Working-tree scan unverified'
    [[ $(cat "$src/lib.rs") == visible ]]
    git restore -- "$src/lib.rs"
fi
chmod 700 "$src/restricted"
rm -r "$src/restricted"

mkdir -p .git/info
printf '*.rs text eol=lf\n' > .git/info/attributes
for layer in working staged mixed; do
    printf 'edited\r\n\r\n' > "$src/lib.rs"
    if [[ $layer != working ]]; then git add "$src/lib.rs"; fi
    if [[ $layer == mixed ]]; then printf 'different\r\n' > "$src/lib.rs"; fi
    cp "$src/lib.rs" "$scratch/expected"
    refuse_save 'Git would change the working bytes'
    same_bytes "$src/lib.rs" "$scratch/expected"
    git restore --source=HEAD --staged --worktree -- "$src/lib.rs"
done
printf '*.rs text eol=crlf\n' > .git/info/attributes
printf 'reversible\r\n\r\n' > "$src/lib.rs"
cp "$src/lib.rs" "$scratch/expected"
stash="$(python3 -I scripts/source_stash.py | tail -n 1)"
echo base > "$src/lib.rs"
if python3 -I scripts/restore_source.py "$stash" > log 2>&1; then exit 1; fi
grep -Fq 'Resolve the overlap' log
printf 'base\r\n' > "$src/lib.rs"
python3 -I scripts/restore_source.py "$stash"
same_bytes "$src/lib.rs" "$scratch/expected"
rm .git/info/attributes
git restore --source=HEAD --staged --worktree -- "$src/lib.rs"
git stash drop -q

echo hidden > "$src/lib.rs"
echo visible > "$src/unrelated.rs"
for flag in assume-unchanged skip-worktree; do
    git update-index "--$flag" -- "$src/lib.rs"
    refuse_save "$flag"
    if python3 -I scripts/worktree_diff.py "$base" > diff.out 2> log; then exit 1; fi
    grep -Fq "$flag" log
    [[ ! -s diff.out && $(cat "$src/lib.rs") == hidden && $(cat "$src/unrelated.rs") == visible ]]
    git update-index "--no-$flag" -- "$src/lib.rs"
done
git update-index --assume-unchanged -- notes
python3 -I scripts/worktree_diff.py "$base" > diff.out
grep -Fxq '+hidden' diff.out
grep -Fxq '+visible' diff.out
git update-index --no-assume-unchanged -- notes
git restore -- "$src"

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
echo working-only > "$src/indented.rs"
touch "$src/editor.swp"
for new in new.rs ignored.rs; do
    touch "$src/$new"
    if python3 -I scripts/source_stash.py > log 2>&1; then exit 1; fi
    grep -Fq "$new" log
    rm "$src/$new"
done
echo draft > advisory
echo staged > notes
git rm -q obsolete
git add advisory notes
echo working > notes
stash="$(python3 -I scripts/source_stash.py | tail -n 1)"
for snapshot in "$stash" "$stash^2"; do
    git diff --exit-code "$stash^1" "$snapshot" -- ':(exclude,glob)crates/*/src/**'
done
[[ $(git show :advisory) == draft && $(git show :notes) == staged && $(cat notes) == working ]]
if git ls-files --error-unmatch obsolete >/dev/null 2>&1; then exit 1; fi
git diff --cached --exit-code HEAD -- "$src"
[[ -f "$src/editor.swp" && $(cat "$src/lib.rs") == base ]]

for flag in assume-unchanged skip-worktree; do
    for file in lib.rs staged-only.rs indented.rs; do
        git update-index "--$flag" -- "$src/$file"
        if python3 -I scripts/restore_source.py "$stash" > log 2>&1; then exit 1; fi
        grep -Fq "$flag" log
        [[ $(cat "$src/lib.rs") == base && $(git show ":$src/lib.rs") == base ]]
        git update-index "--no-$flag" -- "$src/$file"
    done
done

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
for flag in assume-unchanged skip-worktree; do
    git update-index "--$flag" -- "$src/unrelated.rs"
    python3 -I scripts/restore_source.py "$stash"
    [[ $(git show ":$src/lib.rs") == staged && $(cat "$src/lib.rs") == working ]]
    [[ $(git show ":$src/generated") == saved && $(cat "$src/generated") == saved ]]
    [[ $(git show ":$src/unrelated.rs") == later && $(cat "$src/unrelated.rs") == later ]]
    tag="$(git ls-files -v -- "$src/unrelated.rs")"
    if [[ $flag == assume-unchanged ]]; then [[ $tag == "h "* ]]; else [[ $tag == "S "* ]]; fi
    git update-index "--no-$flag" -- "$src/unrelated.rs"
    [[ $(git show ":$src/staged-only.rs") == staged && $(cat "$src/staged-only.rs") == later ]]
    [[ ! -e "$src/removed.rs" ]]
    if git ls-files --error-unmatch -- "$src/removed.rs" >/dev/null 2>&1; then exit 1; fi
done

git restore --source=HEAD --staged --worktree -- "$src"
git config core.whitespace indent-with-non-tab
for whitespace in fix error; do
    git config apply.whitespace "$whitespace"
    printf '        fn staged() {}\n' > "$src/indented.rs"
    cp "$src/indented.rs" "$scratch/staged"
    git add "$src/indented.rs"
    printf '        fn working() {}\n' > "$src/indented.rs"
    cp "$src/indented.rs" "$scratch/expected"
    stash="$(python3 -I scripts/source_stash.py | tail -n 1)"
    git diff --exit-code HEAD -- "$src/indented.rs"
    git diff --cached --exit-code HEAD -- "$src/indented.rs"
    python3 -I scripts/restore_source.py "$stash"
    git show ":$src/indented.rs" > "$scratch/restored-index"
    same_bytes "$scratch/restored-index" "$scratch/staged"
    same_bytes "$src/indented.rs" "$scratch/expected"
done

mkdir "$scratch/bin"
export FIXTURE_PATH="$PATH" RACE_LOG="$scratch/race" RACE_PATH="$src/indented.rs"
cat > "$scratch/bin/git" <<'SH'
#!/usr/bin/env bash
export PATH="$FIXTURE_PATH"
if [[ -n ${CANCEL_WITH:-} && -f .git/index.lock ]]; then
    kill -s "$CANCEL_WITH" "$PPID"
    exit 0
fi
case " $* " in
    *' stash push '*|*' restore '*' --staged '*)
        if [[ -n ${CANCEL_AFTER_STASH:-} ]]; then
            git "$@" || exit $?
            kill -s "$CANCEL_AFTER_STASH" "$PPID"
            exit 0
        elif [[ -n ${FAIL_AFTER_STASH:-} ]]; then
            git "$@" || exit $?
            exit 17
        fi
        if env -u GIT_INDEX_FILE git update-index --cacheinfo "100644,$RACE_BLOB,$RACE_PATH"; then
            exit 99
        fi
        echo blocked >> "$RACE_LOG"
        ;;
esac
exec git "$@"
SH
chmod +x "$scratch/bin/git"
result=0
FAIL_AFTER_STASH=1 PATH="$scratch/bin:$PATH" python3 -I scripts/source_stash.py > log || result=$?
[[ $result == 17 ]]
[[ $(tail -n 1 log) == "$(git rev-parse refs/stash)" ]]
python3 -I scripts/restore_source.py "$(tail -n 1 log)"
[[ $(git show ":$src/indented.rs") == '        fn staged() {}' ]]
[[ $(cat "$src/indented.rs") == '        fn working() {}' ]]

lock="$(git rev-parse --git-path index).lock"
printf 'busy\n' > "$lock"
if python3 -I scripts/source_stash.py > log 2>&1; then exit 1; fi
if python3 -I scripts/restore_source.py "$stash" > log 2>&1; then exit 1; fi
[[ $(cat "$lock") == busy && $(git show ":$src/indented.rs") == '        fn staged() {}' ]]
rm "$lock"

before="$(git write-tree)"
for signal in HUP INT TERM; do
    for action in save restore; do
        result=0
        if [[ $action == save ]]; then
            CANCEL_WITH="$signal" PATH="$scratch/bin:$PATH" python3 -I scripts/source_stash.py || result=$?
        else
            CANCEL_WITH="$signal" PATH="$scratch/bin:$PATH" python3 -I scripts/restore_source.py "$stash" || result=$?
        fi
        [[ $result == $((128 + $(kill -l "$signal"))) && ! -e $lock ]]
        [[ $(git write-tree) == "$before" && $(cat "$RACE_PATH") == '        fn working() {}' ]]
    done
    previous="$(git rev-parse refs/stash)"
    result=0
    CANCEL_AFTER_STASH="$signal" PATH="$scratch/bin:$PATH" python3 -I scripts/source_stash.py > log || result=$?
    stored="$(tail -n 1 log)"
    [[ $result == $((128 + $(kill -l "$signal"))) && ! -e $lock ]]
    [[ $stored != "$previous" && $stored == "$(git rev-parse refs/stash)" ]]
    [[ $(cat "$RACE_PATH") == '        fn baseline() {}' ]]
    python3 -I scripts/restore_source.py "$stored"
    [[ $(git write-tree) == "$before" && $(cat "$RACE_PATH") == '        fn working() {}' ]]
done
export RACE_BLOB
RACE_BLOB="$(printf 'concurrent selection\n' | git hash-object -w --stdin)"
stash="$(PATH="$scratch/bin:$PATH" python3 -I scripts/source_stash.py | tail -n 1)"
[[ $(cat "$RACE_LOG") == blocked ]]
rm "$RACE_LOG"
PATH="$scratch/bin:$PATH" python3 -I scripts/restore_source.py "$stash"
[[ $(cat "$RACE_LOG") == blocked ]]
[[ $(git show ":$RACE_PATH") == '        fn staged() {}' ]]
git update-index --cacheinfo "100644,$RACE_BLOB,$RACE_PATH"
[[ $(git show ":$RACE_PATH") == 'concurrent selection' ]]
