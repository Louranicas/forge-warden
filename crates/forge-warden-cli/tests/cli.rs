//! The binary's contract: exit codes, streams, seals and the golden verdict.
#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "test helper: normalizing the golden verdict; a panic is a failed test"
)]

mod common;
use common::{
    fixture_ranker, fixture_ranker_sha256, policy_admitting, read, read_bytes, run, scratch, text,
    write,
};
use serde_json::Value;
use std::fmt::Write as _;

const POLICY: &str = "examples/starter.warden.toml";
const RANK: &str = "fixtures/demo.rank.json";
const PATCH: &str = "fixtures/demo.patch";
const TODAY: &str = "2026-11-05";

/// `check` in advisory mode on `rank` (`-` reads stdin).
fn advisory(policy: &str, patch: &str, rank: &str, stdin: &[u8]) -> common::Run {
    run(
        &[
            "check",
            "--policy",
            policy,
            "--patch",
            patch,
            "--advisory",
            "--rank",
            rank,
            "--today",
            TODAY,
            "--json",
        ],
        stdin,
    )
}

/// The golden form of a verdict: the Git version, which depends on the machine, is a placeholder.
/// Every input seal stays real, so it can be checked with `sha256sum` (B5's seal anchors).
fn golden_form(stdout: &str) -> String {
    let mut v: Value = serde_json::from_str(stdout).unwrap();
    assert_eq!(
        v["seals"]["git"]["sha256"],
        forge_warden_core::seal::sha256_hex(&read_bytes("/usr/bin/git")).as_str()
    );
    v["seals"]["git"]["sha256"] = "0".repeat(64).into();
    v["seals"]["git"]["version"] = "<git --version>".into();
    let mut out = serde_json::to_string_pretty(&v).unwrap();
    out.push('\n');
    out
}

#[test]
fn golden_verdict_for_the_demo_change() {
    // Advisory mode on deep-diff-forge 0.7.0's recorded ranking of the demo patch.
    let out = advisory(POLICY, PATCH, RANK, b"");
    assert!(
        out.stderr.is_empty(),
        "a verdict is not an error: {}",
        out.stderr
    );
    common::assert_golden("fixtures/demo.verdict.json", &golden_form(&out.stdout));
    assert_eq!(
        out.code, 7,
        "the starter policy fails the demo on ci_privilege"
    );
}

#[test]
fn authority_mode_on_the_demo_reaches_the_golden_judgement() {
    // The fixture ranker replays the same recorded ranking, so authority mode must judge the demo
    // exactly as the golden does; only who produced the ranking (and the policy that admits the
    // producer) differ, and nothing is left unverified.
    let dir = scratch("golden-authority");
    let policy = policy_admitting(&dir, POLICY, &[fixture_ranker_sha256()]);
    let ranker = fixture_ranker();
    let out = run(
        &[
            "check",
            "--policy",
            text(&policy),
            "--patch",
            PATCH,
            "--ranker",
            text(&ranker),
            "--today",
            TODAY,
            "--json",
        ],
        b"",
    );
    assert_eq!(out.code, 7, "{}", out.stderr);
    let mut produced: Value = serde_json::from_str(&golden_form(&out.stdout)).unwrap();
    let golden: Value =
        serde_json::from_str(&common::read_golden("fixtures/demo.verdict.json")).unwrap();
    assert_eq!(produced["seals"]["provenance"], "produced");
    assert_eq!(
        produced["seals"]["ranked_by"]["sha256"],
        fixture_ranker_sha256()
    );
    assert_eq!(produced["notes"], serde_json::json!([]));
    assert_eq!(
        golden["notes"],
        serde_json::json!([{"code": "rank_unverified", "detail": "the ranking was supplied, not produced by forge-warden; it can inform, never approve; Git is unpinned: no owner allowlist enrolled; its recorded executable digest is not approved"}])
    );
    produced["seals"]["policy_sha256"] = golden["seals"]["policy_sha256"].clone();
    assert_eq!(produced["seals"]["git"]["admission"], "approved");
    let digest = forge_warden_core::seal::sha256_hex(&read_bytes("/usr/bin/git"));
    let git_policy = format!(
        "schema = \"forge-warden.git-allowlist.v1\"\nrevision = 1\nsha256 = [\"{digest}\"]\n"
    );
    assert_eq!(
        produced["seals"]["git"]["allowlist_sha256"],
        forge_warden_core::seal::sha256_hex(git_policy.as_bytes()).as_str()
    );
    produced["seals"]["git"]["admission"] = golden["seals"]["git"]["admission"].clone();
    produced["seals"]["git"]["allowlist_sha256"] = Value::Null;
    produced["seals"]["provenance"] = "supplied".into();
    produced["seals"]["ranked_by"]["sha256"] = Value::Null;
    produced["notes"] = golden["notes"].clone();
    common::assert_golden_json("fixtures/demo.verdict.json", &produced);
    assert_eq!(
        golden["seals"]["rank_sha256"],
        forge_warden_core::seal::sha256_hex(&read_bytes(RANK)).as_str(),
        "the produced ranking is the recorded one"
    );
}

