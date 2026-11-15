//! Nothing outside the door modules (`common::DOOR_FILES`, formerly the single file doors.rs) can
//! switch off the door lints (ADR 0005 §2.7, Bastion B-C1c-01/02).
//!
//! `clippy.toml` makes `Ranking::attest`, the raw-engine test door and the effect doors disallowed methods;
//! clippy resolves every way of naming them (`tests/mint_lint.rs`). What remains is a lint attribute
//! that switches the lint off. This check reads every first-party Rust file of the CLI crate with a
//! real parser (syn), so comments, whitespace and raw identifiers never reach it, and inspects every
//! attribute by its parsed path: outer and inner, nested in `cfg_attr`, and inside macro
//! invocations. A file that does not parse fails, and so does a lint attribute whose arguments do not.
//! Production code also may not use `include!` (B-C1c-04) or `#[path]` (Q-DOOR-PATH): either would
//! compile a file this check judges under another name. Inside a door module, `#[path]` may only
//! bring test support into a test-only module.
//! Shipped code also may not write an expression in an attribute (Q37): a derive macro may place
//! it in generated code under its own `allow`, where clippy says nothing about a disallowed call.
#![expect(
    clippy::disallowed_methods,
    reason = "test door: reads the CLI crate's source files"
)]

mod common;
use common::attribute_policy::is_plain_doc;
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

/// The offence a `path` attribute records (Q-DOOR-PATH).
const PATH_ATTRIBUTE: &str = "path attribute";

