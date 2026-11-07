//! Nothing outside doors.rs can switch off the door lints (ADR 0005 §2.7, Bastion B-C1c-01/02).
//!
//! `clippy.toml` makes `Ranking::attest`, raw `evaluate` and the effect doors disallowed methods;
//! clippy resolves every way of naming them (`tests/mint_lint.rs`). What remains is a lint attribute
//! that switches the lint off. This check reads every first-party Rust file of the CLI crate with a
//! real parser (syn), so comments, whitespace and raw identifiers never reach it, and inspects every
//! attribute by its parsed path: outer and inner, nested in `cfg_attr`, and inside macro
//! invocations. A file that does not parse fails, and so does a lint attribute whose arguments do not.
#![expect(
    clippy::disallowed_methods,
    reason = "test door: reads the CLI crate's source files"
)]

mod common;
use common::{files_under, root};
use proc_macro2::{Delimiter, TokenStream, TokenTree};
use syn::ext::IdentExt as _;
use syn::punctuated::Punctuated;
use syn::visit::Visit;
use syn::{Attribute, Meta, Path, Token};

/// Lints that would switch off `clippy::disallowed_methods` / `disallowed_types`: the groups that
/// contain them, `warnings`, and the lints themselves (any `clippy::disallowed_*`, old names too).
fn reaches_the_doors(path: &Path) -> bool {
    let segments: Vec<String> = path
        .segments
        .iter()
        .map(|s| s.ident.unraw().to_string())
        .collect();
    match segments.as_slice() {
        [one] => one == "warnings",
        [tool, lint] => {
            tool == "clippy" && (lint == "all" || lint == "style" || lint.starts_with("disallowed"))
        }
        _ => false,
    }
}

fn last(path: &Path) -> String {
    path.segments
        .last()
        .map(|s| s.ident.unraw().to_string())
        .unwrap_or_default()
}

/// What one attribute (as its `Meta`) does to the door lints: a list of offences.
fn check_meta(meta: &Meta, found: &mut Vec<String>) {
    let name = last(meta.path());
    let level = matches!(name.as_str(), "allow" | "expect" | "warn");
    if !level && name != "cfg_attr" {
        return;
    }
    let Meta::List(list) = meta else {
        if level {
            found.push(format!("{name} without a lint list"));
        }
        return;
    };
    let Ok(items) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated) else {
        found.push(format!("{name}(…) whose arguments do not parse"));
        return;
    };
    if name == "cfg_attr" {
        // cfg_attr(predicate, attr, attr, …): every attribute after the predicate applies.
        for nested in items.iter().skip(1) {
            check_meta(nested, found);
        }
        return;
    }
    for item in &items {
        if let Meta::Path(lint) = item
            && reaches_the_doors(lint)
        {
            let names: Vec<String> = lint
                .segments
                .iter()
                .map(|s| s.ident.unraw().to_string())
                .collect();
            found.push(format!("{name}({})", names.join("::")));
        }
    }
}

/// Whether the tokens of an attribute are EXACTLY a doc attribute that cannot change a lint level:
/// `doc = <literal>` or, in a macro body, `doc = $name` (one metavariable). Anything else that
/// starts with `doc` (`doc::allow(..)`, `doc = "a" allow(..)`, `doc = $($t)*`) is not admitted here
/// and goes through the full parse and lint check (Touchstone E3a v4 boundary matrix).
fn is_plain_doc(tokens: &TokenStream) -> bool {
    let trees: Vec<TokenTree> = tokens.clone().into_iter().collect();
    let doc_eq = matches!(trees.first(), Some(TokenTree::Ident(i)) if i.unraw() == "doc")
        && matches!(trees.get(1), Some(TokenTree::Punct(p)) if p.as_char() == '=');
    match trees.as_slice() {
        [_, _, TokenTree::Literal(_)] => doc_eq,
        [_, _, TokenTree::Punct(dollar), TokenTree::Ident(_)] => doc_eq && dollar.as_char() == '$',
        _ => false,
    }
}

