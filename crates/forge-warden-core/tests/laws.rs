//! Independent finite-domain policy oracle and monotonicity laws (WP-B5).
//!
//! The specification covers all five rule kinds, both score scopes, prefix-only path scopes,
//! active/expired waivers, deleted/binary companions and unknown vocabulary. It does not call the
//! production matcher, evaluator, waiver helpers or vocabulary functions to compute expectations.
#![expect(
    clippy::unwrap_used,
    reason = "generated finite test models and parser setup: a panic is a failed property"
)]
use forge_warden_core::policy::parse_policy;
use forge_warden_core::rank::parse_rank;
use forge_warden_core::testing::evaluate;
use forge_warden_core::verdict::{RuleOutcome, Seals};
use forge_warden_core::{Date, Outcome};
use proptest::prelude::*;
use serde_json::json;

const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

/// A test seal from a known-good constant.
fn seal(text: &str) -> forge_warden_core::seal::Sha256Hex {
    forge_warden_core::seal::Sha256Hex::parse(text).unwrap()
}

#[path = "../../../fixtures/adversarial/reference_model.rs"]
mod model;
use model::{Case, CheckSpec, File, ResultSpec, Rule, reference};

fn pattern(scope: u8) -> &'static str {
    match scope {
        1 => "src/**",
        2 => "docs/**",
        _ => "**",
    }
}
fn policy(case: &Case) -> Vec<u8> {
    let rules: Vec<String> = case.rules.iter().enumerate().map(|(i, rule)| {
        let (kind, fields) = match rule.kind {
            CheckSpec::Forbid => ("forbid_signal", format!("signals=[\"secret_like\"]\npaths=[{:?}]",pattern(rule.scope))),
            CheckSpec::Protect => ("protect_paths", format!("paths=[{:?}]",pattern(rule.scope))),
            CheckSpec::Companion => ("require_companion", "when=[\"src/**\"]\nrequire=[\"tests/**\"]".to_owned()),
            CheckSpec::FileScore(limit) | CheckSpec::ChangeScore(limit) => ("max_score", format!("limit={limit}\nper={:?}\npaths=[{:?}]",
                if matches!(rule.kind,CheckSpec::FileScore(_)) { "file" } else { "change" },pattern(rule.scope))),
            CheckSpec::FileCount(limit) => ("max_files", format!("limit={limit}\npaths=[{:?}]",pattern(rule.scope))),
        };
        format!("[[rule]]\nid=\"rule-{i}\"\nwhy=\"Independent specification rule.\"\nkind={kind:?}\nseverity={:?}\n{fields}\n",
            if rule.fail { "fail" } else { "hold" })
    }).collect();
    let waiver = match case.waiver {
        1 => "2026-11-06",
        2 => "2026-11-07",
        _ => "2026-11-05",
    };
    let exemption = if case.waiver == 0 {
        String::new()
    } else {
        format!(
            "[[waiver]]\nrule=\"rule-0\"\npaths=[\"src/**\"]\nreason=\"Finite oracle waiver.\"\nowner=\"Test owner\"\nexpires={waiver:?}\n"
        )
    };
    format!(
        "schema=\"forge-warden.policy.v0\"\n{}{exemption}",
        rules.join("\n")
    )
    .into_bytes()
}
fn actual(case: &Case) -> (u8, Vec<ResultSpec>) {
    let ranked: Vec<_> = case.files.iter().map(|file| {
        let prefix=match file.group { 0=>"src",1=>"tests",_=>"docs" };
        let status=if let Some((true,status)) = &file.unknown { status.as_str() } else { match file.shape {
            1=>"deleted",2..=4=>"binary_changed",_=>"modified" } };
        let mut signals=Vec::new();
        if file.secret { signals.push("secret_like"); }
        if file.shape==3 { signals.push("new_file"); }
        if file.shape==4 { signals.push("deleted_file"); }
        if let Some((false, signal)) = &file.unknown { signals.push(signal.as_str()); }
        json!({"path":format!("{prefix}/f{}.rs",file.id),"status":status,"score":file.score,"signals":signals})
    }).collect();
    let rank = json!({"schema":"deep-diff-forge.rank.v0","input_sha256":HASH,
        "tool":{"name":"deep-diff-forge","version":"0.7.0"},"ranked":ranked});
    let rank = parse_rank(&serde_json::to_vec(&rank).unwrap()).unwrap();
    let policy = parse_policy(&policy(case)).unwrap();
    let seals = Seals::new(seal(HASH), seal(HASH), seal(HASH), "0.7.0".to_owned());
    let verdict = evaluate(&policy, &rank, seals, Date::parse("2026-11-06").unwrap());
    assert_eq!(verdict.summary.rules, case.rules.len());
    (
        match verdict.verdict {
            Outcome::Pass => 0,
            Outcome::Hold => 1,
            Outcome::Fail => 2,
        },
        verdict
            .rules
            .iter()
            .map(|r| match r.outcome {
                RuleOutcome::Pass => ResultSpec::Clear,
                RuleOutcome::Waived => ResultSpec::Exempt,
                RuleOutcome::Hold => ResultSpec::Review,
                RuleOutcome::Fail => ResultSpec::Reject,
            })
            .collect(),
    )
}
fn rule_strategy() -> impl Strategy<Value = Rule> {
    let kind = prop_oneof![
        Just(CheckSpec::Forbid),
        Just(CheckSpec::Protect),
        Just(CheckSpec::Companion),
        (0_u32..101).prop_map(CheckSpec::FileScore),
        (0_u32..101).prop_map(CheckSpec::ChangeScore),
        (1_u32..101)
            .prop_map(|limit| CheckSpec::FileCount(std::num::NonZeroU32::new(limit).unwrap()))
    ];
    (kind, any::<bool>(), 0_u8..3).prop_map(|(kind, fail, scope)| Rule { kind, fail, scope })
}
fn case_strategy() -> impl Strategy<Value = Case> {
    (
        prop::collection::vec(
            (
                0_u8..3,
                0_u32..101,
                any::<bool>(),
                0_u8..5,
                prop::option::of((any::<bool>(), "future_[a-z]{1,12}")),
            ),
            0..9,
        ),
        prop::collection::vec(rule_strategy(), 1..7),
        0_u8..4,
    )
        .prop_map(|(files, rules, waiver)| Case {
            rules,
            waiver,
            files: files
                .into_iter()
                .enumerate()
                .map(|(id, (group, score, secret, shape, unknown))| File {
                    id,
                    group,
                    score,
                    secret,
                    shape: if unknown.as_ref().is_some_and(|(status, _)| *status) {
                        0
                    } else {
                        shape
                    },
                    unknown,
                })
                .collect(),
        })
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    #[test]
    fn all_rule_kinds_match_the_independent_reference(case in case_strategy()) {
        prop_assert_eq!(actual(&case),reference(&case));
    }
    #[test]
    fn adding_a_rule_cannot_loosen_the_verdict(mut case in case_strategy(),extra in rule_strategy()) {
        let before=actual(&case).0;case.rules.push(extra);
        prop_assert!(actual(&case).0>=before);
    }
    #[test]
    fn rank_order_cannot_change_the_outcome(mut case in case_strategy()) {
        let before=actual(&case);case.files.reverse();
        prop_assert_eq!(actual(&case),before);
    }
    #[test]
    fn generated_unknown_status_never_passes(suffix in "[a-z]{1,20}") {
        assert_unknown((true,format!("future_{suffix}")));
    }
    #[test]
    fn generated_unknown_signal_never_passes(suffix in "[a-z]{1,20}") {
        assert_unknown((false,format!("future_{suffix}")));
    }

}
fn assert_unknown(unknown: (bool, String)) {
    let mut case = Case {
        files: vec![File {
            id: 0,
            group: 2,
            score: 0,
            secret: false,
            shape: 0,
            unknown: None,
        }],
        rules: vec![Rule {
            kind: CheckSpec::Forbid,
            fail: true,
            scope: 0,
        }],
        waiver: 0,
    };
    assert_eq!(actual(&case).0, 0);
    case.files.first_mut().unwrap().unknown = Some(unknown);
    assert_eq!(actual(&case).0, 1);
}
#[test]
fn reference_anchor_cases_have_handwritten_outcomes() {
    let file = File {
        id: 0,
        group: 0,
        score: 10,
        secret: true,
        shape: 0,
        unknown: None,
    };
    let rule = Rule {
        kind: CheckSpec::Forbid,
        fail: true,
        scope: 0,
    };
    let mut case = Case {
        files: vec![file],
        rules: vec![rule],
        waiver: 0,
    };
    assert_eq!(reference(&case), (2, vec![ResultSpec::Reject]));
    case.waiver = 1;
    assert_eq!(reference(&case), (0, vec![ResultSpec::Exempt]));
    case.waiver = 3;
    assert_eq!(reference(&case), (2, vec![ResultSpec::Reject]));
    case.waiver = 1;
    case.rules
        .iter_mut()
        .for_each(|r| r.kind = CheckSpec::ChangeScore(0));
    assert_eq!(
        reference(&case),
        (2, vec![ResultSpec::Reject]),
        "change budgets are never waived"
    );
}

