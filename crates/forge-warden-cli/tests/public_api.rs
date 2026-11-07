//! Q-TYPES-1 (Q1-07, Q4-09): the core's public API is a committed snapshot, so every new, changed
//! or removed public item, field, variant, derive, trait impl, `#[non_exhaustive]` or `cfg` gate is
//! a visible, reviewed diff of `crates/forge-warden-core/public-api.txt`.
//!
//! `cargo public-api` is not installed in either verification backend and rustdoc JSON needs a
//! nightly whose format churns (and whose spans move with every edit), so the API is read here with
//! syn from the core's sources: each production item with exactly `pub` visibility, its header
//! sliced verbatim from the source (rustfmt-normalised whitespace). Items compiled only under
//! `test` are skipped (the same cfg-implies-test rule as `door_lints.rs`). On a mismatch the
//! proposed snapshot is written under the target directory, never into the repository: updating
//! the committed file is a deliberate copy in a reviewed diff.
#![expect(
    clippy::disallowed_methods,
    clippy::unwrap_used,
    reason = "test door: reads the core's sources and writes a proposed snapshot under the target dir"
)]

mod common;
use common::{files_under, root};
use forge_warden_core::defaults::NamedDefault as _;
use proc_macro2::Span;
use syn::ext::IdentExt as _;
use syn::punctuated::Punctuated;
use syn::{Attribute, Fields, ImplItem, Item, Meta, Token, Visibility};

const SNAPSHOT: &str = "crates/forge-warden-core/public-api.txt";

/// Whether a `cfg` predicate is true only when `test` is set (as in `door_lints.rs`).
fn implies_test(predicate: &Meta) -> bool {
    match predicate {
        Meta::Path(path) => path.is_ident("test"),
        Meta::List(list) => {
            let Ok(members) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            else {
                return false;
            };
            if list.path.is_ident("all") {
                members.iter().any(implies_test)
            } else if list.path.is_ident("any") {
                !members.is_empty() && members.iter().all(implies_test)
            } else {
                false
            }
        }
        Meta::NameValue(_) => false,
    }
}

fn is_test_only(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg")
            && a.meta.require_list().is_ok_and(|list| {
                list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
                    .is_ok_and(|args| args.len() == 1 && args.iter().all(implies_test))
            })
    })
}

/// The API-relevant attributes, verbatim: `cfg(..)`, `derive(..)`, `non_exhaustive`.
fn markers(source: &str, attrs: &[Attribute]) -> String {
    let mut out = Vec::new();
    for a in attrs {
        let relevant = a.path().get_ident().is_some_and(|ident| {
            let name = ident.unraw().to_string();
            name == "cfg" || name == "derive" || name == "non_exhaustive"
        });
        if relevant {
            out.push(through(
                source,
                a.pound_token.span,
                a.bracket_token.span.close(),
            ));
        }
    }
    if out.is_empty() {
        String::new()
    } else {
        format!("{} ", out.join(" "))
    }
}

