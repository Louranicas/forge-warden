//! Repository invariants that no compiler lint covers. Each one closes a way bad code gets in.
//!
//! If one of these fails, the message says what to do. Do not weaken the check to make it pass:
//! changing this file is a `protect_paths` hold in `warden.toml`, so a person will see it.
#![expect(
    clippy::disallowed_methods,
    reason = "test code reading the repository: a panic is a failed test"
)]

mod common;
#[path = "../../forge-warden-core/tests/support/corpus_manifest.rs"]
mod corpus_manifest;
use common::{files_under, read, root};
use syn::ext::IdentExt as _;

/// Whether `path` (`crates/…`) is compiled into the production binary, by compiler evidence
/// (B-C1c-03): a test exemption is never decided by a directory name.
fn is_production(path: &str) -> bool {
    static PRODUCTION: std::sync::OnceLock<std::collections::BTreeSet<String>> =
        std::sync::OnceLock::new();
    PRODUCTION
        .get_or_init(common::production_sources)
        .contains(path)
}

fn rust_sources() -> Vec<(String, String)> {
    // Every Rust file under crates/, under every workspace member wherever it lives, and every source
    // compiled into production (Touchstone C1c-O3-04, B-C1c-04): nothing first-party is out of view.
    let mut names = std::collections::BTreeSet::new();
    let mut dirs = vec!["crates".to_owned()];
    dirs.extend(common::workspace_members().dirs);
    for dir in &dirs {
        for path in files_under(dir, ".rs") {
            if !path.components().any(|c| c.as_os_str() == "target") {
                names.insert(common::relative(&path));
            }
        }
    }
    names.extend(common::production_sources());
    let out: Vec<(String, String)> = names
        .into_iter()
        .map(|rel| {
            let text = read(&rel);
            (rel, text)
        })
        .collect();
    // Membership only: pinning source content would trivially kill every cargo-mutants plant.
    assert_eq!(
        corpus_manifest::check(
            "rust-sources",
            out.iter()
                .map(|(name, _)| (name.clone(), name.as_bytes().to_vec())),
        ),
        Ok(())
    );
    // Marker scanning excludes this exact scanner; manifest membership above excludes nothing.
    out.into_iter()
        .filter(|(rel, _)| rel != "crates/forge-warden-cli/tests/repo_invariants.rs")
        .collect()
}

