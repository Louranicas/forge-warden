# Contributing

Thank you. forge-warden is a gate, so its own bar is high, and most of that bar is automated:
you find out in seconds, not in review.

## Setup

Use Linux with `/proc` mounted (including `/proc/self/fd`), Git, Bash, coreutils,
Python 3.11 or newer with PyYAML available to `python3 -I`, Rustup and `just` available.
The checkout pins Rust 1.99.1 plus rustfmt/clippy in `rust-toolchain.toml`; Rustup may need
network access to install that toolchain. Provision the dependency cache once with network
access, and provision the separate Rust 1.88.0 MSRV compiler for the full local gate. The pinned
1.99.1 toolchain supplies rustfmt and Clippy. Then the walkthrough and test gates run offline:

```bash
git clone https://gitlab.com/lukeomahoney/forge-warden.git
cd ./forge-warden
rustup toolchain install 1.88.0 --profile minimal
cargo fetch --locked
```

### Install the hooks

Run the installation command from the checkout root.

```bash
bash -c '[[ ! -L scripts && -f scripts/checkout_just.py && ! -L scripts/checkout_just.py ]] && exec python3 -I scripts/checkout_just.py install-hooks'
```

## Local gate commands

Run these commands from the root of the checkout being validated. Each clears inherited
Just options and selects that checkout’s justfile. The launcher accepts one recipe name.

```bash
# Fast gate
bash -c '[[ ! -L scripts && -f scripts/checkout_just.py && ! -L scripts/checkout_just.py ]] && exec python3 -I scripts/checkout_just.py fast'
# Full local gate
bash -c '[[ ! -L scripts && -f scripts/checkout_just.py && ! -L scripts/checkout_just.py ]] && exec python3 -I scripts/checkout_just.py ci'
```

