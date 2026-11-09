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

/// Whitespace-normalised source. Every run of whitespace outside literals becomes one space, so
/// formatting is not API. The characters of every string, byte-string, C-string, raw-string and char
/// literal are kept exactly (Touchstone T-QTYPES8-01: `"a b"` and `"a  b"` are different constants,
/// and so are a raw string's newline and a space). Inside a literal a backslash is doubled and any
/// whitespace or control character other than a plain space is written `\u{..}`, so a record stays
/// on one line and the encoding stays injective. Comments are trivia: their quotes start nothing.
fn normalise(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let at = |k: usize| chars.get(k).copied();
    let mut out = String::new();
    let mut space = false;
    let mut i = 0_usize;
    // Inside a comment (up to this index) nothing opens a literal; its words collapse like code.
    let mut plain_until = 0_usize;
    while let Some(c) = at(i) {
        if c.is_whitespace() {
            space = true;
            i = i.saturating_add(1);
            continue;
        }
        if space && !out.is_empty() {
            out.push(' ');
        }
        space = false;
        if i >= plain_until && c == '/' {
            match at(i.saturating_add(1)) {
                Some('/') => plain_until = comment_end(&chars, i, false),
                Some('*') => plain_until = comment_end(&chars, i, true),
                _ => {}
            }
        }
        if i < plain_until {
            out.push(c);
            i = i.saturating_add(1);
            continue;
        }
        if let Some(end) = literal_end(&chars, i) {
            for ch in chars.get(i..end).unwrap_or_default() {
                match *ch {
                    '\\' => out.push_str("\\\\"),
                    ' ' => out.push(' '),
                    other if other.is_whitespace() || other.is_control() => {
                        out.extend(other.escape_unicode());
                    }
                    other => out.push(other),
                }
            }
            i = end;
            continue;
        }
        out.push(c);
        i = i.saturating_add(1);
    }
    out
}

fn is_ident_char(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_alphanumeric() || c == '_')
}

