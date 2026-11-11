//! JEV Z3: the frozen authority cases (Touchstone CASES.md 9e2497f1) judged through the public
//! API and compared, at the serialized wire level, with the expectations the independent oracle
//! (`fixtures/adversarial/z3_oracle.rs`, standard library only) wrote into
//! `fixtures/z3/manifest.json`.
//! The actual side may parse, admit, check and judge; the expected side never does.
#![expect(
    clippy::disallowed_methods,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test helpers read committed fixtures; a panic is a failed test"
)]

// The oracle's own text admits no attributes beyond docs and standard derives (Q-ORACLE
// registry isolation), so its lint expectation sits on this declaration.
#[expect(
    clippy::too_many_lines,
    reason = "the oracle's case table and manifest entry are one row per case or field"
)]
#[path = "../../../fixtures/adversarial/z3_oracle.rs"]
mod oracle;

use forge_warden_core::authority::{
    AdvisoryOutcome, Ranking, SuppliedCopy, check, evaluate_advisory, evaluate_authority,
};
use forge_warden_core::facts::{PatchFacts, Unverifiable};
use forge_warden_core::policy::parse_policy;
use forge_warden_core::rank::parse_rank;
use forge_warden_core::seal::{Sha256Hex, sha256_hex};
use forge_warden_core::verdict::{Provenance, RankedBy, Seals};
use forge_warden_core::{Date, Verdict};
use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestCaseError, TestRng, TestRunner};
use serde_json::Value;
use std::cell::Cell;
use std::path::PathBuf;

fn z3() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/z3")
}

fn read(case: &str, file: &str) -> Option<Vec<u8>> {
    std::fs::read(z3().join("cases").join(case).join(file)).ok()
}

fn manifest() -> Value {
    serde_json::from_slice(&std::fs::read(z3().join("manifest.json")).unwrap()).unwrap()
}

/// What the actual side produced for one case.
struct Actual {
    stage: &'static str,
    coverage: Option<bool>,
    verdict: Option<Value>,
    advisory: Option<AdvisoryOutcome>,
    /// `from_git`'s refusal kind (Debug form) for a case whose Git triple was parsed.
    facts_refusal: Option<String>,
}

/// Judge one case whose input files `bytes` returns by name (from disk for frozen cases, from the
/// oracle's emission for generated ones).
fn judge(entry: &Value, bytes: &dyn Fn(&str) -> Option<Vec<u8>>) -> Actual {
    let id = entry["case_id"].as_str().unwrap();
    let refused = |stage| Actual {
        stage,
        coverage: None,
        verdict: None,
        advisory: None,
        facts_refusal: None,
    };
    let policy_bytes = bytes("policy.toml").unwrap();
    let Ok(policy) = parse_policy(&policy_bytes) else {
        return refused("policy_refused");
    };
    let rank_bytes = bytes("rank.json").unwrap();
    let patch = bytes("patch").unwrap();
    // A valid case whose ranking fails to parse is a failed setup, never a non-PASS verdict.
    let rank =
        parse_rank(&rank_bytes).unwrap_or_else(|e| panic!("Z3.{id}.setup: rank refused: {e}"));
    let facts = match entry["facts"].as_str().unwrap().strip_prefix("refused:") {
        Some(why) => Err(Unverifiable::GitRefused(why.to_owned())),
        None => PatchFacts::from_git(
            &bytes("numstat-z").unwrap(),
            &bytes("summary").unwrap(),
            &patch,
        ),
    };
    let facts_refusal = match &facts {
        Err(why) if entry["facts"] == "git" => Some(format!("{why:?}")),
        Ok(_) | Err(_) => None,
    };
    let mut seals = Seals::new(
        sha256_hex(&policy_bytes),
        sha256_hex(&rank_bytes),
        sha256_hex(&patch),
        rank.tool_version().to_owned(),
    );
    let today = Date::parse(entry["today"].as_str().unwrap()).unwrap();
    if entry["origin"] == "supplied" {
        let ranking = Ranking::supplied(rank);
        let checked = check(&ranking, facts);
        let coverage = Some(checked.is_covered());
        let advisory = evaluate_advisory(&policy, &checked, seals, today);
        return Actual {
            stage: "judged",
            coverage,
            verdict: Some(serde_json::to_value(advisory.verdict()).unwrap()),
            advisory: Some(advisory.outcome()),
            facts_refusal,
        };
    }
    let asked = Sha256Hex::parse(entry["admit_producer"].as_str().unwrap()).unwrap();
    let Some(token) = policy.admit_producer(&asked) else {
        return refused("admission_refused");
    };
    if entry["false_claim"] == true {
        let m = manifest();
        seals.provenance = Provenance::Supplied;
        seals.ranked_by = RankedBy::new(
            m["claimed_version"].as_str().unwrap().to_owned(),
            Some(Sha256Hex::parse(m["producer_b"].as_str().unwrap()).unwrap()),
        );
    }
    let copy = SuppliedCopy::compare(bytes("supplied.json").as_deref(), &rank_bytes);
    let ranking = Ranking::attest(rank, token);
    let checked = check(&ranking, facts);
    let coverage = Some(checked.is_covered());
    let verdict: Verdict = evaluate_authority(&policy, &checked, copy, seals, today);
    Actual {
        stage: "judged",
        coverage,
        verdict: Some(serde_json::to_value(&verdict).unwrap()),
        advisory: None,
        facts_refusal,
    }
}

