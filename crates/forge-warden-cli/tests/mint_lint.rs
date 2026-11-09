//! Bastion B-C1c-01: only `doors.rs` may mint `Ranking<Produced>`. Since Q-TYPES-1, `attest` needs an
//! `AdmittedProducer` that only the policy's allowlist makes, and raw `evaluate` is not nameable
//! outside the core (`tests/api_boundary.rs` proves both for any downstream crate). Which file of the
//! binary may mint is still the binary's rule: `clippy.toml` makes `attest` a disallowed method,
//! clippy resolves the *path*, so every way of naming it is caught. This test runs the real clippy,
//! with this repository's `clippy.toml`, on a scratch crate that names `attest` in every form
//! measured for C1c v2, and requires each form to be refused and the two controls (the door's
//! `#[expect]`, and an allowed function) not to be.
//!
//! G-04 (WP-B7, B7.1): the same real clippy, run once with each crate's own `clippy.toml`, refuses
//! every way of choosing a raw default (`unwrap_or*`, `map_or*`, `get_or_insert*`), and admits the
//! named `NamedDefault` defaults and the boolean predicates (`is_some_and`, `is_none_or`, `is_ok_and`),
//! whose default is spelled by the method name.
#![expect(
    clippy::disallowed_methods,
    clippy::unwrap_used,
    reason = "test door: writes a scratch crate and runs cargo clippy on it; a panic is a failed test"
)]

mod common;
#[path = "../../forge-warden-core/tests/support/corpus_manifest.rs"]
mod corpus_manifest;
use common::root;
use std::path::Path;

/// Each line carrying `// FORM:` must be reported as a disallowed method; `// CONTROL:` lines must not.
const PLANTS: &str = r#"
use forge_warden_core::Policy;
use forge_warden_core::authority::{Produced, Ranking};
use forge_warden_core::authority::Ranking as R;
use forge_warden_core::policy::AdmittedProducer;
use forge_warden_core::rank::RankDoc;

fn apply<A, B, T>(f: impl Fn(A, B) -> T, a: A, b: B) -> T {
    f(a, b)
}

/// A token, as the door gets one: from the policy's allowlist. (Raw `evaluate` and a bare-string
/// `attest` no longer compile downstream at all; tests/api_boundary.rs measures that, Q-TYPES-1.)
fn token(policy: &Policy) -> AdmittedProducer {
    policy.admit_producer(&forge_warden_core::seal::sha256_hex(b"0")).unwrap()
}

pub fn forms(doc: RankDoc, policy: &Policy) -> usize {
    let mut n = 0;
    let _a = Ranking::attest(doc.clone(), token(policy)); // FORM: attest direct
    let forge = Ranking::attest; // FORM: attest alias binding (B-C1c-01)
    let _b = forge(doc.clone(), token(policy));
    let _c = forge_warden_core::authority::Ranking::<Produced>::attest(doc.clone(), token(policy)); // FORM: attest qualified
    let _d = R::attest(doc.clone(), token(policy)); // FORM: attest via use-as
    let _e = apply(Ranking::attest, doc.clone(), token(policy)); // FORM: attest passed as a function
    let p: fn(RankDoc, AdmittedProducer) -> Ranking<Produced> = Ranking::attest; // FORM: attest as fn pointer
    let _f = <Ranking<Produced>>::attest(doc.clone(), token(policy)); // FORM: attest UFCS
    let _g = Some(doc.clone()).map(|d| Ranking::attest(d, token(policy))); // FORM: attest in a closure
    let _h = vec![Ranking::attest]; // FORM: attest inside an external macro
    let _i = std::hint::black_box(Ranking::<Produced>::attest); // FORM: attest through black_box
    let _y = p(doc.clone(), token(policy));
    n += door(doc.clone(), policy);
    n += forge_warden_core::seal::sha256_hex(b"").as_str().len(); // CONTROL: an allowed function
    n
}

#[expect(clippy::disallowed_methods, reason = "door: the positive control")]
fn door(doc: RankDoc, policy: &Policy) -> usize {
    Ranking::attest(doc, token(policy)).producer_sha256().map_or(0, |seal| seal.as_str().len()) // CONTROL: the door's expect
}
"#;

