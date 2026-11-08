//! Oracle source isolation and independently reviewed golden contracts.
mod common;
use proc_macro2::{TokenStream, TokenTree};
use syn::ext::IdentExt as _;
use syn::parse::Parser as _;
use syn::visit::Visit as _;

fn forbidden(name: &str) -> bool {
    matches!(
        name,
        "forge_warden_core" | "forge_warden_cli" | "crate" | "super"
    )
}

fn token_coupling(tokens: TokenStream) -> bool {
    tokens.into_iter().any(|token| match token {
        TokenTree::Group(group) => token_coupling(group.stream()),
        TokenTree::Ident(ident) => {
            let name = ident.unraw().to_string();
            forbidden(&name) || matches!(name.as_str(), "include" | "include_str" | "include_bytes")
        }
        TokenTree::Literal(_) | TokenTree::Punct(_) => false,
    })
}

fn standard_import(tree: &syn::UseTree) -> bool {
    match tree {
        syn::UseTree::Path(path) => matches!(
            path.ident.unraw().to_string().as_str(),
            "std" | "core" | "alloc"
        ),
        syn::UseTree::Name(name) => matches!(
            name.ident.unraw().to_string().as_str(),
            "std" | "core" | "alloc"
        ),
        syn::UseTree::Rename(name) => matches!(
            name.ident.unraw().to_string().as_str(),
            "std" | "core" | "alloc"
        ),
        syn::UseTree::Group(group) => group.items.iter().all(standard_import),
        syn::UseTree::Glob(_) => false,
    }
}

fn oracle_roots(file: &syn::File) -> std::collections::BTreeSet<String> {
    fn imports(tree: &syn::UseTree, names: &mut std::collections::BTreeSet<String>) {
        match tree {
            syn::UseTree::Path(p) => imports(&p.tree, names),
            syn::UseTree::Name(n) => {
                names.insert(n.ident.unraw().to_string());
            }
            syn::UseTree::Rename(n) => {
                names.insert(n.rename.unraw().to_string());
            }
            syn::UseTree::Group(g) => g.items.iter().for_each(|t| imports(t, names)),
            syn::UseTree::Glob(_) => {}
        }
    }
    fn items(source: &[syn::Item], names: &mut std::collections::BTreeSet<String>) {
        for item in source {
            let ident = if let syn::Item::Struct(i) = item {
                Some(&i.ident)
            } else if let syn::Item::Enum(i) = item {
                Some(&i.ident)
            } else if let syn::Item::Type(i) = item {
                Some(&i.ident)
            } else if let syn::Item::Trait(i) = item {
                Some(&i.ident)
            } else if let syn::Item::Union(i) = item {
                Some(&i.ident)
            } else if let syn::Item::Mod(i) = item {
                Some(&i.ident)
            } else {
                None
            };
            if let Some(name) = ident {
                names.insert(name.unraw().to_string());
            }
            if let syn::Item::Use(i) = item {
                imports(&i.tree, names);
            }
            if let syn::Item::Mod(i) = item
                && let Some((_, nested)) = &i.content
            {
                items(nested, names);
            }
        }
    }
    let mut names = [
        "std", "core", "alloc", "Self", "Vec", "String", "Option", "Result", "Box", "bool", "char",
        "str", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128",
        "isize", "f32", "f64",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    items(&file.items, &mut names);
    names
}

// Parse only the argument grammars of the admitted standard macros, with syn nodes.
// Opaque macro tokens never acquire an exemption from path checking.
fn macro_arguments(mac: &syn::Macro, name: &str) -> syn::Result<(Vec<syn::Expr>, Vec<syn::Pat>)> {
    let parse = |input: syn::parse::ParseStream<'_>| {
        if name == "vec" && input.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        let mut expressions = vec![input.parse::<syn::Expr>()?];
        let mut patterns = Vec::new();
        if name == "matches" {
            input.parse::<syn::Token![,]>()?;
            patterns.push(input.call(syn::Pat::parse_multi_with_leading_vert)?);
            if input.peek(syn::Token![if]) {
                input.parse::<syn::Token![if]>()?;
                expressions.push(input.parse::<syn::Expr>()?);
            }
            if input.peek(syn::Token![,]) {
                input.parse::<syn::Token![,]>()?;
            }
        } else if name == "vec" && input.peek(syn::Token![;]) {
            input.parse::<syn::Token![;]>()?;
            expressions.push(input.parse::<syn::Expr>()?);
        } else {
            while input.peek(syn::Token![,]) {
                input.parse::<syn::Token![,]>()?;
                if input.is_empty() {
                    break;
                }
                expressions.push(input.parse::<syn::Expr>()?);
            }
        }
        if !input.is_empty() {
            return Err(input.error("unsupported standard-macro arguments"));
        }
        Ok((expressions, patterns))
    };
    parse.parse2(mac.tokens.clone())
}

