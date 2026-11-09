//! Q-TYPES-1 (Q1-01, Zen B1): provenance is a property of the core's TYPES, not of the CLI's
//! clippy.toml, which a downstream crate never sees. This test compiles a scratch crate that depends
//! on forge-warden-core with default features, the way any library consumer would, once per form:
//! every way of reaching a pass without an admitted producer, or of judging a ranking with another
//! ranking's coverage proof, must be a compile error with the named rustc code, and the legitimate
//! paths (admit, attest, check, judge; and the advisory path) must compile.
#![expect(
    clippy::disallowed_methods,
    clippy::unwrap_used,
    reason = "test door: writes a scratch crate and runs cargo check on it; a panic is a failed test"
)]

mod common;
use common::root;
use std::path::PathBuf;

/// Imports every form shares; unused ones are only warnings.
const PRELUDE: &str = "#![allow(warnings)]
use forge_warden_core::Date;
use forge_warden_core::authority::*;
use forge_warden_core::facts::{PatchFacts, Unverifiable};
use forge_warden_core::policy::Policy;
use forge_warden_core::rank::RankDoc;
use forge_warden_core::verdict::{Seals, Verdict};
";

/// `(what, source, the rustc error code it must fail with)`.
const REFUSED: [(&str, &str, &str); 10] = [
    (
        "attest with a bare string instead of an admitted producer",
        "pub fn f(doc: RankDoc) -> Ranking<Produced> { Ranking::attest(doc, String::new()) }",
        "E0308",
    ),
    (
        "raw evaluate through the crate root",
        "pub fn f(p: &Policy, d: &RankDoc, s: Seals, t: Date) -> Verdict { forge_warden_core::evaluate(p, d, s, t) }",
        "E0425",
    ),
    (
        "raw evaluate through its module",
        "pub fn f(p: &Policy, d: &RankDoc, s: Seals, t: Date) -> Verdict { forge_warden_core::eval::evaluate(p, d, s, t) }",
        "E0603",
    ),
    (
        "an admitted producer built by hand",
        "pub fn f() -> forge_warden_core::policy::AdmittedProducer { forge_warden_core::policy::AdmittedProducer { sha256: String::new() } }",
        // non_exhaustive (E0639) is reported first; the private field (E0451) refuses it too.
        "E0639",
    ),
    (
        "a verdict's outcome rewritten after judging (Q-TYPES-2: fields are private, derived once)",
        "pub fn f(mut v: Verdict) -> Verdict { v.verdict = forge_warden_core::Outcome::Pass; v }",
        "E0616",
    ),
    (
        "T-GITPIN-01: a judged verdict cloned, then its outcome rewritten",
        "pub fn f(v: &Verdict) -> Verdict { let mut w = v.clone(); w.verdict = forge_warden_core::Outcome::Pass; w }",
        "E0616",
    ),
    (
        "T-GITPIN-01: a note's detail rewritten on an owned verdict",
        "pub fn f(mut v: Verdict) -> Verdict { for n in &mut v.notes { n.detail.push_str(\"x\"); } v }",
        "E0616",
    ),
    (
        "a seal built by hand (Q-TYPES-2: only Sha256Hex::of and ::parse make one)",
        "pub fn f() -> forge_warden_core::seal::Sha256Hex { forge_warden_core::seal::Sha256Hex(String::new()) }",
        "E0603",
    ),
    (
        "one ranking judged with another ranking's coverage proof (the old separate argument)",
        "pub fn f(p: &Policy, a: &Ranking<Produced>, b: &Ranking<Produced>, x: Result<PatchFacts, Unverifiable>, s: Seals, t: Date) -> Verdict { \
         let checked = check(a, x); evaluate_authority(p, b, &checked, SuppliedCopy::Absent, s, t) }",
        "E0061",
    ),
    (
        "a supplied ranking's proof judged as produced",
        "pub fn f(p: &Policy, doc: RankDoc, x: Result<PatchFacts, Unverifiable>, s: Seals, t: Date) -> Verdict { \
         let supplied = Ranking::supplied(doc); evaluate_authority(p, &check(&supplied, x), SuppliedCopy::Absent, s, t) }",
        "E0308",
    ),
];

/// The legitimate paths: these must compile.
const ADMITTED: [(&str, &str); 2] = [
    (
        "admit, attest, check, judge",
        "pub fn f(p: &Policy, doc: RankDoc, pin: &forge_warden_core::seal::Sha256Hex, x: Result<PatchFacts, Unverifiable>, s: Seals, t: Date) -> Option<Verdict> { \
         let admitted = p.admit_producer(pin)?; let ranking = Ranking::attest(doc, admitted); \
         let checked = check(&ranking, x); Some(evaluate_authority(p, &checked, SuppliedCopy::Absent, s, t)) }",
    ),
    (
        "advisory: supply, check, judge",
        "pub fn f(p: &Policy, doc: RankDoc, x: Result<PatchFacts, Unverifiable>, s: Seals, t: Date) -> AdvisoryOutcome { \
         let supplied = Ranking::supplied(doc); evaluate_advisory(p, &check(&supplied, x), s, t).outcome() }",
    ),
];

/// `cargo check` the scratch crate with `body`; returns whether it built and its stderr.
fn check_form(dir: &std::path::Path, body: &str) -> (bool, String) {
    std::fs::write(dir.join("src/lib.rs"), format!("{PRELUDE}\n{body}\n")).unwrap();
    let out = common::command(env!("CARGO"))
        .args(["check", "--offline", "--quiet", "--message-format=short"])
        .current_dir(dir)
        // Hermetic, as in mint_lint.rs: the caller's flags are not what this measures.
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_BUILD_RUSTFLAGS")
        .env("CARGO_TARGET_DIR", dir.join("target"))
        .output()
        .unwrap();
    (out.status.success(), String::from_utf8(out.stderr).unwrap())
}

#[test]
fn a_downstream_crate_cannot_mint_judge_raw_or_swap_a_coverage_proof() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("api-boundary");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let core = root().join("crates/forge-warden-core");
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"api-boundary-consumer\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n[dependencies]\nforge-warden-core = {{ path = {:?} }}\n\n[workspace]\n",
            core.to_str().unwrap()
        ),
    )
    .unwrap();
    std::fs::copy(root().join("Cargo.lock"), dir.join("Cargo.lock")).unwrap();
    for (what, body) in ADMITTED {
        let (built, stderr) = check_form(&dir, body);
        assert!(built, "the legitimate path must compile: {what}\n{stderr}");
    }
    for (what, body, code) in REFUSED {
        let (built, stderr) = check_form(&dir, body);
        assert!(!built, "compiled, but must not: {what}");
        assert!(
            stderr.contains(&format!("error[{code}]")),
            "{what}: expected error[{code}]\n{stderr}"
        );
    }
}