#[test]
fn stdin_and_file_give_identical_bytes() {
    let rank = read_bytes(RANK);
    let from_dash = advisory(POLICY, PATCH, "-", &rank);
    let from_file = advisory(POLICY, PATCH, RANK, b"");
    assert_eq!(from_file.code, 7, "{}", from_file.stderr);
    assert_eq!(from_dash.stdout, from_file.stdout);
}

#[test]
fn a_patch_that_is_not_the_ranked_one_is_refused() {
    let out = advisory(POLICY, "README.md", RANK, b"");
    assert_eq!(out.code, 4);
    assert!(out.stdout.is_empty(), "on error stdout is empty");
    assert!(
        out.stderr.contains("re-rank the patch you mean to judge"),
        "{}",
        out.stderr
    );
    assert_eq!(
        serde_json::from_str::<Value>(&out.stderr).unwrap()["next"],
        "change_input"
    );
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
    let out = advisory(
        POLICY,
        PATCH,
        "-",
        br#"{"schema":"something.else","ranked":[]}"#,
    );
    assert_eq!(out.code, 4);
    assert_eq!(out.stdout, "", "stdout must be empty on error");
}

#[test]
fn a_policy_over_its_own_budget_is_refused_by_lint_and_by_check() {
    // POLICY-CAP (ledger Q44): the policy has its own 1 MiB budget, far under the ranking's. A
    // valid policy padded past it is refused for its size (exit 4) by both commands.
    let dir = scratch("policy-cap");
    let mut bytes = read_bytes(POLICY);
    bytes.resize(forge_warden_core::MAX_POLICY_BYTES, b'\n');
    let at = write(&dir, "at.warden.toml", &bytes);
    assert_eq!(
        run(&["lint", text(&at)], b"").code,
        0,
        "at the budget it is read"
    );
    bytes.push(b'\n');
    let over = write(&dir, "over.warden.toml", &bytes);
    for out in [
        run(&["lint", text(&over), "--json"], b""),
        advisory(text(&over), PATCH, RANK, b""),
    ] {
        assert_eq!(out.code, 4, "{}", out.stderr);
        assert!(
            out.stderr
                .contains("policy is 1048577 bytes, over the 1048576 byte budget"),
            "{}",
            out.stderr
        );
        assert_eq!(out.stdout, "");
    }
}

#[test]
fn over_budget_input_is_invalid_input_not_an_io_error() {
    // One byte over the budget is the caller's input problem (exit 4, change_input), not the
    // environment's (exit 3, check_environment): the action differs, so the code must too.
    let over = vec![b' '; forge_warden_core::MAX_INPUT_BYTES.saturating_add(1)];
    let out = advisory(POLICY, PATCH, "-", &over);
    assert_eq!(out.code, 4, "{}", out.stderr);
    assert!(out.stderr.contains("byte budget"), "{}", out.stderr);
    assert_eq!(
        serde_json::from_str::<Value>(&out.stderr).unwrap()["next"],
        "reduce_input_size"
    );
    assert_eq!(out.stdout, "");
}