struct Isolation {
    errors: Vec<String>,
    roots: std::collections::BTreeSet<String>,
}

impl<'ast> syn::visit::Visit<'ast> for Isolation {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        if matches!(item, syn::Item::Verbatim(_)) {
            self.errors.push("unclassified oracle item".to_owned());
        }
        syn::visit::visit_item(self, item);
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        if matches!(item, syn::ImplItem::Verbatim(_)) {
            self.errors.push("unclassified oracle impl item".to_owned());
        }
        syn::visit::visit_impl_item(self, item);
    }

    fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
        if matches!(item, syn::TraitItem::Verbatim(_)) {
            self.errors
                .push("unclassified oracle trait item".to_owned());
        }
        syn::visit::visit_trait_item(self, item);
    }

    fn visit_expr(&mut self, expr: &'ast syn::Expr) {
        if matches!(expr, syn::Expr::Verbatim(_)) {
            self.errors
                .push("unclassified oracle expression".to_owned());
        }
        syn::visit::visit_expr(self, expr);
    }

    fn visit_type(&mut self, ty: &'ast syn::Type) {
        if matches!(ty, syn::Type::Verbatim(_)) {
            self.errors.push("unclassified oracle type".to_owned());
        }
        syn::visit::visit_type(self, ty);
    }

    fn visit_pat(&mut self, pat: &'ast syn::Pat) {
        if matches!(pat, syn::Pat::Verbatim(_)) {
            self.errors.push("unclassified oracle pattern".to_owned());
        }
        syn::visit::visit_pat(self, pat);
    }

    fn visit_foreign_item(&mut self, item: &'ast syn::ForeignItem) {
        self.errors
            .push("foreign oracle binding is not admitted".to_owned());
        syn::visit::visit_foreign_item(self, item);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        if path.segments.len() > 1
            && path
                .segments
                .first()
                .is_some_and(|segment| !self.roots.contains(&segment.ident.unraw().to_string()))
        {
            self.errors
                .push("undeclared qualified oracle root".to_owned());
        }
        if path
            .segments
            .iter()
            .any(|s| forbidden(&s.ident.unraw().to_string()))
        {
            self.errors
                .push("production or parent-module path".to_owned());
        }
        syn::visit::visit_path(self, path);
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        if !standard_import(&item.tree) {
            self.errors
                .push("only standard-library imports are admitted".to_owned());
        }
        syn::visit::visit_item_use(self, item);
    }

    fn visit_item_extern_crate(&mut self, _item: &'ast syn::ItemExternCrate) {
        self.errors.push("extern crate is not admitted".to_owned());
    }

    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if item.content.is_none() {
            self.errors
                .push("out-of-line oracle module; declare its source explicitly".to_owned());
        }
        syn::visit::visit_item_mod(self, item);
    }

    fn visit_visibility(&mut self, visibility: &'ast syn::Visibility) {
        // pub(super) exposes an oracle result to its harness; it is not a value/type dependency.
        if matches!(visibility, syn::Visibility::Restricted(r) if r.path.is_ident("super")) {
            return;
        }
        syn::visit::visit_visibility(self, visibility);
    }

    fn visit_attribute(&mut self, attr: &'ast syn::Attribute) {
        if attr.path().is_ident("doc")
            && matches!(&attr.meta, syn::Meta::NameValue(n) if matches!(&n.value,
                syn::Expr::Lit(lit) if matches!(lit.lit, syn::Lit::Str(_))))
        {
            return;
        }
        if attr.path().is_ident("derive") {
            let parser = syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated;
            let valid = attr.parse_args_with(parser).is_ok_and(|paths| {
                paths.iter().all(|path| {
                    path.segments.len() == 1
                        && path.get_ident().is_some_and(|id| {
                            [
                                "Clone",
                                "Copy",
                                "Debug",
                                "PartialEq",
                                "Eq",
                                "PartialOrd",
                                "Ord",
                                "Default",
                            ]
                            .contains(&id.unraw().to_string().as_str())
                        })
                })
            });
            if valid {
                return;
            }
        }
        self.errors
            .push("oracle attribute or derive expansion is not admitted".to_owned());
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        let names: Vec<String> = mac
            .path
            .segments
            .iter()
            .map(|s| s.ident.unraw().to_string())
            .collect();
        let known = names.len() == 2
            && names.first().is_some_and(|s| s == "std" || s == "core")
            && names.last().is_some_and(|s| {
                [
                    "matches",
                    "vec",
                    "assert",
                    "assert_eq",
                    "assert_ne",
                    "debug_assert",
                    "debug_assert_eq",
                    "debug_assert_ne",
                ]
                .contains(&s.as_str())
            });
        if !known || token_coupling(mac.tokens.clone()) {
            self.errors
                .push("oracle macro expansion/include is not admitted".to_owned());
        } else if let Some(name) = names.last() {
            match macro_arguments(mac, name) {
                Ok((expressions, patterns)) => {
                    for expr in &expressions {
                        self.visit_expr(expr);
                    }
                    for pat in &patterns {
                        self.visit_pat(pat);
                    }
                }
                Err(error) => self
                    .errors
                    .push(format!("oracle macro arguments refused: {error}")),
            }
        }
        syn::visit::visit_macro(self, mac);
    }
}

