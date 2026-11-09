//! Adversarial input contracts from Mission 3; no dependency on mutable domain fields.
#![expect(
    clippy::unwrap_used,
    reason = "test setup fails loudly when a parser unexpectedly refuses the fixed control"
)]

use forge_warden_core::policy::parse_policy;
use forge_warden_core::rank::{RankError, parse_rank};
use forge_warden_core::seal::{Sha256Hex, sha256_hex};
use forge_warden_core::testing::evaluate;
use forge_warden_core::verdict::Seals;
use forge_warden_core::{Date, Outcome};
use serde_json::{Value, json};

const HEADER: &str = "schema=\"forge-warden.policy.v0\"\n";
const SECRET: &str = r#"
[[rule]]
id="no-secrets"
why="Credentials require immediate remediation."
kind="forbid_signal"
severity="fail"
signals=["secret_like"]
"#;
const PROTECT: &str = r#"
[[rule]]
id="protect-source"
why="Source changes require explicit review."
kind="protect_paths"
severity="fail"
paths=["src/**"]
"#;
const COMPANION: &str = r#"
[[rule]]
id="tests"
why="Source changes need accompanying tests."
kind="require_companion"
severity="fail"
when=["src/**"]
require=["tests/**"]
"#;
const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn file(path: &str, status: &str, signals: &[&str]) -> Value {
    json!({"path":path,"status":status,"score":1,"signals":signals})
}

fn document(files: Vec<Value>) -> Value {
    let files = Value::Array(files);
    json!({"schema":"deep-diff-forge.rank.v0","input_sha256":HASH,
        "tool":{"name":"deep-diff-forge","version":"0.7.0"},"ranked":files})
}

fn judge(body: &str, input: &Value, day: &str) -> forge_warden_core::Verdict {
    let policy_bytes = format!("{HEADER}{body}").into_bytes();
    let rank_bytes = serde_json::to_vec(input).unwrap();
    let policy = parse_policy(&policy_bytes).unwrap();
    let rank = parse_rank(&rank_bytes).unwrap();
    let seals = Seals::new(
        sha256_hex(&policy_bytes),
        sha256_hex(&rank_bytes),
        Sha256Hex::parse(HASH).unwrap(),
        "0.7.0".to_owned(),
    );
    evaluate(&policy, &rank, seals, Date::parse(day).unwrap())
}

#[test]
fn unknowns_cannot_pass_an_otherwise_passing_scope() {
    let clean = document(vec![file("README.md", "modified", &[])]);
    assert_eq!(judge(SECRET, &clean, "2026-11-06").verdict, Outcome::Pass);
    for (status, signals) in [
        ("unknown", vec![]),
        ("future_status", vec![]),
        ("renamed", vec![]),
        ("copied", vec![]),
        ("modified", vec!["future_signal"]),
    ] {
        let rank = document(vec![file("README.md", status, &signals)]);
        let v = judge(SECRET, &rank, "2026-11-06");
        assert_eq!(v.verdict, Outcome::Hold, "{status} {signals:?}");
        assert_eq!(v.notes.len(), 1);
    }
    assert_eq!(
        judge(SECRET, &document(vec![]), "2026-11-06").verdict,
        Outcome::Hold
    );
}

#[test]
fn a_failure_is_not_downgraded_by_an_unknown() {
    let rank = document(vec![file("README.md", "future_status", &["secret_like"])]);
    let v = judge(SECRET, &rank, "2026-11-06");
    assert_eq!(v.verdict, Outcome::Fail);
    assert_eq!(v.notes.len(), 1);
}

#[test]
fn malformed_rank_members_are_refused_as_a_whole() {
    let base = document(vec![file("src/lib.rs", "modified", &[])]);
    assert!(parse_rank(&serde_json::to_vec(&base).unwrap()).is_ok());
    for (field, value) in [
        ("score", json!(-1)),
        ("score", json!(4_294_967_296_u64)),
        ("score", json!(null)),
        ("signals", json!(null)),
        ("signals", json!("secret_like")),
    ] {
        let mut rank = base.clone();
        rank["ranked"][0][field] = value;
        assert!(
            matches!(
                parse_rank(&serde_json::to_vec(&rank).unwrap()),
                Err(RankError::Malformed(_))
            ),
            "{rank}"
        );
    }
    for field in ["path", "status", "score", "signals"] {
        let mut rank = base.clone();
        rank["ranked"][0].as_object_mut().unwrap().remove(field);
        assert!(
            matches!(
                parse_rank(&serde_json::to_vec(&rank).unwrap()),
                Err(RankError::Malformed(_))
            ),
            "missing {field}"
        );
    }
    for input in [b"".as_slice(), b"{", b"[]", b"null", b"\xff"] {
        assert!(matches!(parse_rank(input), Err(RankError::Malformed(_))));
    }
}

