//! The built-in rule `builtin:binary-on-executable` (M7-Q64, ledger Q64, ruling R-a): a binary
//! section on a file whose mode after the change is executable or a symlink holds, whatever the
//! policy says. Rows from `lanes/bastion/M7-Q64/DESIGN.md` section 6, each judged through
//! `evaluate_authority` over Git's facts as `PatchFacts::from_git` reads them.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers build fixed inputs; a panic is a failed test"
)]

use forge_warden_core::authority::{Ranking, SuppliedCopy, check, evaluate_authority};
use forge_warden_core::facts::{PatchFacts, Unverifiable};
use forge_warden_core::policy::parse_policy;
use forge_warden_core::rank::parse_rank;
use forge_warden_core::verdict::{RuleOutcome, RuleResult, Seals};
use forge_warden_core::{Date, Outcome, Policy, Verdict};

const HASH: &str = "bbf580931c6739af491bb5e2258550631327ca07f1078135cb008797e22b836e";
const ID: &str = "builtin:binary-on-executable";

fn seal() -> forge_warden_core::seal::Sha256Hex {
    forge_warden_core::seal::Sha256Hex::parse(HASH).unwrap()
}

fn policy(body: &str) -> Policy {
    parse_policy(
        format!("schema = \"forge-warden.policy.v0\"\n[producer]\nsha256 = [\"{HASH}\"]\n{body}")
            .as_bytes(),
    )
    .unwrap()
}

/// A rule nothing here trips.
const PERMISSIVE: &str = r#"
[[rule]]
id = "no-secrets"
why = "Credentials require immediate remediation."
kind = "forbid_signal"
severity = "fail"
signals = ["secret_like"]
"#;

/// One file of a patch: its numstat record, summary line (if any) and section, as Git writes them.
struct Section {
    numstat: String,
    summary: Option<String>,
    text: String,
    /// The ranking's status and signals for it (only the coverage notes depend on these).
    status: &'static str,
    signals: &'static [&'static str],
}

const BODY: &str =
    "GIT binary patch\nliteral 5\nMcmZQzO3KUw00MIXJOBUy\n\nliteral 5\nMcmZQzOv=my00M6TI{*Lx\n\n";

fn binary(path: &str, headers: &str, summary: Option<&str>) -> Section {
    Section {
        numstat: format!("-\t-\t{path}\0"),
        summary: summary.map(|line| format!(" {line} {path}\n")),
        text: format!("diff --git a/{path} b/{path}\n{headers}{BODY}"),
        status: "binary_changed",
        signals: &["binary_change"],
    }
}

/// Git's facts and an authority verdict for `sections` under `body` (a policy's rules).
fn judge(sections: &[Section], body: &str) -> Verdict {
    let numstat: String = sections.iter().map(|s| s.numstat.as_str()).collect();
    let summary: String = sections
        .iter()
        .filter_map(|s| s.summary.as_deref())
        .collect();
    let patch: String = sections.iter().map(|s| s.text.as_str()).collect();
    let facts = PatchFacts::from_git(numstat.as_bytes(), summary.as_bytes(), patch.as_bytes());
    let files: Vec<String> = sections
        .iter()
        .map(|s| {
            let file = s.text.split(' ').nth(2).unwrap().trim_start_matches("a/");
            let signals: Vec<String> = s.signals.iter().map(|x| format!("\"{x}\"")).collect();
            format!(
                r#"{{"path":"{file}","status":"{}","score":3,"signals":[{}]}}"#,
                s.status,
                signals.join(",")
            )
        })
        .collect();
    let rank = format!(
        r#"{{"schema":"deep-diff-forge.rank.v0","input_sha256":"{HASH}","tool":{{"name":"deep-diff-forge","version":"0.7.0"}},"ranked":[{}]}}"#,
        files.join(",")
    );
    let p = policy(body);
    let ranking = Ranking::attest(
        parse_rank(rank.as_bytes()).unwrap(),
        p.admit_producer(&seal()).unwrap(),
    );
    let checked = check(&ranking, facts);
    let seals = Seals::new(seal(), seal(), seal(), "0.7.0".to_owned());
    evaluate_authority(
        &p,
        &checked,
        SuppliedCopy::Absent,
        seals,
        Date::parse("2026-11-16").unwrap(),
    )
}

