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

/// Every attribute but doc comments, verbatim: `cfg`, `cfg_attr`, `derive`, `non_exhaustive`,
/// `must_use`, `serde(..)`, lint levels, anything (Bastion: `cfg_attr(all(), non_exhaustive)` was
/// invisible when only three names were kept). Doc text is prose, not API.
fn markers(source: &str, attrs: &[Attribute]) -> String {
    let mut out = Vec::new();
    for a in attrs {
        let doc = a
            .path()
            .get_ident()
            .is_some_and(|ident| ident.unraw() == "doc");
        if !doc {
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

/// The member that starts on the line holding `span` (after indentation), through its terminator:
/// the first of `stops` at bracket depth 0 (`()`, `[]`, `{}`, `<>`; `->` and `=>` are not brackets),
/// so a multi-line variant, field or trait method is read whole.
fn member_text(source: &str, span: Span, stops: &[char]) -> String {
    let at = span.byte_range().start;
    let line_start = match source.get(..at).and_then(|s| s.rfind('\n')) {
        Some(newline) => newline.saturating_add(1),
        None => 0,
    };
    let Some(rest) = source.get(line_start..) else {
        return format!("<unreadable member at {at}>");
    };
    let mut depth = 0_usize;
    let mut previous = ' ';
    let mut end = rest.len();
    for (offset, c) in rest.char_indices() {
        if depth == 0 && stops.contains(&c) {
            end = offset.saturating_add(c.len_utf8());
            break;
        }
        match c {
            '(' | '[' | '{' | '<' => depth = depth.saturating_add(1),
            '>' if previous == '-' || previous == '=' => {}
            ')' | ']' | '}' | '>' => {
                if depth == 0 {
                    end = offset;
                    break;
                }
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
        previous = c;
    }
    slice(source, line_start, line_start.saturating_add(end))
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
            out.push(format!(
                "{module} {}.{}{}",
                s.ident,
                markers(source, &field.attrs),
                member_text(source, span, &[','])
            ));
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
            "{module} {}::{}{}",
            e.ident,
            markers(source, &variant.attrs),
            member_text(source, variant.ident.span(), &[','])
        ));
    }
}

/// A public trait's header and every item in it (all trait items are public with the trait).
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "syn::TraitItem is #[non_exhaustive]: an item kind this reader does not model is recorded as UNKNOWN, so it changes the snapshot"
)]
fn trait_lines(source: &str, module: &str, tr: &syn::ItemTrait, out: &mut Vec<String>) {
    let Some(start) = public(&tr.vis) else { return };
    let header = between(source, start, tr.brace_token.span.open());
    out.push(format!("{module} {}{header}", markers(source, &tr.attrs)));
    let name = tr.ident.unraw();
    for item in &tr.items {
        let line = match item {
            syn::TraitItem::Fn(f) if !is_test_only(&f.attrs) => format!(
                "{}{}",
                markers(source, &f.attrs),
                member_text(source, f.sig.ident.span(), &[';', '{'])
            ),
            syn::TraitItem::Const(c) if !is_test_only(&c.attrs) => format!(
                "{}{}",
                markers(source, &c.attrs),
                member_text(source, c.ident.span(), &[';'])
            ),
            syn::TraitItem::Type(ty) if !is_test_only(&ty.attrs) => format!(
                "{}{}",
                markers(source, &ty.attrs),
                member_text(source, ty.ident.span(), &[';'])
            ),
            syn::TraitItem::Macro(m) if !is_test_only(&m.attrs) => {
                format!("macro {}", macro_line(source, &m.mac))
            }
            syn::TraitItem::Fn(_)
            | syn::TraitItem::Const(_)
            | syn::TraitItem::Type(_)
            | syn::TraitItem::Macro(_) => continue,
            _ => "UNKNOWN TRAIT ITEM KIND: review the public API by hand".to_owned(),
        };
        out.push(format!("{module} {name}::{line}"));
    }
}

/// A macro definition or item-level invocation, verbatim from its path to its closing delimiter.
/// The reader cannot expand a macro, so the whole text is the API record: any change to what a macro
/// generates (Bastion: `note_codes!` emits the public `NoteCode`, `ALL` and `name`) changes it.
fn macro_line(source: &str, mac: &syn::Macro) -> String {
    let close = match &mac.delimiter {
        syn::MacroDelimiter::Paren(d) => d.span.close(),
        syn::MacroDelimiter::Brace(d) => d.span.close(),
        syn::MacroDelimiter::Bracket(d) => d.span.close(),
    };
    match mac.path.segments.first() {
        Some(first) => through(source, first.ident.span(), close),
        None => "<macro without a path>".to_owned(),
    }
}