#[test]
fn invalid_rank_meaning_is_distinct_from_malformed_shape() {
    let base = document(vec![file("src/lib.rs", "modified", &[])]);
    assert!(parse_rank(&serde_json::to_vec(&base).unwrap()).is_ok());
    for (field, value) in [
        ("path", json!("")),
        ("path", json!("x\0y")),
        ("status", json!("")),
        ("signals", json!([""])),
    ] {
        let mut rank = base.clone();
        rank["ranked"][0][field] = value;
        assert!(
            matches!(
                parse_rank(&serde_json::to_vec(&rank).unwrap()),
                Err(RankError::Invalid(_))
            ),
            "{rank}"
        );
    }
    for (field, value) in [
        ("schema", json!("future.v1")),
        ("input_sha256", json!("A".repeat(64))),
    ] {
        let mut rank = base.clone();
        rank[field] = value;
        assert!(matches!(
            parse_rank(&serde_json::to_vec(&rank).unwrap()),
            Err(RankError::Invalid(_))
        ));
    }
    for (field, value) in [("name", "foreign"), ("version", "")] {
        let mut rank = base.clone();
        rank["tool"][field] = json!(value);
        assert!(matches!(
            parse_rank(&serde_json::to_vec(&rank).unwrap()),
            Err(RankError::Invalid(_))
        ));
    }
}

#[test]
fn malformed_evidence_never_becomes_approval() {
    let valid = json!({"signal":"secret_like","address":HASH,"side":"new","line":1});
    let mut base = file("src/lib.rs", "modified", &["secret_like"]);
    base["evidence"] = json!([valid]);
    assert!(parse_rank(&serde_json::to_vec(&document(vec![base.clone()])).unwrap()).is_ok());
    for (field, value) in [
        ("signal", json!("unsafe_added")),
        ("address", json!("bad")),
        ("side", json!("both")),
        ("line", json!(0)),
        ("line", json!(-1)),
    ] {
        let mut entry = base.clone();
        entry["evidence"][0][field] = value;
        assert!(parse_rank(&serde_json::to_vec(&document(vec![entry])).unwrap()).is_err());
    }
}

#[test]
fn policy_typos_and_duplicate_rules_are_refused() {
    for body in [
        String::new(),
        format!("{SECRET}{SECRET}"),
        SECRET.replace("Credentials require immediate remediation.", "because"),
        SECRET.replace("secret_like", "secret_typo"),
        format!("{SECRET}typo=true\n"),
        SECRET.replace("[\"secret_like\"]", "[]"),
        SECRET.replace("severity=\"fail\"", "severity=\"pass\""),
        SECRET.replace("forbid_signal", "future_kind"),
        format!("{SECRET}limit=10\n"),
    ] {
        assert!(
            parse_policy(format!("{HEADER}{body}").as_bytes()).is_err(),
            "{body}"
        );
    }
}

#[test]
fn protected_scopes_respect_segment_boundaries_and_exceptions() {
    for (path, expected) in [
        ("src/lib.rs", Outcome::Fail),
        ("src/deep/lib.rs", Outcome::Fail),
        ("README.md", Outcome::Pass),
    ] {
        assert_eq!(
            judge(
                PROTECT,
                &document(vec![file(path, "modified", &[])]),
                "2026-11-06"
            )
            .verdict,
            expected
        );
    }
    let narrow = PROTECT.replace("src/**", "src/*.rs");
    assert_eq!(
        judge(
            &narrow,
            &document(vec![file("src/deep/lib.rs", "modified", &[])]),
            "2026-11-06"
        )
        .verdict,
        Outcome::Pass
    );
    let except = format!("{PROTECT}except=[\"src/generated/**\"]\n");
    assert_eq!(
        judge(
            &except,
            &document(vec![file("src/generated/a.rs", "modified", &[])]),
            "2026-11-06"
        )
        .verdict,
        Outcome::Pass
    );
    assert_eq!(
        judge(
            &except,
            &document(vec![file("src/a.rs", "modified", &[])]),
            "2026-11-06"
        )
        .verdict,
        Outcome::Fail
    );
    for pattern in ["/src/**", "./src/**", "src/["] {
        assert!(
            parse_policy(format!("{HEADER}{}", PROTECT.replace("src/**", pattern)).as_bytes())
                .is_err()
        );
    }
}