/// Where the literal starting at `i` ends (exclusive), or `None` if no literal starts there. A raw
/// string `r#"..."#` (also `br`/`cr`) needs `r` not to continue an identifier; a char literal is
/// `'x'` or an escape, while `'a` alone is a lifetime.
fn literal_end(chars: &[char], i: usize) -> Option<usize> {
    let at = |k: usize| chars.get(k).copied();
    let next = |k: usize| k.saturating_add(1);
    match at(i)? {
        '"' => {
            let mut j = next(i);
            loop {
                match at(j)? {
                    '\\' => j = j.saturating_add(2),
                    '"' => return Some(next(j)),
                    _ => j = next(j),
                }
            }
        }
        'r' => {
            let before = i.checked_sub(1).and_then(at);
            let prefix_ok = !is_ident_char(before)
                || (matches!(before, Some('b' | 'c'))
                    && !is_ident_char(i.checked_sub(2).and_then(at)));
            if !prefix_ok {
                return None;
            }
            let mut j = next(i);
            let mut hashes = 0_usize;
            while at(j) == Some('#') {
                hashes = hashes.saturating_add(1);
                j = next(j);
            }
            if at(j) != Some('"') {
                return None;
            }
            j = next(j);
            loop {
                if at(j)? == '"' && (1..=hashes).all(|h| at(j.saturating_add(h)) == Some('#')) {
                    return Some(j.saturating_add(hashes).saturating_add(1));
                }
                j = next(j);
            }
        }
        '\'' => {
            if at(next(i)) == Some('\\') {
                let mut j = i.saturating_add(3);
                while at(j).is_some_and(|c| c != '\'') && j < i.saturating_add(12) {
                    j = next(j);
                }
                (at(j) == Some('\'')).then(|| next(j))
            } else if at(next(i)).is_some() && at(i.saturating_add(2)) == Some('\'') {
                Some(i.saturating_add(3))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Where the comment starting at `i` ends (exclusive): the end of the line for `//`, the matching
/// close for a nested `/* */` (or the end of the text if it never closes).
fn comment_end(chars: &[char], i: usize, block: bool) -> usize {
    let at = |k: usize| chars.get(k).copied();
    if !block {
        let mut j = i;
        while at(j).is_some_and(|c| c != '\n') {
            j = j.saturating_add(1);
        }
        return j;
    }
    let mut depth = 0_usize;
    let mut j = i;
    while let Some(c) = at(j) {
        let n = at(j.saturating_add(1));
        if c == '/' && n == Some('*') {
            depth = depth.saturating_add(1);
            j = j.saturating_add(2);
        } else if c == '*' && n == Some('/') {
            depth = depth.saturating_sub(1);
            j = j.saturating_add(2);
            if depth == 0 {
                return j;
            }
        } else {
            j = j.saturating_add(1);
        }
    }
    j
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

/// Q-TYPES-1 v4 (Bastion B-QTYPES-01): a member of a braced body (field, variant, trait item, impl
/// item, foreign item) is the exact source between the end of its predecessor (or the opening
/// brace) and its own terminator, read from the parser's token spans, never guessed from lines:
/// so a compact `{ fn a(); }`, a multi-line member and a member whose visibility is inherited
/// (every item of a trait impl) are all read whole. Leading trivia is dropped: whitespace,
/// comments (doc comments are prose, not API) and outer attributes (recorded by [`markers`]).
fn member(source: &str, from: usize, to: usize) -> String {
    match source.get(from..to) {
        Some(text) => slice(source, from.saturating_add(leading_trivia(text)), to),
        None => format!("<unreadable span {from}..{to}>"),
    }
}

/// Bytes of whitespace, comments and `#[..]` attributes at the start of `text`. A string literal
/// inside an attribute is skipped whole, so a `]` in it does not end the attribute; anything this
/// cannot classify ends the trivia, which can only make a record longer, never drop API text.
fn leading_trivia(text: &str) -> usize {
    let mut at = 0_usize;
    loop {
        let Some(rest) = text.get(at..) else {
            return at;
        };
        let trimmed = rest.trim_start();
        at = at.saturating_add(rest.len().saturating_sub(trimmed.len()));
        let skipped = if trimmed.starts_with("//") {
            trimmed.find('\n').unwrap_or(trimmed.len())
        } else if trimmed.starts_with("/*") {
            block_comment(trimmed)
        } else if trimmed.starts_with("#[") {
            attribute(trimmed)
        } else {
            return at;
        };
        if skipped == 0 {
            return at;
        }
        at = at.saturating_add(skipped);
    }
}

/// Length of the (nested) block comment that starts `text`, or 0 if it never closes.
fn block_comment(text: &str) -> usize {
    let mut depth = 0_usize;
    let mut chars = text.char_indices().peekable();
    while let Some((offset, c)) = chars.next() {
        let next = chars.peek().map(|&(_, n)| n);
        if c == '/' && next == Some('*') {
            depth = depth.saturating_add(1);
            chars.next();
        } else if c == '*' && next == Some('/') {
            depth = depth.saturating_sub(1);
            chars.next();
            if depth == 0 {
                return offset.saturating_add(2);
            }
        }
    }
    0
}

/// Length of the `#[..]` attribute that starts `text` (brackets balanced, string literals skipped),
/// or 0 if it never closes.
fn attribute(text: &str) -> usize {
    let mut depth = 0_usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, c) in text.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '[' => depth = depth.saturating_add(1),
            ']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return offset.saturating_add(1);
                }
            }
            _ => {}
        }
    }
    0
}

fn start(span: Span) -> usize {
    span.byte_range().start
}

fn end(span: Span) -> usize {
    span.byte_range().end
}

/// Where a macro invocation ends: its `;` if it has one, else its closing delimiter.
fn macro_end(mac: &syn::Macro, semi: Option<&Token![;]>) -> usize {
    match semi {
        Some(semi) => end(semi.span),
        None => end(match &mac.delimiter {
            syn::MacroDelimiter::Paren(d) => d.span.close(),
            syn::MacroDelimiter::Brace(d) => d.span.close(),
            syn::MacroDelimiter::Bracket(d) => d.span.close(),
        }),
    }
}

