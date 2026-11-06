//! Authority mode at the process boundary (ADR 0005 §2, §6): forge-warden runs the pinned ranker
//! on the patch it holds, and only that can pass. The ranker is the fixture ranker
//! (`tests/support/fixture_ranker.rs`): it replays deep-diff-forge's recorded rankings, synthesizes
//! one from the patch headers otherwise, and misbehaves on request (`+fixture-ranker: …` lines).
#![expect(
    clippy::disallowed_methods,
    clippy::unwrap_used,
    clippy::panic,
    reason = "test door: builds scratch rankers and policies; a panic is a failed test"
)]

mod common;
use common::{
    Run, fixture_ranker, fixture_ranker_sha256, policy_admitting, read_bytes, run, run_with_env,
    scratch, text, write,
};
use forge_warden_core::seal::sha256_hex;
use serde_json::Value;
use std::path::{Path, PathBuf};

const TODAY: &str = "2026-11-06";
const NO_SECRETS: &str = "fixtures/adversarial/no-secrets.toml";
const STARTER: &str = "examples/starter.warden.toml";
const DEMO: &str = "fixtures/demo.patch";

/// A one-file docs change; `extra` lines are added to the file (directives go here).
fn docs_patch(extra: &[&str]) -> Vec<u8> {
    let mut lines = vec![
        "diff --git a/docs/notes.md b/docs/notes.md",
        "index 1111111..2222222 100644",
        "--- a/docs/notes.md",
        "+++ b/docs/notes.md",
    ];
    let hunk = format!("@@ -1 +1,{} @@", extra.len().saturating_add(1));
    lines.push(&hunk);
    lines.push(" notes");
    let added: Vec<String> = extra.iter().map(|line| format!("+{line}")).collect();
    let mut text: String = lines.join("\n");
    for line in &added {
        text.push('\n');
        text.push_str(line);
    }
    text.push('\n');
    text.into_bytes()
}

/// `check` in authority mode with `ranker`, plus `extra` arguments.
fn authority(policy: &Path, patch: &Path, ranker: &Path, extra: &[&str]) -> Run {
    let mut args = vec![
        "check",
        "--policy",
        text(policy),
        "--patch",
        text(patch),
        "--ranker",
        text(ranker),
        "--today",
        TODAY,
        "--json",
    ];
    args.extend_from_slice(extra);
    run(&args, b"")
}

fn verdict(out: &Run) -> Value {
    assert!(
        out.stderr.is_empty() || out.stderr.contains("preloaded"),
        "a verdict is not an error: {}",
        out.stderr
    );
    serde_json::from_str(&out.stdout).unwrap_or_else(|e| panic!("{e}: {:?}", out.stdout))
}