fn oracle_violations(source: &str) -> Vec<String> {
    let tree = match syn::parse_file(source) {
        Ok(tree) => tree,
        Err(error) => return vec![format!("oracle syntax refused: {error}")],
    };
    let mut visitor = Isolation {
        errors: Vec::new(),
        roots: oracle_roots(&tree),
    };
    visitor.visit_file(&tree);
    visitor.errors
}

#[test]
fn oracle_isolation_rejects_production_coupling_in_every_syntax_position() {
    for source in [
        "use forge_warden_core as production;",
        "use ::forge_warden_cli::Cli;",
        "extern crate forge_warden_core as model;",
        "fn oracle() { let _ = external_alias::evaluate(); }",
        "fn oracle() { let _ = std::matches!(x, external_alias::State); }",
        "fn oracle() { let _ = forge_warden_core::TOOL_VERSION; }",
        "fn oracle() { let _ = r#forge_warden_core::TOOL_VERSION; }",
        "fn oracle() { let _ = crate::matcher(); }",
        "fn oracle() { let _ = super::matcher(); }",
        "fn oracle() { std::matches!(x, forge_warden_core::Outcome::Pass); }",
        "include!(\"../../crates/forge-warden-core/src/eval.rs\");",
        "const MODEL: &str = include_str!(\"../../crates/forge-warden-core/src/eval.rs\");",
        "#[path = \"../../crates/forge-warden-core/src/eval.rs\"] mod model;",
        "#[cfg_attr(any(), path = \"hidden.rs\")] mod model {}",
        "mod external;",
        "#[doc = include_str!(\"../../crates/forge-warden-core/src/eval.rs\")] fn oracle() {}",
        "fn oracle() { host_macro!(); }",
        "macro_rules! model { () => { 1 }; }",
        "#[derive(ProductionOracle)] struct Case;",
    ] {
        assert!(
            !oracle_violations(source).is_empty(),
            "oracle coupling escaped: {source}"
        );
    }
    for source in [
        "use std::num::NonZeroU32; #[derive(Clone, Debug)] struct Case { n: NonZeroU32 }",
        "enum State { Clear } fn oracle() { let _ = State::Clear; }",
        "fn oracle() { let _ = std::matches!(Some(1), Some(_)); }",
        "fn oracle() { std::assert_eq!(Vec::<u8>::new(), std::vec![]); }",
        "use std::num::NonZeroU32 as Count; fn oracle() { let _ = Count::new(1); }",
        "mod local { pub fn value() -> u8 { 1 } } fn oracle() { let _ = local::value(); }",
        "// forge_warden_core::evaluate\nconst DOC: &str = \"forge_warden_core\";",
    ] {
        assert!(
            oracle_violations(source).is_empty(),
            "independent source refused: {source}"
        );
    }
}

