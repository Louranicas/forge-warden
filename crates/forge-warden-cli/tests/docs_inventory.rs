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
        "mutants.outline/source.md",
        "mutants.out.backup/source.md",
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
        "mutants.outline/source.md",
        "mutants.out.backup/source.md",
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

#[derive(Clone, Debug, PartialEq)]
enum AdrStatus {
    Proposed,
    Accepted,
    Deprecated,
    Superseded(String),
}

fn adr_number(text: &str) -> bool {
    text.len() == 4 && text.bytes().all(|b| b.is_ascii_digit()) && text != "0000"
}

fn adr_errors(documents: &[(String, String)]) -> Vec<String> {
    let mut errors = Vec::new();
    let mut statuses = std::collections::BTreeMap::new();
    let mut replacements = std::collections::BTreeMap::new();
    for (number, text) in documents {
        if !adr_number(number) || statuses.contains_key(number) {
            errors.push(format!("ADR_STATUS: invalid/duplicate ADR number {number}"));
            continue;
        }
        let lines: Vec<_> = text
            .lines()
            .filter(|line| line.trim().replace("**", "").starts_with("Status:"))
            .collect();
        let status = match lines.as_slice() {
            [line] => match line.strip_prefix("Status: ") {
                Some("Proposed") => Some(AdrStatus::Proposed),
                Some("Accepted") => Some(AdrStatus::Accepted),
                Some("Deprecated") => Some(AdrStatus::Deprecated),
                Some(value) => value
                    .strip_prefix("Superseded-by-")
                    .filter(|n| adr_number(n))
                    .map(|n| AdrStatus::Superseded(n.to_owned())),
                None => None,
            },
            _ => None,
        };
        let Some(status) = status else {
            errors.push(format!(
                "ADR_STATUS: {number} needs exactly one canonical status line"
            ));
            continue;
        };
        statuses.insert(number.clone(), status);
        let claim_lines: Vec<_> = text
            .lines()
            .filter(|line| line.trim().replace("**", "").starts_with("Supersedes:"))
            .collect();
        let claims: Vec<_> = claim_lines
            .iter()
            .filter_map(|line| line.strip_prefix("Supersedes: "))
            .collect();
        if claims.len() != claim_lines.len() {
            errors.push(format!(
                "ADR_STATUS: noncanonical Supersedes field in {number}"
            ));
        }
        if claims.len() > 1 {
            errors.push(format!(
                "ADR_STATUS: duplicate Supersedes field in {number}"
            ));
        }
        let mut previous = BTreeSet::new();
        for claim in claims {
            for old in claim.split(',').map(str::trim) {
                if !adr_number(old) || !previous.insert(old.to_owned()) {
                    errors.push(format!(
                        "ADR_STATUS: invalid/duplicate replacement in {number}"
                    ));
                }
            }
        }
        replacements.insert(number.clone(), previous);
    }
    for (number, status) in &statuses {
        if let AdrStatus::Superseded(target) = status {
            if target <= number || !statuses.contains_key(target) {
                errors.push(format!(
                    "ADR_STATUS: {number} has dangling/backward replacement {target}"
                ));
            }
            if !replacements
                .get(target)
                .is_some_and(|previous| previous.contains(number))
            {
                errors.push(format!(
                    "ADR_STATUS: {number} replacement lacks reciprocal claim"
                ));
            }
        }
        if let Some(previous) = replacements.get(number) {
            for old in previous {
                if !matches!(status, AdrStatus::Accepted | AdrStatus::Superseded(_))
                    || old >= number
                    || statuses.get(old) != Some(&AdrStatus::Superseded(number.clone()))
                {
                    errors.push(format!(
                        "ADR_STATUS: {number} claims {old} without retiring its current status"
                    ));
                }
            }
        }
    }
    errors
}