/// `#[…]` / `#![…]` written as tokens inside a macro invocation, checked as attributes.
fn check_tokens(tokens: TokenStream, found: &mut Vec<String>) {
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    for (index, tree) in trees.iter().enumerate() {
        if let TokenTree::Group(group) = tree {
            check_tokens(group.stream(), found);
        }
        if let TokenTree::Ident(ident) = tree
            && ident.unraw() == "include"
            && matches!(trees.get(index.saturating_add(1)), Some(TokenTree::Punct(p)) if p.as_char() == '!')
        {
            found.push("include!".to_owned());
        }
        let TokenTree::Punct(pound) = tree else {
            continue;
        };
        if pound.as_char() != '#' {
            continue;
        }
        let mut next = index.saturating_add(1);
        if matches!(trees.get(next), Some(TokenTree::Punct(p)) if p.as_char() == '!') {
            next = next.saturating_add(1);
        }
        if let Some(TokenTree::Group(group)) = trees.get(next)
            && group.delimiter() == Delimiter::Bracket
            && !is_plain_doc(&group.stream())
        {
            match syn::parse2::<Meta>(group.stream()) {
                Ok(meta) => check_meta(&meta, found),
                Err(_) => found.push("an attribute in a macro that does not parse".to_owned()),
            }
        }
    }
}

struct Scan {
    found: Vec<String>,
}

impl<'ast> Visit<'ast> for Scan {
    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        check_meta(&attribute.meta, &mut self.found);
        syn::visit::visit_attribute(self, attribute);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if last(&mac.path) == "macro_rules" {
            // A macro could assemble a lint attribute from fragments no parser sees as one.
            self.found.push("macro_rules!".to_owned());
        }
        if last(&mac.path) == "include" {
            // Code from a file this check never parses (B-C1c-04); data uses include_str!/bytes!.
            self.found.push("include!".to_owned());
        }
        check_tokens(mac.tokens.clone(), &mut self.found);
        syn::visit::visit_macro(self, mac);
    }
}

/// Every offence in `source`; a file that does not parse is one offence.
fn door_lint_controls(source: &str) -> Vec<String> {
    match syn::parse_file(source) {
        Ok(file) => {
            let mut scan = Scan { found: Vec::new() };
            scan.visit_file(&file);
            scan.found
        }
        Err(e) => vec![format!("does not parse: {e}")],
    }
}

#[test]
fn the_parser_finds_every_suppression_form() {
    let forms = [
        ("plain", "#[expect(clippy::all, reason = \"x\")]\nfn f() {}"),
        (
            "block comment (B-C1c-02)",
            "#[expect(clippy /* boundary */ :: all, reason = \"x\")]\nfn f() {}",
        ),
        (
            "line comment + newline",
            "#[expect(clippy // boundary\n :: all, reason = \"x\")]\nfn f() {}",
        ),
        (
            "nested comment",
            "#[expect(clippy /* a /* b */ c */ :: all, reason = \"x\")]\nfn f() {}",
        ),
        (
            "comment inside the lint name path end",
            "#[expect(clippy:: /**/ style, reason = \"x\")]\nfn f() {}",
        ),
        (
            "raw identifier",
            "#[expect(clippy::r#all, reason = \"x\")]\nfn f() {}",
        ),
        (
            "cfg_attr",
            "#[cfg_attr(all(), expect(clippy::all, reason = \"x\"))]\nfn f() {}",
        ),
        (
            "nested cfg_attr",
            "#[cfg_attr(all(), cfg_attr(unix, expect(clippy::style, reason = \"x\")))]\nfn f() {}",
        ),
        (
            "inner attribute in a module file",
            "#![allow(warnings)]\nfn f() {}",
        ),
        (
            "inner attribute in a module",
            "mod m {\n    #![expect(clippy::all, reason = \"x\")]\n}",
        ),
        (
            "on a statement",
            "fn f() {\n    #[expect(clippy::all, reason = \"x\")]\n    let forge = 1;\n}",
        ),
        (
            "on an expression",
            "fn f() -> u8 {\n    #[expect(clippy::all, reason = \"x\")]\n    { 1 }\n}",
        ),
        (
            "the lint itself",
            "#[expect(clippy::disallowed_methods, reason = \"x\")]\nfn f() {}",
        ),
        (
            "old lint name",
            "#[expect(clippy::disallowed_method, reason = \"x\")]\nfn f() {}",
        ),
        ("warn downgrade", "#[warn(clippy::all)]\nfn f() {}"),
        (
            "leading colons",
            "#[expect(::clippy::all, reason = \"x\")]\nfn f() {}",
        ),
        (
            "among other lints",
            "#[expect(dead_code, clippy::style, reason = \"x\")]\nfn f() {}",
        ),
        (
            "inside a macro invocation",
            "some_macro! {\n    #[expect(clippy::all, reason = \"x\")]\n    fn f() {}\n}",
        ),
        ("macro_rules", "macro_rules! hide { () => {} }"),
        (
            "a metavariable attribute in a macro body",
            "some_macro! { #[$lint] fn f() {} }",
        ),
        (
            "a doc-prefixed path hiding allow",
            "some_macro! { #[doc::allow(warnings)] fn f() {} }",
        ),
        (
            "a doc literal with a trailing allow",
            "some_macro! { #[doc = \"a\" allow(warnings)] fn f() {} }",
        ),
        (
            "a doc metavariable repetition",
            "some_macro! { #[doc = $($t)*] fn f() {} }",
        ),
        (
            "unparseable lint list",
            "#[expect(clippy::all reason)]\nfn f() {}",
        ),
        ("unparseable file", "fn f( {"),
    ];
    for (name, source) in forms {
        assert!(
            !door_lint_controls(source).is_empty(),
            "missed: {name}\n{source}"
        );
    }
}

