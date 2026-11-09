//! Independent output anchors: external seal digests and explicit citation ordering.

use forge_warden_core::Date;
use forge_warden_core::policy::parse_policy;
use forge_warden_core::rank::parse_rank;
use forge_warden_core::testing::evaluate;
use forge_warden_core::verdict::Seals;
use serde_json::{Value, json};

#[test]
fn golden_seals_match_external_sha256_references() {
    let expected: Value = serde_json::from_slice(include_bytes!(
        "../../../fixtures/adversarial/seal-reference.json"
    ))
    .unwrap();
    let golden: Value =
        serde_json::from_slice(include_bytes!("../../../fixtures/demo.verdict.json")).unwrap();
    let inputs: [(&str, &[u8]); 3] = [
        (
            "policy_sha256",
            include_bytes!("../../../examples/starter.warden.toml"),
        ),
        (
            "rank_sha256",
            include_bytes!("../../../fixtures/demo.rank.json"),
        ),
        (
            "patch_sha256",
            include_bytes!("../../../fixtures/demo.patch"),
        ),
    ];
    for (name, input) in inputs {
        assert_eq!(
            golden["seals"][name], expected[name],
            "golden seal {name} differs from independent bytes"
        );
        assert_eq!(
            forge_warden_core::seal::sha256_hex(input),
            expected[name].as_str().unwrap(),
            "hash implementation {name}"
        );
    }
}
#[test]
fn citations_follow_explicit_address_side_line_order() {
    let a = "a".repeat(64);
    let b = "b".repeat(64);
    let evidence = vec![
        json!({"signal":"secret_like","address":b,"side":"new","line":7}),
        json!({"signal":"secret_like","address":a,"side":"old","line":20}),
        json!({"signal":"secret_like","address":a,"side":"new","line":4}),
    ];
    let rank = json!({"schema":"deep-diff-forge.rank.v0","input_sha256":a,
        "tool":{"name":"deep-diff-forge","version":"0.7.0"},
        "ranked":[{"path":"src/a.rs","status":"modified","score":1,
            "signals":["secret_like"],"evidence":evidence,"evidence_truncated":true}]});
    let rank = parse_rank(&serde_json::to_vec(&rank).unwrap()).unwrap();
    let policy=parse_policy(b"schema=\"forge-warden.policy.v0\"\n[[rule]]\nid=\"secrets\"\nwhy=\"No credentials in source.\"\nkind=\"forbid_signal\"\nseverity=\"fail\"\nsignals=[\"secret_like\"]\n").unwrap();
    let seals = Seals::new(a.clone(), a.clone(), a.clone(), "0.7.0".to_owned());
    let verdict = evaluate(&policy, &rank, seals, Date::parse("2026-11-06").unwrap());
    let violation = verdict
        .rules
        .iter()
        .flat_map(|r| &r.violations)
        .next()
        .unwrap();
    assert_eq!(violation.signal.as_deref(), Some("secret_like"));
    assert!(violation.evidence_truncated);
    let actual: Vec<_> = verdict
        .rules
        .iter()
        .flat_map(|r| &r.violations)
        .flat_map(|v| &v.evidence)
        .map(|e| (e.address.as_str(), e.side.as_str(), e.line))
        .collect();
    assert_eq!(
        actual,
        vec![
            (a.as_str(), "new", 4),
            (a.as_str(), "old", 20),
            (b.as_str(), "new", 7)
        ]
    );
}