#[test]
fn a_deleted_or_unknowable_test_is_not_a_live_companion() {
    let source = file("src/lib.rs", "modified", &[]);
    assert_eq!(
        judge(COMPANION, &document(vec![source.clone()]), "2026-11-06").verdict,
        Outcome::Fail
    );
    for (status, signals, expected) in [
        ("modified", vec![], Outcome::Pass),
        ("deleted", vec![], Outcome::Fail),
        ("binary_changed", vec![], Outcome::Fail),
        ("binary_changed", vec!["deleted_file"], Outcome::Fail),
        ("binary_changed", vec!["new_file"], Outcome::Pass),
    ] {
        let rank = document(vec![source.clone(), file("tests/it.rs", status, &signals)]);
        assert_eq!(
            judge(COMPANION, &rank, "2026-11-06").verdict,
            expected,
            "{status} {signals:?}"
        );
    }
}

fn waived() -> String {
    format!(
        "{PROTECT}\n[[waiver]]\nrule=\"protect-source\"\npaths=[\"src/**\"]\nreason=\"Temporary migration with owner review.\"\nowner=\"@owner\"\nexpires=\"2026-11-06\"\n"
    )
}

#[test]
fn waiver_expiry_is_inclusive_and_expired_unused_waivers_fail() {
    let protected = document(vec![file("src/lib.rs", "modified", &[])]);
    for (day, outcome) in [
        ("2026-11-05", Outcome::Pass),
        ("2026-11-06", Outcome::Pass),
        ("2026-11-07", Outcome::Fail),
    ] {
        assert_eq!(judge(&waived(), &protected, day).verdict, outcome);
    }
    let outside = document(vec![file("README.md", "modified", &[])]);
    assert_eq!(
        judge(&waived(), &outside, "2026-11-07").verdict,
        Outcome::Fail
    );
    for (from, to) in [
        ("@owner", ""),
        ("Temporary migration with owner review.", "short"),
        ("rule=\"protect-source\"", "rule=\"absent\""),
        ("2026-11-06", "2026-11-06T00:00:00Z"),
    ] {
        assert!(
            parse_policy(format!("{HEADER}{}", waived().replace(from, to)).as_bytes()).is_err()
        );
    }
    for bad in [
        "2026-02-29",
        "2026-11-06T23:59:59Z",
        "2026-11-06+11:00",
        "2026-1-06",
    ] {
        assert_eq!(Date::parse(bad), None);
    }
    assert!(Date::parse("2028-02-29").is_some());
}

#[test]
fn noncanonical_paths_are_refused_not_silently_out_of_scope() {
    for path in [
        "./src/lib.rs",
        "/src/lib.rs",
        "../src/lib.rs",
        "src/../src/lib.rs",
        "src//lib.rs",
    ] {
        let rank = document(vec![file(path, "modified", &[])]);
        assert!(
            parse_rank(&serde_json::to_vec(&rank).unwrap()).is_err(),
            "accepted {path}"
        );
    }
}

#[test]
fn duplicate_ranked_paths_hold() {
    let f = file("README.md", "modified", &[]);
    let verdict = judge(SECRET, &document(vec![f.clone(), f]), "2026-11-06");
    assert_eq!(verdict.verdict, Outcome::Hold);
    assert_eq!(verdict.exit.code, 8);
    assert!(
        verdict
            .notes
            .iter()
            .any(|note| note.code == "duplicate_paths")
    );
}

#[test]
fn whitespace_producer_version_is_refused() {
    let mut rank = document(vec![file("README.md", "modified", &[])]);
    rank["tool"]["version"] = json!(" \t\n");
    assert!(parse_rank(&serde_json::to_vec(&rank).unwrap()).is_err());
}

proptest::proptest! {
    #[test]
    fn unfamiliar_signals_hold_even_outside_rule_scope(suffix in "[a-z0-9_]{1,32}") {
        let unknown = format!("future_{suffix}");
        let scoped = format!("{SECRET}paths=[\"src/**\"]\n");
        let input = document(vec![file("README.md", "modified", &[&unknown])]);
        let verdict = judge(&scoped, &input, "2026-11-06");
        proptest::prop_assert_eq!(verdict.verdict, Outcome::Hold);
    }
}