fn builtin(verdict: &Verdict) -> Option<&RuleResult> {
    verdict.rules().iter().find(|rule| rule.id == ID)
}

/// The paths the built-in rule holds, or none when it is not listed.
fn held(verdict: &Verdict) -> Vec<&str> {
    match builtin(verdict) {
        Some(rule) => rule
            .violations
            .iter()
            .map(|v| v.path.as_deref().unwrap())
            .collect(),
        None => Vec::new(),
    }
}

#[test]
fn an_executable_created_as_a_binary_section_holds() {
    // `bin/tool`, created 100755, a NUL in its second line: no policy names it.
    let verdict = judge(
        &[binary(
            "bin/tool",
            "new file mode 100755\nindex 0000000..3e3315e\n",
            Some("create mode 100755"),
        )],
        PERMISSIVE,
    );
    let rule = builtin(&verdict).unwrap();
    assert_eq!(
        (rule.kind, rule.severity, rule.outcome),
        ("builtin", "hold", RuleOutcome::Hold)
    );
    assert_eq!(held(&verdict), ["bin/tool"]);
    assert_eq!(
        rule.violations[0].message,
        "a binary section on an executable file (100755)"
    );
    assert!(rule.violations[0].waived_by.is_none());
    assert_eq!(verdict.outcome(), Outcome::Hold);
    // The summary counts it with the policy's rules.
    let summary = verdict.summary();
    assert_eq!((summary.rules, summary.held), (2, 1));
}

#[test]
fn a_binary_edit_of_an_executable_holds_by_its_index_line_mode() {
    // An edit that keeps mode 100755: Git states it on the `index` line (Git's own in bound mode).
    let edit = |mode: &str| {
        judge(
            &[binary(
                "tools/run",
                &format!("index 88768ef..3e3315e {mode}\n"),
                None,
            )],
            PERMISSIVE,
        )
    };
    assert_eq!(held(&edit("100755")), ["tools/run"]);
    // Stated limit (ADR 0012): in unbound mode a falsely stated plain mode is the author's text;
    // only bound mode, where the patch is Git's own diff, makes it a fact.
    assert_eq!(held(&edit("100644")), Vec::<&str>::new());
    assert!(
        builtin(&edit("100644")).is_none(),
        "listed only when it holds"
    );
}

#[test]
fn a_binary_section_made_executable_or_a_symlink_holds() {
    let made_executable = judge(
        &[binary(
            "data/blob.bin",
            "old mode 100644\nnew mode 100755\nindex 88768ef..3e3315e\n",
            Some("mode change 100644 => 100755"),
        )],
        PERMISSIVE,
    );
    assert_eq!(held(&made_executable), ["data/blob.bin"]);
    let symlink = judge(
        &[binary(
            "link",
            "new file mode 120000\nindex 0000000..3e3315e\n",
            Some("create mode 120000"),
        )],
        PERMISSIVE,
    );
    assert_eq!(held(&symlink), ["link"]);
    assert_eq!(
        builtin(&symlink).unwrap().violations[0].message,
        "a binary section on a symlink (120000)"
    );
}