/// Where a token stream ends (a group ends at its closing delimiter); `None` if it is empty.
fn stream_end(tokens: &proc_macro2::TokenStream) -> Option<usize> {
    tokens.clone().into_iter().last().map(|last| match last {
        proc_macro2::TokenTree::Group(g) => end(g.span_close()),
        proc_macro2::TokenTree::Ident(i) => end(i.span()),
        proc_macro2::TokenTree::Punct(p) => end(p.span()),
        proc_macro2::TokenTree::Literal(l) => end(l.span()),
    })
}

/// The record for an item kind this reader cannot model. The snapshot test refuses any record
/// containing it, so an unmodelled kind can never be blessed into a stable, change-blind line.
const UNSUPPORTED: &str = "<UNSUPPORTED>";

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

/// One member of a braced body, as the reader sees it: its attributes, its own visibility (`None`
/// when it has none of its own: an associated type, a macro, unparsed tokens), where its record
/// ends (a function's record is its signature, never its body) and where the member ends.
struct Member<'a> {
    attrs: &'a [Attribute],
    vis: Option<&'a Visibility>,
    record: usize,
    whole: usize,
}

impl<'a> Member<'a> {
    fn ending(attrs: &'a [Attribute], vis: Option<&'a Visibility>, at: usize) -> Self {
        Self {
            attrs,
            vis,
            record: at,
            whole: at,
        }
    }
}

/// Records each member of a braced body that `shown` admits, and an [`UNSUPPORTED`] record for a
/// member kind `read` does not model. Each record is the source from the end of the previous
/// member (or `open`) to the member's record end, so nothing between two members is ever skipped.
fn members<'a, T>(
    source: &str,
    prefix: &str,
    open: usize,
    list: &'a [T],
    read: impl Fn(&'a T) -> Option<Member<'a>>,
    shown: impl Fn(&Member<'a>) -> bool,
    out: &mut Vec<String>,
) {
    let mut from = open;
    for item in list {
        let Some(m) = read(item) else {
            out.push(format!(
                "{prefix}{UNSUPPORTED} member kind: extend public_api.rs"
            ));
            continue;
        };
        if shown(&m) && !is_test_only(m.attrs) {
            out.push(format!(
                "{prefix}{}{}",
                markers(source, m.attrs),
                member(source, from, m.record)
            ));
        }
        from = m.whole;
    }
}

/// A public struct's header, its public fields, and how many fields are private: adding a private
/// field to a struct whose fields were all public changes who can build it.
fn struct_lines(source: &str, module: &str, s: &syn::ItemStruct, out: &mut Vec<String>) {
    let Some(vis) = public(&s.vis) else { return };
    let Fields::Named(named) = &s.fields else {
        // A tuple or unit struct is one record through its `;`, private field types included.
        let close = s
            .semi_token
            .as_ref()
            .map_or_else(|| s.ident.span(), |semi| semi.span);
        out.push(format!(
            "{module} {}{}",
            markers(source, &s.attrs),
            through(source, vis, close)
        ));
        return;
    };
    let brace = named.brace_token.span;
    out.push(format!(
        "{module} {}{}",
        markers(source, &s.attrs),
        between(source, vis, brace.open())
    ));
    field_lines(
        source,
        &format!("{module} {}.", s.ident),
        brace,
        &named.named,
        out,
    );
}

/// The public fields of a braced body (struct or union) and a count of the private ones.
fn field_lines(
    source: &str,
    prefix: &str,
    brace: proc_macro2::extra::DelimSpan,
    fields: &Punctuated<syn::Field, Token![,]>,
    out: &mut Vec<String>,
) {
    let mut from = end(brace.open());
    let mut private = 0_usize;
    for pair in fields.pairs() {
        let to = pair
            .punct()
            .map_or_else(|| start(brace.close()), |comma| end(comma.span));
        let field = pair.value();
        if public(&field.vis).is_some() {
            out.push(format!(
                "{prefix}{}{}",
                markers(source, &field.attrs),
                member(source, from, to)
            ));
        } else {
            private = private.saturating_add(1);
        }
        from = to;
    }
    if private > 0 {
        out.push(format!("{prefix}<{private} private field(s)>"));
    }
}

/// A public enum's header and every variant, whole (fields and discriminant included).
fn enum_lines(source: &str, module: &str, e: &syn::ItemEnum, out: &mut Vec<String>) {
    let Some(vis) = public(&e.vis) else { return };
    let brace = e.brace_token.span;
    out.push(format!(
        "{module} {}{}",
        markers(source, &e.attrs),
        between(source, vis, brace.open())
    ));
    let mut from = end(brace.open());
    for pair in e.variants.pairs() {
        let to = pair
            .punct()
            .map_or_else(|| start(brace.close()), |comma| end(comma.span));
        let variant = pair.value();
        out.push(format!(
            "{module} {}::{}{}",
            e.ident,
            markers(source, &variant.attrs),
            member(source, from, to)
        ));
        from = to;
    }
}

/// How the reader sees a trait item (every one is public with its trait). Every kind syn has today
/// is named; the `_` arm (required: `syn::TraitItem` is `#[non_exhaustive]`) is a future kind,
/// which [`members`] records as [`UNSUPPORTED`].
fn trait_member(item: &syn::TraitItem) -> Option<Member<'_>> {
    Some(match item {
        syn::TraitItem::Fn(f) => match (&f.default, &f.semi_token) {
            (Some(body), _) => Member {
                attrs: &f.attrs,
                vis: None,
                record: start(body.brace_token.span.open()),
                whole: end(body.brace_token.span.close()),
            },
            (None, Some(semi)) => Member::ending(&f.attrs, None, end(semi.span)),
            (None, None) => return None,
        },
        syn::TraitItem::Const(c) => Member::ending(&c.attrs, None, end(c.semi_token.span)),
        syn::TraitItem::Type(ty) => Member::ending(&ty.attrs, None, end(ty.semi_token.span)),
        syn::TraitItem::Macro(m) => {
            Member::ending(&m.attrs, None, macro_end(&m.mac, m.semi_token.as_ref()))
        }
        syn::TraitItem::Verbatim(tokens) => Member::ending(&[], None, stream_end(tokens)?),
        _ => return None,
    })
}