#[test]
fn text_output_lists_every_note() {
    // A HOLD caused only by notes must say why in the human summary too, not just in --json.
    let rank = serde_json::json!({
        "schema": "deep-diff-forge.rank.v0",
        "input_sha256": "bbf580931c6739af491bb5e2258550631327ca07f1078135cb008797e22b836e",
        "tool": {"name": "deep-diff-forge", "version": "0.7.0"},
        "ranked": [
            {"path": "docs/a.md", "status": "unknown", "score": 0, "signals": []},
            {"path": "docs/b.md", "status": "modified", "score": 0, "signals": ["future_signal"]}
        ]
    });
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
            TODAY,
        ],
        &serde_json::to_vec(&rank).unwrap(),
    );
    assert_eq!(out.code, 8, "{}", out.stderr);
    for needle in [
        "[NOTE] unknown_status docs/a.md:",
        "[NOTE] unknown_signal docs/b.md:",
        "[NOTE] rank_unverified the ranking was supplied",
        "[NOTE] rank_incomplete src/lib.rs:",
        "[NOTE] rank_extraneous docs/a.md:",
        "(supplied, unverified)",
    ] {
        assert!(out.stdout.contains(needle), "{needle:?} in {}", out.stdout);
    }
}

#[test]
fn a_refused_today_says_why() {
    for (today, why) in [
        ("2026-02-30", "it is not a calendar date"),
        ("1969-12-31", "it is outside 1970-01-01..=9999-12-31"),
        ("tomorrow", "it is not YYYY-MM-DD"),
    ] {
        let out = run(
            &[
                "check",
                "--policy",
                POLICY,
                "--patch",
                PATCH,
                "--advisory",
                "--rank",
                RANK,
                "--today",
                today,
            ],
            b"",
        );
        assert_eq!(out.code, 2, "{today}: {}", out.stderr);
        assert!(
            out.stderr.contains(&format!(
                "--today {today:?} is not a YYYY-MM-DD date: {why}"
            )),
            "{today}: {}",
            out.stderr
        );
    }
}

/// The command 0.1.0 documented, run against this version: the first thing an upgrading user sees.
/// A snapshot, so a change to the wording or the layout is a reviewed change.
#[test]
fn a_usage_error_is_printed_on_the_argument_parsers_own_lines() {
    let out = run(&["check", "--policy", "p.toml", "--rank", "r.json"], b"");
    assert_eq!(out.code, 2);
    assert_eq!(out.stdout, "");
    assert_eq!(
        out.stderr,
        "error: the following required arguments were not provided:\n  --patch <FILE>\n  \
         <--ranker <FILE>|--advisory>\n\nUsage: forge-warden check --policy <FILE> --patch <FILE> \
         --rank <FILE> <--ranker <FILE>|--advisory>\n\nFor more information, try '--help'.\n"
    );
}

/// Argument text can come from a pull request. Its control characters are escaped inside the
/// lines, and an argument that carries a line feed keeps the whole message on one line, so it
/// can never start a line of its own.
#[test]
fn hostile_argument_text_in_a_usage_error_is_neutralised() {
    let escape = run(&["check", "--policy", "p", "--frob\u{1b}[31m\u{202e}nicate"], b"");
    assert_eq!(escape.code, 2);
    assert!(
        escape.stderr.contains("\\u{001b}[31m\\u{202e}nicate"),
        "{}",
        escape.stderr
    );
    assert!(
        escape.stderr.chars().all(|c| c == '\n' || (' '..='~').contains(&c)),
        "{:?}",
        escape.stderr
    );
    assert!(
        escape.stderr.lines().any(|line| line.starts_with("Usage: ")),
        "the message keeps its lines: {}",
        escape.stderr
    );

    let forged = "forge-warden 0.2.0: PASS (exit 0 pass, next: none)";
    let as_flag = format!("--frob\n{forged}");
    let as_value = format!("p\n{forged}");
    let cases: [&[&str]; 3] = [
        &["check", "--policy", "p", &as_flag],
        &["check", "--policy", &as_value, "--bogus"],
        &["check", "--bogus", "--", &as_value],
    ];
    for args in cases {
        let out = run(args, b"");
        assert_eq!(out.code, 2, "{args:?}");
        assert_eq!(out.stderr.matches('\n').count(), 1, "{args:?}: {:?}", out.stderr);
        assert!(out.stderr.ends_with('\n'), "{args:?}");
        assert!(
            !out.stderr.lines().any(|line| line.starts_with("forge-warden ")),
            "{args:?}: {:?}",
            out.stderr
        );
    }
    let quoted = run(&["check", "--policy", "p", &as_flag], b"");
    assert!(
        quoted.stderr.contains("\\u{000a}forge-warden 0.2.0: PASS"),
        "{:?}",
        quoted.stderr
    );
}