/// A violation as the assertions see it: its identity, not its prose.
fn violation_identity(v: &Value) -> Value {
    serde_json::json!({
        "path": v.get("path").cloned().unwrap_or(Value::Null),
        "signal": v.get("signal").cloned().unwrap_or(Value::Null),
        "waived_by": v.get("waived_by").cloned().unwrap_or(Value::Null),
        "evidence": v.get("evidence").cloned().unwrap_or_else(|| Value::Array(Vec::new())),
    })
}

fn expected_identity(v: &Value) -> Value {
    let evidence: Vec<Value> = v["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| serde_json::json!({"address": e["address"], "side": e["side"], "line": e["line"]}))
        .collect();
    serde_json::json!({"path": v["path"], "signal": v["signal"], "waived_by": v["waived_by"], "evidence": evidence})
}

fn sorted(mut items: Vec<Value>) -> Vec<Value> {
    items.sort_by_key(ToString::to_string);
    items
}

/// Every failed assertion for one case, by assertion ID.
#[expect(
    clippy::too_many_lines,
    reason = "one block per assertion facet keeps the IDs visible"
)]
fn failures(entry: &Value, actual: &Actual) -> Vec<String> {
    let id = entry["case_id"].as_str().unwrap();
    let mut out = Vec::new();
    let mut expect = |facet: &str, ok: bool, detail: String| {
        if !ok {
            out.push(format!("Z3.{id}.{facet}: {detail}"));
        }
    };
    let stage = entry["setup_stage"].as_str().unwrap();
    expect(
        "setup",
        actual.stage == stage,
        format!("stage {} != {stage}", actual.stage),
    );
    // Specificity, separate from safety (Lodestar's Z3-N20-N21 ruling): the exact refusal kind
    // is the specification. Every other parsed triple must be admitted: a refused valid triple
    // is a failed setup, never a quiet non-PASS.
    let kind = entry["expected_facts_refusal"].as_str();
    expect(
        if kind.is_some() {
            "refusal_kind"
        } else {
            "setup"
        },
        actual.facts_refusal.as_deref() == kind,
        format!("facts refusal {:?} != {kind:?}", actual.facts_refusal),
    );
    let Some(v) = &actual.verdict else {
        return out;
    };
    if stage != "judged" {
        return out;
    }
    let cov = entry["expected_coverage"].as_bool();
    expect(
        "coverage",
        actual.coverage == cov,
        format!("{:?} != {cov:?}", actual.coverage),
    );
    let want = &entry["expected_verdict"];
    expect(
        "overall",
        v["verdict"] == *want,
        format!("verdict {} != {want}", v["verdict"]),
    );
    let exit = &entry["expected_exit"];
    expect(
        "overall",
        v["exit"] == *exit,
        format!("exit {} != {exit}", v["exit"]),
    );
    if let Some(advisory) = actual.advisory {
        let name = match advisory {
            AdvisoryOutcome::Hold => "hold",
            AdvisoryOutcome::Fail => "fail",
        };
        expect(
            "overall",
            *want == name,
            format!("advisory accessor {name} != {want}"),
        );
    }
    let summary = &entry["expected_summary"];
    expect(
        "summary",
        v["summary"] == *summary,
        format!("{} != {summary}", v["summary"]),
    );

    let got_rules = v["rules"].as_array().unwrap();
    let want_rules = entry["expected_rule_results"].as_array().unwrap();
    let ids = |rules: &[Value]| rules.iter().map(|r| r["id"].clone()).collect::<Vec<_>>();
    expect(
        "rules",
        ids(got_rules) == ids(want_rules),
        "rule ids or policy order differ".to_owned(),
    );
    for (got, want_rule) in got_rules.iter().zip(want_rules) {
        let rid = want_rule["id"].as_str().unwrap();
        expect(
            "rules",
            got["outcome"] == want_rule["outcome"],
            format!(
                "{rid} outcome {} != {}",
                got["outcome"], want_rule["outcome"]
            ),
        );
        let got_v = got["violations"].as_array().unwrap();
        let want_v = want_rule["violations"].as_array().unwrap();
        let gi = sorted(got_v.iter().map(violation_identity).collect());
        let wi = sorted(want_v.iter().map(expected_identity).collect());
        expect(
            "rules",
            gi == wi,
            format!("{rid} violations {gi:?} != {wi:?}"),
        );
        for w in want_v {
            if let Some(text) = w["message_contains"].as_str() {
                let found = got_v
                    .iter()
                    .any(|g| g["message"].as_str().unwrap().contains(text));
                expect(
                    "rules",
                    found,
                    format!("{rid}: no violation message contains {text:?}"),
                );
            }
        }
    }

    let notes = v["notes"].as_array().unwrap();
    let key = |n: &Value| serde_json::json!({"code": n["code"], "path": n.get("path").cloned().unwrap_or(Value::Null)});
    let got_n = sorted(notes.iter().map(key).collect());
    let want_n = sorted(
        entry["expected_notes"]
            .as_array()
            .unwrap()
            .iter()
            .map(key)
            .collect(),
    );
    expect("notes", got_n == want_n, format!("{got_n:?} != {want_n:?}"));
    for n in entry["expected_notes"].as_array().unwrap() {
        if let Some(text) = n["detail_contains"].as_str() {
            let found = notes
                .iter()
                .any(|g| g["code"] == n["code"] && g["detail"].as_str().unwrap().contains(text));
            expect(
                "notes",
                found,
                format!("no {} note detail contains {text:?}", n["code"]),
            );
        }
    }

    let indices = |list: &Value| {
        list.as_array()
            .unwrap()
            .iter()
            .map(|w| w["index"].clone())
            .collect::<Vec<_>>()
    };
    let expired = entry["expected_expired_waivers"]
        .as_array()
        .unwrap()
        .clone();
    let unused = entry["expected_unused_waivers"].as_array().unwrap().clone();
    expect(
        "waivers",
        indices(&v["expired_waivers"]) == expired,
        format!("expired {} != {expired:?}", v["expired_waivers"]),
    );
    expect(
        "waivers",
        indices(&v["unused_waivers"]) == unused,
        format!("unused {} != {unused:?}", v["unused_waivers"]),
    );

    let prov = &entry["expected_provenance"];
    let seals = &v["seals"];
    expect(
        "provenance",
        seals["provenance"] == prov["provenance"],
        format!("{} != {}", seals["provenance"], prov["provenance"]),
    );
    expect(
        "provenance",
        seals["ranked_by"]["sha256"] == prov["ranked_by_sha256"],
        format!(
            "{} != {}",
            seals["ranked_by"]["sha256"], prov["ranked_by_sha256"]
        ),
    );
    expect(
        "provenance",
        seals["ranked_by"]["version"] == prov["version"],
        format!("{} != {}", seals["ranked_by"]["version"], prov["version"]),
    );
    out
}