/// A public trait's header and every item in it.
fn trait_lines(source: &str, module: &str, tr: &syn::ItemTrait, out: &mut Vec<String>) {
    let Some(vis) = public(&tr.vis) else { return };
    let brace = tr.brace_token.span;
    out.push(format!(
        "{module} {}{}",
        markers(source, &tr.attrs),
        between(source, vis, brace.open())
    ));
    let prefix = format!("{module} {}::", tr.ident.unraw());
    members(
        source,
        &prefix,
        end(brace.open()),
        &tr.items,
        trait_member,
        |_| true,
        out,
    );
}

/// How the reader sees an impl item. `syn::ImplItem` is `#[non_exhaustive]`; a kind not named here
/// returns `None`, which [`members`] records as [`UNSUPPORTED`].
fn impl_member(item: &ImplItem) -> Option<Member<'_>> {
    if let ImplItem::Fn(f) = item {
        return Some(Member {
            attrs: &f.attrs,
            vis: Some(&f.vis),
            record: start(f.block.brace_token.span.open()),
            whole: end(f.block.brace_token.span.close()),
        });
    }
    if let ImplItem::Const(c) = item {
        return Some(Member::ending(
            &c.attrs,
            Some(&c.vis),
            end(c.semi_token.span),
        ));
    }
    if let ImplItem::Type(ty) = item {
        // An associated type is public with its impl, whatever its visibility token.
        return Some(Member::ending(&ty.attrs, None, end(ty.semi_token.span)));
    }
    if let ImplItem::Macro(m) = item {
        let at = macro_end(&m.mac, m.semi_token.as_ref());
        return Some(Member::ending(&m.attrs, None, at));
    }
    if let ImplItem::Verbatim(tokens) = item {
        return Some(Member::ending(&[], None, stream_end(tokens)?));
    }
    None
}

/// Every impl's header; every item of a trait impl (they are public with the trait, whatever their
/// visibility token: Bastion B-QTYPES-01 `impl P for S { const N: u8 = 1; }`); and every public
/// item of an inherent impl.
fn impl_lines(source: &str, module: &str, i: &syn::ItemImpl, out: &mut Vec<String>) {
    let brace = i.brace_token.span;
    out.push(format!(
        "{module} {}{}",
        markers(source, &i.attrs),
        between(source, i.impl_token.span, brace.open())
    ));
    let of_trait = i.trait_.is_some();
    let prefix = format!("{module} {}::", type_name(&i.self_ty));
    let shown = |m: &Member<'_>| of_trait || m.vis.is_none_or(|vis| public(vis).is_some());
    members(
        source,
        &prefix,
        end(brace.open()),
        &i.items,
        impl_member,
        shown,
        out,
    );
}

