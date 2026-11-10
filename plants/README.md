# Named guardrail plants (WP-A4, post-C1c batch)

Run from a neutral directory. The runner creates one disposable `git clone --no-local`, checks out
an exact commit, and applies each plant only there. It never writes the source repository or hub.
The maintained catalog is trusted harness code; this is a verification runner, not a sandbox for
arbitrary repositories. Compiler and test execution need the same isolation as the ordinary gate.

```sh
python3 -I /path/to/plants/run.py --source /path/to/local/hub.git \
  --revision FULL_40_HEX_COMMIT --output /new/evidence/directory
python3 -I /path/to/plants/test_runner.py
```

Native mode uses installed `cargo`, an isolated target inside the temporary run, offline Cargo,
and disabled rustup automatic installation. Optional `--runner /absolute/trusted/executable` invokes
`RUNNER CHECKOUT CARGO_ARGUMENTS...`, allowing an existing offline container launcher without
embedding machine paths in the repository. That launcher must mount CHECKOUT, select the pinned
compiler and caches, and return the command's actual exit code. It is trusted code, recorded in
command receipts, and must not share a target with other concurrent runs. `--case ID` can be repeated.
No installation, blessing, external remote or network access is performed by this Python runner;
the execution backend must enforce network isolation for code it runs.

Each case runs a fresh green baseline before planting. CAUGHT requires a nonzero exit and the
catalog's named diagnostic. Zero is ESCAPED. A failed baseline, unrelated compiler error, missing
anchor, killed command or oversized output is ERROR, never a caught plant. Commands have a five
minute bound and four MiB output bound. Processes and surviving descendants in their process
group are killed on cleanup; this is not containment against hostile process-group escape.
Touched bytes and Cargo.lock are restored between cases; the temporary checkout is removed at exit.

`report.json` records the subject, catalog hash, register revision, commands, real exit codes,
named statuses and log hashes. The caller keeps the new private evidence directory. Receipts
identify local observations and are not authenticated attestations. Failed setup may leave partial
logs. Exit 1 means a plant escaped or a command/recipe failed. Exit 2 means all runnable plants
were caught but the full guardrail survey is incomplete. Explicit case selections return 0 when
the selected plants are caught, without implying whole-register coverage.

This revision implements 91 named plants over 47 guardrail rows; Q3-07/Q3-10 strengthen G-45/G-24, and 13 `z3-*` cases exercise G-45. The remaining 5 register rows are explicitly PENDING
in the catalog; no missing recipe is counted as caught. The source register was a baseline draft;
its hash and binding corrections for duplicate paths, producer authentication, named-intent defaults and
mutation measurement are included. In particular the README table plant checks the currently
implemented missing-row direction; it does not claim an extra-row drift check exists.

The named lint diagnostics may use underscores in Clippy help anchors or hyphens in compiler
command-line notes. Regexes admit both spellings while still requiring the named lint. The direct
Rust plants test current enforcement, not future stronger controls proposed in the register.
No mutation-quality count is produced by this harness. Every future mutation recipe must use
`env -u CARGO_TARGET_DIR cargo mutants --no-config --cap-lints true` with actual gate workspace scope
(no `--test-workspace`), and retain the exact source diff and outcome totals.

Second-batch additions cover reviewed default markers, effects outside the door, unknown-signal
fail-closed behavior, canonical path admission, the duplicate-path HOLD floor, expanded Cargo
dependency tables, ranking schema drift, private dead code, the rank input byte boundary and
Serde additive-member tolerance. These are named counterexamples, not exhaustive guardrail proofs.
The expanded-dependency recipe deliberately omits --locked so Cargo can update its private clone's
lock for an already cached transitive dependency; the manifest and lock bytes are both restored.
The normal gate still uses --locked. Two existing recipes now use B2's validated getters.

