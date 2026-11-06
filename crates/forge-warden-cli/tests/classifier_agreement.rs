//! Rulings #9: two independent classifiers must agree on the same compiler evidence.
#![expect(
    clippy::disallowed_methods,
    reason = "test door: exports compiler evidence and runs the independent Python classifier"
)]

mod common;

#[test]
fn rust_and_python_classify_the_same_build_identically() {
    let (classes, evidence, target) = common::sources_with_evidence();
    let dir = common::scratch("classifier-agreement");
    let evidence_path = common::write(&dir, "artifacts.jsonl", evidence.as_bytes());
    let expected = serde_json::json!({
        "production": classes.production,
        "test_only": classes.test_only,
    });
    let expected_path = common::write(&dir, "rust-sets.json", expected.to_string().as_bytes());
    let output = common::command("python3")
        .arg("-I")
        .arg(common::root().join("scripts/classifier_agreement.py"))
        .arg(evidence_path)
        .arg(expected_path)
        .arg(target)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
