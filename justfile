# `just` lists the recipes. Every recipe is a thin wrapper over a script CI also runs.
default:
    @just --list

# One-time setup: use the versioned git hooks.
install-hooks:
    git config core.hooksPath .githooks
    @echo "hooks installed: pre-commit (fast gate), commit-msg, pre-push (full gate)"

# fmt + clippy + tests: seconds. The pre-commit hook and Claude Code's Stop hook run this.
fast:
    scripts/ci_fast.sh

# Every CI gate that can run locally.
ci:
    scripts/ci_local.sh

# Regenerate golden verdicts after an intended change; review the diff before committing.
bless:
    scripts/bless.sh

# Mutation-test the lines changed since main. A surviving mutant is a missing test.
# --cap-lints: with `warnings = "deny"`, a mutant that leaves an unused binding fails to compile
# and is counted "unviable", hiding survivors (main c44c00e: 3 missed, 59 more viable when capped).
# CARGO_TARGET_DIR unset: each worker builds in its own copy; a shared target dir mixes binaries.
mutants:
    #!/usr/bin/env bash
    set -euo pipefail
    while IFS= read -r name; do unset "$name"; done < <(compgen -v GIT_)
    mutation_diff="$(mktemp)"
    trap 'rm -f -- "$mutation_diff"' EXIT
    comparison="$(bash scripts/make_patch.sh --resolve-comparison refs/remotes/origin/main HEAD)"
    read -r _ mutation_base <<< "$comparison"
    git --no-replace-objects -c core.quotePath=false diff --no-ext-diff --no-textconv --text --no-color --src-prefix=a/ --dst-prefix=b/ "$mutation_base" -- 'crates/*/src/**.rs' > "$mutation_diff"
    env -u CARGO_TARGET_DIR cargo mutants --no-config --cap-lints true --in-diff "$mutation_diff" --workspace

# Check the signal vocabulary against a deep-diff-forge checkout or commit.
ddf_rev := `python3 -I scripts/tool_versions.py ddf_rev`
vocab rev=ddf_rev:
    scripts/check_vocab.sh {{rev}}

# Judge the current branch with this repository's own policy (needs deep-diff-forge on PATH).
# Advisory mode: the ranking is supplied, so the verdict is at best hold. Committed changes only:
# the patch-maker diffs commits, not the work tree.
judge:
    #!/usr/bin/env bash
    set -euo pipefail
    patch="$(mktemp)"
    trap 'rm -f -- "$patch"' EXIT
    scripts/make_patch.sh refs/remotes/origin/main HEAD > "$patch"
    deep-diff-forge --stdin-patch --rank --json < "$patch" | cargo run --quiet --bin forge-warden -- check --policy warden.toml --patch "$patch" --advisory --rank -
