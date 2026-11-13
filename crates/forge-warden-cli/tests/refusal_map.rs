//! REFUSAL-MAP (finish condition 5, ledger Q38): the map from every refusal and exit to the test
//! that observes it.
//!
//! The rows are the arms of the exhaustive matches in `crates/forge-warden-core/src/refusal_rows.rs`
//! and `crates/forge-warden-cli/src/refusal_rows.rs`
//! (layer 1: the compiler refuses a variant without a row). This test parses both files, fails closed
//! on anything in them that is not a row, a constant or a test, and checks layer 2: the rows cover
//! exactly the refusal enums of the map's scope, every arm returns its own variant's name (so no arm
//! borrows another variant's row), every cited test is a runnable `#[test]` function in the file it
//! names (not ignored, not behind another cfg), and `docs/REFUSAL_MAP.md` is exactly the rendering of
//! the rows. Finding a test by name proves nothing about what it observes: three sampled variant-swap
//! plants (guardrail G-53 in `plants/catalog.json`) prove it for three rows; for the others the cited
//! test's assertion was read, not proven by a plant.

mod common;

use common::read;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use syn::visit::Visit;

/// The rows files, core first: their rows render in this order.
const ROWS: [&str; 2] = [
    "crates/forge-warden-core/src/refusal_rows.rs",
    "crates/forge-warden-cli/src/refusal_rows.rs",
];
const MAP: &str = "docs/REFUSAL_MAP.md";
const MISSING: &str = "MISSING";
/// A row with no reachable trigger states why, after this prefix, instead of naming a test.
const UNREACHABLE: &str = "UNREACHABLE: ";
/// Rows stated unreachable. It only goes down.
const UNREACHABLE_AT_MOST: usize = 1;
/// Rows still without an observing test. It only goes down, and is zero when the package is READY.
const MISSING_AT_MOST: usize = 0;
/// The enums the map covers (DESIGN scope: 10 refusal enums and Exit). Rows that drop one are red.
const ENUMS: [&str; 11] = [
    "Unverifiable",
    "PathError",
    "PolicyError",
    "RankError",
    "SealError",
    "DateError",
    "Exit",
    "DoorErrorKind",
    "ChildFailure",
    "RankerFailure",
    "GitFactsFailure",
];
/// The test-module gates the repository uses; any other `cfg` hides a test from the gate.
const TEST_GATES: [&str; 2] = ["test", "not(fw_fixture_cli)"];

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
    let mut out = Vec::new();
    for path in ROWS {
        out.extend(rows_from(path, &read(path))?);
    }
    covers_the_scope(&out)?;
    Ok(out)
}

fn covers_the_scope(rows: &[Row]) -> Result<(), String> {
    let found: BTreeSet<&str> = rows.iter().map(|row| row.refusal.as_str()).collect();
    let expected: BTreeSet<&str> = ENUMS.into_iter().collect();
    if found == expected {
        Ok(())
    } else {
        Err(format!(
            "the rows cover {found:?}; the map covers exactly {expected:?}"
        ))
    }
}

fn is_test(function: &syn::ItemFn) -> bool {
    function.attrs.iter().any(|a| a.path().is_ident("test"))
}

/// Every row in one rows file. Fails closed: each top-level item must be a `use`, a string `const`,
/// the `Row` alias, a `#[test] fn`, or a rows fn (local string consts, then exactly one `match`).
fn rows_from(label: &str, source: &str) -> Result<Vec<Row>, String> {
    let file = syn::parse_file(source).map_err(|e| format!("{label}: {e}"))?;
    let mut file_constants = BTreeMap::new();
    for item in &file.items {
        if let syn::Item::Const(constant) = item {
            let text = literal(&constant.expr).ok_or(format!(
                "{label}: const {} is not a string literal",
                constant.ident
            ))?;
            file_constants.insert(constant.ident.to_string(), text);
        }
    }
    let mut out = Vec::new();
    for item in &file.items {
        if matches!(item, syn::Item::Use(_) | syn::Item::Const(_)) {
            continue;
        }
        if let syn::Item::Type(alias) = item
            && alias.ident == "Row"
        {
            continue;
        }
        let syn::Item::Fn(function) = item else {
            return Err(format!(
                "{label}: an item that is not a use, a const, the Row alias, a #[test] fn or a rows fn"
            ));
        };
        if !is_test(function) {
            rows_of_fn(label, function, &file_constants, &mut out)?;
        }
    }
    Ok(out)
}

