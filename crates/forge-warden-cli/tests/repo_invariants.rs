//! Repository invariants that no compiler lint covers. Each one closes a way bad code gets in.
//!
//! If one of these fails, the message says what to do. Do not weaken the check to make it pass:
//! changing this file is a `protect_paths` hold in `warden.toml`, so a person will see it.
#![expect(
    clippy::disallowed_methods,
    reason = "test code reading the repository: a panic is a failed test"
)]

mod common;
use common::{files_under, read, root};

fn rust_sources() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for dir in ["crates"] {
        for path in files_under(dir, ".rs") {
            let rel = path
                .strip_prefix(root())
                .unwrap_or(&path)
                .to_string_lossy()
                .replace("/crates/../..", "");
            // This file names the markers and attributes it searches for, so it cannot scan itself.
            if rel.ends_with("tests/repo_invariants.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            out.push((rel, text));
        }
    }
    assert!(
        out.len() >= 10,
        "found only {} Rust files; is the walk broken?",
        out.len()
    );
    out
}

#[test]
fn every_crate_inherits_the_workspace_lints() {
    for manifest in files_under("crates", "Cargo.toml") {
        let text = std::fs::read_to_string(&manifest).unwrap();
        assert!(
            text.contains("[lints]\nworkspace = true"),
            "{} must contain `[lints]\\nworkspace = true` so the lint wall applies to it",
            manifest.display()
        );
    }
}

/// Code lines only: a line comment (`//`, `///`, `//!`) is prose, not an attribute.
fn code_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim_start().starts_with("//"))
}

#[test]
fn suppressions_are_expectations_with_reasons() {
    for (path, text) in rust_sources() {
        let lines: Vec<&str> = text.lines().collect();
        for (n, line) in code_lines(&text) {
            let code = line.trim_start();
            assert!(
                !(code.starts_with("#[allow(")
                    || code.starts_with("#![allow(")
                    || (code.contains("cfg_attr(") && code.contains("allow("))),
                "{path}:{}: use #[expect(lint, reason = \"…\")], which fails once it is no longer needed",
                n + 1
            );
            if code.starts_with("#[expect(") || code.starts_with("#![expect(") {
                // The attribute runs until the first line that closes it.
                let attribute: String = lines[n..]
                    .iter()
                    .take_while(|l| !l.trim_end().ends_with(")]"))
                    .chain(lines[n..].iter().find(|l| l.trim_end().ends_with(")]")))
                    .copied()
                    .collect();
                assert!(
                    attribute.contains("reason = \""),
                    "{path}:{}: every #[expect] needs a reason = \"…\"",
                    n + 1
                );
            }
        }
    }
}

#[test]
fn only_doors_rs_opens_doors_in_production_code() {
    for (path, text) in rust_sources() {
        let is_test = path.contains("/tests/");
        if is_test || path.ends_with("src/doors.rs") {
            continue;
        }
        assert!(
            !text.contains("disallowed_methods") && !text.contains("disallowed_types"),
            "{path}: effects go through crates/forge-warden-cli/src/doors.rs (ADR 0001), not a local exemption"
        );
    }
}

#[test]
fn the_core_crate_never_depends_on_the_cli() {
    let manifest = read("crates/forge-warden-core/Cargo.toml");
    assert!(
        !manifest.contains("forge-warden-cli"),
        "the dependency arrow points from cli to core only (ADR 0001)"
    );
    for (path, text) in rust_sources() {
        if path.contains("forge-warden-core/src") {
            assert!(
                !text.contains("std::process"),
                "{path}: the core spawns nothing"
            );
            assert!(!text.contains("std::fs"), "{path}: the core does no I/O");
        }
    }
}

#[test]
fn todo_comments_name_an_issue() {
    for (path, text) in rust_sources() {
        for (n, line) in text.lines().enumerate() {
            for marker in ["TODO", "FIXME", "XXX", "HACK"] {
                for (at, _) in line.match_indices(marker) {
                    // `at` and `at + len` are char boundaries: `match_indices` found an ASCII marker.
                    let before = line.get(..at).and_then(|head| head.chars().next_back());
                    let rest = line.get(at + marker.len()..).unwrap_or_default();
                    let after = rest.chars().next();
                    // Only the whole word counts: `XXXX` in `\u{XXXX}` is not a marker.
                    if before.is_some_and(char::is_alphanumeric)
                        || after.is_some_and(char::is_alphanumeric)
                    {
                        continue;
                    }
                    assert!(
                        rest.starts_with("(#"),
                        "{path}:{}: write {marker}(#<issue>) so the debt is tracked, or fix it now",
                        n + 1
                    );
                }
            }
        }
    }
}

#[test]
fn changelog_has_an_unreleased_section() {
    assert!(
        read("CHANGELOG.md").contains("## [Unreleased]"),
        "CHANGELOG.md needs a `## [Unreleased]` section for the next change"
    );
}

#[test]
fn the_repository_judges_itself() {
    // The repository's own policy must lint clean: CI runs it on every pull request.
    let out = common::run(&["lint", "warden.toml"], b"");
    assert_eq!(out.code, 0, "{}", out.stderr);
}

#[test]
fn readme_exit_table_tracks_the_registry() {
    let readme = read("README.md");
    for entry in forge_warden_core::EXIT_REGISTRY {
        let row = format!(
            "| {} | `{}` | {} | `{}` |",
            entry.code, entry.name, entry.meaning, entry.action
        );
        assert!(
            readme.contains(&row),
            "README.md is missing or has drifted from this row:\n{row}"
        );
    }
}