/// What one attribute (as its `Meta`) does to the door lints: a list of offences.
fn check_meta(meta: &Meta, found: &mut Vec<String>) {
    let name = last(meta.path());
    if name == "path" {
        // Q-DOOR-PATH: `#[path]` compiles a file under another module path, so a protected door
        // file could be compiled a second time outside the door modules, where these fences,
        // which judge by file, would not see it. Only test code and one test-only door form
        // (`a_door_module_names_only_test_support_by_path`) may use it.
        found.push(PATH_ATTRIBUTE.to_owned());
        return;
    }
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

/// The offence an expression in an attribute records (Q37).
const ATTRIBUTE_EXPRESSION: &str = "an expression in an attribute";

/// Attributes that may carry an expression, each written exactly so (whitespace aside). Reviewed:
/// the first builds a clap `ArgGroup` from literals; the second formats a count and a list of the
/// variant's own field; the last two name the crate's own size constant. None reaches an effect.
const ADMITTED_ATTRIBUTE_EXPRESSIONS: [&str; 4] = [
    r#"command(group(ArgGroup::new("mode").required(true).args(["ranker","advisory"])))"#,
    r#"error("policy refused with {} issue(s):\n{}",.0.len(),IssueList(.0))"#,
    r#"error("policy is {0} bytes, over the {max} byte budget",max=crate::MAX_POLICY_BYTES)"#,
    r#"error("ranking is {0} bytes, over the {max} byte budget",max=crate::MAX_INPUT_BYTES)"#,
];

/// `text` without whitespace (inside string literals too: spacing never makes an expression).
fn squeeze(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Attributes whose `name(value)` arguments are expressions (clap's derive reads them so).
const CLAP_ATTRIBUTES: [&str; 5] = ["command", "arg", "clap", "group", "value"];

/// Q37: whether an attribute's arguments hold anything but names, paths, literals, lists of those
/// and `name = value` pairs. A derive macro may put an argument's expression into code it
/// generates under `#[allow(clippy::style)]`, which contains `disallowed_methods`: a call written
/// there reaches an effect and clippy reports nothing (measured on clap's derive). So a call, a
/// method call, a macro call, a block, a closure and every operator are refused wherever they
/// stand in an attribute. A value (after `=`, or inside a clap `name(value)`) must be a literal,
/// `true`, `false`, `None` or a list of those: a bare name or path there may be a function the macro
/// calls for us, and `value_parser = std::env::var` is as much an effect as a call written out
/// (Bastion). `depth` counts enclosing parentheses; `literal_only` carries a value position into
/// a bracketed list.
fn holds_an_expression(tokens: TokenStream, depth: usize, clap: bool, literal_only: bool) -> bool {
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    let mut value = false;
    for (index, tree) in trees.iter().enumerate() {
        let before = index.checked_sub(1).and_then(|at| trees.get(at));
        let strict = literal_only || value || (clap && depth >= 1);
        match tree {
            TokenTree::Punct(punct) => match punct.as_char() {
                '=' => value = true,
                ',' => value = false,
                '-' => {}
                ':' if !strict => {}
                _ => return true,
            },
            TokenTree::Ident(ident) => {
                let name = ident.unraw().to_string();
                if strict && !["true", "false", "None"].contains(&name.as_str()) {
                    return true;
                }
            }
            TokenTree::Group(group) => {
                let inner = group.stream();
                let (nested, nested_clap, nested_literal) = match group.delimiter() {
                    Delimiter::Brace => return true,
                    Delimiter::Parenthesis => {
                        let name = match before {
                            Some(TokenTree::Ident(ident)) => Some(ident.unraw().to_string()),
                            _ => None,
                        };
                        let named = name.is_some();
                        let path_end = index
                            .checked_sub(2)
                            .and_then(|at| trees.get(at))
                            .is_some_and(
                                |t| matches!(t, TokenTree::Punct(p) if p.as_char() == ':'),
                            );
                        if !named || path_end || value || (clap && depth >= 1) {
                            return true;
                        }
                        // A clap attribute met inside another attribute (`cfg_attr`) starts here.
                        let starts_clap =
                            !clap && name.is_some_and(|n| CLAP_ATTRIBUTES.contains(&n.as_str()));
                        if starts_clap {
                            (0, true, false)
                        } else {
                            (depth.saturating_add(1), clap, false)
                        }
                    }
                    Delimiter::Bracket | Delimiter::None => (depth, clap, strict),
                };
                if holds_an_expression(inner, nested, nested_clap, nested_literal) {
                    return true;
                }
            }
            TokenTree::Literal(_) => {}
        }
    }
    false
}

/// Q37 for one attribute: an offence unless it holds no expression or is admitted verbatim.
fn check_expression(meta: &Meta, found: &mut Vec<String>) {
    let list = match meta {
        Meta::Path(_) => return,
        Meta::List(list) => list,
        Meta::NameValue(pair) => {
            // `name = value` on the attribute itself (`#[doc = "…"]`): only a literal.
            if !matches!(pair.value, syn::Expr::Lit(_)) {
                found.push(ATTRIBUTE_EXPRESSION.to_owned());
            }
            return;
        }
    };
    let name = last(meta.path());
    let clap = CLAP_ATTRIBUTES.contains(&name.as_str());
    if !holds_an_expression(list.tokens.clone(), 0, clap, false) {
        return;
    }
    let written = squeeze(&format!("{name}({})", list.tokens));
    if !ADMITTED_ATTRIBUTE_EXPRESSIONS
        .iter()
        .any(|admitted| squeeze(admitted) == written)
    {
        found.push(ATTRIBUTE_EXPRESSION.to_owned());
    }
}

/// `#[…]` / `#![…]` written as tokens inside a macro invocation, checked for expressions. An
/// attribute there that does not parse is already an offence of `check_tokens`.
fn check_expression_tokens(tokens: TokenStream, found: &mut Vec<String>) {
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    for (index, tree) in trees.iter().enumerate() {
        if let TokenTree::Group(group) = tree {
            check_expression_tokens(group.stream(), found);
        }
        if !matches!(tree, TokenTree::Punct(p) if p.as_char() == '#') {
            continue;
        }
        let mut next = index.saturating_add(1);
        if matches!(trees.get(next), Some(TokenTree::Punct(p)) if p.as_char() == '!') {
            next = next.saturating_add(1);
        }
        if let Some(TokenTree::Group(group)) = trees.get(next)
            && group.delimiter() == Delimiter::Bracket
            && let Ok(meta) = syn::parse2::<Meta>(group.stream())
        {
            check_expression(&meta, found);
        }
    }
}

struct Expressions {
    found: Vec<String>,
}

impl<'ast> Visit<'ast> for Expressions {
    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        check_expression(&attribute.meta, &mut self.found);
        syn::visit::visit_attribute(self, attribute);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        check_expression_tokens(mac.tokens.clone(), &mut self.found);
        syn::visit::visit_macro(self, mac);
    }
}

/// Q37: one offence per attribute in `source` that holds an expression; a file that does not
/// parse is one offence.
fn attribute_expressions(source: &str) -> Vec<String> {
    match syn::parse_file(source) {
        Ok(file) => {
            let mut scan = Expressions { found: Vec::new() };
            scan.visit_file(&file);
            scan.found
        }
        Err(e) => vec![format!("does not parse: {e}")],
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

/// Q37 on the tree: no source compiled into production, outside the door modules, writes an
/// expression in an attribute, except the admitted ones.
#[test]
fn shipped_code_writes_no_expression_in_an_attribute() {
    let production = common::production_sources();
    assert!(
        production.len() >= 10,
        "found only {} files",
        production.len()
    );
    let mut admitted_seen = 0_usize;
    for name in production {
        if common::is_door_file(&name) {
            continue;
        }
        let source = std::fs::read_to_string(root().join(&name)).unwrap();
        assert_eq!(
            attribute_expressions(&source),
            Vec::<String>::new(),
            "{name}: an attribute holds an expression (Q37)"
        );
        let squeezed = squeeze(&source);
        admitted_seen = admitted_seen.saturating_add(
            ADMITTED_ATTRIBUTE_EXPRESSIONS
                .iter()
                .filter(|admitted| squeezed.contains(&squeeze(admitted)))
                .count(),
        );
    }
    assert_eq!(
        admitted_seen,
        ADMITTED_ATTRIBUTE_EXPRESSIONS.len(),
        "every admitted expression is still in the tree, once; drop the ones that are gone"
    );
}

/// Q37: every way of writing an expression in an attribute is an offence for the shipped-file scan.
#[test]
fn the_parser_finds_every_attribute_expression_form() {
    let forms = [
        (
            "a method call in a clap value (the measured escape)",
            "#[command(after_help = std::env::var(\"X\").is_ok().to_string())]\nstruct Cli;",
        ),
        (
            "a path call after =",
            "#[arg(long, default_value_os_t = std::env::var_os(\"X\"))]\nstruct Cli;",
        ),
        (
            "an imported function called after =",
            "#[arg(long, default_value_os_t = v(\"X\"))]\nstruct Cli;",
        ),
        (
            "clap's name(value) form holding a call",
            "#[command(after_help(v(\"X\")))]\nstruct Cli;",
        ),
        (
            "a std function named as a value, not called (Bastion)",
            "#[arg(long, value_parser = std::fs::read_to_string)]\nstruct Cli;",
        ),
        (
            "an imported function named as a value",
            "#[arg(long, value_parser = v)]\nstruct Cli;",
        ),
        (
            "a function named in clap's name(value) form",
            "#[arg(long, value_parser(std::env::var))]\nstruct Cli;",
        ),
        (
            "a name inside a list value",
            "#[arg(long, value_parser = [v, \"b\"])]\nstruct Cli;",
        ),
        (
            "a block",
            "#[arg(long, default_value_t = { 1 })]\nstruct Cli;",
        ),
        (
            "a macro call",
            "#[command(about = concat!(\"a\", \"b\"))]\nstruct Cli;",
        ),
        (
            "a closure",
            "#[arg(long, value_parser = |s: &str| s.len())]\nstruct Cli;",
        ),
        (
            "an operator",
            "#[arg(long, default_value_t = 1 + 1)]\nstruct Cli;",
        ),
        (
            "through cfg_attr",
            "#[cfg_attr(all(), command(after_help = v(\"X\")))]\nstruct Cli;",
        ),
        (
            "through cfg_attr, clap's name(value) form",
            "#[cfg_attr(all(), command(after_help(v(\"X\"))))]\nstruct Cli;",
        ),
        (
            "inside a macro invocation",
            "some_macro! {\n    #[command(after_help = v(\"X\"))]\n    struct Cli;\n}",
        ),
        (
            "a non-literal value on the attribute itself",
            "#[doc = concat!(\"a\", \"b\")]\nstruct Cli;",
        ),
        (
            "the admitted group expression with one word changed",
            "#[command(group(ArgGroup::new(\"mode\").required(false).args([\"ranker\", \"advisory\"])))]\nstruct Cli;",
        ),
    ];
    for (name, source) in forms {
        assert!(
            attribute_expressions(source)
                .iter()
                .any(|f| f == ATTRIBUTE_EXPRESSION),
            "missed: {name}\n{source}"
        );
    }
}

/// Q37: what an attribute may hold without being an expression.
#[test]
fn the_parser_admits_names_literals_and_lists_of_literals() {
    let plain = [
        (
            "literals, flags and a list of literals",
            "#[arg(long, value_name = \"FILE\", default_value = \"/usr/bin/git\", value_parser = [\"a\", \"b\"])]\nstruct Cli;",
        ),
        (
            "true, false, None and clap's name(literal) form",
            "#[arg(long, required = true, hide = false, long_help = None, value_name(\"FILE\"))]\nstruct Cli;",
        ),
        (
            "nested lists in a non-clap attribute",
            "#[cfg_attr(all(test, unix), derive(Debug), expect(dead_code, reason = \"x\"))]\nstruct Cli;",
        ),
        (
            "a negative number and a doc string",
            "/// text\n#[arg(long, default_value_t = -1)]\nstruct Cli;",
        ),
        (
            "the admitted group expression, spaced differently",
            "#[command(group(ArgGroup::new(\"mode\").required(true).args([\"ranker\",\n    \"advisory\"])))]\nstruct Cli;",
        ),
    ];
    for (name, source) in plain {
        assert!(
            attribute_expressions(source).is_empty(),
            "refused: {name}\n{source}"
        );
    }
}

/// Q-DOOR-PATH: every way of writing a `path` attribute is an offence for the shipped-file scan.
#[test]
fn the_parser_finds_every_path_attribute_form() {
    let forms = [
        (
            "a path attribute (Q-DOOR-PATH)",
            "#[path = \"doors/file.rs\"]\nmod again;",
        ),
        (
            "a path attribute through cfg_attr",
            "#[cfg_attr(all(), path = \"doors/file.rs\")]\nmod again;",
        ),
        (
            "a path attribute through nested cfg_attr",
            "#[cfg_attr(all(), cfg_attr(unix, path = \"doors/file.rs\"))]\nmod again;",
        ),
        (
            "a path attribute inside a macro invocation",
            "some_macro! {\n    #[path = \"doors/file.rs\"]\n    mod again;\n}",
        ),
        (
            "a path attribute on an inline module",
            "#[path = \"doors\"]\nmod again {\n    mod file;\n}",
        ),
    ];
    for (name, source) in forms {
        assert!(
            door_lint_controls(source)
                .iter()
                .any(|f| f == PATH_ATTRIBUTE),
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
        (
            "path as an identifier, a reason or a doc",
            "/// see path = \"doors/file.rs\"\n#[expect(dead_code, reason = \"path = x\")]\nfn path() -> u8 { let path = 1; path }",
        ),
        ("a module without a path attribute", "mod file;"),
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
    let mut scanned: std::collections::BTreeSet<String> = production.clone();
    scanned.extend(
        files_under("crates/forge-warden-cli", ".rs")
            .iter()
            .filter(|p| !p.components().any(|c| c.as_os_str() == "target"))
            .map(|p| common::relative(p)),
    );
    assert!(scanned.len() >= 10, "found only {} files", scanned.len());
    for name in scanned {
        if common::is_door_file(&name) {
            continue;
        }
        let source = std::fs::read_to_string(root().join(&name)).unwrap();
        let shipped = production.contains(&name);
        let cli = name.starts_with("crates/forge-warden-cli/");
        let found: Vec<String> = door_lint_controls(&source)
            .into_iter()
            .filter(|f| {
                // Test code may open test doors and include fixtures or support files; production
                // may not.
                let test_door = f == "expect(clippy::disallowed_methods)"
                    || f == "expect(clippy::disallowed_types)"
                    || f == "include!"
                    || f == PATH_ATTRIBUTE;
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

/// Inside the door modules, two narrower rules than "doors live here" (Q-DOOR), judged over all of
/// them together. Q2-02: only
/// `open_regular` may open a named file, so it is the only production item under which any lint
/// attribute names `clippy::disallowed_types` (`std::fs::OpenOptions`). Q2-03: production code
/// joins no thread (a zero-argument `.join()` call, or any path ending in `join`, which covers
/// `JoinHandle::join` through an alias); results come back through channels read with a timeout.
///
/// Attributes are read by `check_meta` (raw names normalised, every level, nested `cfg_attr`) on
/// every item, impl item and statement, and inside macro tokens. An item is skipped as test-only
/// only if its `cfg` predicate implies `test` (`test`; `all(..)` with an implying member; `any(..)`
/// whose members all imply it). `not(..)`, `any(test, unix)` and anything else count as production
/// (Bastion's Q-DOOR boundary probes).
#[derive(Default)]
struct DoorShape {
    current: Vec<String>,
    types_doors: Vec<String>,
    joins: Vec<String>,
}

/// Whether a `cfg` predicate is true only when `test` is set.
fn implies_test(predicate: &Meta) -> bool {
    match predicate {
        Meta::Path(path) => path.segments.len() == 1 && last(path) == "test",
        Meta::List(list) => {
            let Ok(members) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            else {
                return false;
            };
            match (list.path.segments.len(), last(&list.path).as_str()) {
                (1, "all") => members.iter().any(implies_test),
                (1, "any") => !members.is_empty() && members.iter().all(implies_test),
                _ => false,
            }
        }
        Meta::NameValue(_) => false,
    }
}

/// Whether an item's own attributes compile it only under `test`.
fn is_test_only(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().segments.len() == 1
            && last(a.path()) == "cfg"
            && a.meta.require_list().is_ok_and(|list| {
                list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
                    .is_ok_and(|args| args.len() == 1 && args.iter().all(implies_test))
            })
    })
}

impl DoorShape {
    /// Walk an item under `name`, unless it is compiled only under `test`.
    fn scoped(&mut self, attrs: &[Attribute], name: String, walk: impl FnOnce(&mut Self)) {
        if is_test_only(attrs) {
            return;
        }
        self.current.push(name);
        walk(self);
        self.current.pop();
    }

    fn context(&self) -> String {
        self.current.join("::")
    }

    fn note_types_door(&mut self, found: &[String]) {
        if found
            .iter()
            .any(|f| f.ends_with("(clippy::disallowed_types)"))
        {
            let here = self.context();
            if !self.types_doors.contains(&here) {
                self.types_doors.push(here);
            }
        }
    }

    /// `.join()` and `…::join` written as tokens inside a macro invocation.
    fn scan_tokens_for_joins(&mut self, tokens: TokenStream) {
        let trees: Vec<TokenTree> = tokens.into_iter().collect();
        for (index, tree) in trees.iter().enumerate() {
            if let TokenTree::Group(group) = tree {
                self.scan_tokens_for_joins(group.stream());
            }
            let TokenTree::Ident(ident) = tree else {
                continue;
            };
            if ident.unraw() != "join" {
                continue;
            }
            let before = index.checked_sub(1).and_then(|i| trees.get(i));
            let after = trees.get(index.saturating_add(1));
            let dot_call = matches!(before, Some(TokenTree::Punct(p)) if p.as_char() == '.')
                && matches!(after, Some(TokenTree::Group(g))
                    if g.delimiter() == Delimiter::Parenthesis && g.stream().is_empty());
            let path = matches!(before, Some(TokenTree::Punct(p)) if p.as_char() == ':');
            if dot_call || path {
                let here = self.context();
                self.joins.push(here);
            }
        }
    }
}

impl<'ast> Visit<'ast> for DoorShape {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        self.scoped(&item.attrs, format!("mod {}", item.ident.unraw()), |s| {
            syn::visit::visit_item_mod(s, item);
        });
    }
    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        self.scoped(&item.attrs, item.sig.ident.unraw().to_string(), |s| {
            syn::visit::visit_item_fn(s, item);
        });
    }
    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        self.scoped(&item.attrs, "impl".to_owned(), |s| {
            syn::visit::visit_item_impl(s, item);
        });
    }
    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        self.scoped(&item.attrs, item.sig.ident.unraw().to_string(), |s| {
            syn::visit::visit_impl_item_fn(s, item);
        });
    }
    fn visit_item_const(&mut self, item: &'ast syn::ItemConst) {
        self.scoped(&item.attrs, format!("const {}", item.ident.unraw()), |s| {
            syn::visit::visit_item_const(s, item);
        });
    }
    fn visit_item_static(&mut self, item: &'ast syn::ItemStatic) {
        self.scoped(&item.attrs, format!("static {}", item.ident.unraw()), |s| {
            syn::visit::visit_item_static(s, item);
        });
    }
    fn visit_item_struct(&mut self, item: &'ast syn::ItemStruct) {
        self.scoped(&item.attrs, format!("struct {}", item.ident.unraw()), |s| {
            syn::visit::visit_item_struct(s, item);
        });
    }
    fn visit_item_enum(&mut self, item: &'ast syn::ItemEnum) {
        self.scoped(&item.attrs, format!("enum {}", item.ident.unraw()), |s| {
            syn::visit::visit_item_enum(s, item);
        });
    }
    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        self.scoped(&item.attrs, "use".to_owned(), |s| {
            syn::visit::visit_item_use(s, item);
        });
    }
    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        let mut found = Vec::new();
        check_meta(&attribute.meta, &mut found);
        self.note_types_door(&found);
        syn::visit::visit_attribute(self, attribute);
    }
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        let mut found = Vec::new();
        check_tokens(mac.tokens.clone(), &mut found);
        self.note_types_door(&found);
        self.scan_tokens_for_joins(mac.tokens.clone());
        syn::visit::visit_macro(self, mac);
    }
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method.unraw() == "join" && call.args.is_empty() {
            let here = self.context();
            self.joins.push(here);
        }
        syn::visit::visit_expr_method_call(self, call);
    }
    fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
        if path.path.segments.len() > 1 && last(&path.path) == "join" {
            let here = self.context();
            self.joins.push(here);
        }
        syn::visit::visit_expr_path(self, path);
    }
}