#[test]
fn the_frozen_manifest_and_case_bytes_are_exactly_what_the_oracle_emits() {
    let frozen = std::fs::read_to_string(z3().join("manifest.json")).unwrap();
    assert_eq!(
        oracle::manifest(),
        frozen,
        "Z3.manifest: the frozen manifest drifted from the oracle"
    );
    for case in oracle::cases() {
        for (name, bytes) in case.files() {
            assert_eq!(
                read(case.id, name).as_deref(),
                Some(bytes.as_bytes()),
                "Z3.{}.bytes: {name} drifted from the oracle",
                case.id
            );
        }
    }
}

#[test]
fn every_frozen_case_meets_its_independent_expectation() {
    let m = manifest();
    let entries = m["cases"].as_array().unwrap();
    // Completeness: every frozen ID is present and executed; none may be skipped.
    let ids: Vec<&str> = entries
        .iter()
        .map(|e| e["case_id"].as_str().unwrap())
        .collect();
    for n in 1..=20 {
        assert!(
            ids.contains(&format!("P{n:02}").as_str()),
            "Z3.P{n:02}.setup: missing"
        );
    }
    for n in 1..=28 {
        assert!(
            ids.contains(&format!("N{n:02}").as_str()),
            "Z3.N{n:02}.setup: missing"
        );
    }
    let mut failed = Vec::new();
    let mut passes = 0_usize;
    for entry in entries {
        let id = entry["case_id"].as_str().unwrap();
        let actual = judge(entry, &|file| read(id, file));
        if actual
            .verdict
            .as_ref()
            .is_some_and(|v| v["verdict"] == "pass")
        {
            passes = passes.saturating_add(1);
        }
        failed.extend(failures(entry, &actual));
    }
    assert!(
        failed.is_empty(),
        "{} Z3 assertions failed:\n{}",
        failed.len(),
        failed.join("\n")
    );
    // Non-vacuity: every P row really passed (an all-HOLD implementation fails here and above).
    assert_eq!(
        passes, 20,
        "Z3.positives: expected exactly the 20 P rows to PASS"
    );
}

