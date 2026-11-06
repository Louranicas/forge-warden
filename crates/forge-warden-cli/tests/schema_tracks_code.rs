//! The committed JSON Schemas and the code describe the same documents.
//!
//! - Every member a real verdict serializes is declared by the schema, and every member the
//!   schema requires is serialized (for each object type: verdict, seals, summary, rule,
//!   violation, citation, waiver note, note).
//! - The policy schema's rule members are exactly the fields the policy reader accepts, and its
//!   signal enum is exactly the vocabulary.
#![expect(clippy::unwrap_used, reason = "test code: a panic is a failed test")]

mod common;
use common::{read, run};
use forge_warden_core::policy::{Field, Kind};
use forge_warden_core::vocab::KNOWN_SIGNALS;
use serde_json::Value;
use std::collections::BTreeSet;

fn keys(value: &Value) -> BTreeSet<String> {
    value.as_object().unwrap().keys().cloned().collect()
}

fn strings(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect()
}

/// Check one object instance against one schema object definition.
fn conforms(instance: &Value, schema: &Value, what: &str) {
    let declared = keys(&schema["properties"]);
    let required = strings(&schema["required"]);
    let present = keys(instance);
    assert!(
        present.is_subset(&declared),
        "{what}: undeclared members {:?}",
        present.difference(&declared).collect::<Vec<_>>()
    );
    assert!(
        required.is_subset(&present),
        "{what}: missing required {:?}",
        required.difference(&present).collect::<Vec<_>>()
    );
    assert_eq!(
        schema["additionalProperties"],
        Value::Bool(false),
        "{what}: objects must be closed"
    );
}

/// A verdict that exercises every optional member: waived and unwaived violations, evidence,
/// a change-wide violation, expired and unused waivers, and notes.
fn rich_verdict() -> Value {
    let out = run(
        &[
            "check",
            "--policy",
            "fixtures/rich.warden.toml",
            "--patch",
            "fixtures/rich.patch",
            "--advisory",
            "--rank",
            "fixtures/rich.rank.json",
            "--base",
            "1234abcd",
            "--head",
            "abcdef01",
            "--today",
            "2026-11-05",
            "--json",
        ],
        b"",
    );
    assert_eq!(out.code, 7, "{}", out.stderr);
    serde_json::from_str(&out.stdout).unwrap()
}

#[test]
fn verdict_schema_tracks_the_serialized_document() {
    let schema: Value = serde_json::from_str(&read("schemas/verdict.v1.schema.json")).unwrap();
    let defs = &schema["$defs"];
    let v = rich_verdict();

    conforms(&v, &schema, "verdict");
    for member in ["tool", "seals", "exit", "summary"] {
        conforms(&v[member], &schema["properties"][member], member);
    }
    let seals = &schema["properties"]["seals"]["properties"];
    conforms(&v["seals"]["ranked_by"], &seals["ranked_by"], "ranked_by");
    conforms(&v["seals"]["git"], &seals["git"]["oneOf"][0], "git");
    conforms(&v["seals"]["claims"], &seals["claims"], "claims");
    let mut seen = BTreeSet::new();
    for rule in v["rules"].as_array().unwrap() {
        conforms(rule, &defs["rule"], "rule");
        for violation in rule["violations"].as_array().unwrap() {
            conforms(violation, &defs["violation"], "violation");
            seen.extend(keys(violation));
            for citation in violation
                .get("evidence")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                conforms(citation, &defs["citation"], "citation");
            }
        }
    }
    for list in ["expired_waivers", "unused_waivers"] {
        let items = v[list].as_array().unwrap();
        assert!(!items.is_empty(), "the rich fixture must exercise {list}");
        for item in items {
            conforms(item, &defs["waiver_note"], list);
        }
    }
    let notes = v["notes"].as_array().unwrap();
    assert!(!notes.is_empty(), "the rich fixture must exercise notes");
    for note in notes {
        conforms(note, &defs["note"], "note");
    }
    assert_eq!(
        seen,
        keys(&defs["violation"]["properties"]),
        "the rich fixture must exercise every violation member"
    );
}

#[test]
fn policy_schema_tracks_the_reader() {
    let schema: Value = serde_json::from_str(&read("schemas/policy.v0.schema.json")).unwrap();
    let rule = &schema["$defs"]["rule"];
    let mut accepted: BTreeSet<String> = ["id", "why", "kind", "severity"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    accepted.extend(Field::ALL.iter().map(|field| field.name().to_owned()));
    assert_eq!(keys(&rule["properties"]), accepted);

    let enum_signals = strings(&schema["$defs"]["signal"]["enum"]);
    let known: BTreeSet<String> = KNOWN_SIGNALS.iter().map(|s| (*s).to_owned()).collect();
    assert_eq!(enum_signals, known);

    let kinds = strings(&rule["properties"]["kind"]["enum"]);
    let code_kinds: BTreeSet<String> = Kind::ALL.iter().map(|k| k.name().to_owned()).collect();
    assert_eq!(kinds, code_kinds);

    // Per kind, the schema forbids (`false`) exactly the optional fields the reader refuses.
    let clauses = rule["allOf"].as_array().unwrap();
    for kind in Kind::ALL {
        let clause = clauses
            .iter()
            .find(|c| c["if"]["properties"]["kind"]["const"] == kind.name())
            .unwrap_or_else(|| panic!("schema has no allOf clause for {}", kind.name()));
        let forbidden: BTreeSet<String> = clause["then"]["properties"]
            .as_object()
            .unwrap()
            .iter()
            .filter(|(_, v)| **v == Value::Bool(false))
            .map(|(k, _)| k.clone())
            .collect();
        let refused: BTreeSet<String> = Field::ALL
            .iter()
            .filter(|f| !kind.fields().contains(f))
            .map(|f| f.name().to_owned())
            .collect();
        assert_eq!(forbidden, refused, "schema vs reader for {}", kind.name());
    }
}

#[test]
fn schema_subcommand_prints_the_committed_files() {
    assert_eq!(
        run(&["schema", "verdict"], b"").stdout,
        read("schemas/verdict.v1.schema.json")
    );
    assert_eq!(
        run(&["schema", "policy"], b"").stdout,
        read("schemas/policy.v0.schema.json")
    );
    let registry: Value =
        serde_json::from_str(&run(&["schema", "exit-codes"], b"").stdout).unwrap();
    assert_eq!(
        registry.as_array().unwrap().len(),
        forge_warden_core::EXIT_REGISTRY.len()
    );
}