fn rows_of_fn(
    label: &str,
    function: &syn::ItemFn,
    file_constants: &BTreeMap<String, String>,
    out: &mut Vec<Row>,
) -> Result<(), String> {
    let where_ = format!("{label}: fn {}", function.sig.ident);
    let Some((last, before)) = function.block.stmts.split_last() else {
        return Err(format!("{where_}: an empty rows fn"));
    };
    let mut names = file_constants.clone();
    for stmt in before {
        let syn::Stmt::Item(syn::Item::Const(constant)) = stmt else {
            return Err(format!(
                "{where_}: a rows fn holds only string consts before its one match"
            ));
        };
        let text = literal(&constant.expr).ok_or(format!(
            "{where_}: const {} is not a string literal",
            constant.ident
        ))?;
        names.insert(constant.ident.to_string(), text);
    }
    let syn::Stmt::Expr(syn::Expr::Match(matched), None) = last else {
        return Err(format!("{where_}: a rows fn ends in exactly one match"));
    };
    for arm in &matched.arms {
        let path =
            pattern_path(&arm.pat).ok_or(format!("{where_}: an arm is not Enum::Variant"))?;
        let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
        let [refusal, variant] = segments.as_slice() else {
            return Err(format!(
                "{where_}: pattern {segments:?} is not Enum::Variant"
            ));
        };
        let syn::Expr::Tuple(pair) = &*arm.body else {
            return Err(format!(
                "{where_}: {refusal}::{variant} does not return (name, test)"
            ));
        };
        let elems: Vec<&syn::Expr> = pair.elems.iter().collect();
        let [name, test] = elems.as_slice() else {
            return Err(format!(
                "{where_}: {refusal}::{variant} does not return a pair"
            ));
        };
        let name = literal(name).ok_or(format!("{where_}: {variant}'s name is not a literal"))?;
        if name != *variant {
            return Err(format!(
                "{where_}: the arm for {refusal}::{variant} names {name:?}; each arm names its own variant"
            ));
        }
        let test = cited(test, &names).ok_or(format!(
            "{where_}: {variant}'s test is not a literal or a constant"
        ))?;
        out.push(Row {
            refusal: refusal.clone(),
            variant: variant.clone(),
            test,
        });
    }
    Ok(())
}

/// Whether an attribute list hides an item from the gate: any `cfg_attr`, or a `cfg` other than
/// the repository's test-module gates.
fn gated(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attribute| {
        let path = attribute.path();
        if path.is_ident("cfg_attr") {
            return true;
        }
        if !path.is_ident("cfg") {
            return false;
        }
        // A cfg whose condition cannot be read is treated as hiding the test (fail closed).
        let Ok(list) = attribute.meta.require_list() else {
            return true;
        };
        let condition: String = list
            .tokens
            .to_string()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        !TEST_GATES.contains(&condition.as_str())
    })
}

/// The runnable `#[test]` fns of a file: not `#[ignore]`d, and neither the fn nor an enclosing `mod`
/// behind a `cfg` other than the repository's test-module gates.
#[derive(Default)]
struct Tests {
    runnable: Vec<String>,
    hidden: Vec<bool>,
}

impl<'ast> Visit<'ast> for Tests {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        self.hidden.push(gated(&item.attrs));
        syn::visit::visit_item_mod(self, item);
        self.hidden.pop();
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        let ignored = item.attrs.iter().any(|a| a.path().is_ident("ignore"));
        if is_test(item) && !ignored && !gated(&item.attrs) && !self.hidden.iter().any(|h| *h) {
            self.runnable.push(item.sig.ident.to_string());
        }
        syn::visit::visit_item_fn(self, item);
    }
}

fn runnable_tests(label: &str, source: &str) -> Result<Vec<String>, String> {
    let file = syn::parse_file(source).map_err(|e| format!("{label}: {e}"))?;
    let mut tests = Tests::default();
    tests.visit_file(&file);
    Ok(tests.runnable)
}

fn render(rows: &[Row]) -> Result<String, String> {
    let mut text = String::from(
        "# Refusal map\n\n\
         Every variant of the 11 enums listed below (10 refusal enums and `Exit`), with the one test that\n\
         observes it. Generated from `crates/forge-warden-core/src/refusal_rows.rs` and\n\
         `crates/forge-warden-cli/src/refusal_rows.rs`, and checked byte for byte by\n\
         `crates/forge-warden-cli/tests/refusal_map.rs`; edit the rows, not this file.\n\
         \n\
         Scope: exactly these 11 enums (`Unverifiable`, `PathError`, `PolicyError`, `RankError`, `SealError`,\n\
         `DateError`, `Exit` in the core; `DoorErrorKind`, `ChildFailure`, `RankerFailure`, `GitFactsFailure` in\n\
         the CLI). Outside it: the reasons inside `RankError::Invalid` (one row for all of them), `Disagreement`\n\
         (a disagreement with Git becomes a note, not a refusal), the Git allowlist's admission refusals and the\n\
         CLI's named error messages (each reaches the user as one of the exits below), and the refusals of the\n\
         repository scripts (`scripts/hold_ack.py`, `scripts/dogfood.sh`), which have their own tests.\n\
         \n\
         A row names its test by `path::function`; the test is checked to exist and to run (not ignored,\n\
         not behind another cfg). Three rows are proven by a plant that swaps the variant for a sibling\n\
         (guardrail G-53 in `plants/catalog.json`); for every other row the test's assertion was read, not\n\
         proven by a plant.\n\
         \n\
         | Refusal | Observing test |\n| --- | --- |\n",
    );
    for row in rows {
        let cell = if row.test == MISSING || row.test.starts_with(UNREACHABLE) {
            row.test.clone()
        } else {
            format!("`{}`", row.test)
        };
        writeln!(text, "| `{}::{}` | {cell} |", row.refusal, row.variant)
            .map_err(|e| e.to_string())?;
    }
    Ok(text)
}

