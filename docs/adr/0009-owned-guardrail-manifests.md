# ADR 0009: Guardrail configuration has a checked source of truth

Status: Accepted

Historical status record: Status: proposed implementation, WP-D6. Author: Bastion (O4).

`protected-paths.txt` is the canonical sorted root-relative list. It uses literal segments,
whole-segment `*`, and trailing `**`; arbitrary glob syntax refuses. Both structured edits and
literal shell paths in the hook consume that file. CI compares it exactly with the quality-policy
rule, CODEOWNERS and Edit/Write permission entries. The policy cannot add exceptions or weaken to
an unknown severity. Missing, extra or duplicate paths fail. The list includes member manifests,
Cargo.lock, the floor/ledger/checker, scripts and managed-agent templates as well as existing controls.
A directory pattern also protects the directory itself in the hook. Only policy's protect_paths
rule changed; no output schema or rule-kind contract changes.

`lint-floor.toml` captures the reviewed workspace lint levels and group priorities. Structural TOML
comparison rejects missing/weaker entries or changed priorities, and rejects new allow/warn
exceptions outside that reviewed floor. Existing named allow exceptions remain explicit in the
floor. Every member manifest must inherit workspace lints. The floor is itself protected: changing
the floor together with Cargo.toml is an owner-reviewed policy change, not an automatic proof that
a weakening is safe. CI must run the trusted enforcement path under ADR 0006's external authority.

`lint-expectations.toml` lists every (file, lint, exact source_reason) with multiplicity, plus a
review_reason of at least 20 non-whitespace-trimmed characters. An attribute-aware scanner skips
nested comments, quoted/raw strings and character literals, tracks balanced attribute brackets,
and inspects multiline and conditional expect/allow forms. Unknown expectation syntax, empty
reasons, any allow attribute, unlisted expectations and stale ledger entries fail. It is a lexical
admission scanner, not a Rust parser or proof of lint semantics; compiler/clippy remain required.

Some existing named-door source reasons are shorter than 20 characters (for example `door: emit`).
Their exact bytes remain pinned, with longer review explanations in this ledger, avoiding concurrent
changes to C1-owned doors.rs. This does **not** claim that every source comment itself meets 20 chars.
A newly added short expectation still fails unless explicitly added to the protected ledger with a
review explanation. Review of such a ledger change remains the owner's responsibility.

The checker runs in CI, ci_fast and ci_local; its adversarial fixture suite runs in CI/ci_fast.
It never regenerates or blesses manifests during a check. For a legitimate change, edit the source
manifest and affected representations in one reviewed diff; update the expectation ledger only
for reviewed source changes. Candidate-controlled simultaneous edits to checker and floor cannot
establish authority; external trusted checks and ownership remain necessary.

Rung 4 CI invariants carry equality/floor/ledger obligations; runtime glob and attribute parsing are
admission. Types cannot express equality across TOML, JSON, CODEOWNERS and source files. Tests plant
independent policy/owner/settings/guard drift, table-form lint relaxation, inherited-lint removal,
unlisted/stale/empty expectations and multiline conditional allow. A second set of plants disables
one checker at a time to demonstrate that the tests detect loss of enforcement.

## Compiler source admission (D6 v2)

The production check runs fresh `cargo check --workspace --all-targets --locked --offline` in a
unique temporary target directory. Cargo metadata identifies workspace artifacts; each artifact
must have exactly one fresh rustc dep-info file. Every member must produce evidence. This executes
the normal trusted build gate; it is not a sandbox for untrusted build scripts or procedural macros.
First-party build scripts are unsupported. Compiler, Cargo configuration and dependencies remain
trusted admission inputs under the external workflow, not authority inferred from candidate files.

Every discovered input is checked for symlink components before canonicalization and must remain
inside the repository. Rust inputs must be in `crates/*/src` or `crates/*/tests`; the latter and
`fixtures/adversarial` helpers are admitted only in test artifacts. Helpers are scanned normally,
with no filename exemption. The compiled Rust set under crates must equal the source walk: dormant
or unsupported feature-only source fails instead of silently disappearing from ledger coverage.
This profile covers default features/all targets on the gate platform, not arbitrary cfg expansion.

Dep-info parsing consumes every rule. Paths currently accept ASCII alphanumerics and `_./+-` only;
Make escapes, whitespace within paths, colons, dollar signs and non-UTF-8 input are refused rather
than partially decoded. Ordinary parent-relative module paths are accepted only after component
and canonical containment checks. Generated sources outside these roots are unsupported.

`include!` can load source without a `.rs` suffix. Consequently every non-Rust compiler input needs
a protected `[[data_input]]` ledger entry with an exact SHA256 and owner review explanation. The
initial entries are the two embedded JSON schemas and starter TOML policy, inspected as data.
Changed, unknown, duplicate and stale entries fail. This is reviewed data admission, not an
extension-based assertion that arbitrary bytes cannot contain Rust. Attribute expansion from
external procedural macros remains outside this explicit-source lexical ledger's claim.

The reviewed ledger integration delta on base 4637d34 covers B3's scratch-repository vocabulary
test and C1a's descriptor/child-process doors and test fixtures. Its temporary dead-code exception
must be removed when C1c wires those doors. It is not an automatic ledger regeneration workflow.

D6 v3 integrates on main ec45925 (C1b, A4 and B3.1). Six additional reviewed rows cover
C1b private_dir, git_facts temp-base lookup and admit_base canonicalization, plus coverage-test
fixture I/O, explicit panic on missing fixtures and unwraps of committed corpus fields. Each row
records its distinct intent; no source or data pin changed. The four D6 v2 commits replay unchanged.

## Rulings #9: compilation-context agreement (D6 v4)

This supersedes the earlier profile.test-only description. Production means a source appears in
an artifact of kind lib, rlib, bin, proc-macro, dylib, cdylib or staticlib with profile.test=false.
An observation from any such artifact wins over all non-production observations. Test, example
and bench artifacts and test-profile builds are non-production. Unknown artifact kinds refuse.
Any production source containing a `tests` path component is refused as an ambiguous layout.
Admitted source roots include crates/*/{src,tests,examples,benches}; examples and benches remain
subject to the same explicit expectation ledger. The compiled/walked equality covers these roots.
The fixture-ranker example under tests/support is non-production even with profile.test=false.

Keel's Rust classifier API sources_with_evidence exports its classification and the
exact Cargo JSON stream and target directory it consumed. The classifier_agreement Rust test
passes those to Python: no second cargo check occurs. Python independently resolves workspace
membership using metadata, locates the same dep-info files, applies its admission and compares
both complete sorted sets. Cached artifact evidence from the Rust consumer is accepted for this
comparison only; the production Python discovery still requires its own fresh target. This is
same-build agreement, not a claim that two implementations independently returning the same wrong
answer prove correctness. The kind matrix, compiled layout cases and planted example-as-production
divergence independently exercise the rule. A divergence or either side's refusal fails agreement.

The C1c stack's ledger delta removes staged dead_code, adds the reviewed production door and
integration-test expectations, replaces the embedded verdict schema v0 pin with reviewed v1 JSON,
and updates the producer-policy schema pin. Starter policy bytes are unchanged. The agreement
test adds one test I/O exception. None of these changes bless a golden or waive C1c boundary findings.
