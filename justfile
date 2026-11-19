# `just` lists the recipes. Every recipe is a thin wrapper over a script CI also runs.
default:
    @just --list

# One-time setup: use the versioned git hooks.
install-hooks:
    #!/usr/bin/env bash
    set -euo pipefail
    unset "${!GIT_@}"
    [[ ! -L .git && ( -d .git || -f .git ) && -d .githooks && ! -L .githooks ]]
    for hook in pre-commit commit-msg pre-push; do
        [[ -f ".githooks/$hook" && ! -L ".githooks/$hook" && -x ".githooks/$hook" ]]
    done
    repo=(env GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null git --git-dir="$PWD/.git")
    config=("${repo[@]}" config)
    local_config="$("${repo[@]}" rev-parse --path-format=absolute --git-path config)"
    [[ -f "$local_config" && ! -L "$local_config" ]]
    scratch="$(mktemp -d "${local_config%/*}/hooks-install.XXXXXXXX")"
    trap 'rm -rf -- "$scratch"' EXIT
    read_setting() {
        "${config[@]}" --show-origin --null --get core.hooksPath > "$scratch/setting" || return $?
        { IFS= read -r -d '' origin && IFS= read -r -d '' value; } < "$scratch/setting"
    }
    refuse() {
        if [[ -f "$scratch/prior" ]]; then mv -- "$scratch/prior" "$local_config"; fi
        echo "Hook installation refused: effective core.hooksPath must be .githooks in this worktree" >&2
        exit 1
    }
    (set -C; : > "$local_config.lock")
    trap 'rm -f -- "$local_config.lock"; rm -rf -- "$scratch"' EXIT
    if [[ "$(git rev-parse --show-toplevel && printf .)" != "$(pwd -P && printf .)" ]]; then
        echo "Hook installation refused: Git worktree differs from the selected checkout" >&2
        exit 1
    fi
    value=""
    if read_setting; then
        if [[ "$value" != .githooks && ( "$origin" != file:* || ! "${origin#file:}" -ef "$local_config" ) ]]; then
            refuse
        fi
    elif [[ $? != 1 ]]; then
        refuse
    fi
    if [[ "$value" != .githooks ]]; then
        cp -p -- "$local_config" "$scratch/prior"
        cp -p -- "$local_config" "$scratch/new"
        "${config[@]}" --file "$scratch/new" core.hooksPath .githooks
        mv -- "$scratch/new" "$local_config"
        if ! read_setting || [[ "$value" != .githooks || "$origin" != file:* || ! "${origin#file:}" -ef "$local_config" ]]; then
            refuse
        fi
    fi
    [[ .githooks -ef "$(git rev-parse --path-format=absolute --git-path hooks)" ]] || refuse
    echo "hooks installed: pre-commit (fast gate), commit-msg, pre-push (full gate)"

# fmt + clippy + tests: seconds. The pre-commit hook and Claude Code's Stop hook run this.
fast: (_check-script "ci_fast.sh")
    scripts/ci_fast.sh

# Every CI gate that can run locally.
ci: (_check-script "ci_local.sh")
    scripts/ci_local.sh

# Regenerate golden verdicts after an intended change; review the diff before committing.
bless: (_check-script "bless.sh")
    scripts/bless.sh

# Mutation-test the lines changed since main. A surviving mutant is a missing test.
# --cap-lints: with `warnings = "deny"`, a mutant that leaves an unused binding fails to compile
# and is counted "unviable", hiding survivors (main c44c00e: 3 missed, 59 more viable when capped).
# CARGO_TARGET_DIR unset: each worker builds in its own copy; a shared target dir mixes binaries.
mutants: (_check-script "make_patch.sh") _check-inputs
    #!/usr/bin/env bash
    set -euo pipefail
    while IFS= read -r name; do unset "$name"; done < <(compgen -v GIT_)
    mutation_diff="$(mktemp)"
    trap 'rm -f -- "$mutation_diff"' EXIT
    comparison="$(bash scripts/make_patch.sh --resolve-comparison refs/remotes/origin/main HEAD)"
    read -r _ mutation_base <<< "$comparison"
    python3 -I scripts/worktree_diff.py "$mutation_base" > "$mutation_diff"
    env -u CARGO_TARGET_DIR cargo mutants --no-config --cap-lints true --in-diff "$mutation_diff" --workspace

# Check the signal vocabulary against a deep-diff-forge checkout or commit.
vocab $rev="": (_check-script "check_vocab.sh")
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "$rev" ]]; then
        if [[ -L scripts/tool_versions.py || ! -f scripts/tool_versions.py ]]; then
            echo "Gate input refused: scripts/tool_versions.py" >&2
            exit 2
        fi
        rev="$(python3 -I scripts/tool_versions.py ddf_rev)"
    fi
    scripts/check_vocab.sh "$rev"

_check-script $script:
    @test ! -L scripts && test ! -L "scripts/$script" && test -f "scripts/$script" || { echo "Gate input refused: scripts/$script" >&2; exit 2; }

_check-inputs: (_check-script "worktree_diff.py")
    @python3 -I scripts/worktree_diff.py --check-inputs

# Judge the current branch with this repository's own policy (needs deep-diff-forge on PATH).
# Advisory mode: the ranking is supplied, so the verdict is at best hold. Committed changes only:
# the patch-maker diffs commits, not the work tree.
judge: (_check-script "make_patch.sh") _check-inputs
    #!/usr/bin/env bash
    set -euo pipefail
    patch="$(mktemp)"
    trap 'rm -f -- "$patch"' EXIT
    scripts/make_patch.sh refs/remotes/origin/main HEAD > "$patch"
    deep-diff-forge --stdin-patch --rank --json < "$patch" | cargo run --quiet --bin forge-warden -- check --policy warden.toml --patch "$patch" --advisory --rank -

judge-bound: (_check-script "make_patch.sh") _check-inputs
    #!/usr/bin/env bash
    set -euo pipefail
    unset "${!GIT_@}"
    base="$(git rev-parse --verify 'refs/remotes/origin/main^{commit}')"
    head="$(git rev-parse --verify 'HEAD^{commit}')"
    scratch="$(mktemp -d)"
    trap 'rm -rf -- "$scratch"' EXIT
    scripts/make_patch.sh "$base" "$head" > "$scratch/change.patch"
    ranker="$(type -P deep-diff-forge)"
    repo="$(git rev-parse --path-format=absolute --git-common-dir)"
    cargo run --quiet --bin forge-warden -- check --policy warden.toml --patch "$scratch/change.patch" \
        --ranker "$ranker" --repo "$repo" --base "$base" --head "$head"

review: (_check-script "make_patch.sh")
    #!/usr/bin/env bash
    set -euo pipefail
    umask 077
    unset "${!GIT_@}"
    comparison="$(bash scripts/make_patch.sh --review-comparison)"
    read -r base _ head <<< "$comparison"
    scratch="$(mktemp -d .review-XXXXXXXX)"
    printf 'Review directory: %s\n' "$scratch"
    publish_evidence() (
        name="$1"
        shift
        pending="$(mktemp "$scratch/.pending-XXXXXXXX")"
        trap 'rm -f -- "$pending"' EXIT
        "$@" > "$pending"
        python3 -I -c 'import os; os.fsync(0)' <> "$pending"
        mv -- "$pending" "$scratch/$name"
    )
    publish_evidence review.patch scripts/make_patch.sh "$base" "$head"
    if command -v deep-diff-forge >/dev/null; then
        if [[ -L scripts || -L scripts/worktree_diff.py || ! -f scripts/worktree_diff.py ]]; then
            echo "Gate input refused: scripts/worktree_diff.py" >&2
            exit 2
        fi
        python3 -I scripts/worktree_diff.py --check-inputs
        deep-diff-forge --stdin-patch --outline < "$scratch/review.patch"
        publish_evidence review.rank.json deep-diff-forge --stdin-patch --rank --json < "$scratch/review.patch"
        cargo run -q --bin forge-warden -- check --policy warden.toml --advisory --rank "$scratch/review.rank.json" --patch "$scratch/review.patch"
    fi