#[test]
fn usage_errors_exit_2_and_io_errors_exit_3() {
    let ranker = fixture_ranker();
    let ranker = text(&ranker);
    let base = ["check", "--policy", POLICY, "--patch", PATCH];
    let usage = |extra: &[&str]| {
        let args: Vec<&str> = base.iter().chain(extra).copied().collect();
        let out = run(&args, b"");
        assert_eq!(out.code, 2, "{args:?}: {}", out.stderr);
        assert_eq!(out.stdout, "");
    };
    usage(&[]); // no mode
    usage(&["--advisory"]); // advisory without a ranking
    usage(&["--ranker", ranker, "--advisory", "--rank", RANK]); // both modes
    usage(&["--advisory", "--rank", RANK, "--today", "tomorrow"]);
    usage(&["--advisory", "--rank", RANK, "--git", "git"]);
    usage(&["--advisory", "--rank", RANK, "--base", "HEAD~1"]);
    usage(&["--advisory", "--rank", RANK, "--head", "abc"]);
    usage(&["--advisory", "--rank", RANK, "--head", &"a".repeat(65)]);
    assert_eq!(run(&["check"], b"").code, 2);
    assert_eq!(
        run(
            &["check", "--policy", POLICY, "--advisory", "--rank", RANK],
            b""
        )
        .code,
        2,
        "--patch is required (R-05)"
    );

    let missing = run(
        &[
            "check",
            "--policy",
            "no/such/file.toml",
            "--patch",
            PATCH,
            "--advisory",
            "--rank",
            RANK,
            "--today",
            TODAY,
        ],
        b"",
    );
    assert_eq!(missing.code, 3);
    assert!(missing.stderr.contains("next: check_input_output"));
    let no_git = run(
        &[
            "check",
            "--policy",
            POLICY,
            "--patch",
            PATCH,
            "--advisory",
            "--rank",
            RANK,
            "--git",
            "/no/such/git",
            "--today",
            TODAY,
        ],
        b"",
    );
    assert_eq!(no_git.code, 3, "{}", no_git.stderr);
    assert_eq!(no_git.stdout, "");
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
    let by_code: Value = serde_json::from_str(&run(&["explain", "8"], b"").stdout).unwrap();
    let by_name: Value =
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
    // The hostile ranking is sealed to the empty patch, which Git refuses: the rules still run.
    let dir = scratch("hostile");
    let empty = write(&dir, "empty.patch", b"");
    let rank = read("fixtures/hostile.rank.json");
    let out = run(
        &[
            "check",
            "--policy",
            "fixtures/protect-all.warden.toml",
            "--patch",
            text(&empty),
            "--advisory",
            "--rank",
            "-",
            "--today",
            TODAY,
        ],
        rank.as_bytes(),
    );
    assert_eq!(out.code, 8, "{}", out.stderr);
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
    assert!(
        out.stdout
            .contains("[NOTE] patch_unverifiable git refused the patch"),
        "{}",
        out.stdout
    );
}