#[test]
fn every_adr_has_one_current_status_and_reciprocal_replacements() {
    let docs: Vec<_> = files_under("docs/adr", ".md")
        .into_iter()
        .map(|path| {
            let relative = common::relative(&path);
            let filename = path.file_name().unwrap().to_str().unwrap();
            let number = filename.split_once('-').unwrap().0;
            (number.to_owned(), read(&relative))
        })
        .collect();
    assert_ne!(docs, Vec::<(String, String)>::new());
    assert_eq!(adr_errors(&docs), Vec::<String>::new());
}

#[test]
fn adr_status_fixtures_reject_ambiguous_stale_and_dangling_decisions() {
    let pair = |old: &str, new: &str| {
        vec![
            ("0001".to_owned(), old.to_owned()),
            ("0002".to_owned(), new.to_owned()),
        ]
    };
    for text in [
        "",
        "Status: accepted",
        "Status: Accepted, yesterday",
        "Status: Accepted\nStatus: Proposed",
        "**Status:** Accepted",
    ] {
        assert!(
            !adr_errors(&pair(text, "Status: Accepted")).is_empty(),
            "{text}"
        );
    }
    for (old, new) in [
        ("Status: Accepted", "Status: Accepted\nSupersedes: 0001"),
        ("Status: Superseded-by-0003", "Status: Accepted"),
        (
            "Status: Superseded-by-0002",
            "Status: Proposed\nSupersedes: 0001",
        ),
        (
            "Status: Superseded-by-0002",
            "Status: Deprecated\nSupersedes: 0001",
        ),
        (
            "Status: Superseded-by-0002",
            "Status: Accepted\nSupersedes: 0001, 0001",
        ),
        (
            "Status: Superseded-by-0002",
            "Status: Superseded-by-0001\nSupersedes: 0001",
        ),
    ] {
        assert!(!adr_errors(&pair(old, new)).is_empty(), "{old}; {new}");
    }
    for (old, new) in [
        ("Status: Accepted", "Status: Proposed"),
        ("Status: Deprecated", "Status: Accepted"),
        (
            "Status: Superseded-by-0002",
            "Status: Accepted\nSupersedes: 0001",
        ),
    ] {
        assert_eq!(adr_errors(&pair(old, new)), Vec::<String>::new());
    }
}

fn glossary_rows(text: &str) -> Result<Vec<(String, String, Vec<String>)>, String> {
    let mut rows = Vec::new();
    let mut concepts = BTreeSet::new();
    for line in text.lines().filter(|line| line.starts_with('|')) {
        let cells: Vec<_> = line.split('|').map(str::trim).collect();
        let ["", concept, canonical, forbidden, _meaning, ""] = cells.as_slice() else {
            return Err("GLOSSARY: malformed row".to_owned());
        };
        if *concept == "Concept" || concept.starts_with("---") {
            continue;
        }
        if canonical.is_empty() || !concepts.insert((*concept).to_owned()) {
            return Err("GLOSSARY: empty/duplicate concept".to_owned());
        }
        let phrases: Vec<String> = forbidden
            .split(';')
            .map(str::trim)
            .filter(|p| *p != "-")
            .map(str::to_owned)
            .collect();
        if phrases.iter().any(String::is_empty) {
            return Err("GLOSSARY: empty forbidden phrase".to_owned());
        }
        rows.push(((*concept).to_owned(), (*canonical).to_owned(), phrases));
    }
    Ok(rows)
}

fn exact_phrase(text: &str, phrase: &str) -> bool {
    let normalized = text
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    normalized.match_indices(phrase).any(|(start, found)| {
        let before = normalized.get(..start).and_then(|s| s.chars().next_back());
        let after = normalized
            .get(start.saturating_add(found.len())..)
            .and_then(|s| s.chars().next());
        before.is_none_or(|c| !c.is_alphanumeric() && c != '_')
            && after.is_none_or(|c| !c.is_alphanumeric() && c != '_')
    })
}

