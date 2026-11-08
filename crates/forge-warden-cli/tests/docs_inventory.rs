//! Documentation names real schemas, CI checks and corpus members, not copied inventories.
mod common;
use common::{files_under, read};
use std::collections::BTreeSet;

fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_ascii_alphanumeric() && !"/._-".contains(c))
}

#[test]
fn front_door_schema_tokens_exist_and_match_code() {
    let names: BTreeSet<_> = files_under("schemas", ".json")
        .into_iter()
        .map(|path| common::relative(&path))
        .collect();
    let known = [
        forge_warden_core::policy::POLICY_SCHEMA,
        forge_warden_core::verdict::VERDICT_SCHEMA,
    ];
    let mut references = 0;
    for path in [
        "README.md",
        "CLAUDE.md",
        "CONTRIBUTING.md",
        "SECURITY.md",
        "docs/POLICY_REFERENCE.md",
    ] {
        let text = read(path);
        for raw in tokens(&text) {
            let token = raw.trim_start_matches("./");
            if token.starts_with("schemas/")
                && std::path::Path::new(token)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
            {
                assert!(names.contains(token), "{path}: unknown schema file {token}");
                references += 1;
            }
            if token.starts_with("verdict.v") && !token.contains('/') {
                let stem = token.trim_end_matches(".schema.json");
                assert_eq!(
                    format!("forge-warden.{stem}"),
                    known[1],
                    "{path}: stale verdict version {token}"
                );
            }
            if token.starts_with("forge-warden.verdict.v")
                || token.starts_with("forge-warden.policy.v")
            {
                assert!(
                    known.contains(&token),
                    "{path}: unknown schema identifier {token}"
                );
                let suffix = token.strip_prefix("forge-warden.").unwrap();
                assert!(
                    names.contains(&format!("schemas/{suffix}.schema.json")),
                    "{path}: {token} has no schema"
                );
                references += 1;
            }
        }
    }
    assert!(
        references >= 6,
        "schema reference discovery lost its inputs"
    );
}

#[test]
fn contributing_required_checks_equal_actual_ci_job_names() {
    // Both inventories are declarations, deliberately read independently. This is a YAML subset
    // contract: job display names must be literal scalars, not generated matrix expressions.
    let ci = read(".github/workflows/ci.yml");
    let actual: BTreeSet<_> = ci
        .lines()
        .filter_map(|line| line.strip_prefix("    name: "))
        .collect();
    assert!(!actual.is_empty(), "no CI job display names discovered");
    let jobs = ci
        .split_once("\njobs:\n")
        .unwrap()
        .1
        .lines()
        .filter(|line| line.starts_with("  ") && !line.starts_with("   ") && line.ends_with(':'))
        .count();
    assert_eq!(
        actual.len(),
        jobs,
        "each CI job must have a unique literal display name"
    );
    assert!(
        actual.iter().all(|name| !name.contains("${{")),
        "expand the name contract before adding generated check names"
    );
    let contributing = read("CONTRIBUTING.md");
    let Some((_, tail)) = contributing.split_once("checks:") else {
        panic!("missing required-check declaration")
    };
    let Some((declaration, _)) = tail.split_once("; require") else {
        panic!("unterminated required-check declaration")
    };
    let declared: BTreeSet<_> = declaration
        .split('`')
        .enumerate()
        .filter(|(index, _)| index % 2 == 1)
        .map(|(_, name)| name)
        .collect();
    assert_eq!(
        declared, actual,
        "CONTRIBUTING required checks drifted from ci.yml"
    );
}

#[test]
fn readme_corpus_count_is_derived_from_the_recorded_rankings() {
    let count = files_under("fixtures/corpus", ".rank.json").len();
    assert!(count > 0, "the real producer corpus disappeared");
    let readme = read("README.md");
    let rows: Vec<_> = readme
        .lines()
        .filter(|line| line.starts_with("| **Real corpus** |"))
        .collect();
    assert_eq!(rows.len(), 1, "missing or ambiguous README corpus row");
    assert!(
        rows[0].contains(&format!("| {count} rankings from a real")),
        "README corpus count must be {count}: {}",
        rows[0]
    );
}

#[test]
fn front_door_docs_do_not_claim_the_child_door_spawns_nothing() {
    for path in [
        "README.md",
        "CLAUDE.md",
        "CONTRIBUTING.md",
        "SECURITY.md",
        "docs/POLICY_REFERENCE.md",
    ] {
        let text = read(path).to_lowercase();
        let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
        for claim in [
            "spawns nothing",
            "never spawns",
            "spawns no child",
            "no subprocesses",
        ] {
            assert!(
                !normalized.contains(claim),
                "{path}: stale child-process claim {claim:?}"
            );
        }
    }
}

#[test]
fn every_source_markdown_document_has_an_exact_command_inventory() {
    let mut discovered = common::markdown_documents(&common::root());
    discovered.insert("justfile".to_owned());
    let inventory = read("docs/command-sources.tsv");
    let mut indexed = BTreeSet::new();
    for row in inventory.lines() {
        let (path, count) = row
            .split_once('\t')
            .expect("command inventory row needs path TAB count");
        assert!(
            count.parse::<usize>().is_ok(),
            "invalid command count for {path}"
        );
        assert!(
            indexed.insert(path.to_owned()),
            "duplicate command document {path}"
        );
    }
    assert_eq!(
        indexed, discovered,
        "every source Markdown document and justfile must be indexed; new or untracked docs cannot escape command checks"
    );
}

#[test]
fn source_walk_includes_untracked_and_nested_docs_but_excludes_reviewed_outputs() {
    let dir = common::scratch("documentation-tree");
    for path in [
        "README.md",
        "docs/new.md",
        "docs/UPPER.MD",
        "docs/target/source.md",
        "mutants.out.source.md",
        "target/generated.md",
        "mutants.out/report.md",
        "mutants.out.old/report.md",
        ".git/internal.md",
        "nested/.git/internal.md",
    ] {
        common::write(&dir, path, b"document\n");
    }
    let expected: BTreeSet<_> = [
        "README.md",
        "docs/new.md",
        "docs/UPPER.MD",
        "docs/target/source.md",
        "mutants.out.source.md",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(common::markdown_documents(&dir), expected);
}

#[test]
#[cfg(unix)]
fn source_walk_refuses_symlinks_instead_of_following_or_hiding_them() {
    let outside = common::scratch("documentation-link-target");
    let doc = common::write(&outside, "outside.md", b"document\n");
    for (name, target) in [("file.md", doc.as_path()), ("directory", outside.as_path())] {
        let dir = common::scratch(name);
        common::fixture_symlink(target, &dir.join("linked"));
        assert!(
            std::panic::catch_unwind(|| common::markdown_documents(&dir)).is_err(),
            "source link hidden or followed: {name}"
        );
    }
}