#[test]
fn every_crate_inherits_the_workspace_lints() {
    let mut manifests = files_under("crates", "Cargo.toml");
    for dir in common::workspace_members().dirs {
        manifests.push(root().join(dir).join("Cargo.toml"));
    }
    for manifest in manifests {
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
    let mut names = common::sources().test_only;
    names.extend(rust_sources().into_iter().map(|(path, _)| path));
    names.insert("crates/forge-warden-cli/tests/repo_invariants.rs".to_owned());
    for path in names {
        let result = common::attribute_policy::expectations(&read(&path));
        assert!(result.is_ok(), "{path}: {result:?}");
    }
}

#[test]
fn only_doors_rs_opens_doors_in_production_code() {
    for (path, text) in rust_sources() {
        let is_test = !is_production(&path);
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
fn raw_defaults_are_disallowed_in_both_crates() {
    // G-04 (WP-B7, B7.1): `.unwrap_or(Outcome::Pass)` decided a verdict silently; `map_or` and
    // `get_or_insert*` pick a default the same way. A default is now chosen
    // through forge_warden_core::defaults::NamedDefault (or_fail_closed / or_fail_safe / or_display),
    // and clippy refuses the raw methods in every target of both crates (measured: method call,
    // UFCS, fn pointer, alias, use-as, closure). The configs are PARSED (Touchstone B7-O3-01): only an
    // active `disallowed-methods` entry counts, never a commented-out line that keeps the text.
    for config in ["clippy.toml", "crates/forge-warden-core/clippy.toml"] {
        let active = disallowed_method_paths(&read(config));
        for method in [
            "core::option::Option::unwrap_or",
            "core::option::Option::unwrap_or_else",
            "core::option::Option::unwrap_or_default",
            "core::result::Result::unwrap_or",
            "core::result::Result::unwrap_or_else",
            "core::result::Result::unwrap_or_default",
            "core::option::Option::map_or",
            "core::option::Option::map_or_else",
            "core::result::Result::map_or",
            "core::result::Result::map_or_else",
            "core::option::Option::get_or_insert",
            "core::option::Option::get_or_insert_with",
            "core::option::Option::get_or_insert_default",
        ] {
            assert!(
                active.contains(method),
                "{config} must actively disallow {method}"
            );
        }
    }
}

/// The `path` of every active `disallowed-methods` entry in a clippy.toml (table or string form).
fn disallowed_method_paths(text: &str) -> std::collections::BTreeSet<String> {
    // An unparseable config activates nothing, so every required method is then reported missing.
    let Ok(config) = text.parse::<toml::Table>() else {
        return std::collections::BTreeSet::new();
    };
    config
        .get("disallowed-methods")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| match entry {
            toml::Value::String(path) => Some(path.clone()),
            toml::Value::Table(table) => table
                .get("path")
                .and_then(toml::Value::as_str)
                .map(str::to_owned),
            toml::Value::Integer(_)
            | toml::Value::Float(_)
            | toml::Value::Boolean(_)
            | toml::Value::Datetime(_)
            | toml::Value::Array(_) => None,
        })
        .collect()
}

#[test]
fn only_active_config_entries_count() {
    // B7-O3-01 regression: a commented entry keeps its text but disallows nothing.
    let active = "disallowed-methods = [\n    { path = \"core::option::Option::unwrap_or\", reason = \"x\" },\n    \"core::result::Result::unwrap_or\",\n]\n";
    let commented = "disallowed-methods = [\n    # { path = \"core::option::Option::unwrap_or\", reason = \"x\" },\n]\n";
    let paths = disallowed_method_paths(active);
    assert!(paths.contains("core::option::Option::unwrap_or"));
    assert!(paths.contains("core::result::Result::unwrap_or"));
    assert!(disallowed_method_paths(commented).is_empty());
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
    // `forge-warden-core` itself: the engine tests' self dev-dependency that enables the
    // `unattested-evaluate` test door (Q-TYPES-1); a normal dependency may never enable it
    // (`no_normal_dependency_opens_the_unattested_evaluate_door`).
    const ALLOWED_DEV: [&str; 2] = ["forge-warden-core", "proptest"];
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
        if !is_production(&path) || path.ends_with("crates/forge-warden-cli/src/doors.rs") {
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
        if !is_production(&path) || path.ends_with("crates/forge-warden-core/src/authority.rs") {
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

#[test]
fn the_clippy_config_disallows_minting_and_raw_judging() {
    let config = read("clippy.toml");
    for path in [
        "path = \"forge_warden_core::authority::Ranking::attest\"",
        "path = \"forge_warden_core::testing::evaluate\"",
    ] {
        assert!(config.contains(path), "clippy.toml lost {path}");
    }
}

#[test]
fn no_normal_dependency_opens_the_unattested_evaluate_door() {
    // Q-TYPES-1: the raw engine (`forge_warden_core::testing::evaluate`, which can pass with no
    // provenance) exists only with the `unattested-evaluate` feature, and only [dev-dependencies]
    // may enable it. Judged on the RESOLVED graph, not on manifest tables (Bastion: a
    // target-specific, renamed or transitive dependency can enable it too): cargo tree over every
    // member, every target, normal and build edges only, with feature nodes.
    let core: toml::Table = toml::from_str(&read("crates/forge-warden-core/Cargo.toml")).unwrap();
    let defaults = core
        .get("features")
        .and_then(|f| f.get("default"))
        .and_then(toml::Value::as_array)
        .map(Vec::len);
    assert!(
        matches!(defaults, None | Some(0)),
        "forge-warden-core has default features: {defaults:?}"
    );
    let out = common::command(env!("CARGO"))
        .args([
            "tree",
            "--offline",
            "--locked",
            "--workspace",
            "--target",
            "all",
            "-e",
            "normal,build,features",
            "--prefix",
            "none",
        ])
        .current_dir(root())
        .output()
        .unwrap();
    let tree = String::from_utf8(out.stdout).unwrap();
    assert!(
        out.status.success() && tree.contains("forge-warden-core v"),
        "cargo tree did not run: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let opened: Vec<&str> = tree
        .lines()
        .filter(|l| l.contains("feature \"unattested-evaluate\""))
        .collect();
    assert!(
        opened.is_empty(),
        "a normal or build dependency enables unattested-evaluate: {opened:?}"
    );

    // The declarations, too, so a dependency or feature that is off by default (an optional or
    // target-specific dependency, a renamed key, a member feature forwarding to it) is refused
    // before anyone turns it on (Bastion B-QTYPES-02): every member, every dependency kind but
    // dev, every [features] value.
    let meta = common::command(env!("CARGO"))
        .args([
            "metadata",
            "--offline",
            "--locked",
            "--no-deps",
            "--format-version",
            "1",
        ])
        .current_dir(root())
        .output()
        .unwrap();
    assert!(meta.status.success(), "cargo metadata did not run");
    let meta: serde_json::Value = serde_json::from_slice(&meta.stdout).unwrap();
    let feature = "unattested-evaluate";
    let mut declared = Vec::new();
    for package in meta["packages"].as_array().unwrap() {
        let name = package["name"].as_str().unwrap();
        for dep in package["dependencies"].as_array().unwrap() {
            let enables = dep["name"] == "forge-warden-core"
                && dep["kind"].as_str() != Some("dev")
                && dep["features"]
                    .as_array()
                    .is_some_and(|f| f.iter().any(|v| v == feature));
            if enables {
                declared.push(format!("{name}: dependency {dep}"));
            }
        }
        for (key, values) in package["features"].as_object().unwrap() {
            for value in values.as_array().unwrap() {
                let text = value.as_str().unwrap();
                if text.ends_with("/unattested-evaluate") {
                    declared.push(format!("{name}: feature {key} = {text}"));
                }
            }
        }
    }
    assert!(
        declared.is_empty(),
        "unattested-evaluate is declared outside [dev-dependencies]: {declared:#?}"
    );
}

/// Inspect parsed attributes, including conditional attributes; comments and strings are inert.
fn unexplained_ignores(text: &str) -> usize {
    struct Ignores(usize);
    impl Ignores {
        fn inspect(&mut self, meta: &syn::Meta) {
            if meta
                .path()
                .get_ident()
                .is_some_and(|name| name.unraw() == "ignore")
            {
                let explained = match meta {
                    syn::Meta::NameValue(value) => matches!(&value.value,
                        syn::Expr::Lit(expr) if matches!(&expr.lit,
                            syn::Lit::Str(reason) if !reason.value().trim().is_empty())),
                    syn::Meta::Path(_) | syn::Meta::List(_) => false,
                };
                if !explained {
                    self.0 = self.0.saturating_add(1);
                }
            } else if meta
                .path()
                .get_ident()
                .is_some_and(|name| name.unraw() == "cfg_attr")
                && let syn::Meta::List(list) = meta
            {
                let Ok(nested) = list.parse_args_with(
                    syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
                ) else {
                    self.0 = usize::MAX;
                    return;
                };
                for attr in nested.iter().skip(1) {
                    self.inspect(attr);
                }
            }
        }
    }
    impl Ignores {
        fn tokens(&mut self, tokens: proc_macro2::TokenStream) {
            let mut tokens = tokens.into_iter().peekable();
            while let Some(token) = tokens.next() {
                match token {
                    proc_macro2::TokenTree::Punct(mark) if mark.as_char() == '#' => {
                        if matches!(tokens.peek(), Some(proc_macro2::TokenTree::Punct(bang)) if bang.as_char() == '!')
                        {
                            tokens.next();
                        }
                        if let Some(proc_macro2::TokenTree::Group(group)) = tokens.peek()
                            && group.delimiter() == proc_macro2::Delimiter::Bracket
                            && let Ok(meta) = syn::parse2::<syn::Meta>(group.stream())
                        {
                            self.inspect(&meta);
                        }
                    }
                    proc_macro2::TokenTree::Group(group) => self.tokens(group.stream()),
                    proc_macro2::TokenTree::Ident(_)
                    | proc_macro2::TokenTree::Literal(_)
                    | proc_macro2::TokenTree::Punct(_) => {}
                }
            }
        }
    }
    impl<'ast> syn::visit::Visit<'ast> for Ignores {
        fn visit_macro(&mut self, mac: &'ast syn::Macro) {
            self.tokens(mac.tokens.clone());
            syn::visit::visit_macro(self, mac);
        }

        fn visit_attribute(&mut self, attr: &'ast syn::Attribute) {
            self.inspect(&attr.meta);
            syn::visit::visit_attribute(self, attr);
        }
    }
    let Ok(parsed) = syn::parse_file(text) else {
        return usize::MAX;
    };
    let mut visitor = Ignores(0);
    syn::visit::Visit::visit_file(&mut visitor, &parsed);
    visitor.0
}

#[test]
fn ignored_tests_require_an_explicit_reason() {
    let mut names = common::sources().test_only;
    names.extend(common::production_sources());
    names.extend(rust_sources().into_iter().map(|(path, _)| path));
    // Unlike the legacy text scans, parsed attributes cannot confuse fixture strings with code.
    // Include this file explicitly even if compiler source discovery changes.
    names.insert("crates/forge-warden-cli/tests/repo_invariants.rs".to_owned());
    for path in names {
        assert_eq!(
            unexplained_ignores(&read(&path)),
            0,
            "{path}: ignored tests require a nonempty #[ignore = \"reason\"]"
        );
    }
}

#[test]
fn ignore_reason_check_reads_conditional_attributes_and_ignores_prose() {
    for bad in [
        "#[r#ignore]",
        "#[r#cfg_attr(test, r#ignore)]",
        "#[cfg_attr(test, r#cfg_attr(unix, r#ignore))]",
        "#[ignore]",
        "#[ignore = \" \"]",
        "#[cfg_attr(test, ignore)]",
        "#[cfg_attr(test, cfg_attr(unix, ignore))]",
    ] {
        assert_eq!(
            unexplained_ignores(&format!("{bad} fn probe() {{}}")),
            1,
            "{bad}"
        );
    }
    assert_eq!(
        unexplained_ignores("#[ignore = \"requires external hardware\"] fn probe() {}"),
        0
    );
    assert_eq!(unexplained_ignores("// #[ignore]\nfn probe() {}"), 0);
    assert_eq!(
        unexplained_ignores("#[r#ignore = \"hardware\"] fn probe() {}"),
        0
    );
    assert_eq!(
        unexplained_ignores("macro_rules! probe { () => { #[r#ignore] fn probe() {} } }"),
        1
    );
    assert_eq!(
        unexplained_ignores("macro_rules! probe { () => { #[ignore] fn probe() {} } }"),
        1
    );
}

#[test]
fn suppression_and_ignore_scanners_match_the_shared_corpus() {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(&read("fixtures/attribute-policy.json")).unwrap();
    corpus_manifest::check(
        "attribute-cases",
        cases.iter().map(|case| {
            (
                case["name"].as_str().unwrap().to_owned(),
                serde_json::to_vec(case).unwrap(),
            )
        }),
    )
    .unwrap();
    for case in cases {
        let source = case
            .get("source")
            .and_then(serde_json::Value::as_str)
            .unwrap();
        let expected = case.get("expectations").unwrap();
        let actual = common::attribute_policy::expectations(source);
        if expected.is_null() {
            assert!(actual.is_err(), "{case}: {actual:?}");
        } else {
            let pairs: Vec<(String, String)> = serde_json::from_value(expected.clone()).unwrap();
            assert_eq!(actual, Ok(pairs), "{case}");
        }
        let ignored = case
            .get("unexplained_ignores")
            .and_then(serde_json::Value::as_u64)
            .unwrap();
        assert_eq!(
            u64::try_from(unexplained_ignores(source)).unwrap(),
            ignored,
            "{case}"
        );
    }
}

/// Reject fixture-only compiler switches in operational build surfaces. Normalize shell/YAML/
/// TOML separators so split argv arrays and encoded rustflags cannot hide the two switches.
fn fixture_compiler_switch(source: &str) -> bool {
    let compact: String = source
        .chars()
        .filter(|ch| {
            !ch.is_whitespace()
                && !ch.is_control()
                && !matches!(ch, '\'' | '"' | ',' | '[' | ']' | '\\')
        })
        .collect();
    (compact.contains("--cfgfw_fixture_cli") || compact.contains("--cfg=fw_fixture_cli"))
        || compact.contains("--cfgtest")
        || compact.contains("--cfg=test")
}

#[test]
fn production_build_routes_cannot_enable_fixture_cli() {
    let mut paths = vec![root().join("justfile")];
    for directory in ["scripts", ".github", ".cargo", ".githooks", ".claude"] {
        if root().join(directory).exists() {
            paths.extend(files_under(directory, ""));
        }
    }
    for path in paths {
        if path.is_file() {
            let bytes = std::fs::read(&path).unwrap();
            if let Ok(source) = std::str::from_utf8(&bytes) {
                assert!(
                    !fixture_compiler_switch(source),
                    "fixture compiler switch in operational build surface {}",
                    path.display()
                );
            }
        }
    }
    for source in [
        "cargo rustc -- --cfg test",
        "rustflags = [\"--cfg\", \"test\"]",
        "RUSTFLAGS='--cfg=test'",
        "CARGO_ENCODED_RUSTFLAGS=--cfg\u{1f}test",
        "cargo rustc -- --cfg fw_fixture_cli",
        "RUSTFLAGS='--cfg=fw_fixture_cli'",
    ] {
        assert!(fixture_compiler_switch(source), "missed {source:?}");
    }
    assert!(!fixture_compiler_switch(
        "cargo build --release\nRUSTFLAGS='-D warnings'"
    ));
}

fn named_call(expression: &syn::Expr, expected: &str) -> bool {
    matches!(expression, syn::Expr::Call(call)
        if matches!(call.func.as_ref(), syn::Expr::Path(path)
            if path.path.segments.last().is_some_and(|part| part.ident == expected)))
}

fn final_expression(block: &syn::Block) -> Option<&syn::Expr> {
    if let syn::Stmt::Expr(expression, None) = block.stmts.last()? {
        Some(expression)
    } else {
        None
    }
}

/// Secondary shape check only: two immutable result holders feed one shared view and the
/// immediately following report. Verdict's absent Clone implementation is the compiler rung
/// against copying a judged view; Q-TYPES-2 separately makes all verdict fields private.
fn read_only_judgement(source: &str) -> bool {
    let Ok(file) = syn::parse_file(source) else {
        return false;
    };
    let Some(check) = file.items.iter().find_map(|item| {
        if let syn::Item::Fn(function) = item {
            (function.sig.ident == "check").then_some(function)
        } else {
            None
        }
    }) else {
        return false;
    };
    let Some(syn::Stmt::Local(binding)) = check.block.stmts.iter().rev().nth(1) else {
        return false;
    };
    if !matches!(&binding.pat, syn::Pat::Type(pattern)
        if matches!(pattern.pat.as_ref(), syn::Pat::Ident(name) if name.ident == "verdict" && name.mutability.is_none())
        && matches!(pattern.ty.as_ref(), syn::Type::Reference(reference) if reference.mutability.is_none()
            && matches!(reference.elem.as_ref(), syn::Type::Path(path) if path.path.is_ident("Verdict"))))
    {
        return false;
    }
    let Some(initializer) = &binding.init else {
        return false;
    };
    let syn::Expr::If(branches) = initializer.expr.as_ref() else {
        return false;
    };
    if !matches!(final_expression(&branches.then_branch), Some(syn::Expr::Reference(reference))
        if reference.mutability.is_none()
            && matches!(reference.expr.as_ref(), syn::Expr::Path(path) if path.path.is_ident("authority")))
        || !assigned_evaluation(&branches.then_branch, "authority", "evaluate_authority")
    {
        return false;
    }
    let Some((_, otherwise)) = &branches.else_branch else {
        return false;
    };
    let syn::Expr::Block(block) = otherwise.as_ref() else {
        return false;
    };
    if !matches!(final_expression(&block.block), Some(syn::Expr::MethodCall(borrow))
        if borrow.method == "verdict" && borrow.args.is_empty()
            && matches!(borrow.receiver.as_ref(), syn::Expr::Path(path) if path.path.is_ident("advisory")))
        || !assigned_evaluation(&block.block, "advisory", "evaluate_advisory")
    {
        return false;
    }
    let Some(syn::Expr::Call(report)) = final_expression(&check.block) else {
        return false;
    };
    if !matches!(report.func.as_ref(), syn::Expr::Path(path) if path.path.is_ident("report"))
        || !matches!(report.args.first(), Some(syn::Expr::Path(path)) if path.path.is_ident("verdict"))
    {
        return false;
    }
    let mut visitor = JudgementReferences::default();
    syn::visit::Visit::visit_file(&mut visitor, &file);
    visitor.evaluators == 2 && !visitor.renamed
}

fn assigned_evaluation(block: &syn::Block, holder: &str, evaluator: &str) -> bool {
    matches!(block.stmts.iter().rev().nth(1), Some(syn::Stmt::Expr(syn::Expr::Assign(assignment), Some(_)))
        if matches!(assignment.left.as_ref(), syn::Expr::Path(path) if path.path.is_ident(holder))
            && named_call(&assignment.right, evaluator))
}

#[derive(Default)]
struct JudgementReferences {
    evaluators: usize,
    renamed: bool,
}

impl<'ast> syn::visit::Visit<'ast> for JudgementReferences {
    fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
        if path.path.segments.last().is_some_and(|part| {
            matches!(
                part.ident.to_string().as_str(),
                "evaluate_authority" | "evaluate_advisory"
            )
        }) {
            self.evaluators = self.evaluators.saturating_add(1);
        }
        syn::visit::visit_expr_path(self, path);
    }
    fn visit_use_rename(&mut self, rename: &'ast syn::UseRename) {
        self.renamed |= matches!(
            rename.ident.to_string().as_str(),
            "evaluate_authority" | "evaluate_advisory"
        );
        syn::visit::visit_use_rename(self, rename);
    }
}

#[test]
fn cli_borrows_judged_verdicts_directly_and_never_takes_ownership() {
    let source = read("crates/forge-warden-cli/src/main.rs");
    assert!(
        read_only_judgement(&source),
        "CLI evaluator results must be captured directly as &Verdict and reported by shared reference"
    );
    for broken in [
        source.replace("let verdict: &Verdict = if", "let mut verdict = if"),
        source.replace(
            "let verdict: &Verdict = if",
            "let verdict: &mut Verdict = if",
        ),
        source.replace(
            "evaluate_authority(&policy, &checked, copy, seals, today)",
            "rewrite(evaluate_authority(&policy, &checked, copy, seals, today))",
        ),
        format!("{source}\nfn extra() {{ let f = evaluate_authority; }}"),
        format!("{source}\nuse forge_warden_core::authority::evaluate_authority as alias;"),
        source.replace(
            "report(verdict, args.json)",
            "report(&rewritten, args.json)",
        ),
        source.replace("report(verdict, args.json)", "let mut rewritten = (*verdict).clone(); for note in &mut rewritten.notes { note.detail.push_str(\"; post-evaluation edit\"); } let verdict = &rewritten; report(verdict, args.json)"),

    ] {
        assert!(
            !read_only_judgement(&broken),
            "judged-value boundary regression escaped"
        );
    }
}