#[test]
fn every_row_names_its_own_variant_and_cites_a_test_that_exists() -> Result<(), String> {
    let rows = rows()?;
    if rows.is_empty() {
        return Err("no rows found".to_owned());
    }
    let mut found: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in rows
        .iter()
        .filter(|row| row.test != MISSING && !row.test.starts_with(UNREACHABLE))
    {
        let (path, function) = row.test.split_once("::").ok_or(format!(
            "{}::{}: test {:?} is not path::function",
            row.refusal, row.variant, row.test
        ))?;
        if !found.contains_key(path) {
            found.insert(path.to_owned(), runnable_tests(path, &read(path))?);
        }
        if !found
            .get(path)
            .is_some_and(|names| names.iter().any(|n| n == function))
        {
            return Err(format!(
                "{}::{}: no runnable #[test] fn {function} in {path}",
                row.refusal, row.variant
            ));
        }
    }
    Ok(())
}

#[test]
fn the_refusal_map_is_the_rendering_of_the_rows() -> Result<(), String> {
    let rendered = render(&rows()?)?;
    if read(MAP) == rendered {
        Ok(())
    } else {
        Err(format!(
            "{MAP} differs from the rows; re-render it as:\n{rendered}"
        ))
    }
}

#[test]
fn rows_without_an_observing_test_only_go_down() -> Result<(), String> {
    let rows = rows()?;
    let missing = rows.iter().filter(|row| row.test == MISSING).count();
    let unreachable = rows
        .iter()
        .filter(|row| row.test.starts_with(UNREACHABLE))
        .count();
    if missing > MISSING_AT_MOST {
        return Err(format!(
            "{missing} rows are MISSING; at most {MISSING_AT_MOST} may be"
        ));
    }
    if unreachable > UNREACHABLE_AT_MOST {
        return Err(format!(
            "{unreachable} rows are stated UNREACHABLE; at most {UNREACHABLE_AT_MOST} may be"
        ));
    }
    Ok(())
}

#[test]
fn a_rows_file_holding_anything_but_rows_is_refused() -> Result<(), String> {
    // R3 (Touchstone): rows moved into a module, a second match, other statements, a non-string
    // const, or an enum dropped from the rows must all be red, never silently skipped.
    for (what, source) in [
        (
            "rows in a module",
            "mod hidden { pub(crate) const fn seal(e: &SealError) -> Row { match e { SealError::Length { .. } => (\"Length\", \"a::b\") } } }",
        ),
        (
            "two matches",
            "const fn f(e: &E) -> Row { match e { E::A => (\"A\", \"a::b\") }; match e { E::A => (\"A\", \"a::b\") } }",
        ),
        (
            "a let before the match",
            "const fn f(e: &E) -> Row { let x = 1; match e { E::A => (\"A\", \"a::b\") } }",
        ),
        ("a non-string const", "const N: usize = 1;"),
        ("a struct", "struct S;"),
    ] {
        if rows_from(what, source).is_ok() {
            return Err(format!("{what}: admitted"));
        }
    }
    let dropped: Vec<Row> = rows()?
        .into_iter()
        .filter(|row| row.refusal != "SealError")
        .collect();
    if covers_the_scope(&dropped).is_ok() {
        return Err("rows without SealError cover the scope".to_owned());
    }
    Ok(())
}

#[test]
fn an_ignored_or_otherwise_gated_test_is_not_runnable() -> Result<(), String> {
    // R2 (Touchstone): an #[ignore]d cited test passed the whole gate.
    let source = "
        #[test] #[ignore = \"x\"] fn ignored() {}
        #[cfg(feature = \"x\")] #[test] fn behind_a_feature() {}
        #[cfg_attr(unix, ignore)] #[test] fn conditionally_ignored() {}
        #[cfg(unix)] mod gated { #[test] fn in_a_gated_module() {} }
        #[cfg(test)] #[cfg(not(fw_fixture_cli))] mod tests { #[test] fn runs() {} }
        #[test] fn also_runs() {}
    ";
    let runnable = runnable_tests("control", source)?;
    if runnable == ["runs", "also_runs"] {
        Ok(())
    } else {
        Err(format!("runnable tests were {runnable:?}"))
    }
}