// ---------------------------------------------------------------- bounded generated laws

/// The committed generator seed (`ChaCha`, 32 bytes) and the cases per law (`DESIGN.md`: 256).
const LAW_SEED: [u8; 32] = *b"forge-warden JEV-Z3 laws seed v1";
const LAW_CASES: u32 = 256;
const PATHS: [&str; 4] = ["src/a.rs", "src/b.rs", "tests/a.rs", "docs/b.rs"];
const SCORES: [u32; 6] = [0, 1, 9, 10, 11, u32::MAX];
const RULE_IDS: [&str; 3] = ["r0", "r1", "r2"];
const EXTRA_SIGNALS: [&str; 6] = [
    "binary_change",
    "new_file",
    "deleted_file",
    "executable_bit",
    "symlink_change",
    "secret_like",
];

/// One bounded draw: 1-4 distinct paths, each with a supported Git shape and a score, 1-3 rules,
/// and the knobs the laws use to pick a fault, a file, an addition and a permutation.
#[derive(Clone, Debug)]
struct Draw {
    files: Vec<(usize, usize, usize)>,
    rules: Vec<u8>,
    copy: bool,
    rotate: usize,
    fault: u8,
    pick: usize,
    extra: u8,
}

fn draws() -> impl Strategy<Value = Draw> {
    (
        prop::sample::subsequence(vec![0_usize, 1, 2, 3], 1..=4),
        prop::collection::vec((0_usize..14, 0_usize..6), 4),
        prop::collection::vec(0_u8..4, 1..=3),
        any::<bool>(),
        0_usize..4,
        0_u8..4,
        0_usize..4,
        0_u8..3,
    )
        .prop_map(
            |(paths, shapes, rules, copy, rotate, fault, pick, extra)| Draw {
                files: paths
                    .iter()
                    .zip(&shapes)
                    .map(|(path, (shape, score))| (*path, *shape, *score))
                    .collect(),
                rules,
                copy,
                rotate,
                fault,
                pick,
                extra,
            },
        )
}