fn notes(v: &Value) -> Vec<String> {
    v["notes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["code"].as_str().unwrap().to_owned())
        .collect()
}

/// A scratch dir with the no-secrets policy admitting the fixture ranker.
fn setup(name: &str) -> (PathBuf, PathBuf, PathBuf) {
    let dir = scratch(name);
    let policy = policy_admitting(&dir, NO_SECRETS, &[fixture_ranker_sha256()]);
    (dir, policy, fixture_ranker())
}

#[test]
fn a_produced_ranking_that_covers_the_patch_passes_and_says_who_produced_it() {
    let (dir, policy, ranker) = setup("pass");
    let patch = write(&dir, "change.patch", &docs_patch(&["more notes"]));
    let out = authority(
        &policy,
        &patch,
        &ranker,
        &["--base", "1234abcd", "--head", "abcdef0"],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let v = verdict(&out);
    assert_eq!(v["verdict"], "pass");
    assert_eq!(v["notes"], serde_json::json!([]));
    let seals = &v["seals"];
    assert_eq!(seals["provenance"], "produced");
    assert_eq!(seals["ranked_by"]["sha256"], fixture_ranker_sha256());
    assert_eq!(seals["ranked_by"]["version"], "0.7.0");
    assert_eq!(
        seals["patch_sha256"],
        sha256_hex(&std::fs::read(&patch).unwrap())
    );
    assert_eq!(
        seals["policy_sha256"],
        sha256_hex(&std::fs::read(&policy).unwrap())
    );
    assert!(
        seals["patch_facts_sha256"]
            .as_str()
            .is_some_and(|s| s.len() == 64)
    );
    assert_eq!(seals["git"]["path"], "/usr/bin/git");
    assert!(
        seals["git"]["version"]
            .as_str()
            .unwrap()
            .starts_with("git version ")
    );
    // Claims are recorded, unverified, and change nothing else.
    assert_eq!(
        seals["claims"],
        serde_json::json!({"base": "1234abcd", "head": "abcdef0", "verified": false})
    );
    let mut plain = verdict(&authority(&policy, &patch, &ranker, &[]));
    assert!(plain["seals"].get("claims").is_none());
    plain["seals"]["claims"] = seals["claims"].clone();
    assert_eq!(
        plain, v,
        "claims have no effect on the outcome (§6: fake --base/--head)"
    );
}

#[test]
fn the_same_ranking_supplied_cannot_pass() {
    // The produced bytes, supplied as a file, are judged in advisory mode: hold at best.
    let (dir, policy, ranker) = setup("advisory");
    let bytes = docs_patch(&["more notes"]);
    let patch = write(&dir, "change.patch", &bytes);
    let produced = std::process::Command::new(&ranker)
        .args(["--stdin-patch", "--rank", "--json"])
        .stdin(std::fs::File::open(&patch).unwrap())
        .output()
        .unwrap();
    let rank = write(&dir, "rank.json", &produced.stdout);
    let out = run(
        &[
            "check",
            "--policy",
            text(&policy),
            "--patch",
            text(&patch),
            "--advisory",
            "--rank",
            text(&rank),
            "--today",
            TODAY,
            "--json",
        ],
        b"",
    );
    assert_eq!(out.code, 8, "{}", out.stderr);
    let v = verdict(&out);
    assert_eq!(v["verdict"], "hold");
    assert_eq!(notes(&v), ["rank_unverified"]);
    assert_eq!(v["seals"]["provenance"], "supplied");
    assert_eq!(v["seals"]["ranked_by"]["sha256"], Value::Null);
}

#[test]
fn a_supplied_ranking_that_drops_a_signal_holds_with_rank_mismatch() {
    // §6: a supplied rank with the real file set but `ci_privilege` removed.
    let dir = scratch("mismatch");
    let policy = policy_admitting(&dir, STARTER, &[fixture_ranker_sha256()]);
    let ranker = fixture_ranker();
    let patch = common::root().join(DEMO);
    let mut forged: Value = serde_json::from_slice(&read_bytes("fixtures/demo.rank.json")).unwrap();
    for file in forged["ranked"].as_array_mut().unwrap() {
        file["signals"]
            .as_array_mut()
            .unwrap()
            .retain(|s| s != "ci_privilege");
        if let Some(evidence) = file.get_mut("evidence").and_then(Value::as_array_mut) {
            evidence.retain(|e| e["signal"] != "ci_privilege");
        }
    }
    let forged = write(
        &dir,
        "forged.json",
        &serde_json::to_vec_pretty(&forged).unwrap(),
    );
    let out = authority(&policy, &patch, &ranker, &["--rank", text(&forged)]);
    let v = verdict(&out);
    assert_eq!(
        out.code, 7,
        "the produced ranking is judged: ci_privilege still fails"
    );
    assert!(
        notes(&v).contains(&"rank_mismatch".to_owned()),
        "{:?}",
        notes(&v)
    );
    let ci = v["rules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "no-ci-privilege")
        .unwrap();
    assert_eq!(ci["outcome"], "fail");

    // The identical bytes supplied: nothing to note.
    let same = authority(&policy, &patch, &ranker, &["--rank", demo_rank().as_str()]);
    assert!(!notes(&verdict(&same)).contains(&"rank_mismatch".to_owned()));

    // Advisory mode on the forged ranking cannot pass either.
    let advisory = run(
        &[
            "check",
            "--policy",
            text(&policy),
            "--patch",
            text(&patch),
            "--advisory",
            "--rank",
            text(&forged),
            "--today",
            TODAY,
            "--json",
        ],
        b"",
    );
    assert_ne!(advisory.code, 0, "{}", advisory.stdout);
}

/// The demo ranking's absolute path.
fn demo_rank() -> String {
    text(&common::root().join("fixtures/demo.rank.json")).to_owned()
}

#[test]
fn a_ranker_the_policy_does_not_list_is_refused_without_a_verdict() {
    let (dir, _, ranker) = setup("not-admitted");
    let patch = write(&dir, "change.patch", &docs_patch(&["x"]));
    let other = policy_admitting(&dir, NO_SECRETS, &["f".repeat(64)]);
    let out = authority(&other, &patch, &ranker, &[]);
    assert_eq!(out.code, 4, "{}", out.stderr);
    assert_eq!(out.stdout, "");
    assert!(out.stderr.contains("does not list"), "{}", out.stderr);
    assert!(
        out.stderr.contains(&fixture_ranker_sha256()),
        "{}",
        out.stderr
    );

    // A policy without [producer] admits no ranker at all.
    let bare = common::root().join(NO_SECRETS);
    let unlisted = authority(&bare, &patch, &ranker, &[]);
    assert_eq!(unlisted.code, 4, "{}", unlisted.stderr);
    assert_eq!(unlisted.stdout, "");
    assert!(
        unlisted.stderr.contains("no [producer] sha256 allowlist"),
        "{}",
        unlisted.stderr
    );
}

#[test]
fn a_ranker_failure_gives_exit_3_and_no_verdict() {
    // §6: nonzero exit with valid-looking JSON, malformed output, oversized output, and a
    // ranking of some other patch. Each leaves nothing to judge.
    let (dir, policy, ranker) = setup("ranker-fails");
    for (name, directive, needle) in [
        ("exit", "fixture-ranker: exit 1", "the ranker exited 1"),
        ("garbage", "fixture-ranker: garbage", "not a ranking"),
        ("flood", "fixture-ranker: flood", "stdout budget"),
        ("seal", "fixture-ranker: wrong-seal", "sealed patch"),
    ] {
        let patch = write(&dir, &format!("{name}.patch"), &docs_patch(&[directive]));
        let out = authority(&policy, &patch, &ranker, &[]);
        assert_eq!(out.code, 3, "{name}: {}", out.stderr);
        assert_eq!(out.stdout, "", "{name}");
        assert!(out.stderr.contains(needle), "{name}: {}", out.stderr);
        assert!(out.stderr.contains("next: check_environment"), "{name}");
    }
}

#[test]
fn a_ranker_that_cannot_be_pinned_or_executed_is_refused() {
    use std::os::unix::fs::PermissionsExt as _;
    let (dir, _, ranker) = setup("ranker-pin");
    let patch = write(&dir, "change.patch", &docs_patch(&["x"]));
    let bytes = std::fs::read(&ranker).unwrap();

    // A symlink to the admitted ranker: the door does not follow it.
    let link = dir.join("link");
    std::os::unix::fs::symlink(&ranker, &link).unwrap();
    // A group-writable copy of the admitted bytes.
    let writable = write(&dir, "writable", &bytes);
    std::fs::set_permissions(&writable, std::fs::Permissions::from_mode(0o775)).unwrap();
    // A script, admitted by its hash: it is pinned, but a script cannot run from the pinned
    // descriptor (native executables only, ADR 0005 §2.4).
    let script = write(&dir, "script", b"#!/bin/sh\necho '{}'\n");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let policy = policy_admitting(
        &dir,
        NO_SECRETS,
        &[
            sha256_hex(&bytes),
            sha256_hex(&std::fs::read(&script).unwrap()),
        ],
    );
    for (what, candidate) in [
        ("symlink", link),
        ("group-writable", writable),
        ("script", script),
        ("missing", dir.join("missing")),
    ] {
        let out = authority(&policy, &patch, &candidate, &[]);
        assert_eq!(out.code, 3, "{what}: {}", out.stderr);
        assert_eq!(out.stdout, "", "{what}");
    }
}

#[test]
fn git_still_checks_the_producer_and_refusals_hold() {
    let (dir, policy, ranker) = setup("git-checks");
    // The producer leaves a file out: Git sees it, so the verdict holds.
    let mut two = docs_patch(&["fixture-ranker: omit docs/other.md"]);
    two.extend_from_slice(
        b"diff --git a/docs/other.md b/docs/other.md\nindex 1111111..2222222 100644\n--- a/docs/other.md\n+++ b/docs/other.md\n@@ -1 +1,2 @@\n other\n+more\n",
    );
    let patch = write(&dir, "omit.patch", &two);
    let v = verdict(&authority(&policy, &patch, &ranker, &[]));
    assert_eq!(v["verdict"], "hold");
    assert_eq!(notes(&v), ["rank_incomplete"]);

    // Git refuses a corrupt hunk: the patch is unverifiable, so nothing passes by default.
    let corrupt = write(
        &dir,
        "corrupt.patch",
        b"diff --git a/docs/notes.md b/docs/notes.md\n--- a/docs/notes.md\n+++ b/docs/notes.md\n@@ -1,9 +1,9 @@\n+x\n",
    );
    let refused = authority(&policy, &corrupt, &ranker, &[]);
    assert_eq!(refused.code, 8, "{}", refused.stderr);
    let held = verdict(&refused);
    assert_eq!(notes(&held), ["patch_unverifiable"]);
    assert_eq!(held["seals"]["patch_facts_sha256"], Value::Null);
}

#[test]
fn a_poisoned_parent_environment_changes_nothing() {
    // §6: PATH, LD_PRELOAD, GIT_CONFIG_* and ~/.gitconfig poisoned in the parent environment.
    let (dir, policy, ranker) = setup("poison");
    let patch = write(&dir, "change.patch", &docs_patch(&["more notes"]));
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let bad_config =
        b"[apply]\n\tignoreWhitespace = definitely-invalid\n[core]\n\thooksPath = /nonexistent\n";
    write(&home, ".gitconfig", bad_config);
    let global = write(&dir, "global.gitconfig", bad_config);
    let args = [
        "check",
        "--policy",
        text(&policy),
        "--patch",
        text(&patch),
        "--ranker",
        text(&ranker),
        "--today",
        TODAY,
        "--json",
    ];
    let clean = run(&args, b"");
    assert_eq!(clean.code, 0, "{}", clean.stderr);
    let poisoned = run_with_env(
        &args,
        b"",
        &[
            ("PATH", "/nonexistent"),
            ("LD_PRELOAD", "/nonexistent/libpoison.so"),
            ("HOME", text(&home)),
            ("GIT_CONFIG_GLOBAL", text(&global)),
            ("GIT_CONFIG_SYSTEM", text(&global)),
            ("GIT_CONFIG_COUNT", "1"),
            ("GIT_CONFIG_KEY_0", "apply.ignoreWhitespace"),
            ("GIT_CONFIG_VALUE_0", "definitely-invalid"),
            ("GIT_DIR", "/nonexistent"),
        ],
    );
    assert_eq!(poisoned.code, clean.code, "{}", poisoned.stderr);
    assert_eq!(poisoned.stdout, clean.stdout);

    // A TMPDIR that cannot bound Git's discovery (relative, or with ':') is refused, never used
    // (Bastion B-C1b-01, ADR 0010).
    for tmpdir in ["relative/tmp", "/tmp:/x"] {
        let out = run_with_env(&args, b"", &[("TMPDIR", tmpdir)]);
        assert_eq!(out.code, 3, "{tmpdir}: {}", out.stderr);
        assert_eq!(out.stdout, "", "{tmpdir}");
        assert!(
            out.stderr.contains("temporary directory"),
            "{tmpdir}: {}",
            out.stderr
        );
    }
}

#[test]
fn one_claim_is_recorded_alone_and_the_text_summary_names_the_producer() {
    let (dir, policy, ranker) = setup("one-claim");
    let patch = write(&dir, "change.patch", &docs_patch(&["more notes"]));
    for (flag, key) in [("--base", "base"), ("--head", "head")] {
        let v = verdict(&authority(&policy, &patch, &ranker, &[flag, "abcd"]));
        let mut claims = serde_json::json!({"verified": false});
        claims[key] = "abcd".into();
        assert_eq!(v["seals"]["claims"], claims, "{flag}");
    }
    let summary = run(
        &[
            "check",
            "--policy",
            text(&policy),
            "--patch",
            text(&patch),
            "--ranker",
            text(&ranker),
            "--today",
            TODAY,
        ],
        b"",
    );
    assert_eq!(summary.code, 0, "{}", summary.stderr);
    let short = fixture_ranker_sha256().get(..12).unwrap().to_owned();
    assert!(
        summary.stdout.contains(&format!(
            "ranked by deep-diff-forge 0.7.0 (produced by ranker {short})"
        )),
        "{}",
        summary.stdout
    );
}