fn door_shape(source: &str) -> DoorShape {
    let mut shape = DoorShape::default();
    match syn::parse_file(source) {
        Ok(file) => shape.visit_file(&file),
        // An unparseable file passes neither check: it names an impossible type door.
        Err(e) => shape.types_doors.push(format!("does not parse: {e}")),
    }
    shape
}

#[test]
fn the_door_shape_scanner_sees_joins_and_type_doors_and_skips_only_tests() {
    let shape = door_shape(
        r#"
        #[expect(clippy::disallowed_types, reason = "door: open_regular")]
        fn open_regular() {}
        #[r#expect(r#clippy::r#disallowed_types, reason = "raw")]
        fn raw_names() {}
        #[cfg_attr(unix, expect(clippy::disallowed_types, reason = "conditional"))]
        fn conditional() {}
        fn inner() { #![allow(clippy::disallowed_types)] }
        fn statement() { #[expect(clippy::disallowed_types, reason = "s")] let _o = 1; }
        #[cfg(not(test))]
        fn not_test() { let h = std::thread::spawn(|| ()); let _ = h.join(); }
        #[cfg(any(test, unix))]
        fn any_test_or_unix() { let _ = std::thread::spawn(|| ()).r#join(); }
        fn ufcs(h: std::thread::JoinHandle<()>) { let _ = std::thread::JoinHandle::join(h); }
        fn alias(h: std::thread::JoinHandle<()>) { use std::thread::JoinHandle as H; let _ = H::join(h); }
        fn in_macro(h: std::thread::JoinHandle<()>) { let _ = format!("{:?}", h.join()); }
        fn paths() { let _ = std::path::Path::new("a").join("b"); let _ = ["a"].join(","); }
        #[cfg(test)]
        mod tests {
            #[expect(clippy::disallowed_types, reason = "test")]
            fn t() { let _ = std::thread::spawn(|| ()).join(); }
        }
        #[cfg(all(unix, test))]
        fn all_unix_test() { let _ = std::thread::spawn(|| ()).join(); }
        #[cfg(any(test, all(test, unix)))]
        fn any_all_test() { let _ = std::thread::spawn(|| ()).join(); }
        "#,
    );
    assert_eq!(
        shape.types_doors,
        [
            "open_regular",
            "raw_names",
            "conditional",
            "inner",
            "statement"
        ]
    );
    assert_eq!(
        shape.joins,
        ["not_test", "any_test_or_unix", "ufcs", "alias", "in_macro"]
    );
}

#[test]
fn only_a_predicate_that_implies_test_is_test_only() {
    for (cfg, test_only) in [
        ("#[cfg(test)]", true),
        ("#[r#cfg(r#test)]", true),
        ("#[cfg(all(unix, test))]", true),
        ("#[cfg(any(test, all(test, unix)))]", true),
        ("#[cfg(not(test))]", false),
        ("#[cfg(any(test, unix))]", false),
        ("#[cfg(not(not(test)))]", false),
        ("#[cfg(feature = \"test\")]", false),
        ("#[cfg_attr(test, cfg(test))]", false),
        ("#[cfg(any())]", false),
    ] {
        let item: syn::ItemFn = syn::parse_str(&format!("{cfg} fn f() {{}}")).unwrap();
        assert_eq!(is_test_only(&item.attrs), test_only, "{cfg}");
    }
}

#[test]
fn only_open_regular_opens_a_named_file_and_no_door_joins_without_a_deadline() {
    let mut types_doors = Vec::new();
    let mut joins = Vec::new();
    for door in common::DOOR_FILES {
        let source = std::fs::read_to_string(root().join(door)).unwrap();
        let shape = door_shape(&source);
        types_doors.extend(shape.types_doors);
        joins.extend(
            shape
                .joins
                .into_iter()
                .map(|join| format!("{door}: {join}")),
        );
    }
    assert_eq!(
        types_doors,
        ["open_regular"],
        "Q2-02: only open_regular may open a named file (and expect clippy::disallowed_types)"
    );
    assert!(
        joins.is_empty(),
        "Q2-03: a join has no deadline; read the thread's result from a channel with a timeout: {joins:?}"
    );
}

/// Q-DOOR-SPLIT: door rights are granted by exact listing, never by location. Every file under the
/// door directory, of any kind, must be a listed door module. A file added under `src/doors/`
/// therefore fails here until the protected list names it. The comparison is equality, so a listed
/// module that is missing would fail too; but deleting a listed module is a compile error first (its
/// `mod` line), and for that direction the compiler is the guard (ledger Q30).
///
/// The door directory itself, and each directory above it, must be a real directory: a linked
/// `src/doors` would be walked through the link and fail only as a mismatch or a panic in
/// `common::relative`, not for its reason (ledger Q35). Nothing under the door directory may be a
/// symlink. The listing below compares canonical paths,
/// so a link `doors/alias.rs -> file.rs` would collapse onto the listed `file.rs` and pass as if it
/// were not there (Touchstone F1): one module's rights under a second name. A linked directory is
/// refused here by name as well, before any path is resolved.
#[test]
fn door_files_are_exactly_the_listed_modules() {
    let mut ancestor = root();
    for part in std::path::Path::new(common::DOOR_DIR).components() {
        ancestor.push(part);
        let kind = std::fs::symlink_metadata(&ancestor).unwrap().file_type();
        assert!(
            kind.is_dir() && !kind.is_symlink(),
            "{}: the door directory and each directory above it are real directories, never symlinks",
            ancestor.display()
        );
    }
    let mut links = Vec::new();
    let mut pending = vec![root().join(common::DOOR_DIR)];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let kind = std::fs::symlink_metadata(&path).unwrap().file_type();
            if kind.is_symlink() {
                links.push(path.display().to_string());
            } else if kind.is_dir() {
                pending.push(path);
            }
        }
    }
    links.sort();
    assert!(
        links.is_empty(),
        "a door module is a real file and a door directory a real directory, never a symlink: {links:?}"
    );
    let present: std::collections::BTreeSet<String> = files_under(common::DOOR_DIR, "")
        .iter()
        .map(|p| common::relative(p))
        .collect();
    let listed: std::collections::BTreeSet<String> =
        common::DOOR_FILES.iter().map(|p| (*p).to_owned()).collect();
    assert_eq!(
        present,
        listed,
        "files under {} must be exactly DOOR_FILES",
        common::DOOR_DIR
    );
    for door in common::DOOR_FILES {
        assert!(
            door.starts_with(common::DOOR_DIR)
                && std::path::Path::new(door).extension() == Some("rs".as_ref()),
            "{door}: a door module lives under {} and is Rust",
            common::DOOR_DIR
        );
    }
}

#[test]
fn door_scanner_matches_its_role_in_the_shared_corpus() {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(&common::read("fixtures/attribute-policy.json")).unwrap();
    assert!(cases.len() >= 30, "attribute corpus shrank");
    for case in cases {
        let source = case
            .get("source")
            .and_then(serde_json::Value::as_str)
            .unwrap();
        let expected = case
            .get("door_violation")
            .and_then(serde_json::Value::as_bool)
            .unwrap();
        assert_eq!(!door_lint_controls(source).is_empty(), expected, "{case}");
    }
}

/// Whether `target` (repository-relative, as a door's `#[path]` resolves it) names a Rust file
/// under some crate's `tests/`. Normalised lexically: `..` may not climb above the repository
/// root, and an absolute path is refused. Nothing on disk is read or followed.
fn names_test_support(target: &std::path::Path) -> bool {
    let mut parts: Vec<&std::ffi::OsStr> = Vec::new();
    for component in target.components() {
        match component {
            std::path::Component::Normal(part) => parts.push(part),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if parts.pop().is_none() {
                    return false;
                }
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => return false,
        }
    }
    matches!(parts.as_slice(), [crates, _, tests, _, ..] if *crates == "crates" && *tests == "tests")
        && target.extension() == Some("rs".as_ref())
}

/// Every `path` attribute in a door module other than the admitted ones.
struct DoorPaths<'ast> {
    admitted: Vec<&'ast Attribute>,
    found: Vec<String>,
}

