//! Bastion B-C1c-01: only doors.rs may mint `Ranking<Produced>` or judge with raw `evaluate`.
//! The rule is a compiler check, not a text search: `clippy.toml` makes both paths disallowed
//! methods for this crate, and clippy resolves the *path*, so every way of naming it is caught.
//! This test runs the real clippy, with this repository's `clippy.toml`, on a scratch crate that
//! names the two functions in every form measured for C1c v2, and requires each form to be refused
//! and the two controls (the door's `#[expect]`, and an allowed function) not to be.
#![expect(
    clippy::disallowed_methods,
    reason = "test door: writes a scratch crate and runs cargo clippy on it; a panic is a failed test"
)]

mod common;
use common::root;
use std::path::PathBuf;
use std::process::Command;

/// Each line carrying `// FORM:` must be reported as a disallowed method; `// CONTROL:` lines must not.
const PLANTS: &str = r#"
use forge_warden_core::authority::{Produced, Ranking};
use forge_warden_core::authority::Ranking as R;
use forge_warden_core::evaluate as judge;
use forge_warden_core::rank::RankDoc;
use forge_warden_core::verdict::{Seals, Verdict};
use forge_warden_core::{Date, Policy};

fn apply<A, B, T>(f: impl Fn(A, B) -> T, a: A, b: B) -> T {
    f(a, b)
}

pub fn forms(doc: RankDoc, policy: &Policy, seals: Seals, today: Date) -> usize {
    let mut n = 0;
    let _a = Ranking::attest(doc.clone(), String::new()); // FORM: attest direct
    let forge = Ranking::attest; // FORM: attest alias binding (B-C1c-01)
    let _b = forge(doc.clone(), String::new());
    let _c = forge_warden_core::authority::Ranking::<Produced>::attest(doc.clone(), String::new()); // FORM: attest qualified
    let _d = R::attest(doc.clone(), String::new()); // FORM: attest via use-as
    let _e = apply(Ranking::attest, doc.clone(), String::new()); // FORM: attest passed as a function
    let p: fn(RankDoc, String) -> Ranking<Produced> = Ranking::attest; // FORM: attest as fn pointer
    let _f = <Ranking<Produced>>::attest(doc.clone(), String::new()); // FORM: attest UFCS
    let _g = Some(doc.clone()).map(|d| Ranking::attest(d, String::new())); // FORM: attest in a closure
    let _h = vec![Ranking::attest]; // FORM: attest inside an external macro
    let _i = std::hint::black_box(Ranking::<Produced>::attest); // FORM: attest through black_box
    let _v1 = forge_warden_core::evaluate(policy, &doc, seals.clone(), today); // FORM: evaluate direct (re-export)
    let ev = forge_warden_core::eval::evaluate; // FORM: evaluate alias binding
    let _v2 = forge_warden_core::eval::evaluate(policy, &doc, seals.clone(), today); // FORM: evaluate qualified
    let _v3 = judge(policy, &doc, seals.clone(), today); // FORM: evaluate via use-as
    let q: fn(&Policy, &RankDoc, Seals, Date) -> Verdict = judge; // FORM: evaluate as fn pointer
    let _v4 = format!("{:?}", forge_warden_core::evaluate as fn(&Policy, &RankDoc, Seals, Date) -> Verdict); // FORM: evaluate inside format!
    let _v5 = Some(seals.clone()).map(|s| q(policy, &doc, s, today));
    let _x = ev(policy, &doc, seals.clone(), today);
    let _y = p(doc.clone(), String::new());
    n += door(doc.clone());
    n += forge_warden_core::seal::sha256_hex(b"").len(); // CONTROL: an allowed function
    n
}

#[expect(clippy::disallowed_methods, reason = "door: the positive control")]
fn door(doc: RankDoc) -> usize {
    Ranking::attest(doc, String::new()).producer_sha256().map_or(0, str::len) // CONTROL: the door's expect
}
"#;

#[test]
fn clippy_refuses_every_way_of_naming_attest_or_raw_evaluate_outside_the_door() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("mint-lint");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let core = root().join("crates/forge-warden-core");
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"mint-lint-plants\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n[dependencies]\nforge-warden-core = {{ path = {:?} }}\n\n[workspace]\n",
            core.to_str().unwrap()
        ),
    )
    .unwrap();
    std::fs::write(dir.join("src/lib.rs"), PLANTS).unwrap();
    std::fs::copy(root().join("Cargo.lock"), dir.join("Cargo.lock")).unwrap();
    let out = Command::new(env!("CARGO"))
        .args([
            "clippy",
            "--offline",
            "--quiet",
            "--message-format=short",
            "--",
            "-A",
            "warnings",
            "-D",
            "clippy::disallowed_methods",
        ])
        .current_dir(&dir)
        // Hermetic: an inherited `--cap-lints` (cargo-mutants sets RUSTFLAGS=--cap-lints=allow)
        // silences every lint, which is the boundary this rule does not claim; the regression
        // measures clippy and clippy.toml, not the caller's flags.
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_BUILD_RUSTFLAGS")
        .env("CLIPPY_CONF_DIR", root())
        .env("CARGO_TARGET_DIR", dir.join("target"))
        .output()
        .unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("disallowed method"),
        "clippy did not run or reported nothing (is clippy installed?): {stderr}"
    );
    let reported: Vec<usize> = stderr
        .lines()
        .filter(|l| l.contains("disallowed method"))
        .filter_map(|l| l.strip_prefix("src/lib.rs:"))
        .filter_map(|l| l.split(':').next()?.parse().ok())
        .collect();
    let mut forms = 0;
    for (index, line) in PLANTS.lines().enumerate() {
        let number = index.saturating_add(1);
        if let Some((_, form)) = line.split_once("// FORM: ") {
            forms += 1;
            assert!(
                reported.contains(&number),
                "clippy missed: {form} (line {number})\n{stderr}"
            );
        }
        if let Some((_, control)) = line.split_once("// CONTROL: ") {
            assert!(
                !reported.contains(&number),
                "control refused: {control}\n{stderr}"
            );
        }
    }
    assert_eq!(forms, 16);
    assert!(!out.status.success(), "planted forms must fail the build");
}
