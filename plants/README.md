# Named guardrail plants (WP-A4, second batch)

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

This second batch implements 25 named plants. The remaining 26 register rows are explicitly PENDING
in the catalog; no missing recipe is counted as caught. The source register was a baseline draft;
its hash and binding corrections for duplicate paths, producer authentication, default markers and
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

Third batch in progress: --exec-runner /absolute/trusted/backend runs BACKEND CHECKOUT PROGRAM
ARGUMENTS for both Cargo and script recipes, without shell evaluation. It is mutually exclusive
with --runner (Cargo only). Selecting a script recipe with a Cargo-only backend refuses before
creating evidence or executing anything; it never falls back to native execution. Native script
execution is explicit by omitting both backends. Generic commands are trusted catalog argv, not
untrusted command text. The receipt records the execution profile and exact command argv. The
backend remains responsible for network/process/filesystem isolation and compiler pins. Eight
runner controls now include actual script argv/cwd checks and refusal of backend downgrade. Seven
new script/hook recipes are awaiting their measured batch run (32 runnable,19 pending recipes).