#[test]
fn front_door_terms_follow_the_reviewed_glossary() {
    let glossary = read("docs/GLOSSARY.md");
    let rows = glossary_rows(&glossary).unwrap();
    let actual: std::collections::BTreeMap<_, _> = rows
        .iter()
        .map(|(concept, term, _)| (concept.as_str(), term.as_str()))
        .collect();
    let required = std::collections::BTreeMap::from([
        ("document", "verdict"),
        ("level", "verdict level"),
        ("rule", "rule outcome"),
        ("effect", "effect door"),
        ("review", "hold"),
        ("uncertainty", "note"),
        ("digest", "seal"),
        ("origin", "provenance"),
        ("authoritative", "authority mode"),
        ("informative", "advisory mode"),
        ("producer data", "ranking"),
    ]);
    assert_eq!(actual, required);
    for path in ["README.md", "CONTRIBUTING.md", "CLAUDE.md"] {
        let source = read(path);
        assert!(
            source.contains("docs/GLOSSARY.md"),
            "{path} must link the canonical glossary"
        );
        for (concept, canonical, forbidden) in &rows {
            for phrase in forbidden {
                assert!(
                    !exact_phrase(&source, phrase),
                    "GLOSSARY: {path}: {phrase:?}; use {canonical} for {concept}"
                );
            }
        }
    }
    for (_, _, phrases) in rows {
        for phrase in phrases {
            assert!(exact_phrase(&format!("A {phrase} is stale."), &phrase));
            assert!(exact_phrase(&phrase.to_ascii_uppercase(), &phrase));
            assert!(exact_phrase(&phrase.replace(' ', "\n"), &phrase));
            assert!(!exact_phrase(&format!("x{phrase}x"), &phrase));
        }
    }
    assert!(!exact_phrase(
        "Outcome is a Rust type; the verdict level is hold.",
        "result type"
    ));
}

#[test]
fn first_rule_walkthrough_runs_its_documented_exercise_offline() {
    let contributing = read("CONTRIBUTING.md");
    let (before, after) = contributing
        .split_once("<!-- first-rule-exercise:start -->")
        .unwrap();
    assert!(!before.contains("first-rule-exercise:start"));
    let (exercise, tail) = after
        .split_once("<!-- first-rule-exercise:end -->")
        .unwrap();
    assert!(!tail.contains("first-rule-exercise:"));
    let mut script = String::new();
    let mut blocks = 0;
    let mut in_shell = false;
    for event in pulldown_cmark::Parser::new(exercise) {
        if let pulldown_cmark::Event::Start(pulldown_cmark::Tag::CodeBlock(kind)) = event {
            if let pulldown_cmark::CodeBlockKind::Fenced(language) = kind {
                assert_eq!(language.as_ref(), "bash");
                blocks += 1;
                in_shell = true;
            }
        } else if let pulldown_cmark::Event::End(pulldown_cmark::TagEnd::CodeBlock) = event {
            in_shell = false;
        } else if let pulldown_cmark::Event::Text(body) = event
            && in_shell
        {
            script.push_str(&body);
        }
    }
    assert_eq!(blocks, 1, "one executable exercise is required");
    let (output, records) = common::run_first_rule_exercise(&script);
    assert!(output.status.success(), "walkthrough failed: {output:?}");
    let records: Vec<serde_json::Value> = records
        .iter()
        .map(|text| serde_json::from_str(text).unwrap())
        .collect();
    for (record, expected) in records.iter().zip(["pass", "fail", "pass"]) {
        assert_eq!(record["verdict"], expected);
        assert_eq!(record["rules"][0]["id"], "first-rule-private-files");
        assert_eq!(record["rules"][0]["outcome"], expected);
        assert_eq!(record["notes"], serde_json::json!([]));
        assert_eq!(record["seals"]["provenance"], "produced");
    }
    assert_eq!(records.len(), 3);
}
