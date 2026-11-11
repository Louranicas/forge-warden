//! REFUSAL-MAP (finish condition 5, ledger Q38): the map from every refusal and exit to the test
//! that observes it.
//!
//! The rows are the arms of the exhaustive matches in `crates/forge-warden-core/src/refusal_rows.rs`
//! (layer 1: the compiler refuses a variant without a row). This test parses that file and checks
//! layer 2: every arm returns its own variant's name (so no arm borrows another variant's row),
//! every cited test exists as a `#[test]` function in the file it names, and `docs/REFUSAL_MAP.md`
//! is exactly the rendering of the rows. Finding a test by name proves nothing about what it
//! observes; layer 3, one plant per row in `plants/catalog.json`, proves that.

mod common;

use common::read;
use std::collections::BTreeMap;
use syn::visit::Visit;

const ROWS: &str = "crates/forge-warden-core/src/refusal_rows.rs";
const MAP: &str = "docs/REFUSAL_MAP.md";
const MISSING: &str = "MISSING";
/// Rows still without an observing test. It only goes down, and is zero when the package is READY.
const MISSING_AT_MOST: usize = 14;

/// One row of the map, in the order the rows file declares it.
struct Row {
    refusal: String,
    variant: String,
    test: String,
}

fn literal(expr: &syn::Expr) -> Option<String> {
    if let syn::Expr::Lit(syn::ExprLit {
        lit: syn::Lit::Str(text),
        ..
    }) = expr
    {
        Some(text.value())
    } else {
        None
    }
}

fn constants<'a>(items: impl IntoIterator<Item = &'a syn::Item>, into: &mut BTreeMap<String, String>) {
    for item in items {
        if let syn::Item::Const(constant) = item
            && let Some(text) = literal(&constant.expr)
        {
            into.insert(constant.ident.to_string(), text);
        }
    }
}

/// The arm's pattern as `Enum::Variant`.
fn pattern_path(pat: &syn::Pat) -> Option<&syn::Path> {
    if let syn::Pat::Path(path) = pat {
        Some(&path.path)
    } else if let syn::Pat::TupleStruct(tuple) = pat {
        Some(&tuple.path)
    } else if let syn::Pat::Struct(fields) = pat {
        Some(&fields.path)
    } else {
        None
    }
}

/// The test an arm cites: a string literal, or a constant declared in the file or the function.
fn cited(expr: &syn::Expr, names: &BTreeMap<String, String>) -> Option<String> {
    if let Some(text) = literal(expr) {
        return Some(text);
    }
    if let syn::Expr::Path(path) = expr {
        path.path
            .get_ident()
            .and_then(|ident| names.get(&ident.to_string()).cloned())
    } else {
        None
    }
}

fn rows() -> Result<Vec<Row>, String> {
    let file = syn::parse_file(&read(ROWS)).map_err(|e| format!("{ROWS}: {e}"))?;
    let mut file_constants = BTreeMap::new();
    constants(&file.items, &mut file_constants);
    let mut out = Vec::new();
    for item in &file.items {
        let syn::Item::Fn(function) = item else {
            continue;
        };
        let mut names = file_constants.clone();
        constants(
            function.block.stmts.iter().filter_map(|stmt| {
                if let syn::Stmt::Item(declared) = stmt {
                    Some(declared)
                } else {
                    None
                }
            }),
            &mut names,
        );
        for stmt in &function.block.stmts {
            let syn::Stmt::Expr(syn::Expr::Match(matched), _) = stmt else {
                continue;
            };
            for arm in &matched.arms {
                let where_ = format!("{ROWS}: fn {}", function.sig.ident);
                let path = pattern_path(&arm.pat).ok_or(format!("{where_}: an arm is not Enum::Variant"))?;
                let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
                let [refusal, variant] = segments.as_slice() else {
                    return Err(format!("{where_}: pattern {segments:?} is not Enum::Variant"));
                };
                let syn::Expr::Tuple(pair) = &*arm.body else {
                    return Err(format!("{where_}: {refusal}::{variant} does not return (name, test)"));
                };
                let elems: Vec<&syn::Expr> = pair.elems.iter().collect();
                let [name, test] = elems.as_slice() else {
                    return Err(format!("{where_}: {refusal}::{variant} does not return a pair"));
                };
                let name = literal(name).ok_or(format!("{where_}: {variant}'s name is not a literal"))?;
                if name != *variant {
                    return Err(format!(
                        "{where_}: the arm for {refusal}::{variant} names {name:?}; each arm names its own variant"
                    ));
                }
                let test = cited(test, &names).ok_or(format!("{where_}: {variant}'s test is not a literal or a constant"))?;
                out.push(Row {
                    refusal: refusal.clone(),
                    variant: variant.clone(),
                    test,
                });
            }
        }
    }
    Ok(out)
}