fn normalise(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Source text between two spans (start of `from`, start of `to`), whitespace-normalised.
fn between(source: &str, from: Span, to: Span) -> String {
    slice(source, from.byte_range().start, to.byte_range().start)
}

/// `source[start..end]`, whitespace-normalised; a span that does not slice the source is named.
fn slice(source: &str, start: usize, end: usize) -> String {
    match source.get(start..end) {
        Some(text) => normalise(text),
        None => format!("<unreadable span {start}..{end}>"),
    }
}

/// Source text from the start of `from` to the end of `to`.
fn through(source: &str, from: Span, to: Span) -> String {
    slice(source, from.byte_range().start, to.byte_range().end)
}

/// The whole source line holding `span`, trimmed.
fn line_of(source: &str, span: Span) -> String {
    let at = span.byte_range().start;
    let begin = match source.get(..at).and_then(|s| s.rfind('\n')) {
        Some(newline) => newline.saturating_add(1),
        None => 0,
    };
    let end = match source.get(at..).and_then(|s| s.find('\n')) {
        Some(offset) => at.saturating_add(offset),
        None => source.len(),
    };
    slice(source, begin, end)
}

fn public(vis: &Visibility) -> Option<Span> {
    match vis {
        Visibility::Public(token) => Some(token.span),
        Visibility::Restricted(_) | Visibility::Inherited => None,
    }
}

fn type_name(ty: &syn::Type) -> String {
    if let syn::Type::Path(p) = ty {
        p.path
            .segments
            .iter()
            .map(|s| s.ident.unraw().to_string())
            .collect::<Vec<_>>()
            .join("::")
    } else {
        "<type>".to_owned()
    }
}

/// A public struct's header and public fields.
fn struct_lines(source: &str, module: &str, s: &syn::ItemStruct, out: &mut Vec<String>) {
    let Some(start) = public(&s.vis) else { return };
    let end = match &s.fields {
        Fields::Named(named) => named.brace_token.span.open(),
        Fields::Unnamed(_) | Fields::Unit => match &s.semi_token {
            Some(semi) => semi.span,
            None => s.ident.span(),
        },
    };
    out.push(format!(
        "{module} {}{}",
        markers(source, &s.attrs),
        between(source, start, end)
    ));
    for field in &s.fields {
        if let Some(span) = public(&field.vis) {
            out.push(format!("{module} {}.{}", s.ident, line_of(source, span)));
        }
    }
}

/// A public enum's header and every variant.
fn enum_lines(source: &str, module: &str, e: &syn::ItemEnum, out: &mut Vec<String>) {
    let Some(start) = public(&e.vis) else { return };
    let header = between(source, start, e.brace_token.span.open());
    out.push(format!("{module} {}{header}", markers(source, &e.attrs)));
    for variant in &e.variants {
        out.push(format!(
            "{module} {}::{}",
            e.ident,
            line_of(source, variant.ident.span())
        ));
    }
}

/// A trait impl's header, and every public method or constant of any impl.
fn impl_lines(source: &str, module: &str, i: &syn::ItemImpl, out: &mut Vec<String>) {
    if i.trait_.is_some() {
        out.push(format!(
            "{module} {}",
            between(source, i.impl_token.span, i.brace_token.span.open())
        ));
    }
    let owner = type_name(&i.self_ty);
    for member in &i.items {
        if let ImplItem::Fn(f) = member
            && !is_test_only(&f.attrs)
            && let Some(start) = public(&f.vis)
        {
            let sig = between(source, start, f.block.brace_token.span.open());
            out.push(format!(
                "{module} {owner}::{}{sig}",
                markers(source, &f.attrs)
            ));
        }
        if let ImplItem::Const(c) = member
            && let Some(start) = public(&c.vis)
        {
            out.push(format!(
                "{module} {owner}::{}",
                through(source, start, c.semi_token.span)
            ));
        }
    }
}

/// A public item that ends at a `;` (const, static, type alias, use), whole.
fn whole(
    source: &str,
    module: &str,
    attrs: &[Attribute],
    vis: &Visibility,
    semi: Span,
    out: &mut Vec<String>,
) {
    if let Some(start) = public(vis) {
        out.push(format!(
            "{module} {}{}",
            markers(source, attrs),
            through(source, start, semi)
        ));
    }
}

/// Every public line of one file's items under `module`.
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "syn::Item is #[non_exhaustive]; macros, extern crates and foreign modules carry no public API in the core"
)]
fn items(source: &str, module: &str, list: &[Item], out: &mut Vec<String>) {
    for item in list {
        match item {
            Item::Fn(f) if !is_test_only(&f.attrs) => {
                if let Some(start) = public(&f.vis) {
                    let header = between(source, start, f.block.brace_token.span.open());
                    out.push(format!("{module} {}{header}", markers(source, &f.attrs)));
                }
            }
            Item::Struct(s) if !is_test_only(&s.attrs) => struct_lines(source, module, s, out),
            Item::Enum(e) if !is_test_only(&e.attrs) => enum_lines(source, module, e, out),
            Item::Trait(tr) if !is_test_only(&tr.attrs) => {
                if let Some(start) = public(&tr.vis) {
                    let header = between(source, start, tr.brace_token.span.open());
                    out.push(format!("{module} {}{header}", markers(source, &tr.attrs)));
                }
            }
            Item::Const(c) if !is_test_only(&c.attrs) => {
                whole(source, module, &c.attrs, &c.vis, c.semi_token.span, out);
            }
            Item::Static(s) if !is_test_only(&s.attrs) => {
                whole(source, module, &s.attrs, &s.vis, s.semi_token.span, out);
            }
            Item::Type(ty) if !is_test_only(&ty.attrs) => {
                whole(source, module, &ty.attrs, &ty.vis, ty.semi_token.span, out);
            }
            Item::Use(u) if !is_test_only(&u.attrs) => {
                whole(source, module, &u.attrs, &u.vis, u.semi_token.span, out);
            }
            Item::Mod(m) if !is_test_only(&m.attrs) => {
                if let Some(start) = public(&m.vis) {
                    let end = match (&m.semi, &m.content) {
                        (Some(semi), _) => semi.span,
                        (None, Some((brace, _))) => brace.span.open(),
                        (None, None) => m.ident.span(),
                    };
                    out.push(format!(
                        "{module} {}{}",
                        markers(source, &m.attrs),
                        between(source, start, end)
                    ));
                }
                if let Some((_, inner)) = &m.content {
                    items(
                        source,
                        &format!("{module}::{}", m.ident.unraw()),
                        inner,
                        out,
                    );
                }
            }
            Item::Impl(i) if !is_test_only(&i.attrs) => impl_lines(source, module, i, out),
            _ => {}
        }
    }
}

