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

/// The part of a source file before its unit tests.
fn production_part(text: &str) -> &str {
    text.find("#[cfg(test)]")
        .map_or(text, |end| text.get(..end).unwrap_or(text))
}

#[test]
fn defaults_say_which_way_they_fail() {
    // G-04: `.unwrap_or(Outcome::Pass)` decided a verdict silently. A default is a decision about
    // which way a missing value fails, so every one in production code is marked, one marker per
    // default, at most six lines above it.
    const MARKERS: [&str; 3] = ["// fails closed:", "// fails safe:", "// display only:"];
    for (path, text) in rust_sources() {
        if path.contains("/tests/") {
            continue;
        }
        let lines: Vec<&str> = production_part(&text).lines().collect();
        let mut defaults = 0_usize;
        let mut markers = 0_usize;
        for (index, line) in lines.iter().enumerate() {
            let trimmed = line.trim_start();
            if MARKERS.iter().any(|m| trimmed.starts_with(m)) {
                markers = markers.saturating_add(1);
                continue;
            }
            if trimmed.starts_with("//") {
                continue;
            }
            let calls = line.matches("unwrap_or").count();
            if calls == 0 {
                continue;
            }
            defaults = defaults.saturating_add(calls);
            let window = lines
                .get(index.saturating_sub(6)..index)
                .unwrap_or_default();
            assert!(
                window
                    .iter()
                    .any(|l| MARKERS.iter().any(|m| l.trim_start().starts_with(m))),
                "{path}:{}: say which way this default fails: a `// fails closed: …`, `// fails safe: …` or `// display only: …` line just above it",
                index.saturating_add(1)
            );
        }
        assert_eq!(
            markers, defaults,
            "{path}: {markers} default marker(s) for {defaults} default(s); each default gets its own"
        );
    }
}

#[test]
fn the_core_imports_no_effect_module() {
    // G-05: `use std::{fs}; fs::metadata(p)` contains no `std::fs` text. Read the `use` trees.
    const EFFECTS: [&str; 5] = ["fs", "process", "net", "env", "thread"];
    for (path, text) in rust_sources() {
        if !path.contains("forge-warden-core/src") {
            continue;
        }
        let compact: String = production_part(&text)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        for (start, _) in compact.match_indices("usestd::") {
            let tree = compact
                .get(start..)
                .and_then(|rest| rest.split(';').next())
                .unwrap_or_default();
            for effect in EFFECTS {
                let named = [
                    format!("::{effect}"),
                    format!("{{{effect},"),
                    format!("{{{effect}}}"),
                    format!(",{effect},"),
                    format!(",{effect}}}"),
                ];
                assert!(
                    !named.iter().any(|n| tree.contains(n.as_str())),
                    "{path}: `{tree}` imports std::{effect}; the core does no I/O (ADR 0001)"
                );
            }
        }
    }
}

/// Every dependency a manifest declares, as `(table, crate)`, in every form Cargo accepts: the
/// `[dependencies]` table and its `[dependencies.<name>]` sub-tables (one TOML table either way),
/// dev and build dependencies (and their `_` spellings), `[target.<cfg>.*]` tables, and renames
/// (`alias = { package = "<crate>" }` is the crate `<crate>`).
fn declared_dependencies(manifest: &str) -> Result<Vec<(String, String)>, String> {
    const KINDS: [&str; 5] = [
        "dependencies",
        "dev-dependencies",
        "dev_dependencies",
        "build-dependencies",
        "build_dependencies",
    ];
    fn collect(out: &mut Vec<(String, String)>, table: &str, deps: &toml::Value) {
        for (key, spec) in deps.as_table().into_iter().flatten() {
            let package = spec.get("package").and_then(toml::Value::as_str);
            out.push((table.to_owned(), package.unwrap_or(key).to_owned()));
        }
    }
    let doc: toml::Table = toml::from_str(manifest).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for kind in KINDS {
        if let Some(deps) = doc.get(kind) {
            collect(&mut out, kind, deps);
        }
    }
    let targets = doc.get("target").and_then(toml::Value::as_table);
    for (cfg, target) in targets.into_iter().flatten() {
        for kind in KINDS {
            if let Some(deps) = target.get(kind) {
                collect(&mut out, &format!("target.{cfg}.{kind}"), deps);
            }
        }
    }
    Ok(out)
}

/// The dependencies of a core manifest that are not on the core's allow-list. A manifest that is
/// not valid TOML is itself a violation (fails closed).
fn core_dependency_violations(manifest: &str) -> Vec<String> {
    const ALLOWED: [&str; 6] = [
        "globset",
        "serde",
        "serde_json",
        "sha2",
        "thiserror",
        "toml",
    ];
    const ALLOWED_DEV: [&str; 1] = ["proptest"];
    let declared = match declared_dependencies(manifest) {
        Ok(declared) => declared,
        Err(error) => return vec![format!("the manifest is not valid TOML: {error}")],
    };
    declared
        .into_iter()
        .filter(|(table, name)| match table.as_str() {
            "dependencies" => !ALLOWED.contains(&name.as_str()),
            "dev-dependencies" => !ALLOWED_DEV.contains(&name.as_str()),
            _ => true,
        })
        .map(|(table, name)| format!("{table}: {name}"))
        .collect()
}