#[test]
fn finite_boundary_matrix_matches_the_independent_reference() {
    use CheckSpec::{ChangeScore, Companion, FileCount, FileScore, Forbid, Protect};
    let kinds = [
        Forbid,
        Protect,
        Companion,
        FileScore(0),
        FileScore(1),
        ChangeScore(0),
        ChangeScore(1),
        FileCount(std::num::NonZeroU32::new(1).unwrap()),
        FileCount(std::num::NonZeroU32::new(2).unwrap()),
    ];
    for kind in kinds {
        for count in 0..3 {
            for shape in 0..5 {
                for secret in [false, true] {
                    for fail in [false, true] {
                        for waiver in 0..4 {
                            for scope in 0..3 {
                                let case = Case {
                                    files: (0..count)
                                        .map(|id| File {
                                            id,
                                            group: 0,
                                            score: 1,
                                            secret,
                                            shape,
                                            unknown: None,
                                        })
                                        .collect(),
                                    rules: vec![Rule { kind, fail, scope }],
                                    waiver,
                                };
                                assert_eq!(actual(&case), reference(&case), "{case:?}");
                            }
                        }
                    }
                }
            }
        }
    }
}
#[test]
fn companion_liveness_has_handwritten_outcomes() {
    for (shape, want) in [
        (0, ResultSpec::Clear),
        (1, ResultSpec::Reject),
        (2, ResultSpec::Reject),
        (3, ResultSpec::Clear),
        (4, ResultSpec::Reject),
    ] {
        let case = Case {
            files: vec![
                File {
                    id: 0,
                    group: 0,
                    score: 1,
                    secret: false,
                    shape: 0,
                    unknown: None,
                },
                File {
                    id: 1,
                    group: 1,
                    score: 1,
                    secret: false,
                    shape,
                    unknown: None,
                },
            ],
            rules: vec![Rule {
                kind: CheckSpec::Companion,
                fail: true,
                scope: 0,
            }],
            waiver: 0,
        };
        assert_eq!(reference(&case).1, vec![want]);
        assert_eq!(actual(&case), reference(&case));
    }
}