// Q2-06: generated diagnostics traverse actual CLI failure paths, not only an escaper unit.
proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config { cases: 16, max_shrink_iters: 0, failure_persistence: None, ..proptest::test_runner::Config::default() })]
    #[test]
    fn diagnostics_are_printable_across_failure_paths(bytes in proptest::collection::vec(proptest::num::u8::ANY, 1..64)) {
        let dir = scratch("diagnostic-property");
        let mut hostile = b"context\x1b\0\n\xe2\x80\xae".to_vec();
        hostile.extend_from_slice(&bytes);
        let arg = String::from_utf8_lossy(&hostile).replace('\0', "_");
        let missing = dir.join(arg.replace('/', "_"));
        let ranker = fixture_ranker();
        let policy = policy_admitting(&dir, POLICY, &[fixture_ranker_sha256()]);
        let hex = hostile.iter().fold(String::new(), |mut out, b| { write!(out, "{b:02x}").unwrap(); out });
        let mut outcomes = vec![
            run(&["lint", "--policy", text(&missing)], b""),
            run(&["explain", &arg], b""),
            run(&["check", "--policy", POLICY, "--patch", PATCH, "--advisory", "--rank", RANK, "--git", text(&ranker)], b""),
        ];
        for mode in ["stderr-hex", "schema-hex"] {
            let patch = write(&dir, mode, format!("+fixture-ranker: {mode} {hex}\n").as_bytes());
            let out = run(&["check", "--policy", text(&policy), "--patch", text(&patch), "--ranker", text(&ranker), "--today", TODAY], b"");
            proptest::prop_assert!(out.stderr.contains("context"), "failure context was lost: {:?}", out.stderr);
            outcomes.push(out);
        }
        for out in outcomes {
            proptest::prop_assert!(out.code != 0 && !out.stderr.is_empty());
            proptest::prop_assert!(out.stderr.bytes().all(|b| b == b'\n' || (0x20..=0x7e).contains(&b)), "{:?}", out.stderr);
        }
    }
}

#[test]
fn machine_contract_lint_json_is_a_versioned_document() {
    let out = run(&["lint", POLICY, "--json"], b"");
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stderr, "");
    let value: Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(value["schema"], "forge-warden.lint.v1");
    assert_eq!(
        value["policy_sha256"],
        forge_warden_core::seal::sha256_hex(&read_bytes(POLICY)).as_str()
    );
    assert!(value["rules"].as_u64().unwrap() > 0);
    assert!(value["waivers"].is_u64());
}

#[test]
fn machine_contract_errors_are_one_json_line_including_clap_failures() {
    for args in [
        vec!["lint", "missing-\"quoted\"-policy", "--json"],
        vec!["lint", "--json"],
        vec!["check", "--json", "--unknown"],
        vec!["explain", "unknown-code", "--json"],
        vec!["schema", "unknown-schema", "--json"],
    ] {
        let out = run(&args, b"");
        assert!(out.code != 0, "{args:?}");
        assert!(out.stdout.is_empty(), "{args:?}");
        assert_eq!(out.stderr.lines().count(), 1, "{args:?}: {}", out.stderr);
        let error: Value = serde_json::from_str(&out.stderr).unwrap();
        assert_eq!(error["exit"], out.code);
        assert!(error["name"].is_string());
        assert_ne!(error["message"].as_str().unwrap(), "");
        assert!(error["next"].is_string());
    }
}

#[test]
fn machine_contract_patch_stdin_is_explicitly_refused_before_io() {
    let out = run(
        &[
            "check",
            "--policy",
            "missing-policy",
            "--patch",
            "-",
            "--advisory",
            "--rank",
            "-",
            "--json",
        ],
        b"unused",
    );
    assert_eq!(out.code, 4, "{}", out.stderr);
    assert_eq!(out.stdout, "");
    let error: Value = serde_json::from_str(&out.stderr).unwrap();
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("patch_stdin_unsupported")
    );
    let help = run(&["check", "--help"], b"");
    assert!(help.stdout.contains("--patch -"));
    assert!(help.stdout.contains("--rank -"));
}

#[test]
fn machine_contract_every_subcommand_has_json_or_an_explicit_exception() {
    // Discover the public command list from clap's actual help so a new command cannot disappear.
    let help = run(&["--help"], b"");
    let commands = help
        .stdout
        .split_once("Commands:\n")
        .unwrap()
        .1
        .split_once("\nOptions:")
        .unwrap()
        .0;
    let mut seen = std::collections::BTreeSet::new();
    for line in commands.lines().filter(|line| !line.trim().is_empty()) {
        let name = line.split_whitespace().next().unwrap();
        seen.insert(name);
        match name {
            "init" => {
                // Init is the explicit TOML producer; --json is refused rather than silently ignored.
                assert_eq!(run(&[name, "--json"], b"").code, 2);
            }
            "help" => {} // clap's documentation command is human text, as are --help/--version.
            other => assert!(
                run(&[other, "--help"], b"").stdout.contains("--json"),
                "{other} lacks a machine-output contract"
            ),
        }
    }
    assert!(seen.contains("lint"));
    assert!(seen.contains("check"));
}

