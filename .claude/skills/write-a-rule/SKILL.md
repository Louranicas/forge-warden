---
name: write-a-rule
description: Add a new rule kind to forge-warden end to end (policy reader, evaluation, schemas, docs, tests), or extend an existing kind with a field. Use whenever a change touches crates/forge-warden-core/src/policy.rs or eval.rs semantics.
when_to_use: The user asks for a new kind of check (e.g. "require a CHANGELOG entry when the public API changes"), or a new field on an existing kind.
---

# Adding a rule kind

A rule kind touches seven places. Missing one is caught by a test, but it is faster to do them
in order. Work in this order and run `scripts/ci_fast.sh` after each numbered step.

1. **Decide the semantics first, in writing.** One sentence: "violated when …". Then answer:
   - Is a violation tied to a path (waivable) or to the whole change (never waivable)?
   - What does it do with `deleted` files? (Companions and triggers ignore them; say why if not.)
   - Can it ever make a verdict *looser* than without it? It must not: the law
     `adding_a_rule_cannot_loosen_the_verdict` in `crates/forge-warden-core/tests/laws.rs` will
     try. Add the new kind to `CheckSpec` and the independent `reference` in
     `fixtures/adversarial/reference_model.rs`, and to `rule_strategy` in `laws.rs`.
2. **Reader** (`policy.rs`): a `Check` variant with typed fields; its name in `KINDS`; each new
   field in `RawRule` and in `RULE_FIELD_KINDS`; validation in `build_check` that reports
   problems with `issue(location, …)` and never panics. Required fields have no defaults.
3. **Evaluation** (`eval.rs`): one function per kind, called from `find_violations`. Keep
   deep-diff-forge's file order; sort anything you collect.
4. **Schemas**: `schemas/policy.v0.schema.json` (the property, plus an `allOf` branch for the
   kind) and, if the verdict changes, `schemas/verdict.v0.schema.json`.
   `tests/schema_tracks_code.rs` fails until they match the code.
5. **Tests**: an example test per clause of the semantics, including the boundary
   (limit, limit + 1), a deleted file, an excepted path, and a waiver. Then ask the `falsifier`
   subagent to attack your one-sentence semantics.
6. **Docs**: the kind table in `README.md`, `docs/POLICY_REFERENCE.md`, and `CHANGELOG.md`
   under `## [Unreleased]`.
7. **Starter policy**: only if most repositories should have the rule. Changing
   `examples/starter.warden.toml` changes the golden verdict: ask the human to run
   `scripts/bless.sh` and review the diff.

Never loosen an existing kind's semantics to make a new one fit. If two kinds overlap, say so
and ask whether one should go.
