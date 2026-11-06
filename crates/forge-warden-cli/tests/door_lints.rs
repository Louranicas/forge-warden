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

/// `#[…]` / `#![…]` written as tokens inside a macro invocation, checked as attributes.
fn check_tokens(tokens: TokenStream, found: &mut Vec<String>) {
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    for (index, tree) in trees.iter().enumerate() {
        if let TokenTree::Group(group) = tree {
            check_tokens(group.stream(), found);
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
    let files = files_under("crates/forge-warden-cli", ".rs");
    assert!(files.len() >= 10, "found only {} files", files.len());
    let doors = root().join("crates/forge-warden-cli/src/doors.rs");
    let production = common::production_sources();
    for path in files {
        if path.components().any(|c| c.as_os_str() == "target") || path == doors {
            continue;
        }
        let shown = common::relative(&path);
        let source = std::fs::read_to_string(&path).unwrap();
        // Production is what the compiler built into the binary (B-C1c-03), never a path guess.
        let test_code = !production.contains(&shown);
        // Tests may open test doors (expect disallowed_methods / disallowed_types); nothing else.
        let found: Vec<String> = door_lint_controls(&source)
            .into_iter()
            .filter(|f| {
                !(test_code
                    && (f == "expect(clippy::disallowed_methods)"
                        || f == "expect(clippy::disallowed_types)"))
            })
            .collect();
        assert_eq!(
            found,
            Vec::<String>::new(),
            "{shown} could switch off the door lints"
        );
    }
}