/// G-04: every way of choosing a raw default; each `// FORM:` line must be refused by both crates'
/// configurations, and the named defaults and a same-named method on another type must not be.
const DEFAULT_PLANTS: &str = r"
use forge_warden_core::Outcome;
use forge_warden_core::defaults::NamedDefault as _;
use std::option::Option as Maybe;

fn apply<T>(f: impl Fn(Option<T>, T) -> T, o: Option<T>, d: T) -> T {
    f(o, d)
}

struct Lookalike;
impl Lookalike {
    fn unwrap_or(self, d: u8) -> u8 {
        d
    }
}

pub fn forms(o: Option<u8>, r: Result<u8, String>, verdict: Option<Outcome>) -> usize {
    let mut n = 0_usize;
    let _a = verdict.unwrap_or(Outcome::Pass); // FORM: Option::unwrap_or (the G-04 case)
    let _b = o.unwrap_or_else(|| 0); // FORM: Option::unwrap_or_else
    let _c = o.unwrap_or_default(); // FORM: Option::unwrap_or_default
    let _d = r.clone().unwrap_or(0); // FORM: Result::unwrap_or
    let _e = r.clone().unwrap_or_else(|_| 0); // FORM: Result::unwrap_or_else
    let _f = r.clone().unwrap_or_default(); // FORM: Result::unwrap_or_default
    let _g = Option::unwrap_or(o, 0); // FORM: UFCS
    let _h = std::option::Option::<u8>::unwrap_or(o, 0); // FORM: qualified UFCS
    let _i = Maybe::unwrap_or(o, 0); // FORM: via use-as
    let pick = Option::<u8>::unwrap_or; // FORM: alias binding
    let _j = pick(o, 0);
    let _k = apply(Option::unwrap_or, o, 0); // FORM: passed as a function
    let p: fn(Option<u8>, u8) -> u8 = Option::unwrap_or; // FORM: fn pointer
    let _l = p(o, 0);
    let _m = Some(o).map(|x| x.unwrap_or(0)); // FORM: in a closure
    let _n = vec![o.unwrap_or(0)]; // FORM: inside an external macro
    let _q = <Result<u8, String>>::unwrap_or_default(r.clone()); // FORM: Result UFCS
    let _s = r.clone().ok().unwrap_or(0); // FORM: Result converted to Option
    n += usize::from(o.or_fail_closed(0)); // CONTROL: a named fail-closed default
    n += usize::from(r.clone().or_fail_safe(0)); // CONTROL: a named fail-safe default on Result
    n += o.map(usize::from).or_display(0); // CONTROL: a named display default
    n += usize::from(Lookalike.unwrap_or(0)); // CONTROL: the same name on another type
    n
}

/// WP-B7.1: the other value-default pickers. `map_or` / `map_or_else` substitute a value for a
/// missing one exactly like `unwrap_or`; `get_or_insert*` store one.
pub fn pickers(
    o: Option<u8>,
    r: Result<u8, String>,
    verdict: Option<Outcome>,
    mut slot: Option<u8>,
) -> usize {
    let mut n = 0_usize;
    let _a = verdict.map_or(Outcome::Pass, |v| v); // FORM: Option::map_or (the G-04 case again)
    let _b = o.map_or_else(|| 0, |x| x); // FORM: Option::map_or_else
    let _c = r.clone().map_or(0, |x| x); // FORM: Result::map_or
    let _d = r.clone().map_or_else(|_| 0, |x| x); // FORM: Result::map_or_else
    let _e = Option::map_or(o, 0, |x| x); // FORM: map_or UFCS
    let _f = std::option::Option::<u8>::map_or_else(o, || 0, |x| x); // FORM: map_or_else qualified UFCS
    let choose = Option::<u8>::map_or::<u8, fn(u8) -> u8>; // FORM: map_or alias binding
    let _g = choose(o, 0, |x| x);
    let _h = Some(o).map(|x| x.map_or(0, |y| y)); // FORM: map_or in a closure
    let _i = *slot.get_or_insert(0); // FORM: Option::get_or_insert
    let _j = *slot.get_or_insert_with(|| 0); // FORM: Option::get_or_insert_with
    let _k = *slot.get_or_insert_default(); // FORM: Option::get_or_insert_default
    n += o.map(usize::from).or_fail_safe(0); // CONTROL: map, then a named default
    n += usize::from(o.is_some_and(|x| x > 1)); // CONTROL: is_some_and boolean predicate (default named by the method)
    n += usize::from(o.is_none_or(|x| x > 1)); // CONTROL: is_none_or boolean predicate (default named by the method)
    n += usize::from(r.is_ok_and(|x| x > 1)); // CONTROL: a boolean predicate on Result
    n
}
";

