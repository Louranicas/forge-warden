//! Syn authority for source-written suppression syntax; Python independently checks the ledger.
use proc_macro2::{Delimiter, TokenStream, TokenTree};
use syn::ext::IdentExt as _;
use syn::punctuated::Punctuated;
use syn::visit::Visit;
use syn::{Meta, Token};

/// A documentation literal or a single macro documentation parameter cannot lower a lint.
pub(crate) fn is_plain_doc(tokens: &TokenStream) -> bool {
    let trees: Vec<TokenTree> = tokens.clone().into_iter().collect();
    let doc_eq = matches!(trees.first(), Some(TokenTree::Ident(i)) if i.unraw() == "doc")
        && matches!(trees.get(1), Some(TokenTree::Punct(p)) if p.as_char() == '=');
    match trees.as_slice() {
        [_, _, TokenTree::Literal(_)] => doc_eq,
        [_, _, TokenTree::Punct(dollar), TokenTree::Ident(_)] => doc_eq && dollar.as_char() == '$',
        _ => false,
    }
}

fn name(meta: &Meta) -> String {
    meta.path()
        .segments
        .iter()
        .map(|s| s.ident.unraw().to_string())
        .collect::<Vec<_>>()
        .join("::")
}

#[derive(Default)]
struct Scan {
    expectations: Vec<(String, String)>,
    errors: Vec<String>,
}

impl Scan {
    fn inspect(&mut self, meta: &Meta) {
        let kind = name(meta);
        if kind == "allow" {
            self.errors.push("allow attribute forbidden".to_owned());
            return;
        }
        if kind != "expect" && kind != "cfg_attr" {
            return;
        }
        let Meta::List(list) = meta else {
            self.errors.push(format!("{kind} requires arguments"));
            return;
        };
        let Ok(items) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
        else {
            self.errors.push(format!("{kind} arguments do not parse"));
            return;
        };
        if kind == "cfg_attr" {
            if items.len() < 2 {
                self.errors
                    .push("cfg_attr requires predicate and attribute".to_owned());
            }
            for nested in items.iter().skip(1) {
                self.inspect(nested);
            }
            return;
        }
        let mut reasons = Vec::new();
        let mut lints = Vec::new();
        for item in &items {
            match item {
                Meta::NameValue(value) if name(item) == "reason" => {
                    if let syn::Expr::Lit(expr) = &value.value
                        && let syn::Lit::Str(reason) = &expr.lit
                    {
                        reasons.push(reason.value());
                    } else {
                        self.errors
                            .push("expect reason must be a string literal".to_owned());
                    }
                }
                Meta::Path(_) => lints.push(name(item)),
                Meta::NameValue(_) | Meta::List(_) => {
                    self.errors
                        .push("unsupported expectation argument".to_owned());
                }
            }
        }
        if reasons.len() != 1
            || reasons.first().is_none_or(|r| r.trim().is_empty())
            || lints.is_empty()
        {
            self.errors
                .push("expect needs lints and one nonempty literal reason".to_owned());
            return;
        }
        if let Some(reason) = reasons.first() {
            self.expectations
                .extend(lints.into_iter().map(|lint| (lint, reason.clone())));
        }
    }

    fn tokens(&mut self, tokens: TokenStream) {
        let trees: Vec<TokenTree> = tokens.into_iter().collect();
        for (index, tree) in trees.iter().enumerate() {
            if let TokenTree::Group(group) = tree {
                self.tokens(group.stream());
            }
            if !matches!(tree, TokenTree::Punct(mark) if mark.as_char() == '#') {
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
                    Ok(meta) => self.inspect(&meta),
                    Err(error) => self
                        .errors
                        .push(format!("macro attribute does not parse: {error}")),
                }
            }
        }
    }
}

impl<'ast> Visit<'ast> for Scan {
    fn visit_attribute(&mut self, attr: &'ast syn::Attribute) {
        self.inspect(&attr.meta);
        syn::visit::visit_attribute(self, attr);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        self.tokens(mac.tokens.clone());
        syn::visit::visit_macro(self, mac);
    }
}

/// Parse all source-written attributes, including inactive cfg arms and macro token bodies.
pub(crate) fn expectations(source: &str) -> Result<Vec<(String, String)>, String> {
    let parsed = syn::parse_file(source).map_err(|e| format!("Rust source does not parse: {e}"))?;
    let mut scan = Scan::default();
    scan.visit_file(&parsed);
    if scan.errors.is_empty() {
        Ok(scan.expectations)
    } else {
        Err(scan.errors.join("; "))
    }
}
