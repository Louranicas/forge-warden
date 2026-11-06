//! `scripts/check_vocab.sh` against synthetic deep-diff-forge commits (WP-B3). Each case is a real
//! commit in a scratch Git repository built from `fixtures/vocab/`, so the script reads it the way
//! it reads deep-diff-forge: by commit, through `git archive`.
#![expect(
    clippy::disallowed_methods,
    clippy::unwrap_used,
    reason = "test harness: builds a scratch repository and runs the script; a panic is a failed test"
)]

mod common;

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn git(repo: &Path, args: &[&str]) -> String {
    let out = common::command("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@x",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

/// A scratch repository whose `crates/deep-diff-forge-graph/src/{risk,evidence}.rs` are the
/// fixtures with `edit` applied to one of them, committed; returns (repo, commit).
fn commit_with(name: &str, file: &str, from: &str, to: &str) -> (PathBuf, String) {
    let repo = std::env::temp_dir().join(format!("fw-vocab-{}-{name}", std::process::id()));
    let _gone = std::fs::remove_dir_all(&repo);
    let src = repo.join("crates/deep-diff-forge-graph/src");
    std::fs::create_dir_all(&src).unwrap();
    for f in ["risk.rs", "evidence.rs"] {
        let text = std::fs::read_to_string(root().join("fixtures/vocab").join(f)).unwrap();
        let text = if f == file {
            assert!(text.contains(from), "{name}: anchor missing");
            text.replacen(from, to, 1)
        } else {
            text
        };
        std::fs::write(src.join(f), text).unwrap();
    }
    git(&repo, &["init", "-q"]);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", name]);
    let commit = git(&repo, &["rev-parse", "HEAD"]);
    (repo, commit)
}

/// Run the script from a copy of the two files it needs, staged outside any Git repository (as in a
/// mutation run's scratch tree or a release tarball), so it must find its tree without Git.
fn check(repo: &Path, rev: &str) -> (i32, String) {
    let tree = std::env::temp_dir().join(format!(
        "fw-vocab-tree-{}-{}",
        std::process::id(),
        rev.get(..12).unwrap()
    ));
    let _gone = std::fs::remove_dir_all(&tree);
    for file in [
        "scripts/check_vocab.sh",
        "crates/forge-warden-core/src/vocab.rs",
    ] {
        let to = tree.join(file);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::copy(root().join(file), &to).unwrap();
    }
    let out = common::command("bash")
        .arg(tree.join("scripts/check_vocab.sh"))
        .args([rev, repo.to_str().unwrap()])
        .current_dir(&tree)
        // Git may not discover any repository around the staged tree.
        .env("GIT_CEILING_DIRECTORIES", std::env::temp_dir())
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.code().unwrap_or(-1), text)
}

const MANY: &str = r#"Self::ManyHunks => "many_hunks","#;

#[test]
fn the_script_attests_exactly_what_the_commit_labels() {
    let (repo, base) = commit_with("base", "risk.rs", MANY, MANY);
    let (code, text) = check(&repo, &base);
    assert_eq!(code, 0, "{text}");
    assert!(
        text.contains(&format!("matches deep-diff-forge {base}")),
        "{text}"
    );
    // A dirty working tree is never read: the named commit is.
    let risk = repo.join("crates/deep-diff-forge-graph/src/risk.rs");
    let dirty = std::fs::read_to_string(&risk)
        .unwrap()
        .replace(MANY, r#"Self::ManyHunks => "hunks","#);
    std::fs::write(&risk, dirty).unwrap();
    assert_eq!(check(&repo, &base).0, 0);
    assert_eq!(check(&repo, "deadbeefnotacommit").0, 1);
}

#[test]
fn every_label_form_the_script_cannot_read_exactly_fails_closed() {
    let cases = [
        (
            "drift-block-arm",
            "risk.rs",
            MANY,
            "Self::ManyHunks => {\n                \"many_hunks_v2\"\n            }",
            "drift",
        ),
        (
            "prefix-split-at",
            "risk.rs",
            MANY,
            r#"Self::ManyHunks => "many_hunks".split_at(5).1,"#,
            "extraction incomplete",
        ),
        (
            "prefix-concat",
            "risk.rs",
            MANY,
            r#"Self::ManyHunks => concat!("many_", "hunks"),"#,
            "extraction incomplete",
        ),
        (
            "prefix-plus",
            "risk.rs",
            MANY,
            r#"Self::ManyHunks => "many_hunks".to_owned() + "","#,
            "extraction incomplete",
        ),
        (
            "block-with-tail",
            "risk.rs",
            MANY,
            r#"Self::ManyHunks => { "many_hunks".trim() }"#,
            "extraction incomplete",
        ),
        (
            "const-arm",
            "risk.rs",
            MANY,
            "Self::ManyHunks => MANY_HUNKS,",
            "extraction incomplete",
        ),
        (
            "delegation-other",
            "risk.rs",
            "Self::Code(signal) => signal.label(),",
            "Self::Code(signal) => signal.name(),",
            "extraction incomplete",
        ),
    ];
    for (name, file, from, to, want) in cases {
        let (repo, commit) = commit_with(name, file, from, to);
        let (code, text) = check(&repo, &commit);
        assert_eq!(code, 1, "{name}: {text}");
        assert!(text.contains(want), "{name}: {text}");
        let _cleaned = std::fs::remove_dir_all(&repo);
    }
}

#[test]
fn git_fixture_environment_cannot_redirect_the_index() {
    let repo = common::scratch("git-env-boundary");
    let outside = common::write(&repo, "outer-index", b"untouched");
    let mut cmd = common::command("git");
    cmd.env("GIT_INDEX_FILE", &outside)
        .env("GIT_DIR", "/nonexistent")
        .env("GIT_CONFIG_COUNT", "invalid");
    common::hermetic(&mut cmd);
    let output = cmd
        .args(["init", "--quiet"])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    git(&repo, &["add", "-A"]);
    assert_eq!(std::fs::read(outside).unwrap(), b"untouched");
    assert!(repo.join(".git/index").is_file());
}