#[test]
fn clippy_refuses_every_way_of_naming_attest_or_raw_evaluate_outside_the_door() {
    check_plants("mint-lint", PLANTS, &root(), "mint-plants", &[]);
}

#[test]
fn clippy_refuses_every_raw_default_under_the_cli_configuration() {
    check_plants(
        "default-lint-cli",
        DEFAULT_PLANTS,
        &root(),
        "default-plants",
        &[],
    );
}

#[test]
fn clippy_refuses_every_raw_default_under_the_core_configuration() {
    let core = root().join("crates/forge-warden-core");
    check_plants(
        "default-lint-core",
        DEFAULT_PLANTS,
        &core,
        "default-plants",
        &[],
    );
}

#[test]
fn an_outer_cap_lints_cannot_silence_the_regression() {
    // Touchstone C1c-O3-03: cargo-mutants runs tests with --cap-lints allow (in RUSTFLAGS or
    // CARGO_ENCODED_RUSTFLAGS); inherited, it silenced the nested clippy and turned the mutation
    // baseline red. The same hostile environment is applied here before the nested command clears it.
    check_plants(
        "mint-lint-capped",
        PLANTS,
        &root(),
        "mint-plants",
        &[
            ("RUSTFLAGS", "--cap-lints=allow"),
            ("CARGO_ENCODED_RUSTFLAGS", "--cap-lints\u{1f}allow"),
            ("CARGO_BUILD_RUSTFLAGS", "--cap-lints=allow"),
        ],
    );
}

/// Run clippy, configured by the `clippy.toml` in `config`, on `plants` in a scratch crate under
/// `name`, with `hostile` environment variables set first (the nested command must clear them), and
/// check that all manifest forms are refused and no control is.
fn check_plants(name: &str, plants: &str, config: &Path, group: &str, hostile: &[(&str, &str)]) {
    corpus_manifest::check(
        group,
        plants.lines().filter_map(|line| {
            line.split_once("// FORM: ")
                .map(|(_, identity)| (format!("form:{identity}"), line.as_bytes().to_vec()))
                .or_else(|| {
                    line.split_once("// CONTROL: ").map(|(_, identity)| {
                        (format!("control:{identity}"), line.as_bytes().to_vec())
                    })
                })
        }),
    )
    .unwrap();
    common::require_locked_deps();
    let dir = common::scratch(name);
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
    std::fs::write(dir.join("src/lib.rs"), plants).unwrap();
    std::fs::copy(root().join("Cargo.lock"), dir.join("Cargo.lock")).unwrap();
    let out = common::command(env!("CARGO"))
        .args([
            "clippy",
            "--offline",
            "--quiet",
            "--message-format=short",
        ])
        .arg("--target-dir")
        .arg(dir.join("target"))
        .args([
            "--",
            "-A",
            "warnings",
            "-D",
            "clippy::disallowed_methods",
        ])
        .current_dir(&dir)
        .envs(hostile.iter().copied())
        // Hermetic: an inherited `--cap-lints` (cargo-mutants sets RUSTFLAGS=--cap-lints=allow)
        // silences every lint, which is the boundary this rule does not claim; the regression
        // measures clippy and clippy.toml, not the caller's flags.
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_BUILD_RUSTFLAGS")
        .env("CLIPPY_CONF_DIR", config)
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
    let mut expected_lines = std::collections::BTreeSet::new();
    for (index, line) in plants.lines().enumerate() {
        let number = index.saturating_add(1);
        if let Some((_, form)) = line.split_once("// FORM: ") {
            expected_lines.insert(number);
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
    assert_eq!(
        reported
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>(),
        expected_lines,
        "unexpected lint diagnostic locations"
    );
    assert!(!out.status.success(), "planted forms must fail the build");
}
