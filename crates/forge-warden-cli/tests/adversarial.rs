//! CLI trust-boundary controls; R-01 and R-05 (Mission 3) are closed by C1c (ADR 0005).
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: failure to decode the fixture verdict fails the test"
)]

mod common;
use common::{Run, read_bytes, run};
use forge_warden_core::seal::sha256_hex;
use serde_json::{Value, json};

const PATCH: &str = "fixtures/adversarial/change.patch";
const POLICY: &str = "fixtures/adversarial/no-secrets.toml";
const PROTECT: &str = "fixtures/adversarial/protect-source.toml";

fn rank(path: &str, status: &str, signals: &[&str]) -> Value {
    json!({"schema":"deep-diff-forge.rank.v0",
        "input_sha256":sha256_hex(&read_bytes(PATCH)),
        "tool":{"name":"deep-diff-forge","version":"0.7.0"},
        "ranked":[{"path":path,"status":status,"score":1,"signals":signals}]})
}

fn check(policy: &str, input: &Value, patch: Option<&str>) -> Run {
    let mut args = vec![
        "check",
        "--policy",
        policy,
        "--advisory",
        "--rank",
        "-",
        "--today",
        "2026-11-06",
        "--json",
    ];
    if let Some(path) = patch {
        args.extend(["--patch", path]);
    }
    run(&args, &serde_json::to_vec(input).unwrap())
}

#[test]
fn matching_patch_and_known_ranking_hold_in_advisory_mode() {
    // C1c (ADR 0005 §2.1): a supplied ranking is advisory, so its best outcome is HOLD with only
    // `rank_unverified`; the authority-mode PASS control is in authority_mode.rs.
    let out = check(POLICY, &rank("src/lib.rs", "modified", &[]), Some(PATCH));
    assert_eq!(out.code, 8, "{}", out.stderr);
    let v: Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(v["verdict"], "hold");
    assert_eq!(v["notes"][0]["code"], "rank_unverified");
    assert_eq!(v["notes"].as_array().unwrap().len(), 1);
    assert_eq!(v["seals"]["provenance"], "supplied");
    assert_eq!(v["seals"]["patch_sha256"], sha256_hex(&read_bytes(PATCH)));
}

#[test]
fn protection_and_forbidden_signal_have_fail_controls() {
    for (policy, input) in [
        (PROTECT, rank("src/lib.rs", "modified", &[])),
        (POLICY, rank("src/lib.rs", "modified", &["secret_like"])),
    ] {
        let out = check(policy, &input, Some(PATCH));
        assert_eq!(out.code, 7, "{}", out.stderr);
        let v: Value = serde_json::from_str(&out.stdout).unwrap();
        assert_eq!(v["verdict"], "fail");
    }
}

#[test]
fn a_hash_mismatch_refuses_without_emitting_a_verdict() {
    let mut input = rank("src/lib.rs", "modified", &[]);
    input["input_sha256"] = json!("a".repeat(64));
    let out = check(POLICY, &input, Some(PATCH));
    assert_eq!(out.code, 4);
    assert_eq!(out.stdout, "");
    assert!(out.stderr.contains("hashes to"), "{}", out.stderr);
}

#[test]
fn unknown_status_and_signal_hold_at_the_process_boundary() {
    for input in [
        rank("src/lib.rs", "unknown", &[]),
        rank("src/lib.rs", "modified", &["future_signal"]),
    ] {
        let out = check(POLICY, &input, Some(PATCH));
        assert_eq!(out.code, 8, "{}", out.stderr);
        let v: Value = serde_json::from_str(&out.stdout).unwrap();
        assert_eq!(v["verdict"], "hold");
    }
}

#[test]
fn malformed_stdin_refuses_with_empty_stdout() {
    for bytes in [b"".as_slice(), b"{", b"[]", b"null", b"\xff"] {
        let out = run(
            &[
                "check",
                "--policy",
                POLICY,
                "--patch",
                PATCH,
                "--advisory",
                "--rank",
                "-",
                "--today",
                "2026-11-06",
                "--json",
            ],
            bytes,
        );
        assert_eq!(out.code, 4, "{}", out.stderr);
        assert_eq!(out.stdout, "");
    }
}

#[test]
fn a_forged_file_list_with_a_real_patch_hash_cannot_pass() {
    let control = check(PROTECT, &rank("src/lib.rs", "modified", &[]), Some(PATCH));
    assert_eq!(control.code, 7);
    let forged = check(PROTECT, &rank("README.md", "modified", &[]), Some(PATCH));
    assert!(
        matches!(forged.code, 4 | 8),
        "forged coverage returned {}: {}",
        forged.code,
        forged.stdout
    );
}

#[test]
fn omitting_the_patch_requires_review_or_refusal() {
    let out = check(POLICY, &rank("src/lib.rs", "modified", &[]), None);
    assert!(
        matches!(out.code, 2 | 4 | 8),
        "unverified binding returned {}: {}",
        out.code,
        out.stdout
    );
}

#[test]
fn dot_prefixed_paths_are_refused_at_the_cli_boundary() {
    let out = check(PROTECT, &rank("./src/lib.rs", "modified", &[]), Some(PATCH));
    assert_eq!(out.code, 4, "{}", out.stdout);
    assert_eq!(out.stdout, "");
}
