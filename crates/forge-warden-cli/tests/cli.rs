//! The binary's contract: exit codes, streams, seals and the golden verdict.

mod common;
use common::{read, read_bytes, run};

const POLICY: &str = "examples/starter.warden.toml";
const RANK: &str = "fixtures/demo.rank.json";
const PATCH: &str = "fixtures/demo.patch";
const TODAY: &str = "2026-11-05";

#[test]
fn golden_verdict_for_the_demo_change() {
    let out = run(
        &[
            "check", "--policy", POLICY, "--rank", RANK, "--patch", PATCH, "--today", TODAY,
            "--json",
        ],
        b"",
    );
    let golden = read("fixtures/demo.verdict.json");
    assert_eq!(
        out.stdout, golden,
        "the verdict changed. If that is intended, run scripts/bless.sh and review the diff of fixtures/demo.verdict.json"
    );
    assert_eq!(
        out.code, 7,
        "the starter policy fails the demo on ci_privilege"
    );
    assert!(
        out.stderr.is_empty(),
        "a verdict is not an error: {}",
        out.stderr
    );
}

#[test]
fn stdin_and_file_give_identical_bytes() {
    let rank = read_bytes(RANK);
    let from_stdin = run(
        &["check", "--policy", POLICY, "--today", TODAY, "--json"],
        &rank,
    );
    let from_dash = run(
        &[
            "check", "--policy", POLICY, "--rank", "-", "--today", TODAY, "--json",
        ],
        &rank,
    );
    let from_file = run(
        &[
            "check", "--policy", POLICY, "--rank", RANK, "--today", TODAY, "--json",
        ],
        b"",
    );
    assert_eq!(from_stdin.stdout, from_file.stdout);
    assert_eq!(from_dash.stdout, from_file.stdout);
}

#[test]
fn a_patch_that_is_not_the_ranked_one_is_refused() {
    let out = run(
        &[
            "check",
            "--policy",
            POLICY,
            "--rank",
            RANK,
            "--patch",
            "README.md",
            "--today",
            TODAY,
        ],
        b"",
    );
    assert_eq!(out.code, 4);
    assert!(out.stdout.is_empty(), "on error stdout is empty");
    assert!(
        out.stderr.contains("re-rank the patch you mean to judge"),
        "{}",
        out.stderr
    );
    assert!(out.stderr.contains("next: change_input"));
}

#[test]
fn an_invalid_policy_lists_every_issue() {
    let out = run(&["lint", "fixtures/bad.warden.toml"], b"");
    assert_eq!(out.code, 4);
    assert_eq!(out.stdout, "", "stdout must be empty on error");
    for needle in [
        "unknown signal \"secrets_like\" (did you mean \"secret_like\"?)",
        "kebab-case",
        "`why`",
        "does not declare",
    ] {
        assert!(
            out.stderr.contains(needle),
            "missing {needle:?} in:\n{}",
            out.stderr
        );
    }
}

#[test]
fn a_ranking_from_another_tool_is_refused() {
    let out = run(
        &["check", "--policy", POLICY, "--today", TODAY],
        br#"{"schema":"something.else","ranked":[]}"#,
    );
    assert_eq!(out.code, 4);
    assert_eq!(out.stdout, "", "stdout must be empty on error");
}

#[test]
fn over_budget_input_is_invalid_input_not_an_io_error() {
    // One byte over the budget is the caller's input problem (exit 4, change_input), not the
    // environment's (exit 3, check_environment): the action differs, so the code must too.
    let over = vec![b' '; forge_warden_core::MAX_INPUT_BYTES.saturating_add(1)];
    let out = run(&["check", "--policy", POLICY, "--today", TODAY], &over);
    assert_eq!(out.code, 4, "{}", out.stderr);
    assert!(out.stderr.contains("byte budget"), "{}", out.stderr);
    assert!(out.stderr.contains("next: change_input"), "{}", out.stderr);
    assert_eq!(out.stdout, "");
}

#[test]
fn usage_errors_exit_2_and_io_errors_exit_3() {
    assert_eq!(run(&["check"], b"").code, 2);
    assert_eq!(
        run(&["check", "--policy", POLICY, "--today", "tomorrow"], b"").code,
        2
    );
    let missing = run(
        &["check", "--policy", "no/such/file.toml", "--today", TODAY],
        b"",
    );
    assert_eq!(missing.code, 3);
    assert!(missing.stderr.contains("next: check_environment"));
}

#[test]
fn help_and_version_go_to_stdout_with_exit_0() {
    let help = run(&["--help"], b"");
    assert_eq!(help.code, 0);
    assert!(help.stdout.contains("forge-warden"));
    assert_eq!(run(&["--version"], b"").code, 0);
}

#[test]
fn explain_accepts_codes_and_names() {
    let by_code: serde_json::Value =
        serde_json::from_str(&run(&["explain", "8"], b"").stdout).unwrap();
    let by_name: serde_json::Value =
        serde_json::from_str(&run(&["explain", "policy_hold"], b"").stdout).unwrap();
    assert_eq!(by_code, by_name);
    assert_eq!(by_code["action"], "review_evidence");
    assert_eq!(run(&["explain", "42"], b"").code, 2);
}

#[test]
fn init_prints_a_policy_that_lints_clean() {
    let out = run(&["init"], b"");
    assert_eq!(out.code, 0);
    assert_eq!(out.stdout, read(POLICY));
    assert_eq!(run(&["lint", POLICY], b"").code, 0);
}

#[test]
fn hostile_paths_cannot_reach_the_terminal_raw() {
    let rank = read("fixtures/hostile.rank.json");
    let out = run(
        &[
            "check",
            "--policy",
            "fixtures/protect-all.warden.toml",
            "--today",
            TODAY,
        ],
        rank.as_bytes(),
    );
    assert_eq!(out.code, 8);
    assert!(
        out.stdout
            .bytes()
            .all(|b| b == b'\n' || (0x20..=0x7e).contains(&b)),
        "{:?}",
        out.stdout
    );
    assert!(
        out.stdout.contains("\\u{001b}"),
        "escape sequences are shown, not run"
    );
    assert!(
        out.stdout.contains("\\u{202e}"),
        "bidi controls are shown, not applied"
    );
}