impl<'ast> Visit<'ast> for DoorPaths<'ast> {
    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        if !self.admitted.iter().any(|a| std::ptr::eq(*a, attribute)) {
            let mut found = Vec::new();
            check_meta(&attribute.meta, &mut found);
            if found.iter().any(|f| f == PATH_ATTRIBUTE) {
                self.found.push(
                    "a path attribute that is not on a test-only top-level `mod name;` naming a \
                     file under crates/*/tests/"
                        .to_owned(),
                );
            }
        }
        syn::visit::visit_attribute(self, attribute);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        let mut found = Vec::new();
        check_tokens(mac.tokens.clone(), &mut found);
        if found.iter().any(|f| f == PATH_ATTRIBUTE) {
            self.found
                .push("a path attribute inside a macro invocation".to_owned());
        }
        syn::visit::visit_macro(self, mac);
    }
}

/// Q-DOOR-PATH: inside the door module `door`, a `#[path]` is admitted only to bring test support
/// into a test-only module. It must be written directly (not through `cfg_attr` or a macro) as a
/// string literal on a top-level `mod name;` that is compiled only under `test`, and it must name a
/// Rust file under `crates/<crate>/tests/`. Every other `path` attribute is an offence.
fn door_path_offences(door: &str, source: &str) -> Vec<String> {
    let Ok(file) = syn::parse_file(source) else {
        return vec![format!("{door}: does not parse")];
    };
    let dir = std::path::Path::new(door)
        .parent()
        .unwrap_or(std::path::Path::new(""));
    let mut admitted = Vec::new();
    for item in &file.items {
        let syn::Item::Mod(module) = item else {
            continue;
        };
        if module.content.is_some() || !is_test_only(&module.attrs) {
            continue;
        }
        for attribute in &module.attrs {
            if let Meta::NameValue(pair) = &attribute.meta
                && pair.path.segments.len() == 1
                && last(&pair.path) == "path"
                && let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(target),
                    ..
                }) = &pair.value
                && names_test_support(&dir.join(target.value()))
            {
                admitted.push(attribute);
            }
        }
    }
    let mut scan = DoorPaths {
        admitted,
        found: Vec::new(),
    };
    scan.visit_file(&file);
    scan.found
        .into_iter()
        .map(|f| format!("{door}: {f}"))
        .collect()
}