/// The draw as a valid positive, constructed valid first: exact coverage with complete
/// change-kind signals (the oracle's handwritten table) and rules clear by construction.
fn positive(d: &Draw) -> oracle::Case {
    let mut case = oracle::cases().remove(0);
    case.id = "LAW";
    case.git = Vec::new();
    case.rank = Vec::new();
    for &(path, shape, score) in &d.files {
        let (shape, old, new) = oracle::LAW_SHAPES[shape];
        let git = oracle::GitFile {
            path: PATHS[path],
            shape,
            old,
            new,
        };
        let (status, signals) = oracle::ranked_for(&git);
        case.rank.push(oracle::Ranked {
            path: PATHS[path],
            status,
            score: SCORES[score],
            signals,
            evidence: Vec::new(),
        });
        case.git.push(git);
    }
    let count = u32::try_from(case.rank.len()).unwrap();
    let max = case.rank.iter().map(|r| r.score).max().unwrap();
    let sum: u64 = case.rank.iter().map(|r| u64::from(r.score)).sum();
    case.rules = d
        .rules
        .iter()
        .zip(RULE_IDS)
        .map(|(kind, id)| oracle::Rule {
            id,
            check: match (kind, u32::try_from(sum)) {
                (0, _) => oracle::Check::Forbid("secret_like"),
                (1, _) => oracle::Check::MaxFiles(count),
                (3, Ok(total)) => oracle::Check::SumScore(total),
                _ => oracle::Check::FileScore(max),
            },
        })
        .collect();
    case.copy = if d.copy {
        oracle::Supply::Identical
    } else {
        oracle::Supply::Absent
    };
    case
}

/// Make rule r0 forbid `secret_like` and give file `at` that producer-only hazard.
fn with_secret(mut case: oracle::Case, at: usize) -> oracle::Case {
    case.rules[0] = oracle::Rule {
        id: "r0",
        check: oracle::Check::Forbid("secret_like"),
    };
    case.rank[at].signals.push("secret_like");
    case
}

/// One coverage fault from DESIGN.md's law 3, and the note it must produce. `None` when the draw
/// cannot express it (no free path for an extraneous entry).
fn fault(
    mut case: oracle::Case,
    d: &Draw,
) -> Option<(oracle::Case, &'static str, Option<&'static str>)> {
    let at = d.pick.checked_rem(case.rank.len())?;
    let path = case.rank[at].path;
    match d.fault {
        0 => {
            case.rank.remove(at);
            Some((case, "rank_incomplete", Some(path)))
        }
        1 => {
            let free = PATHS
                .iter()
                .find(|p| case.git.iter().all(|g| g.path != **p))?;
            case.rank.push(oracle::Ranked {
                path: free,
                status: "modified",
                score: 0,
                signals: Vec::new(),
                evidence: Vec::new(),
            });
            Some((case, "rank_extraneous", Some(*free)))
        }
        2 => {
            let flipped = if case.rank[at].status == "modified" {
                "added"
            } else {
                "modified"
            };
            case.rank[at].status = flipped;
            Some((case, "status_mismatch", Some(path)))
        }
        _ => {
            case.refused = Some("Z3 law refusal");
            Some((case, "patch_unverifiable", None))
        }
    }
}

fn run(case: &oracle::Case) -> Actual {
    let entry: Value = serde_json::from_str(&case.manifest_entry()).unwrap();
    let files = case.files();
    judge(&entry, &|name| {
        files
            .iter()
            .find(|(file, _)| *file == name)
            .map(|(_, bytes)| bytes.clone().into_bytes())
    })
}