/// A trait impl's header, and every public method or constant of any impl.
fn impl_lines(source: &str, module: &str, i: &syn::ItemImpl, out: &mut Vec<String>) {
    // Every impl header (inherent too: its generics and where clause bound the methods).
    out.push(format!(
        "{module} {}{}",
        markers(source, &i.attrs),
        between(source, i.impl_token.span, i.brace_token.span.open())
    ));
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
        if let ImplItem::Macro(m) = member
            && !is_test_only(&m.attrs)
        {
            out.push(format!(
                "{module} {owner}::macro {}",
                macro_line(source, &m.mac)
            ));
        }
        if let ImplItem::Type(ty) = member
            && !is_test_only(&ty.attrs)
        {
            // An associated type is public with its trait impl, whatever its visibility token.
            out.push(format!(
                "{module} {owner}::{}",
                member_text(source, ty.ident.span(), &[';'])
            ));
        }
        if !matches!(
            member,
            ImplItem::Fn(_) | ImplItem::Const(_) | ImplItem::Type(_) | ImplItem::Macro(_)
        ) {
            out.push(format!(
                "{module} {owner}::UNKNOWN IMPL ITEM KIND: review the public API by hand"
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
    reason = "syn::Item is #[non_exhaustive]: an item kind this reader does not model is recorded as UNKNOWN, so it changes the snapshot"
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
            Item::Trait(tr) if !is_test_only(&tr.attrs) => trait_lines(source, module, tr, out),
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
            Item::Macro(m) if !is_test_only(&m.attrs) => {
                out.push(format!(
                    "{module} {}macro {}",
                    markers(source, &m.attrs),
                    macro_line(source, &m.mac)
                ));
            }
            Item::ExternCrate(e) if !is_test_only(&e.attrs) => {
                whole(source, module, &e.attrs, &e.vis, e.semi_token.span, out);
            }
            Item::Union(u) if !is_test_only(&u.attrs) => {
                if let Some(start) = public(&u.vis) {
                    let header = between(source, start, u.fields.brace_token.span.open());
                    out.push(format!("{module} {}{header}", markers(source, &u.attrs)));
                    for field in &u.fields.named {
                        if let Some(span) = public(&field.vis) {
                            out.push(format!(
                                "{module} {}.{}",
                                u.ident,
                                member_text(source, span, &[','])
                            ));
                        }
                    }
                }
            }
            // The modelled kinds whose guard said test-only: nothing to record.
            Item::Fn(_)
            | Item::Struct(_)
            | Item::Enum(_)
            | Item::Trait(_)
            | Item::Const(_)
            | Item::Static(_)
            | Item::Type(_)
            | Item::Use(_)
            | Item::Mod(_)
            | Item::Impl(_)
            | Item::Macro(_)
            | Item::ExternCrate(_)
            | Item::Union(_) => {}
            // Anything else (foreign modules, trait aliases, unparsed tokens, future kinds) is
            // recorded, never skipped: it changes the snapshot and gets a reviewer's eyes.
            _ => out.push(format!(
                "{module} UNKNOWN ITEM KIND: review the public API by hand"
            )),
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

/// Parser controls: one of every public item kind, Bastion's probes, and test-only items.
const CONTROL: &str = r#"
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
macro_rules! make {
    ($name:ident) => {
        #[derive(PartialOrd, Ord)]
        pub enum $name { A }
    };
}
make!(Generated);
pub extern crate std as stdx;
pub union U {
    pub a: u8,
}
extern "C" {}
#[cfg(test)]
macro_rules! test_only_macro {
    () => {};
}
pub trait Pick<T> {
    /// prose is not API
    fn pick(self, d: T) -> T;
    fn twice(self) -> u8 {
        2
    }
}
#[cfg_attr(all(), non_exhaustive)]
pub enum Wide {
    Struct {
        a: u8,
        b: Vec<(u8, u8)>,
    },
}
pub struct Long {
    pub f: std::collections::BTreeMap<
        String,
        Vec<u8>,
    >,
}
impl Iterator for Long {
    type Item = u8;
    fn next(&mut self) -> Option<u8> {
        None
    }
}
"#;

/// What the reader must record for [`CONTROL`].
const CONTROL_LINES: &[&str] = &[
    "m #[derive(Debug, Clone)] #[non_exhaustive] pub struct S",
    "m S.pub a: u8,",
    "m pub enum E",
    "m E::One,",
    "m E::Two(u8),",
    "m pub fn f(x: u8) -> u8",
    "m pub const C: u8 = 1;",
    "m pub use std::fmt::Debug;",
    "m impl S",
    "m S::pub fn new() -> Self",
    "m impl std::fmt::Display for S",
    "m #[cfg(feature = \"door\")] pub mod door",
    "m::door pub fn open()",
    "m #[cfg(any(test, unix))] pub fn production_on_unix()",
    "m macro macro_rules! make { ($name:ident) => { #[derive(PartialOrd, Ord)] pub enum $name { A } }; }",
    "m macro make!(Generated)",
    "m pub extern crate std as stdx;",
    "m pub union U",
    "m U.pub a: u8,",
    "m UNKNOWN ITEM KIND: review the public API by hand",
    "m pub trait Pick<T>",
    "m Pick::fn pick(self, d: T) -> T;",
    "m Pick::fn twice(self) -> u8 {",
    "m #[cfg_attr(all(), non_exhaustive)] pub enum Wide",
    "m Wide::Struct { a: u8, b: Vec<(u8, u8)>, },",
    "m pub struct Long",
    "m Long.pub f: std::collections::BTreeMap< String, Vec<u8>, >,",
    "m impl Iterator for Long",
    "m Long::type Item = u8;",
];

#[test]
fn the_snapshot_reader_sees_every_kind_of_public_item_and_skips_tests() {
    let source = CONTROL;
    let file = syn::parse_file(source).unwrap();
    let mut out = Vec::new();
    items(source, "m", &file.items, &mut out);
    assert_eq!(out, CONTROL_LINES);
}