/// Q-DOOR-PATH on the tree: no door module names another file by `#[path]`, except test support
/// into a test-only module (today: `doors/mod.rs`'s `test_environment`).
#[test]
fn a_door_module_names_only_test_support_by_path() {
    for door in common::DOOR_FILES {
        let source = std::fs::read_to_string(root().join(door)).unwrap();
        assert_eq!(
            door_path_offences(door, &source),
            Vec::<String>::new(),
            "{door}"
        );
    }
}

#[test]
fn the_door_path_rule_admits_only_a_test_only_mod_naming_test_support() {
    let door = "crates/forge-warden-cli/src/doors/mod.rs";
    for (name, source) in [
        (
            "cfg(test)",
            "#[cfg(test)]\n#[path = \"../../tests/support/env.rs\"]\nmod env;",
        ),
        (
            "cfg(test) plus another cfg, as in doors/mod.rs",
            "#[cfg(test)]\n#[cfg(not(fw_fixture_cli))]\n#[path = \"../../tests/support/env.rs\"]\nmod env;",
        ),
        (
            "cfg(all(test, unix)) and a ./ segment",
            "#[cfg(all(test, unix))]\n#[path = \"./../../tests/support/env.rs\"]\nmod env;",
        ),
        ("no path attribute at all", "mod child;"),
    ] {
        assert_eq!(
            door_path_offences(door, source),
            Vec::<String>::new(),
            "refused: {name}\n{source}"
        );
    }
    for (name, source) in [
        (
            "no cfg(test)",
            "#[path = \"../../tests/support/env.rs\"]\nmod env;",
        ),
        (
            "cfg(not(test))",
            "#[cfg(not(test))]\n#[path = \"../../tests/support/env.rs\"]\nmod env;",
        ),
        (
            "cfg(any(test, unix))",
            "#[cfg(any(test, unix))]\n#[path = \"../../tests/support/env.rs\"]\nmod env;",
        ),
        (
            "a door file, not test support",
            "#[cfg(test)]\n#[path = \"child/git.rs\"]\nmod again;",
        ),
        (
            "the crate's own src",
            "#[cfg(test)]\n#[path = \"../main.rs\"]\nmod again;",
        ),
        (
            "climbs above the repository root, then into a tests/ dir",
            "#[cfg(test)]\n#[path = \"../../../../../crates/forge-warden-cli/tests/support/env.rs\"]\nmod env;",
        ),
        (
            "an absolute path",
            "#[cfg(test)]\n#[path = \"/crates/forge-warden-cli/tests/support/env.rs\"]\nmod env;",
        ),
        (
            "not a Rust file",
            "#[cfg(test)]\n#[path = \"../../tests/support/env.txt\"]\nmod env;",
        ),
        (
            "a tests/ directory itself",
            "#[cfg(test)]\n#[path = \"../../tests\"]\nmod env;",
        ),
        (
            "an inline module",
            "#[cfg(test)]\n#[path = \"../../tests/support/env.rs\"]\nmod env {}",
        ),
        (
            "nested in an inline test module",
            "#[cfg(test)]\nmod t {\n    #[path = \"../../tests/support/env.rs\"]\n    mod env;\n}",
        ),
        (
            "through cfg_attr",
            "#[cfg_attr(test, path = \"../../tests/support/env.rs\")]\nmod env;",
        ),
        (
            "inside a macro invocation",
            "some_macro! {\n    #[cfg(test)]\n    #[path = \"../../tests/support/env.rs\"]\n    mod env;\n}",
        ),
        (
            "not a string literal",
            "#[cfg(test)]\n#[path = concat!(\"../../tests/\", \"env.rs\")]\nmod env;",
        ),
        (
            "on a function",
            "#[cfg(test)]\n#[path = \"../../tests/support/env.rs\"]\nfn env() {}",
        ),
        (
            "a second, unadmitted path beside an admitted one",
            "#[cfg(test)]\n#[path = \"../../tests/support/env.rs\"]\nmod env;\n#[path = \"file.rs\"]\nmod again;",
        ),
        ("unparseable", "fn f( {"),
    ] {
        assert!(
            !door_path_offences(door, source).is_empty(),
            "missed: {name}\n{source}"
        );
    }
}
