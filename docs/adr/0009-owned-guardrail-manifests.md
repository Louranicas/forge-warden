# ADR 0009: Guardrail configuration has a checked source of truth

Status: proposed implementation, WP-D6. Author: Bastion (O4).

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
a weakening is safe. CI must run the trusted enforcement path under ADR0006's external authority.

`lint-expectations.toml` lists every (file, lint, exact source_reason) with multiplicity, plus a
review_reason of at least20 non-whitespace-trimmed characters. An attribute-aware scanner skips
nested comments, quoted/raw strings and character literals, tracks balanced attribute brackets,
and inspects multiline and conditional expect/allow forms. Unknown expectation syntax, empty
reasons, any allow attribute, unlisted expectations and stale ledger entries fail. It is a lexical
admission scanner, not a Rust parser or proof of lint semantics; compiler/clippy remain required.

Some existing named-door source reasons are shorter than20 characters (for example `door: emit`).
Their exact bytes remain pinned, with longer review explanations in this ledger, avoiding concurrent
changes to C1-owned doors.rs. This does **not** claim that every source comment itself meets20 chars.
A newly added short expectation still fails unless explicitly added to the protected ledger with a
review explanation. Review of such a ledger change remains the owner's responsibility.

The checker runs in CI, ci_fast and ci_local; its adversarial fixture suite runs in CI/ci_fast.
It never regenerates or blesses manifests during a check. For a legitimate change, edit the source
manifest and affected representations in one reviewed diff; update the expectation ledger only
for reviewed source changes. Candidate-controlled simultaneous edits to checker and floor cannot
establish authority; external trusted checks and ownership remain necessary.

Rung4 CI invariants carry equality/floor/ledger obligations; runtime glob and attribute parsing are
admission. Types cannot express equality across TOML, JSON, CODEOWNERS and source files. Tests plant
independent policy/owner/settings/guard drift, table-form lint relaxation, inherited-lint removal,
unlisted/stale/empty expectations and multiline conditional allow. A second set of plants disables
one checker at a time to demonstrate that the tests detect loss of enforcement.