/// The module path of a core source file: `lib.rs` is the crate, `a/mod.rs` is `a`.
fn module_of(relative: &str) -> String {
    let path = relative
        .trim_start_matches("crates/forge-warden-core/src/")
        .trim_end_matches(".rs")
        .trim_end_matches("/mod");
    if path == "lib" {
        "crate".to_owned()
    } else {
        format!("crate::{}", path.replace('/', "::"))
    }
}

fn current_api() -> String {
    let mut files: Vec<String> = files_under("crates/forge-warden-core/src", ".rs")
        .iter()
        .map(|p| common::relative(p))
        .collect();
    files.sort();
    let mut out = Vec::new();
    for relative in files {
        let source = std::fs::read_to_string(root().join(&relative)).unwrap();
        let file = match syn::parse_file(&source) {
            Ok(file) => file,
            Err(e) => {
                out.push(format!("{relative} DOES NOT PARSE: {e}"));
                continue;
            }
        };
        // A file compiled only under test (`#![cfg(test)]`) is not API.
        if is_test_only(&file.attrs) {
            continue;
        }
        items(&source, &module_of(&relative), &file.items, &mut out);
    }
    let mut text = out.join("\n");
    text.push('\n');
    text
}

#[test]
fn the_core_public_api_matches_its_committed_snapshot() {
    let current = current_api();
    // A missing snapshot reads as empty, so the test fails and proposes the first one.
    let committed = std::fs::read_to_string(root().join(SNAPSHOT)).or_fail_closed(String::new());
    if current != committed {
        let proposed = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("public-api.txt");
        std::fs::write(&proposed, &current).unwrap();
        let old: std::collections::BTreeSet<&str> = committed.lines().collect();
        let new: std::collections::BTreeSet<&str> = current.lines().collect();
        let removed: Vec<&&str> = old.difference(&new).collect();
        let added: Vec<&&str> = new.difference(&old).collect();
        panic!(
            "the core's public API changed; review it, then copy {} over {SNAPSHOT} in the same diff\nremoved: {removed:#?}\nadded: {added:#?}",
            proposed.display()
        );
    }
}

#[test]
fn the_snapshot_reader_sees_every_kind_of_public_item_and_skips_tests() {
    let source = r#"
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct S {
    pub a: u8,
    b: u8,
}
pub enum E {
    One,
    Two(u8),
}
pub fn f(x: u8) -> u8 {
    x
}
pub(crate) fn hidden() {}
pub const C: u8 = 1;
pub use std::fmt::Debug;
impl S {
    pub fn new() -> Self {
        Self { a: 0, b: 0 }
    }
    fn private(&self) {}
}
impl std::fmt::Display for S {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Ok(())
    }
}
#[cfg(feature = "door")]
pub mod door {
    pub fn open() {}
}
#[cfg(test)]
pub fn test_only() {}
#[cfg(any(test, unix))]
pub fn production_on_unix() {}
"#;
    let file = syn::parse_file(source).unwrap();
    let mut out = Vec::new();
    items(source, "m", &file.items, &mut out);
    assert_eq!(
        out,
        [
            "m #[derive(Debug, Clone)] #[non_exhaustive] pub struct S",
            "m S.pub a: u8,",
            "m pub enum E",
            "m E::One,",
            "m E::Two(u8),",
            "m pub fn f(x: u8) -> u8",
            "m pub const C: u8 = 1;",
            "m pub use std::fmt::Debug;",
            "m S::pub fn new() -> Self",
            "m impl std::fmt::Display for S",
            "m #[cfg(feature = \"door\")] pub mod door",
            "m::door pub fn open()",
            "m #[cfg(any(test, unix))] pub fn production_on_unix()",
        ]
    );
}