fn verdict_of(a: &Actual) -> String {
    a.verdict.as_ref().unwrap()["verdict"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn level(verdict: &str) -> u8 {
    match verdict {
        "pass" => 0,
        "hold" => 1,
        _ => 2,
    }
}

fn has_note(a: &Actual, code: &str, path: Option<&str>) -> bool {
    a.verdict.as_ref().unwrap()["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["code"] == code && n.get("path").and_then(Value::as_str) == path)
}

fn violated(a: &Actual, rule: &str, path: &str, signal: &str) -> bool {
    a.verdict.as_ref().unwrap()["rules"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["id"] == rule && r["outcome"] == "fail")
        .flat_map(|r| r["violations"].as_array().unwrap())
        .any(|v| v["path"] == path && v["signal"] == signal)
}

/// The verdict with set-like parts in a fixed order (rule order stays policy order).
fn normalized(a: &Actual) -> Value {
    let v = a.verdict.as_ref().unwrap();
    let rules: Vec<Value> = v["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let violations = sorted(
                r["violations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(violation_identity)
                    .collect(),
            );
            serde_json::json!({"id": r["id"], "outcome": r["outcome"], "violations": violations})
        })
        .collect();
    let notes = sorted(
        v["notes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| serde_json::json!({"code": n["code"], "path": n.get("path")}))
            .collect(),
    );
    serde_json::json!({"verdict": v["verdict"], "exit": v["exit"], "summary": v["summary"], "rules": rules,
        "notes": notes, "expired": v["expired_waivers"], "unused": v["unused_waivers"]})
}

/// Run one law over the committed seed. `check` returns whether the draw applied (was not
/// vacuous); executed and accepted counts are printed for the receipt and a floor is enforced.
#[expect(
    clippy::print_stderr,
    reason = "the executed and accepted counts are the law's receipt evidence"
)]
fn law(name: &str, check: impl Fn(&Draw) -> Result<bool, TestCaseError>) {
    let config = Config {
        cases: LAW_CASES,
        failure_persistence: None,
        ..Config::default()
    };
    let mut runner =
        TestRunner::new_with_rng(config, TestRng::from_seed(RngAlgorithm::ChaCha, &LAW_SEED));
    let executed = Cell::new(0_usize);
    let accepted = Cell::new(0_usize);
    let result = runner.run(&draws(), |d| {
        executed.set(executed.get().saturating_add(1));
        if check(&d)? {
            accepted.set(accepted.get().saturating_add(1));
        }
        Ok(())
    });
    eprintln!(
        "Z3.law.{name}: cases={LAW_CASES} executed={} accepted={} seed={:?}",
        executed.get(),
        accepted.get(),
        std::str::from_utf8(&LAW_SEED).unwrap()
    );
    if let Err(failure) = result {
        panic!("Z3.law.{name}: {failure}");
    }
    let floor = usize::try_from(LAW_CASES / 2).unwrap();
    assert!(
        accepted.get() >= floor,
        "Z3.law.{name}: vacuous ({} accepted)",
        accepted.get()
    );
}

#[test]
fn law_exact_coverage_and_clear_rules_pass_in_authority_mode() {
    law("positive_pass", |d| {
        let a = run(&positive(d));
        prop_assert_eq!(verdict_of(&a), "pass");
        prop_assert_eq!(a.coverage, Some(true));
        Ok(true)
    });
}

#[test]
fn law_a_supplied_ranking_never_passes_and_a_failure_stays_failed() {
    law("supplied_no_pass", |d| {
        let mut case = positive(d);
        case.copy = oracle::Supply::Absent;
        case.supplied_origin = true;
        let a = run(&case);
        prop_assert!(verdict_of(&a) != "pass");
        prop_assert!(has_note(&a, "rank_unverified", None));
        prop_assert!(a.advisory.is_some());
        let failing = with_secret(case, d.pick.checked_rem(d.files.len()).unwrap());
        prop_assert_eq!(verdict_of(&run(&failing)), "fail");
        let mut produced = failing;
        produced.supplied_origin = false;
        prop_assert_eq!(verdict_of(&run(&produced)), "fail");
        Ok(true)
    });
}

#[test]
fn law_one_coverage_fault_names_itself_and_never_loosens_a_failure() {
    law("one_fault", |d| {
        let Some((case, code, path)) = fault(positive(d), d) else {
            return Ok(false);
        };
        let a = run(&case);
        prop_assert!(verdict_of(&a) != "pass");
        prop_assert!(has_note(&a, code, path), "missing {} {:?}", code, path);
        if !case.rank.is_empty() {
            let failing = with_secret(case, 0);
            prop_assert_eq!(verdict_of(&run(&failing)), "fail");
        }
        Ok(true)
    });
}

#[test]
fn law_adding_a_rule_or_a_diagnostic_never_loosens() {
    law("monotone", |d| {
        let base = if d.extra == 0 {
            positive(d)
        } else {
            fault(positive(d), d).map_or_else(|| positive(d), |f| f.0)
        };
        if base.rank.is_empty() {
            return Ok(false);
        }
        let before = level(&verdict_of(&run(&base)));
        let mut stricter = base;
        let signal = EXTRA_SIGNALS[d.pick.checked_rem(EXTRA_SIGNALS.len()).unwrap()];
        stricter.rules.push(oracle::Rule {
            id: "r9",
            check: oracle::Check::Forbid(signal),
        });
        if d.rotate.checked_rem(2) == Some(1) {
            stricter.rank[0].signals.push("future_z3");
        }
        prop_assert!(level(&verdict_of(&run(&stricter))) >= before);
        Ok(true)
    });
}

#[test]
fn law_permuting_unique_paths_preserves_the_judgement() {
    law("permutation", |d| {
        let base = if d.extra == 0 {
            positive(d)
        } else {
            fault(positive(d), d).map_or_else(|| positive(d), |f| f.0)
        };
        if base.rank.len() < 2 {
            return Ok(false);
        }
        let mut permuted = base.clone();
        let turn = d.rotate.checked_rem(permuted.rank.len()).unwrap();
        permuted.rank.rotate_left(turn);
        permuted.git.reverse();
        prop_assert_eq!(normalized(&run(&base)), normalized(&run(&permuted)));
        Ok(true)
    });
}

#[test]
fn law_an_omitted_git_hazard_still_reaches_its_rule_and_producer_hazards_survive() {
    law("omitted_hazard", |d| {
        let mut case = positive(d);
        let len = case.rank.len();
        let Some(at) = (0..len)
            .map(|k| d.pick.saturating_add(k).checked_rem(len).unwrap())
            .find(|i| !case.rank[*i].signals.is_empty())
        else {
            return Ok(false);
        };
        let hazard = case.rank[at].signals.remove(0);
        case.rank[at].signals.push("secret_like");
        let path = case.rank[at].path;
        case.rules = vec![
            oracle::Rule {
                id: "r0",
                check: oracle::Check::Forbid(hazard),
            },
            oracle::Rule {
                id: "r1",
                check: oracle::Check::Forbid("secret_like"),
            },
        ];
        let a = run(&case);
        prop_assert!(
            violated(&a, "r0", path, hazard),
            "{} lost on {}",
            hazard,
            path
        );
        prop_assert!(violated(&a, "r1", path, "secret_like"));
        prop_assert!(has_note(&a, "status_mismatch", Some(path)));
        prop_assert_eq!(verdict_of(&a), "fail");
        Ok(true)
    });
}

#[test]
fn the_frozen_positives_agree_with_the_oracle_shape_table() {
    for case in oracle::cases().iter().filter(|c| c.id.starts_with('P')) {
        for ranked in &case.rank {
            let git = case.git.iter().find(|g| g.path == ranked.path).unwrap();
            let (status, signals) = oracle::ranked_for(git);
            let mut kinds = ranked.signals.clone();
            kinds.retain(|s| *s != "secret_like");
            assert_eq!(
                (ranked.status, kinds),
                (status, signals),
                "Z3.{}.shape_table",
                case.id
            );
        }
    }
}