#[test]
fn machine_contract_io_resources_are_not_environment_retries() {
    let out = common::run_with_full_stdout(&["lint", POLICY, "--json"]);
    assert_eq!(out.code, 3);
    let value: Value = serde_json::from_str(&out.stderr).unwrap();
    assert_eq!(value["name"], "io");
    assert_eq!(value["next"], "check_input_output");
    assert!(
        value["message"]
            .as_str()
            .unwrap()
            .contains("No space left on device")
    );
}

#[test]
fn machine_contract_git_setup_preserves_environment_vs_io_kind() {
    for (base, expected) in [
        ("relative", "check_environment"),
        (
            "/forge-warden-missing-cli-contract-base",
            "check_input_output",
        ),
    ] {
        let out = common::run_with_env(
            &[
                "check",
                "--policy",
                POLICY,
                "--patch",
                PATCH,
                "--advisory",
                "--rank",
                RANK,
                "--today",
                TODAY,
                "--json",
            ],
            b"",
            &[("TMPDIR", base)],
        );
        assert_eq!(out.code, 3, "{}", out.stderr);
        let value: Value = serde_json::from_str(&out.stderr).unwrap();
        assert_eq!(value["next"], expected, "{}", out.stderr);
    }
}

#[test]
fn machine_contract_option_terminator_preserves_literal_json_filename() {
    let out = run(&["lint", "--", "--json"], b"");
    assert_eq!(out.code, 3);
    assert!(out.stderr.starts_with("forge-warden: reading --json:"));
    assert!(serde_json::from_str::<Value>(&out.stderr).is_err());
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config { cases: 16, max_shrink_iters: 0, failure_persistence: None, ..proptest::test_runner::Config::default() })]
    #[test]
    fn machine_contract_json_errors_are_valid_and_ascii(bytes in proptest::collection::vec(proptest::num::u8::ANY, 0..64)) {
        let dir = scratch("json-errors");
        let mut hostile = b"context\x1b\0\n\xe2\x80\xae\"\\".to_vec();
        hostile.extend_from_slice(&bytes);
        let ranker = fixture_ranker();
        let policy = policy_admitting(&dir, POLICY, &[fixture_ranker_sha256()]);
        let hex = hostile.iter().fold(String::new(), |mut out, b| { write!(out, "{b:02x}").unwrap(); out });
        for mode in ["stderr-hex", "schema-hex"] {
            let patch = write(&dir, mode, format!("+fixture-ranker: {mode} {hex}\n").as_bytes());
            let out = run(&["check", "--policy", text(&policy), "--patch", text(&patch), "--ranker", text(&ranker), "--today", TODAY, "--json"], b"");
            proptest::prop_assert_ne!(out.code, 0);
            proptest::prop_assert_eq!(out.stdout, "");
            proptest::prop_assert_eq!(out.stderr.lines().count(), 1);
            let value: Value = serde_json::from_str(&out.stderr).unwrap();
            proptest::prop_assert!(value["message"].as_str().unwrap().contains("context"));
            proptest::prop_assert!(out.stderr.bytes().all(|b| b == b'\n' || (0x20..=0x7e).contains(&b)));
        }
    }
}

#[test]
fn golden_stderr_io_resources() {
    let out = common::run_with_full_stdout(&["init"]);
    assert_eq!(out.code, 3);
    common::assert_golden("fixtures/cli-stderr/io.txt", &out.stderr);
}

#[test]
fn golden_stderr_refused_file() {
    let out = run(&["lint", "."], b"");
    assert_eq!(out.code, 4);
    common::assert_golden("fixtures/cli-stderr/refused.txt", &out.stderr);
}

#[test]
fn golden_stderr_over_budget() {
    let bytes = vec![b' '; forge_warden_core::MAX_INPUT_BYTES.saturating_add(1)];
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
            TODAY,
        ],
        &bytes,
    );
    assert_eq!(out.code, 4);
    common::assert_golden("fixtures/cli-stderr/over-budget.txt", &out.stderr);
}