/// Every `#[test]` function name in a file, at any depth.
#[derive(Default)]
struct Tests(Vec<String>);

impl<'ast> Visit<'ast> for Tests {
    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if item.attrs.iter().any(|a| a.path().is_ident("test")) {
            self.0.push(item.sig.ident.to_string());
        }
        syn::visit::visit_item_fn(self, item);
    }
}

fn render(rows: &[Row]) -> String {
    let mut text = String::from(
        "# Refusal map\n\n\
         Every refusal variant and every exit, with the one test that observes it. Generated from\n\
         `crates/forge-warden-core/src/refusal_rows.rs` and checked byte for byte by\n\
         `crates/forge-warden-cli/tests/refusal_map.rs`; edit the rows, not this file.\n\n\
         A row names its test by `path::function`. That the test exists is checked by name only; that\n\
         it observes its variant is proven by one plant per row in `plants/catalog.json`.\n\n\
         | Refusal | Observing test |\n| --- | --- |\n",
    );
    let table: String = rows
        .iter()
        .map(|row| {
            let cell = if row.test == MISSING {
                MISSING.to_owned()
            } else {
                format!("`{}`", row.test)
            };
            format!("| `{}::{}` | {cell} |\n", row.refusal, row.variant)
        })
        .collect();
    text.push_str(&table);
    text
}

#[test]
fn every_row_names_its_own_variant_and_cites_a_test_that_exists() -> Result<(), String> {
    let rows = rows()?;
    if rows.is_empty() {
        return Err(format!("{ROWS}: no rows found"));
    }
    let mut found: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in rows.iter().filter(|row| row.test != MISSING) {
        let (path, function) = row
            .test
            .split_once("::")
            .ok_or(format!("{}::{}: test {:?} is not path::function", row.refusal, row.variant, row.test))?;
        if !found.contains_key(path) {
            let file = syn::parse_file(&read(path)).map_err(|e| format!("{path}: {e}"))?;
            let mut tests = Tests::default();
            tests.visit_file(&file);
            found.insert(path.to_owned(), tests.0);
        }
        if !found.get(path).is_some_and(|names| names.iter().any(|n| n == function)) {
            return Err(format!(
                "{}::{}: no #[test] fn {function} in {path}",
                row.refusal, row.variant
            ));
        }
    }
    Ok(())
}

#[test]
fn the_refusal_map_is_the_rendering_of_the_rows() -> Result<(), String> {
    let rendered = render(&rows()?);
    if read(MAP) == rendered {
        Ok(())
    } else {
        Err(format!("{MAP} differs from the rows; re-render it as:\n{rendered}"))
    }
}

#[test]
fn rows_without_an_observing_test_only_go_down() -> Result<(), String> {
    let missing = rows()?.iter().filter(|row| row.test == MISSING).count();
    if missing <= MISSING_AT_MOST {
        Ok(())
    } else {
        Err(format!("{missing} rows are MISSING; at most {MISSING_AT_MOST} may be"))
    }
}
