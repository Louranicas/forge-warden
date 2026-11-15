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
fn setup(name: &str) -> (common::Scratch, PathBuf, PathBuf) {
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
        sha256_hex(&std::fs::read(&patch).unwrap()).as_str()
    );
    assert_eq!(
        seals["policy_sha256"],
        sha256_hex(&std::fs::read(&policy).unwrap()).as_str()
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
    let produced = common::command(&ranker)
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
        assert_eq!(
            serde_json::from_str::<Value>(&out.stderr)
                .unwrap()
                .get("next")
                .unwrap(),
            "check_environment",
            "{name}"
        );
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
    // Created out of process: this test never holds a write fd on a file forge-warden executes.
    let script = common::write_executable(&dir, "script", b"#!/bin/sh\necho '{}'\n");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let policy = policy_admitting(
        &dir,
        NO_SECRETS,
        &[
            sha256_hex(&bytes).to_string(),
            sha256_hex(&std::fs::read(&script).unwrap()).to_string(),
        ],
    );
    // A FIFO, made by a child (`mkfifo`): never opened for a blocking read.
    let fifo = dir.join("fifo");
    let made = common::command("/usr/bin/mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(made.success());
    // Each refusal has its own exact reason and registry code (Q2-05): a ranker the door refuses
    // to pin (a symlink, a FIFO, unsafe mode) is an input to change (4); one that cannot be read or
    // ran and failed is an environment problem (3). The script is pinned and admitted, so it must
    // reach exec and its interpreter must EXIT (it cannot reopen the closed descriptor); "could not
    // be started" (e.g. ETXTBSY) is a different outcome and fails this test (Touchstone C1c v4.4).
    for (what, candidate, code, reason) in [
        ("symlink", link, 4, "Too many levels of symbolic links"),
        ("fifo", fifo, 4, "not a regular file"),
        ("group-writable", writable, 4, "writable by group or others"),
        ("script", script, 3, "the ranker exited "),
        (
            "missing",
            dir.join("missing"),
            3,
            "No such file or directory",
        ),
    ] {
        let out = authority(&policy, &patch, &candidate, &[]);
        assert_eq!(out.code, code, "{what}: {}", out.stderr);
        let next = if code == 4 {
            "change_input"
        } else if what == "missing" {
            "check_input_output"
        } else {
            "check_environment"
        };
        let error: Value = serde_json::from_str(&out.stderr).unwrap();
        assert_eq!(error.get("next").unwrap(), next, "{what}");
        if what == "fifo" || what == "group-writable" {
            // Q2-15: compare the complete JSON envelope and the sole trailing newline.
            let expected = serde_json::json!({
                "exit": 4, "name": "invalid_input",
                "message": format!("pinning {}: {reason}", candidate.display()),
                "next": next,
            });
            assert_eq!(out.stderr, format!("{expected}\n"), "{what}");
        }
        assert_eq!(out.stdout, "", "{what}");
        assert!(out.stderr.contains(reason), "{what}: {}", out.stderr);
        assert!(
            !out.stderr.contains("could not be started"),
            "{what}: {}",
            out.stderr
        );
    }
}

#[test]
fn a_fifo_given_as_any_input_file_is_refused_at_once_with_exit_4() {
    // First use compiles the fixture CLI, including on a cold cargo-mutants target. Complete
    // that prerequisite before starting any watchdog: only input refusal belongs in its budget.
    let _fixture = common::fixture_cli();
    // Q-DOOR.1 (Q2-02 at the binary): `--policy`, `--patch` and `--rank` pointed at a FIFO with no
    // writer must be refused as an input to change (exit 4, change_input), not block the gate
    // forever. A watchdog releases a blocked open (a child opens the FIFO for writing and closes
    // it) so a regression fails instead of hanging the suite.
    let dir = scratch("fifo-inputs");
    let fifo = dir.join("input");
    let made = common::command("/usr/bin/mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(made.success());
    let fifo_text = text(&fifo).to_owned();
    let (good_policy, good_patch, good_rank) = (
        "examples/starter.warden.toml",
        "fixtures/demo.patch",
        "fixtures/demo.rank.json",
    );
    for (flag, policy, patch, rank) in [
        ("--policy", fifo_text.as_str(), good_patch, good_rank),
        ("--patch", good_policy, fifo_text.as_str(), good_rank),
        ("--rank", good_policy, good_patch, fifo_text.as_str()),
    ] {
        let args: Vec<String> = [
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
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        let (tx, rx) = std::sync::mpsc::channel();
        let judging = std::thread::spawn(move || {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let _sent = tx.send(run(&args, b""));
        });
        let outcome = rx.recv_timeout(std::time::Duration::from_secs(10));
        if outcome.is_err() {
            let _released = common::command("/usr/bin/timeout")
                .args(["5", "/bin/sh", "-c", ": > \"$1\"", "sh", fifo_text.as_str()])
                .status();
        }
        let _joined = judging.join();
        let Ok(out) = outcome else {
            panic!("{flag} FIFO: forge-warden blocked for 10 s instead of refusing it");
        };
        assert_eq!(out.code, 4, "{flag} FIFO: {}", out.stderr);
        assert_eq!(out.stdout, "", "{flag} FIFO");
        assert!(
            out.stderr.contains("not a regular file"),
            "{flag} FIFO: {}",
            out.stderr
        );
        assert_eq!(
            serde_json::from_str::<Value>(&out.stderr)
                .unwrap()
                .get("next")
                .unwrap(),
            "change_input",
            "{flag}"
        );
    }
    let _cleaned = std::fs::remove_dir_all(&dir);
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
    // The note names Git's refusal itself (Unverifiable::GitRefused), not another unverifiable form.
    assert!(
        held["notes"][0]["detail"]
            .as_str()
            .is_some_and(|d| d.starts_with("git refused the patch: ")),
        "{held}"
    );
    assert_eq!(held["seals"]["patch_facts_sha256"], Value::Null);
}

/// Freeze review N3b, end to end. With `diff.noprefix` Git writes `docs/notes.md` twice in the
/// header; `git apply` and the ranker both drop `docs/` and agree on `notes.md`. Before the header
/// check this was a PASS, exit 0, for a path the patch does not touch, so no rule keyed on
/// `docs/**` could see it. The patches are what Git 2.56 writes for each setting.
#[test]
fn a_patch_without_the_default_prefixes_never_passes() {
    let (dir, policy, ranker) = setup("patch-prefix");
    let prefixed = write(&dir, "prefixed.patch", &docs_patch(&["more"]));
    let passed = authority(&policy, &prefixed, &ranker, &[]);
    assert_eq!(passed.code, 0, "the control: {}", passed.stderr);
    for (name, old, new) in [
        ("noprefix", "docs/notes.md", "docs/notes.md"),
        ("other-prefixes", "old/docs/notes.md", "new/docs/notes.md"),
        ("mnemonic", "i/docs/notes.md", "w/docs/notes.md"),
    ] {
        let header = format!("diff --git {old} {new}");
        let text = format!(
            "{header}\nindex 1111111..2222222 100644\n--- {old}\n+++ {new}\n@@ -1 +1,2 @@\n notes\n+more\n"
        );
        let patch = write(&dir, &format!("{name}.patch"), text.as_bytes());
        let out = authority(&policy, &patch, &ranker, &[]);
        assert_eq!(out.code, 8, "{name}: {}{}", out.stdout, out.stderr);
        let held = verdict(&out);
        assert_eq!(held["verdict"], "hold", "{name}");
        // With no prefix the fixture ranker, like the real one, agrees with Git on `notes.md`, so
        // this note is the only thing between the patch and a PASS. With other prefixes the
        // fixture ranker lists nothing, which holds on its own (`empty_ranking`) as well.
        let expected: &[&str] = if name == "noprefix" {
            &["patch_unverifiable"]
        } else {
            &["empty_ranking", "patch_unverifiable"]
        };
        assert_eq!(notes(&held), expected, "{name}");
        assert!(
            held["notes"].as_array().unwrap().iter().any(|note| {
                note["detail"]
                    .as_str()
                    .is_some_and(|d| d.contains(&header) && d.contains("--src-prefix=a/"))
            }),
            "{name}: {held}"
        );
    }
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
    for (tmpdir, next) in [
        ("relative/tmp", "check_environment"),
        ("/tmp:/x", "check_input_output"),
    ] {
        let out = run_with_env(&args, b"", &[("TMPDIR", tmpdir)]);
        assert_eq!(out.code, 3, "{tmpdir}: {}", out.stderr);
        assert_eq!(out.stdout, "", "{tmpdir}");
        assert_eq!(
            serde_json::from_str::<Value>(&out.stderr)
                .unwrap()
                .get("next")
                .unwrap(),
            next,
            "{tmpdir}"
        );
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

/// An admitted ranker cannot turn missing Git enrollment into authority.
#[test]
fn authority_requires_git_policy_before_producing_a_verdict() {
    let (dir, policy, ranker) = setup("git-policy-required");
    let patch = write(&dir, "change.patch", &docs_patch(&["more notes"]));
    let out = common::run_without_git_policy(&[
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
    ]);
    assert_eq!(out.code, 3, "stdout={} stderr={}", out.stdout, out.stderr);
    assert!(
        out.stderr
            .contains("Git admission: no Git allowlist enrolled"),
        "{}",
        out.stderr
    );
    assert!(
        out.stdout.is_empty(),
        "refusal must not emit a verdict: {}",
        out.stdout
    );
}