Third-batch execution profile: --exec-runner /absolute/trusted/backend runs BACKEND CHECKOUT PROGRAM
ARGUMENTS for both Cargo and script recipes, without shell evaluation. It is mutually exclusive
with --runner (Cargo only). Selecting a script recipe with a Cargo-only backend refuses before
creating evidence or executing anything; it never falls back to native execution. Native script
execution is explicit by omitting both backends. Generic commands are trusted catalog argv, not
untrusted command text. The receipt records the execution profile and exact command argv. The
backend remains responsible for network/process/filesystem isolation and compiler pins. Nine
runner controls now include actual script argv/cwd checks and refusal of backend downgrade. Seven
new script/hook plants cover swallowed judge exits, lost shell tokenization, malformed hook input,
unsupported tools, candidate judge builds, stale-head acknowledgement replay, and candidate-supplied
review policy/contracts. These fixtures prove local controls; they do not establish a hosted
required-workflow identity, real signer enrollment or managed-provider isolation.

Fourth-batch admission checks reject a diagnostic already present in the green baseline
(ERROR_DIAGNOSTIC_AMBIGUOUS). After every case, actual Git status must prove that source and
Cargo.lock were restored; a failed restore is ERROR_RESTORE and aborts later cases. The harness
neutralizes inherited RUSTFLAGS, CARGO_ENCODED_RUSTFLAGS and CARGO_BUILD_RUSTFLAGS; intentional
mutation commands still set --cap-lints true themselves and unset CARGO_TARGET_DIR. Controls
include a broken-restorer CLI run and hostile parent lint flags.

Three more product plants cover enum wildcards, raw terminal control characters, and actual
mutation-survivor rejection. The last one first requires normal all-targets Clippy to pass, then
runs a bounded selected mutation experiment against an explicitly untested named function. Its
normal baseline has no such function; the plant adds it. Only an exit2 with nonempty missed
mutants of that exact function, no timeout and no unviable outcome emits MISSED_SURVIVOR_REJECTED.
The helper prints the real outcome totals and mutant names into the retained case log. These
intentionally MISSED mutants demonstrate a correctly rejected bad change; they are not a green
mutation-quality receipt for the repository. Source selection is bounded to render.rs.

Batch 5 adds an external-consumer private-constructor probe, a measured normal-compiler/MSRV boundary probe, and corpus vocabulary drift. The MSRV recipe requires the preinstalled 1.88.0 toolchain; backends must expose it without downloads.

B7 v3 compatibility: string-slice targets the named fail-safe helper; unreviewed-default now
plants the fail-open raw `unwrap_or(0)` at the file-count conversion and requires the compiler's
`disallowed_methods` diagnostic. It no longer relies on removing a comment marker. The swallowed
judge-exit recipe targets D8's explicit advisory command. Those three recipe updates preserved
the original 38-case count and require fresh baselines, named red diagnostics and clean restoration.

Post-C1c additions exercise missing-patch usage errors mapped to success, an omitted-file coverage
floor, producer digest admission, canonical citation ordering, oversized-function linting,
unreachable public items, CODEOWNERS drift, and the independent finite reference. The producer
recipe explicitly builds the fixture-ranker example before running the named process-boundary
test. Citation ordering is deliberately reversed, keeping the mutated source valid.

G-29 measures the existing function-length lint, not a 400-line whole-module budget. G-30 measures
unreachable public visibility, not a public API snapshot. These are the implemented controls;
stronger controls proposed in the register remain outside these observations. G-13 specifically
checks that a missing-patch usage refusal cannot report success. G-15 qualifies an explicitly
admitted fixture binary digest, not its release string, provenance, or hostile-runtime isolation.

A second G-17 plant covers the authority command. Its small adapter runs the maintained shell
fixture suite in a private copy and adds a named diagnostic to the existing exact-exit assertion,
without relaxing that assertion or changing checked-out source. These controlled command fixtures
prove script exit propagation; they do not qualify a live ranker or hosted admission.

The five pending rows have individual reasons in the catalog: unavailable local enforcement for
CI toolchain/name drift or documentation references, the unimplemented strict skip contract, and
a missing offline zizmor backend. G-52 (runner routing) has six script plants (two read an ambient variable, RUNNER-ROUTING.1) against
`scripts/tests/runner_routing.py`. Missing tools and missing checks are never counted as caught.

## Required gate execution (Q-PERIMETER)

CI and `scripts/ci_local.sh` run `plants/test_runner.py` and
`scripts/check_plants.py`. The latter requires a clean committed subject, selects
**every runnable catalog ID**, and checks the complete receipt against that exact
HEAD. It fails on an escaped plant, any ERROR, duplicate/missing case or subject
mismatch. Evidence lives under `target/plants-*/evidence`; CI uploads it even on
failure. A successful runnable-set check does not close the catalog's explicitly
PENDING guardrails: those remain printed and present in the receipt.