The full gate requires `cargo-deny` and `cargo-mutants` at the versions pinned in
`scripts/tool-versions.toml`; a missing or different one stops it with a named refusal, never a skip
(G-18). The gate judges committed changes from the merge base of `refs/remotes/origin/main` and HEAD to HEAD.
If that ref is absent, HEAD must have complete history with a single root, which serves as the base.
Both paths require Git 2.41 or later and an [owner-enrolled Git](docs/GIT_RUNTIME.md).
A nonempty patch requires `deep-diff-forge` on `PATH` with its binary hash enrolled in the policy.
For an empty patch the gate runs the unreleased 0.3.0 [`forge-warden empty` command](README.md#authority-and-advisory-mode).
When `zizmor` is absent, the gate's last line names the skipped workflow audit.
See [Where the gate runs](#where-the-gate-runs).

Scripts and their tests run in an offline gate image that has fewer commands than a developer's
machine (no `cmp`, `diff`, `jq`, `xxd` or `file`). `scripts/script_commands.py` (fast gate, and
`GateImageCommands` in `scripts/tests/perimeter.py`) names every command a tracked shell or Python
script calls that is not in its declared list; adding a command to that list means first seeing it
in the gate image.

The fast gate requires Just to check launcher behavior and review evidence permissions.
The measured `localhost/ddf-council-gate:20261026c` image (`8f4ea620d021`) is incompatible
with this gate because Just is absent from its PATH. A hosted-parity claim requires a newly
provisioned replacement image, a measurement confirming Just on its PATH, and a passing replay
in that image. A passing local toolbox run certifies only the toolbox.

### Mutation, judgment, and review

Run the command for the check you need from the checkout root.

Mutation testing

```bash
bash -c '[[ ! -L scripts && -f scripts/checkout_just.py && ! -L scripts/checkout_just.py ]] && exec python3 -I scripts/checkout_just.py mutants'
```

Advisory judgment

```bash
bash -c '[[ ! -L scripts && -f scripts/checkout_just.py && ! -L scripts/checkout_just.py ]] && exec python3 -I scripts/checkout_just.py judge'
```

Bound judgment

```bash
bash -c '[[ ! -L scripts && -f scripts/checkout_just.py && ! -L scripts/checkout_just.py ]] && exec python3 -I scripts/checkout_just.py judge-bound'
```

Review evidence

```bash
bash -c '[[ ! -L scripts && -f scripts/checkout_just.py && ! -L scripts/checkout_just.py ]] && exec python3 -I scripts/checkout_just.py review'
```

## Where the gate runs

The repository of record is GitLab, with mirrors on Gitea and a local Forgejo. Since 2026-11-15 the
project triggers no hosted workflow and uses no result from one: it pushes nothing to GitHub, the
repository has no GitLab pipeline file, and the project's own runners on the Gitea and Forgejo
mirrors take only jobs labelled for them, while the workflow's jobs ask for `ubuntu-latest`. A run
can still start without the project (the weekly schedule in `mutation-baseline.yml` on the old
GitHub repository, or a mirror with Actions turned on); nothing here reads its result.

The workflow files in `.github/workflows/` are kept for a deployer who gates their own repository on
GitHub (see [Hosted workflows](#hosted-workflows-for-deployers-on-github)). They are not run here,
but this repository's own gate reads them:

- `scripts/tool_versions.py --check` (the `tool-versions` step of `scripts/ci_local.sh`) checks their
  tool pins and job budgets.
- A test compares the required checks named below with `ci.yml`'s job names.
- The integrator gate's `hosted_parity` step replays `ci.yml`'s gate job.

A change to them is a change to this repository's gate.

This repository's gate has two layers:

- **In the repository**, use the [local gate commands](#local-gate-commands). The pre-commit and
  agent Stop hooks use the fast gate. The pre-push hook uses the full local gate.
- **The integrator gate**, which the maintainers run on every merge to `main` and on every release
  commit: `fw-verify`, kept with the project's coordination records, not in this repository. It
  builds the subject commit in a fresh clone and runs format, lints, tests, docs, `cargo deny`,
  every script and hook test, the plant catalogue and the `hosted_parity` replay, plus the MSRV
  check with `--msrv` and diff mutation with `--mutants`, each with its real exit code, and writes a
  hashed receipt. [`EVIDENCE.md`](EVIDENCE.md) names the gate run behind each release claim; the
  receipts themselves are kept with the coordination records. A merge into `main` on GitLab is the
  owner's, after a green receipt.

Where each check of the hosted workflows runs now:

| Hosted check (`ci.yml`, `mutation-baseline.yml`) | Where it runs for this repository |
| --- | --- |
| `gate (fmt, clippy, test, doc)` | `scripts/ci_fast.sh` and `scripts/ci_local.sh` together (neither alone); the integrator gate runs the same steps, every script test and every catalogued plant |
| `declared Rust 1.88 compatibility` | the integrator gate's MSRV step (`fw-verify --msrv`: `cargo +1.88.0 check --workspace --all-targets --locked --offline`) |
| `supply chain (cargo-deny)` | `scripts/ci_local.sh` step `deny`, and the integrator gate (`cargo deny --offline check`; the receipt records the advisory database's commit date) |
| `diff mutation (cargo-mutants)` | `scripts/ci_local.sh` step `mutation`, and the integrator gate with `--mutants --parent <previous main>` (in-diff against that parent, `--cap-lints true`) |
| `full workspace mutation baseline` | **not scheduled and not required.** It was run by hand once, for 0.2.0 on its final commit (command and result in [`EVIDENCE.md`](EVIDENCE.md): 2 missed mutants, each explained), and not rerun for 0.2.1 |
| `dogfood (trusted base judges candidate data)` | **not run on each change.** No gate runs the trusted driver `scripts/dogfood.sh` for this repository. `scripts/ci_local.sh` requires a bound judgment for each nonempty comparison. It judges the change with the candidate's own `warden.toml` and a `forge-warden` built from the candidate tree, so a change can loosen the policy that judges it. It is a preview for the developer, not a trusted verdict. Authority mode, in either, needs the owner-enrolled Git allowlist ([`docs/GIT_RUNTIME.md`](docs/GIT_RUNTIME.md)). A local run of `scripts/dogfood.sh` reads a copy of the pinned deep-diff-forge release archive named with `--ranker-archive FILE` and checked against the pinned SHA-256; only the hosted job downloads it. Until the trusted driver is a step of the integrator gate, a merge into `main` is judged by the steps above and by review, not by forge-warden's own policy |
| `vocabulary tracks deep-diff-forge` | **the comparison with the pinned deep-diff-forge commit is in no gate** (the script itself is tested by `check_vocab.rs`). It can be run by hand against a local deep-diff-forge checkout, with no network: `scripts/check_vocab.sh <commit> <checkout>` (with no checkout, as `just vocab` runs it, it fetches deep-diff-forge from github.com) |
| `workflow audit (zizmor)` | **in no gate.** `scripts/ci_local.sh` runs it (step `zizmor`) only when `zizmor` is on `PATH`, and the integrator gate has no zizmor step. The workflow files are not run here, but the integrator gate reads them and deployers copy them, so an unaudited change to them reaches both |
| runner routing (`mutation-baseline.yml` job `route`) | not run: it routes hosted jobs to a self-hosted pool ([`docs/runner-up.md`](docs/runner-up.md)), and the project triggers no hosted job |

The rows in bold are known gaps, not settled states. The dogfood and the vocabulary check are planned
as steps of the integrator gate. Until then this file says so rather than letting a green gate imply
them.

## First rule: a passing example and a failing control

This adds a policy rule using the existing `protect_paths` kind; adding a new Rust rule kind
is a separate change. Build and test the clean checkout first (or use an already provisioned
checkout/cache). The exercise builds an explicitly marked test fixture before using it.
Default Cargo output location `target/` is assumed; unset a custom CARGO_TARGET_DIR for these examples. No installed forge-warden or live ranker is required.

```bash
cargo test --workspace --locked --offline
```

The following script writes a new policy that rejects changes under `private/`. The recorded
demo changes other paths, so this new rule passes. The supplied recorded ranking is advisory:
the rule can pass while the verdict remains HOLD. No Git enrollment or live ranker is needed.
Use the same copied patch bytes for both evaluations. All files stay under ignored `target/`.

<!-- first-rule-exercise:start -->
```bash
set -eu
cargo test --locked --offline -p forge-warden-cli --test oracle_contracts fixture_cli_artifact_receipt
fixture_cli=$(python3 -c 'import json; print(json.load(open("target/fixture-cli.json"))["binary"])')
mkdir -p target/first-rule/empty-policy
export FORGE_WARDEN_TEST_POLICY_ROOT="$PWD/target/first-rule/empty-policy"
forge-warden() { "$fixture_cli" "$@"; }
cp fixtures/demo.patch target/first-rule/change.patch
cat > target/first-rule/warden.toml <<EOF
schema = "forge-warden.policy.v0"
[[rule]]
id = "first-rule-private-files"
why = "Private implementation paths must stay unchanged in this example."
kind = "protect_paths"
severity = "fail"
paths = ["private/**"]
EOF
forge-warden lint target/first-rule/warden.toml
if forge-warden check --policy target/first-rule/warden.toml --patch target/first-rule/change.patch \
  --advisory --rank fixtures/demo.rank.json --today 2026-11-05 --json > target/first-rule/green.json; then
  echo 'advisory evidence must not pass' >&2
  exit 1
else
  test "$?" -eq 8
fi
sed 's|private/|src/|' target/first-rule/warden.toml > target/first-rule/red.warden.toml
if forge-warden check --policy target/first-rule/red.warden.toml --patch target/first-rule/change.patch \
  --advisory --rank fixtures/demo.rank.json --today 2026-11-05 --json > target/first-rule/red.json; then
  echo 'expected the tightened control to fail' >&2
  exit 1
else
  test "$?" -eq 7
fi
if forge-warden check --policy target/first-rule/warden.toml --patch target/first-rule/change.patch \
  --advisory --rank fixtures/demo.rank.json --today 2026-11-05 --json > target/first-rule/green-again.json; then
  echo 'advisory evidence must not pass' >&2
  exit 1
else
  test "$?" -eq 8
fi
```
<!-- first-rule-exercise:end -->

The lint exits 0. Both green **rule outcomes** pass, but the advisory verdicts remain HOLD
(exit 8); the intentionally tighter `src/` control exits 7 with verdict level `fail` and rule id
`first-rule-private-files`. Returning to the original policy restores the passing rule outcome.
The test fixture requires an explicit empty policy root and cannot become a production CLI.
The documentation test runs this exact exercise offline in a fresh source copy, checking the
rule outcomes, advisory notes and verdict levels. E3d parses all four CLI invocations using
the real grammar. Production authority requires an independently admitted real ranker, the
base policy, and [owner-enrolled Git](docs/GIT_RUNTIME.md). Keep policy and tests together in
a pull request and run the [full local gate](#local-gate-commands).


Terms are defined once in the [glossary](docs/GLOSSARY.md). Current design status and whole-ADR
replacement rules are in [ADR status](docs/ADR_STATUS.md).

## What a pull request needs

The [pull request template](.github/pull_request_template.md) is the checklist. In short: the
gate is green, new behaviour has a test that fails without it, contract changes (schemas, exit
codes, vocabulary) are in the CHANGELOG, and the dogfood verdict passes or every hold is
explained (for this repository, see [Where the gate runs](#where-the-gate-runs): no gate runs the
trusted dogfood yet).

Read [`CLAUDE.md`](CLAUDE.md) even if you are not an agent: it is the shortest statement of the
rules, with what to do instead of each forbidden move.

## Command examples

The documentation contract walks source files directly and works without Git metadata, including
in archives and cargo-mutants copies. Untracked `.md` files (including `.MD`) are visible immediately.
Every source Markdown file and the justfile needs an exact invocation count in
`docs/command-sources.tsv`; update that inventory with an intentional example change.

The reviewed exclusions are `.git` metadata entries at any depth and root `target/`,
`mutants.out/`, and `mutants.out.old/` output directories (including output-directory symlinks).
Root `target/` is reserved for Cargo
build artifacts, including generated documentation; source documentation must live elsewhere.
`mutants.out.old/` is cargo-mutants’ rotated previous output. These are exact directory names;
`mutants.outline/` and `mutants.out.backup/` remain source-visible. Nested source directories
named `target` and ordinary Markdown files named `mutants.out.*.md` remain visible. Other source
symlinks refuse rather than hiding or following a document subtree. No ignore file controls this walk.

The test-only pulldown-cmark parser classifies Markdown at every nesting depth. Every code
block receives a language decision: indented code is parsed as shell; fenced code uses the
explicit supported shell, literal YAML, or data language. Unknown or absent fence languages
refuse. Continued shell lines are parsed together; use `text` for recorded output.

Every raw HTML event (block or inline) containing a `pre`, `xmp`, `listing`, `code`, `samp`, or
`kbd` opener refuses. There is no raw-HTML allowlist. Real inline backtick code is a distinct
parser event and remains prose; so do ordinary paragraphs. A fence-looking line inside an
HTML block is HTML, not a data exemption. Genuine data fences after the HTML block ends remain
supported. The parser determines those boundaries; tests do not infer them from tag strings.

Per-file invocation counts remain exact. The shell lexer and literal YAML subset still have
explicit limits: no command expansion or example subprocess is executed, and example paths or
tools need not exist. These tests check actual CLI arguments, not general shell semantics.

## Oracles and golden review

`fixtures/quality-contract.json` declares independent Rust oracle sources and blessed output paths.
Oracle sources are self-contained fixtures: the syn check rejects production crate/parent paths,
non-standard imports, includes/out-of-line modules, and unreviewed attributes/macros. Pure
standard-library macros use explicit `std::` or `core::` paths. This detects structural coupling;
it does not prove that two independently written algorithms cannot share the same conceptual bug.

Ordinary golden tests never write expected output, even if `FORGE_WARDEN_BLESS` is inherited.
An independent verifier builds the reviewed source using
`cargo test --locked --offline -p forge-warden-cli --test oracle_contracts fixture_cli_artifact_receipt`.
The helper publishes exact Cargo artifact paths and a content-addressed build-input subject in
`target/fixture-cli.json`. Pass those three values explicitly to
`python3 -I scripts/golden_output.py --binary PATH --receipt PATH --subject source-sha256:HEX`.
The receipt binds the artifact SHA-256, source-input subject and test cfg set. Missing or
mismatching receipts refuse before execution; the producer executes the verified descriptor
with an explicit empty policy root. The subject works in dirty and Git-less source copies;
it is not a Git commit or a signed attestation. Reviewers still bind the source to the reviewed commit.
The subject is the SHA-256 of the receipt's own `inputs` map of file digests; `golden_output.py`
checks that the two agree but does not recompute those digests from the source files.
Admission state is never normalized away.
The explicit operator-only `scripts/bless.sh --binary PATH --receipt PATH --subject source-sha256:HEX`
entrypoint uses the same artifact receipt and can regenerate after intended changes;
it refuses CI and records binary/output hashes plus an outside-repository backup. The verifier
must compare exact bytes; the builder's own re-blessing is not independent evidence.

All blessed outputs belong under the reviewed `fixtures/` namespace. The trusted base's
`golden-output-needs-review` HOLD covers that entire namespace, including new registered outputs;
it is separate from generic quality ownership. A candidate cannot waive that obligation by
editing its own policy. This new rule becomes authoritative after integration into the trusted
base; an older base retains its existing generic fixture-ownership HOLD. Owner review and the
existing trusted acknowledgement mechanism remain required; an unsigned rationale grants nothing.

## Hosted workflows (for deployers on GitHub)

This section describes the hosted workflows as written, for a deployer who runs them on GitHub. It
does not describe this repository's gate (see [Where the gate runs](#where-the-gate-runs)).

To gate on the hosted workflows, protect `main` with: require a pull request; require branches to
be up to date; require these
checks: `gate (fmt, clippy, test, doc)`, `declared Rust 1.88 compatibility`,
`supply chain (cargo-deny)`, `workflow audit (zizmor)`, `diff mutation (cargo-mutants)`,
`dogfood (trusted base judges candidate data)`, `vocabulary tracks deep-diff-forge`; require
review from code owners; disallow bypass.

The internal `plan mutation shards` and `mutation shard ${{ matrix.index }}` jobs feed the
single required `diff mutation (cargo-mutants)` aggregate; they are not extra ruleset checks.
Planning and every shard must succeed. See [measured budgets](docs/CI_BOUNDS.md).

A `hold` is accepted only with an OpenSSH-signed acknowledgement from a reviewer enrolled in
`.github/hold-reviewers/allowed_signers` on the base branch (ADR 0007). A label grants nothing.
One reviewer, the owner, is enrolled (principal `luke`). The hosted workflow runs dogfood in
advisory mode, which cannot be acknowledged, and no external trust root enforces the check yet
(ADR 0006), so a hosted run fails closed on every hold.

These files under `.github/` are kept for the same deployer. GitLab reads none of them, but this
repository's own gate reads two of them:

- `.github/CODEOWNERS`: `scripts/check_guardrails.py` requires one owner line in it per protected
  path in `protected-paths.txt`, plus the default `*` line (ADR 0009), so it stays a checked copy of
  the protected paths. On the repository of record a merge into `main` is the owner's in any case.
- `.github/pull_request_template.md` is the checklist for a change. GitLab does not load it, so copy
  it into the merge request. Two of its items read differently here: `cargo-deny` and
  `cargo-mutants` are required by the full local gate, not optional, and no gate produces a trusted dogfood
  verdict for this repository yet (see the table above).
- `.github/dependabot.yml` does not run for the repository of record. Dependency updates are made by
  hand, in a reviewed change that passes the gate like any other.
- `.github/hold-reviewers/allowed_signers` is not GitHub-specific: `scripts/hold_ack.py` reads it
  wherever the driver runs, and the path stays as it is. An acknowledgement receipt's `repository`
  field names the repository of record, `lukeomahoney/forge-warden` (ADR 0007).
- `.github/release-signers/allowed_signers` is not GitHub-specific either: `scripts/release_tag.sh`
  verifies a release tag against it as committed in the tagged commit (see "Releases" below).

## Periodic full mutation baseline

For this repository the full baseline is not scheduled. It was run by hand for 0.2.0 (the command is in
`EVIDENCE.md`; full workspace, no diff filter, `--no-config --cap-lints true`) and not rerun for 0.2.1.
Nothing requires it before a release. The rest of this section describes the hosted workflow.

`mutation-baseline.yml` runs each Monday at 04:23 UTC and can be started with
`workflow_dispatch`. It is deliberately not a required pull-request check. Unlike diff mutation,
it selects the full workspace with no diff filter or repository mutation config. It keeps
`--cap-lints true`, clears inherited `CARGO_TARGET_DIR`, and uses the canonical tool pins checked
by `scripts/tool_versions.py`. Tests run for each mutated package (no `--test-workspace`).

Any MISSED mutant makes the job fail; there is no survivor budget or automatic waiver. The
always-uploaded per-shard artifact contains `mutants.out` and exact-subject/tool provenance, including failed
runs. Review caught, missed, unviable and timeout counts separately. An interrupted run is partial
evidence, not a completed baseline. The weekly job does not replace diff mutation on pull requests.

## Releases

A release is two scripts and two people. In a clean clone whose remotes `gitlab`, `gitea` and
`forgejo` are the three services (never GitHub), `scripts/release_prepare.sh VERSION` checks that
HEAD is GitLab's `main` and that the version is the same in `Cargo.toml`, `Cargo.lock`, the
CHANGELOG (a dated section under an empty Unreleased), the README's install line, `EVIDENCE.md` and
`docs/RELEASE-VERSION.md`. It writes the source archive, the release notes and `SHA256SUMS` under
`dist/VERSION/`, creates no tag and pushes nothing, and prints the owner's command. The owner runs
that command, `scripts/release_tag.sh VERSION COMMIT`: it signs the tag with the release key, keeps
it only if it verifies against `.github/release-signers/allowed_signers` as committed in that
commit, and pushes the tag alone to the three services. Both scripts refuse a remote with any URL naming
GitHub; they read the URLs, not where ssh or proxy settings then send them. No signer is enrolled yet, so the script
refuses to tag until the owner enrols the release key in a reviewed change. Tests:
`scripts/tests/release.sh` (local remotes and throwaway keys, no network).

## Decisions

Architecture decisions are in [`docs/adr/`](docs/adr). A change that contradicts one starts with
a new ADR, not a code change.

ADR authors must declare `Owner-deployment: Required` or `Not-required`. Reviewers must
check that classification against the proposed runtime, repository, gate and trust configuration
changes; it is an author declaration, not an automatic deployment assessment. In particular, a
new `Not-required` declaration needs review even when the mechanical status checks pass. An
Accepted decision marked Required must carry the notice and status link described in
[ADR status](docs/ADR_STATUS.md).

The integration tests require the locked dependency set in the selected `CARGO_HOME` before
an offline run. Run `cargo fetch --locked` with network access when provisioning that cache.
Nested Rust test builds perform this check once per test process and report
`TEST_LOCKED_DEPS_UNAVAILABLE` with the provisioning command when it fails. They never fetch
online as a fallback. Test commands resolve executable locations explicitly and report
`TEST_TOOL_UNAVAILABLE` when a prerequisite is absent; the executable-copy helper requires
`/usr/bin/cp`. Signature verification additionally requires executable `/usr/bin/ssh-keygen`
(OpenSSH client), with a `HOLD_ACK_TOOL_UNAVAILABLE` refusal if unavailable.

Scratch directories and nested build targets are unique to each run and held by Drop guards
until their last consumer finishes. They are removed on normal return and panic unwinding;
process termination or abort cannot run Rust destructors. Concurrent runs must use separate
writable source trees, Cargo homes and top-level build targets. Sharing an immutable registry
seed does not share mutable compiler output. Case-list changes require reviewing the protected
coverage manifest as well as the tests; a count alone does not attest retained cases.