#[test]
fn the_core_depends_only_on_its_allow_list() {
    // G-21: a new dependency in the core is a reviewed decision, not a line in Cargo.toml.
    let manifest = read("crates/forge-warden-core/Cargo.toml");
    assert_eq!(
        core_dependency_violations(&manifest),
        Vec::<String>::new(),
        "crates/forge-warden-core/Cargo.toml: not on the core's allow-list (adding one is a reviewed change to repo_invariants.rs)"
    );
}

#[test]
fn the_dependency_scan_reads_every_cargo_form() {
    // Touchstone B6-VERDICT 7192058f: `[dependencies.memchr]` passed a scan that read only the
    // `[dependencies]` header. Each form below adds memchr; each must be reported.
    let base = "[package]\nname = \"x\"\n\n[dependencies]\nserde.workspace = true\n\n[dev-dependencies]\nproptest.workspace = true\n";
    assert_eq!(core_dependency_violations(base), Vec::<String>::new());
    let forms = [
        (
            "[dependencies.memchr] table",
            "\n[dependencies.memchr]\nversion = \"2\"\n",
        ),
        (
            "target cfg",
            "\n[target.'cfg(unix)'.dependencies]\nmemchr = \"2\"\n",
        ),
        ("build", "\n[build-dependencies]\nmemchr = \"2\"\n"),
        (
            "underscore build",
            "\n[build_dependencies]\nmemchr = \"2\"\n",
        ),
        (
            "[dev-dependencies.memchr] table",
            "\n[dev-dependencies.memchr]\nversion = \"2\"\n",
        ),
        (
            "target triple dev",
            "\n[target.x86_64-unknown-linux-gnu.dev-dependencies]\nmemchr = \"2\"\n",
        ),
    ];
    for (form, extra) in forms {
        let found = core_dependency_violations(&format!("{base}{extra}"));
        assert!(
            found.iter().any(|v| v.ends_with(": memchr")),
            "{form}: {found:?}"
        );
    }
    let renamed = base.replace(
        "serde.workspace = true",
        "serde = { package = \"memchr\", version = \"2\" }",
    );
    assert_eq!(
        core_dependency_violations(&renamed),
        ["dependencies: memchr"]
    );
    let one_line = base.replace(
        "[dependencies]\nserde.workspace = true",
        "[dependencies]\nserde.workspace = true\nmemchr = \"2\"",
    );
    assert_eq!(
        core_dependency_violations(&one_line),
        ["dependencies: memchr"]
    );
    assert_eq!(
        core_dependency_violations("not = [toml").len(),
        1,
        "unparseable is refused"
    );
}

#[test]
fn exit_statuses_come_from_the_registry() {
    // R-20: a literal `ExitCode::from(2)` bypassed the registry. Only the exit door in `doors.rs`
    // turns an `Exit` into a process status; everything else returns an `Exit`, so a code cannot
    // drift from the table.
    for (path, text) in rust_sources() {
        if path.contains("/tests/") || path.ends_with("crates/forge-warden-cli/src/doors.rs") {
            continue;
        }
        for (line, code) in code_lines(&text) {
            assert!(
                !code.contains("ExitCode"),
                "{path}:{}: return an `Exit` from the registry (forge_warden_core::Exit); only doors.rs makes an `ExitCode`",
                line.saturating_add(1)
            );
        }
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

#[test]
fn only_the_door_mints_a_produced_ranking_and_the_binary_never_judges_raw() {
    // ADR 0005 §2.7 (rung 4): `Ranking::attest` is called only in doors.rs, right after the pinned
    // ranker ran; and the binary judges only through `evaluate_authority`/`evaluate_advisory`.
    for (path, text) in rust_sources() {
        if path.contains("/tests/") || path.ends_with("crates/forge-warden-core/src/authority.rs") {
            continue;
        }
        let attests = text.matches("attest(").count();
        if path.ends_with("crates/forge-warden-cli/src/doors.rs") {
            assert_eq!(
                attests, 1,
                "{path}: exactly one attest call, in produce_ranking"
            );
        } else {
            assert_eq!(attests, 0, "{path}: only doors.rs may call Ranking::attest");
        }
    }
    let main = read("crates/forge-warden-cli/src/main.rs");
    for (index, line) in main.lines().enumerate() {
        let raw = line
            .match_indices("evaluate(")
            .any(|(at, _)| !line.get(..at).is_some_and(|before| before.ends_with('_')));
        assert!(
            !raw,
            "crates/forge-warden-cli/src/main.rs:{}: judge through evaluate_authority or evaluate_advisory, never raw evaluate",
            index.saturating_add(1)
        );
    }
}
