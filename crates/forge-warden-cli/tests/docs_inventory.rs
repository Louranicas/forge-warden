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
    // contract: required display names remain literal. Only the two explicitly named internal
    // mutation jobs are excluded; check_ci pins their complete aggregate dependency contract.
    let ci = read(".github/workflows/ci.yml");
    let mut actual: BTreeSet<_> = ci
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
        actual.remove("plan mutation shards"),
        "mutation planner missing"
    );
    assert!(
        actual.remove("mutation shard ${{ matrix.index }}"),
        "mutation workers missing"
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

const ADR_DEPLOYMENT_NOTICE: &str =
    "Accepted = design contract, not deployment; see [ADR status](../ADR_STATUS.md).";

fn adr_number(text: &str) -> bool {
    text.len() == 4 && text.bytes().all(|b| b.is_ascii_digit()) && text != "0000"
}

fn deployment_errors(number: &str, status: &AdrStatus, text: &str) -> Vec<String> {
    let mut errors = Vec::new();
    let deployment: Vec<_> = text
        .lines()
        .filter(|line| line.trim().starts_with("Owner-deployment:"))
        .collect();
    let required = match deployment.as_slice() {
        ["Owner-deployment: Required"] => Some(true),
        ["Owner-deployment: Not-required"] => Some(false),
        _ => None,
    };
    if required.is_none()
        || (matches!(number, "0006" | "0007" | "0008" | "0009" | "0010" | "0011")
            && required != Some(true))
    {
        errors.push(format!(
            "ADR_DEPLOYMENT: {number} needs one reviewed owner-deployment classification"
        ));
    }
    if *status == AdrStatus::Accepted
        && required == Some(true)
        && text
            .lines()
            .filter(|line| *line == ADR_DEPLOYMENT_NOTICE)
            .count()
            != 1
    {
        errors.push(format!(
            "ADR_DEPLOYMENT: {number} must distinguish accepted design from deployment"
        ));
    }
    errors
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
        errors.extend(deployment_errors(number, &status, text));
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
            (
                "0001".to_owned(),
                format!("{old}\nOwner-deployment: Not-required"),
            ),
            (
                "0002".to_owned(),
                format!("{new}\nOwner-deployment: Not-required"),
            ),
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

fn glossary_exemptions(
    source: &str,
    documents: &BTreeSet<String>,
) -> Result<std::collections::BTreeMap<String, String>, String> {
    let value: toml::Value = toml::from_str(source).map_err(|e| e.to_string())?;
    let table = value
        .as_table()
        .ok_or("GLOSSARY: expected exemption table")?;
    if table.len() != 1 {
        return Err("GLOSSARY: unknown exemption keys".to_owned());
    }
    let entries = value
        .get("exemption")
        .and_then(toml::Value::as_array)
        .ok_or("GLOSSARY: missing reviewed exemptions")?;
    let mut exempt = std::collections::BTreeMap::new();
    for entry in entries {
        let fields = entry.as_table().ok_or("GLOSSARY: invalid exemption")?;
        if fields.len() != 3
            || fields
                .keys()
                .any(|k| !["path", "kind", "reason"].contains(&k.as_str()))
        {
            return Err("GLOSSARY: unknown exemption fields".to_owned());
        }
        let path = entry
            .get("path")
            .and_then(toml::Value::as_str)
            .ok_or("GLOSSARY: missing exemption path")?;
        let kind = entry
            .get("kind")
            .and_then(toml::Value::as_str)
            .ok_or("GLOSSARY: missing exemption kind")?;
        let reason = entry
            .get("reason")
            .and_then(toml::Value::as_str)
            .ok_or("GLOSSARY: missing exemption reason")?;
        if !documents.contains(path)
            || exempt.insert(path.to_owned(), kind.to_owned()).is_some()
            || !["dated_releases", "dated_evidence", "quoted_terms"].contains(&kind)
            || reason.trim().len() < 20
        {
            return Err("GLOSSARY: stale, duplicate or unreviewed exemption".to_owned());
        }
    }
    Ok(exempt)
}

fn dated_history_heading(heading: &str, kind: &str) -> bool {
    let version = |v: &str| {
        let pieces: Vec<_> = v.split('.').collect();
        pieces.len() == 3
            && pieces
                .iter()
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    };
    let pair = match kind {
        "dated_releases" => heading
            .strip_prefix("## [")
            .and_then(|s| s.split_once("] - ")),
        "dated_evidence" => heading
            .strip_prefix("## ")
            .and_then(|s| s.strip_suffix(')'))
            .and_then(|s| s.rsplit_once(" (")),
        _ => None,
    };
    let Some((label, date)) = pair else {
        return false;
    };
    let label_ok = version(label) || (kind == "dated_evidence" && label == "Independent review");
    label_ok && iso_date(date)
}

/// Whether `date` is `YYYY-MM-DD` with a month in 1..=12 and a day in 1..=31.
fn iso_date(date: &str) -> bool {
    let parts: Vec<_> = date.split('-').collect();
    parts.len() == 3
        && parts
            .iter()
            .zip([4, 2, 2])
            .all(|(p, n)| p.len() == n && p.bytes().all(|b| b.is_ascii_digit()))
        && parts
            .get(1)
            .and_then(|s| s.parse::<u8>().ok())
            .is_some_and(|m| (1..=12).contains(&m))
        && parts
            .get(2)
            .and_then(|s| s.parse::<u8>().ok())
            .is_some_and(|d| (1..=31).contains(&d))
}

fn glossary_current_text(source: &str, kind: Option<&str>) -> String {
    let Some(kind) = kind else {
        return source.to_owned();
    };
    if kind == "quoted_terms" {
        return String::new();
    }
    let mut retained = String::new();
    let mut start = 0;
    let mut historical = false;
    let mut depth = 0_usize;
    for (event, range) in pulldown_cmark::Parser::new(source).into_offset_iter() {
        match event {
            pulldown_cmark::Event::Start(tag) => {
                if depth == 0
                    && matches!(
                        tag,
                        pulldown_cmark::Tag::Heading {
                            level: pulldown_cmark::HeadingLevel::H1
                                | pulldown_cmark::HeadingLevel::H2,
                            ..
                        }
                    )
                {
                    if !historical {
                        let Some(part) = source.get(start..range.start) else {
                            return source.to_owned();
                        };
                        retained.push_str(part);
                    }
                    start = range.start;
                    let Some(section) = source.get(start..range.end) else {
                        return source.to_owned();
                    };
                    let Some(heading) = section.lines().next() else {
                        return source.to_owned();
                    };
                    historical = dated_history_heading(heading, kind);
                }
                depth = depth.saturating_add(1);
            }
            pulldown_cmark::Event::End(_) => depth = depth.saturating_sub(1),
            pulldown_cmark::Event::Text(_)
            | pulldown_cmark::Event::Code(_)
            | pulldown_cmark::Event::InlineMath(_)
            | pulldown_cmark::Event::DisplayMath(_)
            | pulldown_cmark::Event::Html(_)
            | pulldown_cmark::Event::InlineHtml(_)
            | pulldown_cmark::Event::FootnoteReference(_)
            | pulldown_cmark::Event::SoftBreak
            | pulldown_cmark::Event::HardBreak
            | pulldown_cmark::Event::Rule
            | pulldown_cmark::Event::TaskListMarker(_) => {}
        }
    }
    if !historical {
        let Some(part) = source.get(start..) else {
            return source.to_owned();
        };
        retained.push_str(part);
    }
    retained
}

#[test]
fn only_dated_top_level_history_sections_are_exempt_from_glossary_lint() {
    for (kind, dated) in [
        ("dated_releases", "## [0.1.0] - 2026-11-05"),
        ("dated_evidence", "## 0.1.0 (2026-11-05)"),
        ("dated_evidence", "## Independent review (2026-11-05)"),
    ] {
        let text = format!(
            "Preamble soft pass.\n\n## [Unreleased]\n\nCurrent soft pass.\n\n{dated}\n\nHistorical soft pass.\n\n### Details\n\nOld soft pass.\n\n## Not verified here\n\nPending soft pass.\n\n{dated}\n\nAgain historical.\n\n# Current overview\n\nOverview soft pass.\n"
        );
        let current = glossary_current_text(&text, Some(kind));
        assert!(current.contains("Preamble soft pass"));
        assert!(current.contains("Current soft pass"));
        assert!(current.contains("Pending soft pass"));
        assert!(current.contains("Overview soft pass"));
        assert!(!current.contains("Historical soft pass"));
        assert!(!current.contains("Old soft pass"));
    }
    for fake in [
        "## [Unreleased]",
        "## [0.1.0] - soon",
        "## [0.1.0] - 2026-99-00",
        "```md\n## [0.1.0] - 2026-11-05\n```",
        "> ## [0.1.0] - 2026-11-05",
        "- ## [0.1.0] - 2026-11-05",
    ] {
        let text = format!("{fake}\n\nA soft pass is current.\n");
        assert!(
            exact_phrase(
                &glossary_current_text(&text, Some("dated_releases")),
                "soft pass"
            ),
            "{fake}"
        );
    }
}

#[test]
fn deployment_and_glossary_exemptions_have_explicit_refusal_controls() {
    let doc = |body: &str| vec![("0006".to_owned(), body.to_owned())];
    let good = format!("Status: Accepted\nOwner-deployment: Required\n{ADR_DEPLOYMENT_NOTICE}");
    assert_eq!(adr_errors(&doc(&good)), Vec::<String>::new());
    for bad in [
        good.replace(ADR_DEPLOYMENT_NOTICE, ""),
        good.replace("Required", "Not-required"),
        format!("{good}\nOwner-deployment: Required"),
    ] {
        assert!(!adr_errors(&doc(&bad)).is_empty(), "{bad}");
    }
    let documents = BTreeSet::from(["history.md".to_owned()]);
    let exemption = "[[exemption]]\npath='history.md'\nkind='dated_releases'\nreason='A dated historical record, preserved verbatim.'";
    assert_eq!(
        glossary_exemptions(exemption, &documents).unwrap(),
        std::collections::BTreeMap::from([("history.md".to_owned(), "dated_releases".to_owned())])
    );
    for bad in [
        exemption.replace("history.md", "*.md"),
        exemption.replace("history.md", "missing.md"),
        format!("{exemption}\n{exemption}"),
        exemption.replace("kind='dated_releases'", "kind='any'"),
    ] {
        assert!(glossary_exemptions(&bad, &documents).is_err());
    }
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
    let documents = common::markdown_documents(&common::root());
    let exemptions =
        glossary_exemptions(&read("docs/glossary-exemptions.toml"), &documents).unwrap();
    for path in ["README.md", "CONTRIBUTING.md", "CLAUDE.md"] {
        assert!(
            read(path).contains("docs/GLOSSARY.md"),
            "{path} must link the canonical glossary"
        );
    }
    for path in &documents {
        let source = glossary_current_text(&read(path), exemptions.get(path).map(String::as_str));
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
    for ((record, expected), level) in records
        .iter()
        .zip(["pass", "fail", "pass"])
        .zip(["hold", "fail", "hold"])
    {
        assert_eq!(record["verdict"], level);
        assert_eq!(record["rules"][0]["id"], "first-rule-private-files");
        assert_eq!(record["rules"][0]["outcome"], expected);
        assert_eq!(record["notes"][0]["code"], "rank_unverified");
        assert_eq!(record["seals"]["provenance"], "supplied");
    }
    assert_eq!(records.len(), 3);
}

/// G-40 (R-15): a backticked token shaped like a test name (lowercase, four or more `_`-joined
/// words) in documentation must name a `fn` or `def` that exists, or a lint in the workspace lint
/// tables; a backticked `tests/…rs` path must exist. Read from every source Markdown document and
/// every Rust doc comment. Lines under a dated history heading keep the names they recorded.
fn test_shaped(token: &str) -> bool {
    let words: Vec<&str> = token.split('_').collect();
    words.len() >= 4
        && token.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
        && words.iter().all(|w| {
            !w.is_empty()
                && w.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

/// Every name that follows `keyword` (e.g. `fn `) at a word start in `text`.
fn names_after(text: &str, keyword: &str, out: &mut BTreeSet<String>) {
    for (at, _) in text.match_indices(keyword) {
        let before = text.get(..at).and_then(|s| s.chars().next_back());
        if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let Some(rest) = at
            .checked_add(keyword.len())
            .and_then(|from| text.get(from..))
        else {
            continue;
        };
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() {
            out.insert(name);
        }
    }
}

/// The Rust `fn` and Python `def` names in the workspace, and the lints in `cargo`'s workspace
/// tables. A missing lint table adds no names, so more tokens are refused, never fewer.
fn known_names(cargo: &toml::Value) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for path in files_under("crates", ".rs") {
        if !path.components().any(|c| c.as_os_str() == "target") {
            names_after(&read(&common::relative(&path)), "fn ", &mut names);
        }
    }
    for dir in ["scripts", "plants", ".claude/hooks"] {
        for path in files_under(dir, ".py") {
            names_after(&read(&common::relative(&path)), "def ", &mut names);
        }
    }
    let tables = cargo
        .get("workspace")
        .and_then(|w| w.get("lints"))
        .and_then(toml::Value::as_table);
    for table in tables
        .into_iter()
        .flat_map(|t| t.values().filter_map(toml::Value::as_table))
    {
        names.extend(table.keys().cloned());
    }
    names
}

/// The workspace member directories `cargo` lists, or `None` when the table is missing (the
/// caller, a test, then fails rather than resolving nothing).
fn workspace_members(cargo: &toml::Value) -> Option<Vec<String>> {
    cargo
        .get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(toml::Value::as_array)
        .map(|members| {
            members
                .iter()
                .filter_map(toml::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
}

/// `## <n>. Amendment (<label>, YYYY-MM-DD)…`, the dated form of an ADR amendment.
fn adr_amendment_heading(heading: &str) -> bool {
    heading
        .strip_prefix("## ")
        .and_then(|s| s.split_once(". Amendment ("))
        .is_some_and(|(number, rest)| {
            !number.is_empty()
                && number.bytes().all(|b| b.is_ascii_digit())
                && rest
                    .split_once(')')
                    .and_then(|(inside, _)| inside.rsplit_once(", "))
                    .is_some_and(|(_, date)| iso_date(date))
        })
}

fn history_heading(heading: &str) -> bool {
    dated_history_heading(heading, "dated_releases")
        || dated_history_heading(heading, "dated_evidence")
        || adr_amendment_heading(heading)
}

/// Whether a backticked `tests/…rs` reference names a file: as written from the root, or under a crate.
fn test_path_exists(root: &std::path::Path, members: &[String], reference: &str) -> bool {
    if reference.starts_with("crates/") {
        return root.join(reference).is_file();
    }
    members
        .iter()
        .any(|member| root.join(member).join(reference).is_file())
}

/// Each stale reference in `documents` (name, text, whether it is Markdown with headings).
fn stale_test_references(
    root: &std::path::Path,
    members: &[String],
    documents: &[(String, String, bool)],
    known: &BTreeSet<String>,
) -> Vec<String> {
    let mut stale = Vec::new();
    for (name, text, markdown) in documents {
        let mut historical = false;
        for (index, line) in text.lines().enumerate() {
            if *markdown && line.starts_with("## ") {
                historical = history_heading(line);
            }
            if historical {
                continue;
            }
            for token in line.split('`').skip(1).step_by(2) {
                let number = index.saturating_add(1);
                if test_shaped(token) && !known.contains(token) {
                    stale.push(format!(
                        "{name}:{number}: `{token}` names no fn, def or lint"
                    ));
                }
                let path_like = std::path::Path::new(token)
                    .extension()
                    .is_some_and(|e| e == "rs")
                    && token.contains("tests/")
                    && !token.contains([' ', '*']);
                if path_like && !test_path_exists(root, members, token) {
                    stale.push(format!("{name}:{number}: `{token}` names no file"));
                }
            }
        }
    }
    stale
}

/// The Markdown documents, and the doc-comment lines of every Rust source, as checked documents.
fn documentation() -> Vec<(String, String, bool)> {
    let root = common::root();
    let mut documents: Vec<(String, String, bool)> = common::markdown_documents(&root)
        .into_iter()
        .map(|name| {
            let text = read(&name);
            (name, text, true)
        })
        .collect();
    for path in files_under("crates", ".rs") {
        if path.components().any(|c| c.as_os_str() == "target") {
            continue;
        }
        let comments: Vec<String> = read(&common::relative(&path))
            .lines()
            .map(|line| {
                let trimmed = line.trim_start();
                if trimmed.starts_with("///") || trimmed.starts_with("//!") {
                    trimmed.to_owned()
                } else {
                    String::new()
                }
            })
            .collect();
        documents.push((common::relative(&path), comments.join("\n"), false));
    }
    documents
}

#[test]
fn docs_name_only_tests_and_files_that_exist() {
    let cargo: toml::Value = toml::from_str(&read("Cargo.toml")).unwrap();
    let stale = stale_test_references(
        &common::root(),
        &workspace_members(&cargo).unwrap(),
        &documentation(),
        &known_names(&cargo),
    );
    assert_eq!(
        stale,
        Vec::<String>::new(),
        "G-40: documentation names tests or files that do not exist"
    );
}

#[test]
fn the_stale_name_rule_refuses_planted_names_and_keeps_history() {
    let root = common::root();
    let cargo: toml::Value = toml::from_str(&read("Cargo.toml")).unwrap();
    let known = known_names(&cargo);
    let members = workspace_members(&cargo).unwrap();
    let doc = |text: &str| vec![("plant.md".to_owned(), text.to_owned(), true)];
    for (case, text) in [
        (
            "a renamed test",
            "See `duplicate_ranked_paths_are_refused`.",
        ),
        (
            "an invented test",
            "Pinned by `a_test_that_does_not_exist_here`.",
        ),
        ("a missing test file", "See `tests/no_such_file.rs`."),
        (
            "a missing crate test file",
            "See `crates/forge-warden-cli/tests/no_such_file.rs`.",
        ),
        (
            "an undated amendment is not history",
            "## 9. Amendment (G4)\n`a_test_that_does_not_exist_here`",
        ),
        (
            "history ends at the next heading",
            "## [0.1.0] - 2026-01-01\n## Next\n`a_test_that_does_not_exist_here`",
        ),
    ] {
        assert!(
            !stale_test_references(&root, &members, &doc(text), &known).is_empty(),
            "missed: {case}"
        );
    }
    for (case, text) in [
        (
            "an existing test",
            "See `every_file_door_refuses_a_fifo_at_once_not_waited_on`.",
        ),
        (
            "a Python test helper",
            "See `test_removed_expectation_refuses`.",
        ),
        ("a workspace lint", "Denied: `wildcard_enum_match_arm`."),
        ("a short name", "`read_file` and `run_child`"),
        (
            "a glob is not a file reference",
            "Every `crates/*/tests/*.rs` file.",
        ),
        (
            "an existing test file",
            "See `tests/door_lints.rs` and `crates/forge-warden-core/tests/coverage.rs`.",
        ),
        (
            "a dated release",
            "## [0.1.0] - 2026-01-01\n`a_test_that_does_not_exist_here`",
        ),
        (
            "a dated ADR amendment",
            "## 9. Amendment (G4, 2026-11-10): x\n`a_test_that_does_not_exist_here`",
        ),
    ] {
        assert_eq!(
            stale_test_references(&root, &members, &doc(text), &known),
            Vec::<String>::new(),
            "false positive: {case}"
        );
    }
    let comment = vec![(
        "x.rs".to_owned(),
        "/// `a_test_that_does_not_exist_here`\n## [0.1.0] - 2026-01-01\n/// `another_test_that_does_not_exist`"
            .to_owned(),
        false,
    )];
    assert_eq!(
        stale_test_references(&root, &members, &comment, &known).len(),
        2,
        "doc comments have no history headings"
    );
}

/// `(id, kind, severity)` of every rule in a policy file, in file order.
fn policy_rules(path: &str) -> Result<Vec<(String, String, String)>, String> {
    let policy: toml::Value = toml::from_str(&read(path)).map_err(|e| format!("{path}: {e}"))?;
    let rules = policy
        .get("rule")
        .and_then(toml::Value::as_array)
        .ok_or(format!("{path}: no [[rule]] table"))?;
    rules
        .iter()
        .map(|rule| {
            let field = |name: &str| {
                rule.get(name)
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned)
                    .ok_or(format!("{path}: a rule has no string `{name}`"))
            };
            Ok((field("id")?, field("kind")?, field("severity")?))
        })
        .collect()
}

/// A Markdown table row of three backticked cells, `| `a` | `b` | `c` |`, as its three values.
fn backticked_row(line: &str) -> Option<(String, String, String)> {
    let cells: Vec<&str> = line.split('|').map(str::trim).collect();
    let ["", a, b, c, ""] = cells.as_slice() else {
        return None;
    };
    let unquote = |cell: &str| {
        cell.strip_prefix('`')
            .and_then(|rest| rest.strip_suffix('`'))
            .map(str::to_owned)
    };
    Some((unquote(a)?, unquote(b)?, unquote(c)?))
}

/// A rule line of the README's sample output, `  [STATUS] id (kind, severity)`.
fn sample_rule(line: &str) -> Option<(String, String, String)> {
    let rest = line.trim_start().strip_prefix('[')?;
    let (status, rest) = rest.split_once(']')?;
    if !["PASS", "FAIL", "HOLD", "WAIVED"].contains(&status.trim()) {
        return None;
    }
    let (id, rest) = rest.trim().split_once(" (")?;
    let (kind, severity) = rest.strip_suffix(')')?.split_once(", ")?;
    Some((id.to_owned(), kind.to_owned(), severity.to_owned()))
}

/// EVIDENCE-0.2.0 item 11: a rule cannot be weakened unnoticed. In the freeze review a seed that
/// lowered a rule from fail to hold went unseen by every reviewer; now the policy and the table in
/// `docs/POLICY_REFERENCE.md` must change together, and the README's sample output must name the
/// kinds and severities of the starter policy it was made with.
#[test]
fn documented_rule_severities_equal_the_policies() {
    let reference = read("docs/POLICY_REFERENCE.md");
    let (_, section) = reference
        .split_once("## This repository's own policy (`warden.toml`)")
        .expect("POLICY_REFERENCE has the own-policy section");
    let section = section.split("\n## ").next().unwrap();
    let documented: Vec<_> = section.lines().filter_map(backticked_row).collect();
    assert_eq!(
        documented,
        policy_rules("warden.toml").unwrap(),
        "docs/POLICY_REFERENCE.md's own-policy table and warden.toml disagree: change both together"
    );
    let starter = policy_rules("examples/starter.warden.toml").unwrap();
    let sampled: Vec<_> = read("README.md").lines().filter_map(sample_rule).collect();
    assert!(
        !sampled.is_empty(),
        "the README sample output has rule lines"
    );
    for rule in &sampled {
        assert!(
            starter.contains(rule),
            "README sample {rule:?} does not match examples/starter.warden.toml"
        );
    }
}