#[test]
fn controls_a_plain_binary_a_deletion_and_a_text_edit_do_not_hold() {
    let plain = binary(
        "img/logo.png",
        "new file mode 100644\nindex 0000000..3e3315e\n",
        Some("create mode 100644"),
    );
    let deleted = binary(
        "bin/old",
        "deleted file mode 100755\nindex 3e3315e..0000000\n",
        Some("delete mode 100755"),
    );
    let text = Section {
        numstat: "1\t1\tbin/script\0".to_owned(),
        summary: None,
        text: "diff --git a/bin/script b/bin/script\nindex 1111111..2222222 100755\n--- a/bin/script\n+++ b/bin/script\n@@ -1 +1 @@\n-a\n+b\n".to_owned(),
        status: "modified",
        signals: &[],
    };
    let verdict = judge(&[plain, deleted, text], PERMISSIVE);
    assert!(builtin(&verdict).is_none(), "{:?}", verdict.rules());
    assert_eq!(verdict.summary().rules, 1);
}

#[test]
fn the_built_in_hold_is_listed_alongside_a_policy_hold_and_no_waiver_hides_it() {
    // Lodestar's ruling on M7-Q64 LOW-1: a hold is acknowledged by naming its rules, so another
    // rule's hold on the same path (here a path rule; Q63's binary-text-is-hidden-text for run.sh,
    // or a new_file signal rule, alike) never hides the built-in one.
    let section = || binary("scripts/run.sh", "index 88768ef..3e3315e 100755\n", None);
    let protect = r#"
[[rule]]
id = "scripts-reviewed"
why = "A script change needs its owner's review."
kind = "protect_paths"
severity = "hold"
paths = ["scripts/**"]
"#;
    let both = judge(&[section()], protect);
    assert_eq!(held(&both), ["scripts/run.sh"]);
    let held_rules: Vec<&str> = both
        .rules()
        .iter()
        .filter(|rule| rule.outcome == RuleOutcome::Hold)
        .map(|rule| rule.id.as_str())
        .collect();
    assert_eq!(held_rules, ["scripts-reviewed", ID]);
    assert_eq!(both.summary().held, 2);
    // A waiver that covers the policy's hold does not cover the built-in rule.
    let waived = format!(
        "{protect}\n[[waiver]]\nrule = \"scripts-reviewed\"\npaths = [\"scripts/run.sh\"]\nreason = \"The owner reviewed this script change already.\"\nowner = \"release\"\nexpires = \"2099-01-01\"\n"
    );
    let verdict = judge(&[section()], &waived);
    assert_eq!(held(&verdict), ["scripts/run.sh"]);
    assert!(builtin(&verdict).unwrap().violations[0].waived_by.is_none());
    assert_eq!(verdict.outcome(), Outcome::Hold);
}

#[test]
fn no_policy_can_waive_or_declare_the_built_in_rule() {
    let waiver = format!(
        "schema = \"forge-warden.policy.v0\"\n{PERMISSIVE}\n[[waiver]]\nrule = \"{ID}\"\npaths = [\"**\"]\nreason = \"Executables in this repository are reviewed elsewhere.\"\nowner = \"release\"\nexpires = \"2099-01-01\"\n"
    );
    let refused = parse_policy(waiver.as_bytes()).unwrap_err().to_string();
    assert!(
        refused.contains("which the policy does not declare"),
        "{refused}"
    );
    let declared = PERMISSIVE.replace("no-secrets", ID);
    let kebab = parse_policy(format!("schema = \"forge-warden.policy.v0\"\n{declared}").as_bytes())
        .unwrap_err()
        .to_string();
    assert!(kebab.contains("kebab-case"), "{kebab}");
}

#[test]
fn an_index_line_after_the_binary_body_is_not_the_sections_mode() {
    // Touchstone (M7-Q64-MODE verify): Git writes every header line before `GIT binary patch`, so
    // an `index` line appended after the body neither states the mode nor escapes the refusal.
    let patch = format!(
        "diff --git a/tools/run b/tools/run\nindex 88768ef..3e3315e\n{BODY}index 88768ef..3e3315e 100644\n"
    );
    assert_eq!(
        PatchFacts::from_git(b"-\t-\ttools/run\0", b"", patch.as_bytes()).unwrap_err(),
        Unverifiable::BinaryModeUnstated("tools/run".to_owned())
    );
}