#[test]
fn declared_oracles_are_independent() {
    let registry: serde_json::Value = serde_json::from_str(
        &common::read_fixture_regular("fixtures/quality-contract.json").unwrap(),
    )
    .unwrap();
    assert_eq!(
        registry["schema"].as_str(),
        Some("forge-warden.quality-contract.v1")
    );
    assert_eq!(registry.as_object().unwrap().len(), 3);
    let sources = registry["oracles"].as_array().unwrap();
    assert!(!sources.is_empty(), "oracle registry must not be empty");
    assert!(
        sources
            .iter()
            .any(|p| p.as_str() == Some("fixtures/adversarial/reference_model.rs")),
        "the independent reference model must remain declared"
    );
    let mut seen = std::collections::BTreeSet::new();
    for value in sources {
        let name = value.as_str().unwrap();
        assert!(seen.insert(name), "duplicate oracle declaration: {name}");
        let source = common::read_fixture_regular(name).unwrap();
        assert!(
            std::path::Path::new(name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("rs")),
            "oracle must be a Rust source: {name}"
        );
        let errors = oracle_violations(&source);
        assert!(errors.is_empty(), "ORACLE_INDEPENDENCE: {name}: {errors:?}");
    }
}

#[test]
fn golden_outputs_require_independent_review() {
    let registry: serde_json::Value = serde_json::from_str(
        &common::read_fixture_regular("fixtures/quality-contract.json").unwrap(),
    )
    .unwrap();
    let goldens = registry["goldens"].as_array().unwrap();
    assert!(!goldens.is_empty(), "golden registry must not be empty");
    let policy: toml::Value = toml::from_str(&common::read("warden.toml")).unwrap();
    let rules = policy["rule"].as_array().unwrap();
    let Some(rule) = rules
        .iter()
        .find(|r| r["id"].as_str() == Some("golden-output-needs-review"))
    else {
        panic!("GOLDEN_REVIEW_REQUIRED: named trusted-base rule missing");
    };
    assert_eq!(rule["kind"].as_str(), Some("protect_paths"));
    assert_eq!(rule["severity"].as_str(), Some("hold"));
    assert_eq!(
        rule["paths"].as_array().unwrap(),
        &vec![toml::Value::String("fixtures/**".to_owned())]
    );
    assert!(
        rule.get("except").is_none(),
        "golden review must have no exception"
    );
    let mut seen = std::collections::BTreeSet::new();
    for value in goldens {
        let name = value.as_str().unwrap();
        assert!(seen.insert(name), "duplicate golden declaration");
        assert!(
            common::read_fixture_regular(name).is_ok(),
            "golden outside reserved fixture tree or not regular"
        );
    }
    for path in common::files_under("fixtures", ".verdict.json") {
        assert!(
            seen.contains(common::relative(&path).as_str()),
            "undeclared verdict golden"
        );
    }
}

#[test]
fn explicit_golden_producer_checks_bytes_and_refuses_ci_writes() {
    let before = common::read_bytes("fixtures/demo.verdict.json");
    let checked = common::check_golden_output(false);
    assert!(
        checked.status.success(),
        "producer byte check failed: {checked:?}"
    );
    let record: serde_json::Value = serde_json::from_slice(&checked.stdout).unwrap();
    assert_eq!(record["mode"].as_str(), Some("check"));
    assert_eq!(record["binary_sha256"].as_str().unwrap().len(), 64);
    assert_eq!(record["output_sha256"].as_str().unwrap().len(), 64);
    assert_eq!(common::read_bytes("fixtures/demo.verdict.json"), before);
    let refused = common::check_golden_output(true);
    assert_eq!(refused.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&refused.stderr).contains("regeneration is refused in CI"));
    assert_eq!(common::read_bytes("fixtures/demo.verdict.json"), before);
}