/// How the reader sees a foreign item. `syn::ForeignItem` is `#[non_exhaustive]`; a kind not named
/// here returns `None`, which [`members`] records as [`UNSUPPORTED`].
fn foreign_member(item: &syn::ForeignItem) -> Option<Member<'_>> {
    if let syn::ForeignItem::Fn(f) = item {
        return Some(Member::ending(
            &f.attrs,
            Some(&f.vis),
            end(f.semi_token.span),
        ));
    }
    if let syn::ForeignItem::Static(s) = item {
        return Some(Member::ending(
            &s.attrs,
            Some(&s.vis),
            end(s.semi_token.span),
        ));
    }
    if let syn::ForeignItem::Type(ty) = item {
        return Some(Member::ending(
            &ty.attrs,
            Some(&ty.vis),
            end(ty.semi_token.span),
        ));
    }
    if let syn::ForeignItem::Macro(m) = item {
        let at = macro_end(&m.mac, m.semi_token.as_ref());
        return Some(Member::ending(&m.attrs, None, at));
    }
    if let syn::ForeignItem::Verbatim(tokens) = item {
        return Some(Member::ending(&[], None, stream_end(tokens)?));
    }
    None
}

/// An `extern` block's header and each public item in it, whole (Bastion B-QTYPES-01: a changed
/// foreign item inside an existing block was invisible behind one UNKNOWN line).
fn foreign_lines(source: &str, module: &str, f: &syn::ItemForeignMod, out: &mut Vec<String>) {
    let brace = f.brace_token.span;
    let first = f
        .unsafety
        .as_ref()
        .map_or_else(|| f.abi.extern_token.span, |unsafety| unsafety.span);
    out.push(format!(
        "{module} {}{}",
        markers(source, &f.attrs),
        between(source, first, brace.open())
    ));
    let prefix = format!("{module} extern::");
    let shown = |m: &Member<'_>| m.vis.is_none_or(|vis| public(vis).is_some());
    members(
        source,
        &prefix,
        end(brace.open()),
        &f.items,
        foreign_member,
        shown,
        out,
    );
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
    reason = "syn::Item is #[non_exhaustive]: an item kind this reader does not model is refused as UNSUPPORTED, which fails the snapshot test"
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
                if let Some(vis) = public(&u.vis) {
                    let brace = u.fields.brace_token.span;
                    let header = between(source, vis, brace.open());
                    out.push(format!("{module} {}{header}", markers(source, &u.attrs)));
                    let prefix = format!("{module} {}.", u.ident);
                    field_lines(source, &prefix, brace, &u.fields.named, out);
                }
            }
            Item::ForeignMod(f) if !is_test_only(&f.attrs) => foreign_lines(source, module, f, out),
            Item::Verbatim(tokens) => {
                // Tokens syn could not type: recorded verbatim, so any change to them is seen.
                if let (Some(head), Some(last)) =
                    (tokens.clone().into_iter().next(), stream_end(tokens))
                {
                    out.push(format!(
                        "{module} {}",
                        slice(source, start(head.span()), last)
                    ));
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
            | Item::Union(_)
            | Item::ForeignMod(_) => {}
            // Anything else (trait aliases, future kinds) cannot be modelled, so it is refused: an
            // UNSUPPORTED record fails the snapshot test and can never be blessed into it.
            _ => out.push(format!(
                "{module} {UNSUPPORTED} item kind: extend public_api.rs"
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

/// Records the reader cannot vouch for: an item kind it does not model, a span it could not read,
/// a file that does not parse.
fn unvouched(api: &str) -> Vec<&str> {
    api.lines()
        .filter(|line| {
            line.contains(UNSUPPORTED)
                || line.contains("<unreadable span")
                || line.contains(" DOES NOT PARSE: ")
        })
        .collect()
}

#[test]
fn the_core_public_api_matches_its_committed_snapshot() {
    let current = current_api();
    // Refused before any comparison, so they can never be blessed into the snapshot.
    let refused = unvouched(&current);
    assert!(
        refused.is_empty(),
        "the reader cannot vouch for these records, so no snapshot can cover them; extend \
         public_api.rs to model them (do not bless them): {refused:#?}"
    );
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
    "m S.<1 private field(s)>",
    "m pub enum E",
    "m E::One,",
    "m E::Two(u8),",
    "m pub fn f(x: u8) -> u8",
    "m pub const C: u8 = 1;",
    "m pub use std::fmt::Debug;",
    "m impl S",
    "m S::pub fn new() -> Self",
    "m impl std::fmt::Display for S",
    "m S::fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result",
    "m #[cfg(feature = \"door\")] pub mod door",
    "m::door pub fn open()",
    "m #[cfg(any(test, unix))] pub fn production_on_unix()",
    "m macro macro_rules! make { ($name:ident) => { #[derive(PartialOrd, Ord)] pub enum $name { A } }; }",
    "m macro make!(Generated)",
    "m pub extern crate std as stdx;",
    "m pub union U",
    "m U.pub a: u8,",
    "m extern \"C\"",
    "m pub trait Pick<T>",
    "m Pick::fn pick(self, d: T) -> T;",
    "m Pick::fn twice(self) -> u8",
    "m #[cfg_attr(all(), non_exhaustive)] pub enum Wide",
    "m Wide::Struct { a: u8, b: Vec<(u8, u8)>, },",
    "m pub struct Long",
    "m Long.pub f: std::collections::BTreeMap< String, Vec<u8>, >,",
    "m impl Iterator for Long",
    "m Long::type Item = u8;",
    "m Long::fn next(&mut self) -> Option<u8>",
];

#[test]
fn the_snapshot_reader_sees_every_kind_of_public_item_and_skips_tests() {
    let source = CONTROL;
    let file = syn::parse_file(source).unwrap();
    let mut out = Vec::new();
    items(source, "m", &file.items, &mut out);
    assert_eq!(out, CONTROL_LINES);
}

/// Totality control (Q-TYPES-1 v4, Bastion B-QTYPES-01): one of every public item and member kind,
/// in compact and multi-line forms, with every token that is not API named `hidden…`.
const TOTAL: &str = r#"
/// Doc prose is not API.
#[derive(Debug)]
pub struct Named<T: Clone> where T: Default {
    /// Field prose is not API either.
    pub field: Vec<T>,
    hidden_field: hidden_t,
}
pub struct Tuple(pub u16, u8);
pub enum Kind { Unit, Pair(u32, i8), Record { inner: i64 }, Valued = 7 }
pub union Bits { pub word: u32, hidden_word: hidden_f }
pub fn free<'life>(arg: &'life str) -> Option<&'life str> where 'life: 'static { hidden_body(hidden_local) }
pub const LIMIT: usize = 64;
pub static NAME: &str = "warden";
pub type Alias = Result<u8, String>;
pub use core::fmt::Write as WriteAlias;
pub mod inner { pub fn nested() -> bool { hidden_value } }
pub trait Compact { fn compact(&self) -> u8; }
pub trait Wide<Param>: Sized {
    const CAP: usize = 3;
    type Out: Clone;
    fn wide(self, by: Param) -> Self::Out;
    fn defaulted(&self) -> i8 {
        hidden_default
    }
}
impl<T> Compact for Named<T> where T: Clone + Default {
    fn compact(&self) -> u8 { hidden_impl_body }
}
impl Marker for Tuple {
    // A plain comment.
    #[inline]
    const N: u8 = 1;
    #[cfg(test)]
    fn hidden_test_member() {}
    type Assoc = u64;
    fn inherited() {}
}
impl Kind {
    pub fn make() -> Self { hidden_ctor }
    pub const MAX: u8 = 9;
    fn hidden_method() {}
    const hidden_const: hidden_t = hidden_init;
}
unsafe extern "C" {
    pub fn foreign(x: i32) -> i32;
    pub static FOREIGN: u8;
    fn hidden_foreign();
}
macro_rules! generate { ($name:ident) => { pub struct $name; }; }
generate!(Generated);
pub extern crate alloc as alloc_alias;
#[cfg(test)]
pub fn hidden_test_only(hidden_arg: hidden_ty) {}
"#;

fn api_of(source: &str) -> Vec<String> {
    let file = syn::parse_file(source).unwrap();
    let mut out = Vec::new();
    items(source, "m", &file.items, &mut out);
    out
}

/// Every identifier, literal and doc comment of `stream`: its byte range and a replacement.
fn tokens(stream: proc_macro2::TokenStream, source: &str, found: &mut Vec<(usize, usize, String)>) {
    for tree in stream {
        match tree {
            proc_macro2::TokenTree::Group(group) => tokens(group.stream(), source, found),
            proc_macro2::TokenTree::Ident(ident) => {
                let range = ident.span().byte_range();
                found.push((range.start, range.end, format!("{ident}Z")));
            }
            proc_macro2::TokenTree::Literal(literal) => {
                let range = literal.span().byte_range();
                let text = source.get(range.clone()).unwrap_or_default();
                let replacement = if text.starts_with("//") {
                    format!("{text}Z")
                } else if text.starts_with('"') {
                    "\"zz\"".to_owned()
                } else if text == "7" {
                    "8".to_owned()
                } else {
                    "7".to_owned()
                };
                found.push((range.start, range.end, replacement));
            }
            proc_macro2::TokenTree::Punct(_) => {}
        }
    }
}

#[test]
fn every_api_token_is_in_its_record_and_no_private_token_is() {
    // Each identifier, literal and doc comment of TOTAL is changed in turn. A change to an API
    // token must change the records; a change to a `hidden…` token or to doc prose must not. A
    // variant that no longer parses (a keyword changed) proves nothing and is skipped, but the
    // count of checked tokens must stay high, so the control cannot quietly shrink.
    let base = api_of(TOTAL);
    assert!(unvouched(&base.join("\n")).is_empty(), "{base:#?}");
    let mut found = Vec::new();
    tokens(TOTAL.parse().unwrap(), TOTAL, &mut found);
    let mut checked = 0_usize;
    let mut wrong = Vec::new();
    for (from, to, replacement) in found {
        let (Some(head), Some(original), Some(tail)) =
            (TOTAL.get(..from), TOTAL.get(from..to), TOTAL.get(to..))
        else {
            continue;
        };
        let variant = format!("{head}{replacement}{tail}");
        let Ok(file) = syn::parse_file(&variant) else {
            continue;
        };
        let mut out = Vec::new();
        items(&variant, "m", &file.items, &mut out);
        let private = original.starts_with("hidden") || original.starts_with("//");
        if (out == base) != private {
            let why = if private {
                "a private token changed the records"
            } else {
                "an API token is missing from the records"
            };
            wrong.push(format!("{original:?} at byte {from}: {why}"));
        }
        checked = checked.saturating_add(1);
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
    assert!(checked >= 130, "only {checked} tokens were checked");
}

#[test]
fn an_item_kind_the_reader_does_not_model_is_refused_not_recorded() {
    let out = api_of("pub trait Alias = Clone;\n");
    let api = out.join("\n");
    assert_eq!(unvouched(&api).len(), 1, "{out:#?}");
    assert!(api.contains(UNSUPPORTED), "{out:#?}");
}

/// Bastion's B-QTYPES-01 probes, kept as independent regressions: each pair must read differently.
const PROBES: &[(&str, &str, &str)] = &[
    (
        "a trait impl's associated constant (inherited visibility)",
        "impl P for S {\n    const N: u8 = 1;\n}\n",
        "impl P for S {\n    const N: u8 = 2;\n}\n",
    ),
    (
        "a public item inside an existing extern block",
        "unsafe extern \"C\" {\n    pub fn first();\n}\n",
        "unsafe extern \"C\" {\n    pub fn changed();\n}\n",
    ),
    (
        "a compact one-line trait method",
        "pub trait P { fn first(&self); }\n",
        "pub trait P { fn changed(&self); }\n",
    ),
    (
        "a formatted trait method",
        "pub trait P {\n    fn first(&self);\n}\n",
        "pub trait P {\n    fn changed(&self);\n}\n",
    ),
];

#[test]
fn bastion_b_qtypes_01_probes_each_change_the_records() {
    for (what, before, after) in PROBES {
        assert_ne!(api_of(before), api_of(after), "{what}");
    }
}

#[test]
fn a_member_kind_the_reader_does_not_model_is_refused_not_skipped() {
    // No syn kind reaches this path today (every kind is named), so it is driven directly.
    let mut out = Vec::new();
    members("", "m T::", 0, &[()], |()| None, |_| true, &mut out);
    assert_eq!(unvouched(&out.join("\n")).len(), 1, "{out:#?}");
}

/// Touchstone T-QTYPES8-01: literal payloads are API. Each pair is the same item with a different
/// literal value, so the records must differ.
const LITERAL_PAYLOADS: &[(&str, &str, &str)] = &[
    (
        "one space and two spaces in a string",
        "pub const L: &str = \"a b\";\n",
        "pub const L: &str = \"a  b\";\n",
    ),
    (
        "a raw string's newline and a space",
        "pub const R: &str = r#\"a\nb\"#;\n",
        "pub const R: &str = r#\"a b\"#;\n",
    ),
    (
        "a raw tab and a space in a string",
        "pub const T: &str = \"a\tb\";\n",
        "pub const T: &str = \"a b\";\n",
    ),
    (
        "an escaped newline and a real newline",
        "pub const N: &str = \"a\\nb\";\n",
        "pub const N: &str = \"a\nb\";\n",
    ),
    (
        "a byte string's spacing",
        "pub const B: &[u8] = b\"a b\";\n",
        "pub const B: &[u8] = b\"a   b\";\n",
    ),
    (
        "a char literal tab and a space",
        "pub const C: char = '\t';\n",
        "pub const C: char = ' ';\n",
    ),
    (
        "an attribute payload's spacing",
        "#[deprecated(note = \"use b\")]\npub fn a() {}\n",
        "#[deprecated(note = \"use  b\")]\npub fn a() {}\n",
    ),
    (
        "a raw string holding the text \\u{a} and one holding a newline",
        "pub const U: &str = r\"\\u{a}\";\n",
        "pub const U: &str = r\"\n\";\n",
    ),
    (
        "a raw string with embedded quotes and inner spacing",
        "pub const Q: &str = r#\"a \"x  y\" b\"#;\n",
        "pub const Q: &str = r#\"a \"x y\" b\"#;\n",
    ),
    (
        "a macro payload's spacing",
        "macro_rules! m { () => { \"x y\" }; }\n",
        "macro_rules! m { () => { \"x  y\" }; }\n",
    ),
];

/// Formatting outside literals is not API: each pair must read the same.
const FORMATTING_ONLY: &[(&str, &str, &str)] = &[
    (
        "spacing and line breaks between tokens",
        "pub const L: &str = \"a  b\";\npub fn f(x: u8) -> u8 { x }\n",
        "pub const L:   &str =\n    \"a  b\";\npub fn f(x: u8)\n    -> u8 {\n    x\n}\n",
    ),
    (
        "a comment's own spacing",
        "pub fn g() /* a  b */ {}\n",
        "pub fn g() /* a b */ {}\n",
    ),
    (
        "a quote inside a comment opens no literal",
        "pub fn h(/* don't */ x: u8) {}\n",
        "pub fn h(/* don't */  x:   u8) {}\n",
    ),
    (
        "a double quote inside a comment opens no string",
        "pub fn q(/* \" */ x: u8, /* \" */) {}\n",
        "pub fn q(/* \" */ x:  u8, /* \" */) {}\n",
    ),
    (
        "lifetimes are not char literals",
        "pub fn k<'a>(x: &'a str) -> &'a str { x }\n",
        "pub fn k<'a>(x: &'a str)\n    -> &'a str { x }\n",
    ),
];

#[test]
fn a_literal_payload_change_changes_the_records_and_formatting_does_not() {
    for (what, before, after) in LITERAL_PAYLOADS {
        assert_ne!(api_of(before), api_of(after), "{what}");
    }
    for (what, before, after) in FORMATTING_ONLY {
        assert_eq!(api_of(before), api_of(after), "{what}");
    }
}