Dependencies must already be fetched (`cargo fetch --locked`); the harness remains
offline and uses a disposable clone/target. Local uncommitted changes must be
committed before this exact-subject check can qualify them. Neither this gate nor
the candidate workflow authenticates its own hosted merge-admission identity.
Tool versions are owned by `scripts/tool-versions.toml`, Rust by
`rust-toolchain.toml`, and MSRV by `Cargo.toml`; `scripts/tool_versions.py --check`
checks the workflow's declarative copies. Updates require normal review.

The gate preflights the declared MSRV toolchain (already installed; no automatic
installation) and the complete offline locked dependency set. Native Cargo options
are inserted before the `--` test-argument separator, so exact named-test recipes
use the isolated target rather than accidentally forwarding Cargo flags to libtest.

### Hosted tool and diagnostic contract (Q-PERIMETER.1)

Each runnable case declares `required_tools`, including reviewed transitive helper requirements.
`python3 -I scripts/check_plants.py` validates these declarations and probes the union before
creating an evidence run. A missing command/subcommand, wrong pinned cargo-mutants version or
failed probe is a named `plant preflight` refusal, never a skip or a caught plant. The separate
MSRV/dependency prerequisites still run before the catalog. Keep helper dependencies in the
catalog when changing a recipe: the direct-argv check is not general static analysis of Python,
shell or Rust subprocesses.

The gate installs cargo-mutants at the canonical `scripts/tool-versions.toml` version, using the
same canonical install-action SHA as the mutation job. `scripts/tool_versions.py --check` and
`scripts/tests/perimeter.py` check both pins and that required capabilities are available in the
gate job before the plants step. Basic shell/Git/Python/OpenSSH/coreutils capabilities come from
the reviewed ubuntu-latest image contract and are probed at runtime; Rust/clippy are explicitly
installed. A different runner image or workflow shape requires reviewing this provisioning
contract, not silently assuming the tools exist.

The harness pins Cargo colour off, TERM=dumb, NO_COLOR, CLICOLOR, locale and terminal size, and
removes force-colour overrides. Child baselines and plants use the same environment, including
when the caller requests colour. Named diagnostic matching remains strict: unrelated failures
or missing tools cannot become CAUGHT. This is diagnostic normalization, not an execution sandbox.


Oracle/golden cases are owned by Touchstone (O3). `oracle-production-coupling` replaces the local scope check with a production
matcher call and requires the named independence refusal. The syn syntax restrictions are paired with a standalone
Rust metadata target that has no harness or production dependencies; nested declarations cannot
authorize a harness extern alias. The exact compiling alias attack is retained as regression data. `golden-behavior-rebless`
constructs a behaviour-changing candidate as Git data and pairs it with the planted golden
update; HEAD's immutable policy supplies the dedicated golden-review HOLD. The script's pass
is a named-rule property, not an overall authority PASS (the CLI run is advisory). The candidate's
allow-all policy is ignored. `golden-bless-environment` proves an inherited bless switch cannot
make the ordinary golden test rewrite its expected bytes. This runner never calls the operator
regeneration entrypoint.

The quality contract binds both current demo golden consumers to registered assertion helpers.
The dedicated trusted-base HOLD covers their source file and the producer/guard implementation,
so moving or inlining their expected output retains that obligation. This is a declared-surface
contract: arbitrary new inline expectations in unknown future tests require general test-owner
review and registration; the checker does not infer all semantic expectations from Rust code.
Holding the whole consumer file intentionally includes unrelated test edits in that file.
Operator regeneration refuses either CI or GITHUB_ACTIONS and requires a private, owned,
non-symlink backup directory outside the repository. The wrapper explicitly allocates it in /tmp.
Write-control regressions run only against disposable fixture copies; live expected bytes stay read-only.

WP-GIT-PIN adds twelve plants for Git owner/mode/regular-file/symlink/size admission, missing policy, unlisted digest, exact policy snapshot, descriptor-bound version execution, script refusal, acknowledgement revocation and v1 downgrade. Each runs the specific positive/refusal test before and after planting. They extend G-15 and G-36 rather than claiming new historical register coverage.