#[test]
fn the_parser_ignores_names_outside_lint_attributes() {
    for (name, source) in [
        (
            "an unrelated lint",
            "#[expect(clippy::struct_field_names, reason = \"x\")]\nfn f() {}",
        ),
        (
            "the name only in a reason",
            "#[expect(dead_code, reason = \"not clippy::all\")]\nfn f() {}",
        ),
        (
            "the name only in a comment",
            "// #[expect(clippy::all)]\n/// clippy::all\nfn f() {}",
        ),
        (
            "the name only in a string",
            "fn f() -> &'static str { \"#[expect(clippy::all)]\" }",
        ),
        (
            "a doc metavariable in a macro body",
            "some_macro! { #[doc = $doc] fn f() {} }",
        ),
        (
            "a doc literal in a macro body",
            "some_macro! { #[doc = \"text\"] fn f() {} }",
        ),
        (
            "deny is stricter, not looser",
            "#[deny(clippy::all)]\nfn f() {}",
        ),
        (
            "a string in a macro",
            "fn f() { assert!(true, \"#[allow(warnings)]\"); }",
        ),
    ] {
        assert_eq!(
            door_lint_controls(source),
            Vec::<String>::new(),
            "false positive: {name}"
        );
    }
}

#[test]
fn nothing_outside_doors_rs_can_switch_off_the_door_lints() {
    // Scanned: every source compiled into production (whatever crate or directory it lives in, B-C1c-04)
    // and every Rust file of the CLI crate. Production is decided by the compiler (B-C1c-03).
    let production = common::production_sources();
    let doors = "crates/forge-warden-cli/src/doors.rs";
    let mut scanned: std::collections::BTreeSet<String> = production.clone();
    scanned.extend(
        files_under("crates/forge-warden-cli", ".rs")
            .iter()
            .filter(|p| !p.components().any(|c| c.as_os_str() == "target"))
            .map(|p| common::relative(p)),
    );
    assert!(scanned.len() >= 10, "found only {} files", scanned.len());
    for name in scanned {
        if name == doors {
            continue;
        }
        let source = std::fs::read_to_string(root().join(&name)).unwrap();
        let shipped = production.contains(&name);
        let cli = name.starts_with("crates/forge-warden-cli/");
        let found: Vec<String> = door_lint_controls(&source)
            .into_iter()
            .filter(|f| {
                // Test code may open test doors and include fixtures; production may not.
                let test_door = f == "expect(clippy::disallowed_methods)"
                    || f == "expect(clippy::disallowed_types)"
                    || f == "include!";
                // macro_rules! is refused in the CLI crate only (core declares note codes by macro).
                let core_macro = f == "macro_rules!" && !cli;
                !((!shipped && test_door) || core_macro)
            })
            .collect();
        assert_eq!(
            found,
            Vec::<String>::new(),
            "{name} could switch off the door lints (production: {shipped})"
        );
    }
}

#[test]
fn raw_doc_name_has_the_same_literal_admission() {
    let normal: TokenStream = "doc = \"prose\"".parse().unwrap();
    let raw: TokenStream = "r#doc = \"prose\"".parse().unwrap();
    assert!(is_plain_doc(&normal));
    assert!(is_plain_doc(&raw));
}

#[test]
fn raw_attribute_names_and_paths_cannot_suppress_doors() {
    for (name, source) in [
        (
            "raw expect",
            "#[r#expect(clippy::all, reason = \"x\")] fn f() {}",
        ),
        ("raw allow", "#[r#allow(clippy::all)] fn f() {}"),
        ("raw warn", "#[r#warn(clippy::all)] fn f() {}"),
        (
            "raw path",
            "#[expect(r#clippy::r#disallowed_methods, reason = \"x\")] fn f() {}",
        ),
        (
            "raw conditional",
            concat!(
                "#[r#cfg_attr(all(), ",
                "r#allow(r#clippy::r#all))] fn f() {}"
            ),
        ),
    ] {
        assert!(!door_lint_controls(source).is_empty(), "missed: {name}");
    }
}
