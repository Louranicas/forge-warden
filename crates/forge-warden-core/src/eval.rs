//! Evaluating a policy over a ranking.
//!
//! The whole engine is one pure, crate-private function, `evaluate` (the public judges are
//! [`crate::authority::evaluate_authority`] and [`crate::authority::evaluate_advisory`]). Its contract:
//!
//! 1. Rules are evaluated independently, in policy order; violations keep deep-diff-forge's risk
//!    order (highest first).
//! 2. A violation tied to a file can be covered by the first **active** waiver (on or before its
//!    `expires` date) for that rule whose paths match. Budget violations over a whole change are
//!    never waivable: a waiver names paths, and they have none.
//! 3. The verdict is the strictest of: every rule's outcome, `hold` for each [`Note`], and `fail`
//!    for each expired waiver. Unknown vocabulary and empty input can never make it looser.

use crate::date::Date;
use crate::defaults::NamedDefault as _;
use crate::exit::Outcome;
use crate::nonempty::NonEmpty;
use crate::policy::{Check, PathSet, Policy, Rule, Scope, ScorePer, Waiver};
use crate::rank::{RankDoc, RankedFile};
use crate::verdict::{
    Citation, ExitInfo, Note, NoteCode, RuleOutcome, RuleResult, Seals, Summary, Tool,
    VERDICT_SCHEMA, Verdict, Violation, WaiverNote,
};
use crate::vocab::{is_known_signal, is_known_status};
use std::collections::BTreeSet;

/// Evaluate `policy` over `rank`, judging waivers as of `today`. The raw rule engine: it knows
/// nothing of provenance or coverage, so it is crate-private (Q1-01). The only public judges are
/// [`crate::authority::evaluate_authority`] and [`crate::authority::evaluate_advisory`]; engine
/// tests reach this through the off-by-default `unattested-evaluate` feature (`crate::testing`).
#[must_use]
pub(crate) fn evaluate(policy: &Policy, rank: &RankDoc, seals: Seals, today: Date) -> Verdict {
    let mut used_waivers = BTreeSet::new();
    let judged: NonEmpty<RuleResult> = policy
        .rules()
        .map(|rule| judge_rule(rule, rank, policy.waivers(), today, &mut used_waivers));

    let (expired, unused) = classify_waivers(policy.waivers(), today, &used_waivers);
    let notes = notes_for(rank);

    // A policy always has a rule, so the strictest outcome starts from the first one: there is
    // no empty case and no default.
    let verdict = judged
        .iter()
        .map(|r| r.outcome.overall())
        .chain(notes.iter().map(|_| Outcome::Hold))
        .chain(expired.iter().map(|_| Outcome::Fail))
        .fold(judged.first().outcome.overall(), Outcome::max);
    let rules = judged.into_vec();
    let entry = verdict.exit_entry();
    let count = |outcome: RuleOutcome| rules.iter().filter(|r| r.outcome == outcome).count();
    let summary = Summary {
        files: rank.files().len(),
        rules: rules.len(),
        passed: count(RuleOutcome::Pass),
        waived: count(RuleOutcome::Waived),
        held: count(RuleOutcome::Hold),
        failed: count(RuleOutcome::Fail),
    };

    Verdict {
        schema: VERDICT_SCHEMA,
        tool: Tool {
            name: crate::TOOL_NAME,
            version: crate::TOOL_VERSION,
        },
        evaluated_on: today.to_string(),
        seals,
        verdict,
        exit: ExitInfo {
            code: entry.code,
            name: entry.name,
            action: entry.action,
        },
        summary,
        rules,
        expired_waivers: expired,
        unused_waivers: unused,
        notes,
    }
}

fn judge_rule(
    rule: &Rule,
    rank: &RankDoc,
    waivers: &[Waiver],
    today: Date,
    used: &mut BTreeSet<usize>,
) -> RuleResult {
    let mut violations = find_violations(&rule.check, rank.files());
    for violation in &mut violations {
        let Some(path) = violation.path.as_deref() else {
            continue;
        };
        let cover = waivers
            .iter()
            .enumerate()
            .find(|(_, w)| w.rule == rule.id && w.expires >= today && w.paths.matches(path));
        if let Some((index, _)) = cover {
            violation.waived_by = Some(index);
            used.insert(index);
        }
    }
    let outcome = if violations.is_empty() {
        RuleOutcome::Pass
    } else if violations.iter().all(|v| v.waived_by.is_some()) {
        RuleOutcome::Waived
    } else {
        match rule.severity.outcome() {
            Outcome::Fail => RuleOutcome::Fail,
            Outcome::Hold | Outcome::Pass => RuleOutcome::Hold,
        }
    };
    RuleResult {
        id: rule.id.clone(),
        kind: rule.check.kind().name(),
        severity: rule.severity.label(),
        outcome,
        why: rule.why.clone(),
        violations,
    }
}

fn violation(path: Option<&str>, message: String) -> Violation {
    Violation {
        path: path.map(str::to_owned),
        message,
        signal: None,
        evidence: Vec::new(),
        evidence_truncated: false,
        waived_by: None,
    }
}

fn find_violations(check: &Check, files: &[RankedFile]) -> Vec<Violation> {
    match check {
        Check::ForbidSignal { signals, scope } => forbidden_signals(signals, scope, files),
        Check::RequireCompanion { when, require } => missing_companions(when, require, files),
        Check::ProtectPaths { scope } => files
            .iter()
            .filter(|f| scope.contains(f.path().as_str()))
            .map(|f| {
                let message = format!("protected path changed ({})", f.status());
                violation(Some(f.path().as_str()), message)
            })
            .collect(),
        Check::MaxScore {
            limit,
            per: ScorePer::File,
            scope,
        } => files
            .iter()
            .filter(|f| scope.contains(f.path().as_str()) && f.score() > *limit)
            .map(|f| {
                let message = format!(
                    "risk score {} exceeds the per-file limit {limit}",
                    f.score()
                );
                violation(Some(f.path().as_str()), message)
            })
            .collect(),
        Check::MaxScore {
            limit,
            per: ScorePer::Change,
            scope,
        } => {
            let total: u64 = files
                .iter()
                .filter(|f| scope.contains(f.path().as_str()))
                .fold(0_u64, |sum, f| sum.saturating_add(u64::from(f.score())));
            over_budget(total, *limit, |total| {
                format!("total risk score {total} exceeds the change limit {limit}")
            })
        }
        Check::MaxFiles { limit, scope } => {
            let count = files
                .iter()
                .filter(|f| scope.contains(f.path().as_str()))
                .count();
            // An uncountable file count is treated as over every budget.
            let count = u64::try_from(count).or_fail_closed(u64::MAX);
            over_budget(count, *limit, |count| {
                format!("{count} files in scope changed; the limit is {limit}")
            })
        }
    }
}

/// One change-wide violation when `value` exceeds `limit`, none otherwise.
fn over_budget(value: u64, limit: u32, message: impl FnOnce(u64) -> String) -> Vec<Violation> {
    if value > u64::from(limit) {
        vec![violation(None, message(value))]
    } else {
        Vec::new()
    }
}

fn forbidden_signals(signals: &[String], scope: &Scope, files: &[RankedFile]) -> Vec<Violation> {
    let mut out = Vec::new();
    for file in files.iter().filter(|f| scope.contains(f.path().as_str())) {
        for signal in signals.iter().filter(|s| file.signals().contains(s)) {
            let mut evidence: Vec<Citation> = file
                .evidence()
                .iter()
                .filter(|e| e.signal() == signal)
                .map(|e| Citation {
                    address: e.address().to_owned(),
                    side: e.side().to_owned(),
                    line: e.line(),
                })
                .collect();
            evidence.sort();
            let message = format!("carries forbidden signal `{signal}`");
            out.push(Violation {
                signal: Some(signal.clone()),
                evidence,
                evidence_truncated: file.evidence_truncated(),
                ..violation(Some(file.path().as_str()), message)
            });
        }
    }
    out
}

/// Whether a file was removed by the change. deep-diff-forge classifies a deleted binary file as
/// `binary_changed` and marks the deletion only with the `deleted_file` signal, so the status
/// alone is not enough.
fn is_deleted(file: &RankedFile) -> bool {
    file.status() == "deleted" || file.signals().iter().any(|s| s == "deleted_file")
}

/// Whether a file can stand as a companion: it must exist after the change, and its change must be
/// knowable. A `binary_changed` file without `new_file` may be a deletion deep-diff-forge could not
/// label, so it does not count.
fn is_live_companion(file: &RankedFile) -> bool {
    !is_deleted(file)
        && (file.status() != "binary_changed" || file.signals().iter().any(|s| s == "new_file"))
}

fn missing_companions(when: &Scope, require: &PathSet, files: &[RankedFile]) -> Vec<Violation> {
    // A deleted file is not a companion: deleting the tests must not satisfy "tests must change".
    // Nor is it a trigger: removing code asks for no new tests.
    let has_companion = files
        .iter()
        .any(|f| is_live_companion(f) && require.matches(f.path().as_str()));
    if has_companion {
        return Vec::new();
    }
    let wanted = require.patterns().join(", ");
    files
        .iter()
        .filter(|f| !is_deleted(f) && when.contains(f.path().as_str()))
        .map(|f| {
            let message = format!("changed without a companion change matching [{wanted}]");
            violation(Some(f.path().as_str()), message)
        })
        .collect()
}

fn classify_waivers(
    waivers: &[Waiver],
    today: Date,
    used: &BTreeSet<usize>,
) -> (Vec<WaiverNote>, Vec<WaiverNote>) {
    let note = |index: usize, w: &Waiver| WaiverNote {
        index,
        rule: w.rule.clone(),
        owner: w.owner.clone(),
        expires: w.expires.to_string(),
    };
    let expired = waivers
        .iter()
        .enumerate()
        .filter(|(_, w)| w.expires < today)
        .map(|(i, w)| note(i, w))
        .collect();
    let unused = waivers
        .iter()
        .enumerate()
        .filter(|(i, w)| w.expires >= today && !used.contains(i))
        .map(|(i, w)| note(i, w))
        .collect();
    (expired, unused)
}

fn notes_for(rank: &RankDoc) -> Vec<Note> {
    let mut notes = Vec::new();
    if rank.files().is_empty() {
        notes.push(Note {
            code: NoteCode::EmptyRanking,
            path: None,
            detail: "the ranking lists no files; there is nothing to judge, so nothing passes by default".to_owned(),
        });
    }
    // A real producer lists one path twice for a type change (file -> symlink: an addition and a
    // deletion; MATRIX.md). Which entry a rule sees would then be an accident of order, so hold.
    let mut seen = BTreeSet::new();
    let mut duplicated = BTreeSet::new();
    for file in rank.files() {
        if !seen.insert(file.path().as_str()) {
            duplicated.insert(file.path().as_str());
        }
    }
    for path in duplicated {
        notes.push(Note {
            code: NoteCode::DuplicatePaths,
            path: Some(path.to_owned()),
            detail: "the ranking lists this path more than once (a type change, or a forged ranking); rules cannot tell which entry is the file".to_owned(),
        });
    }
    for file in rank.files() {
        for signal in file.signals().iter().filter(|s| !is_known_signal(s)) {
            notes.push(Note {
                code: NoteCode::UnknownSignal,
                path: Some(file.path().to_string()),
                detail: format!(
                    "signal `{signal}` is not in this forge-warden's vocabulary; upgrade forge-warden or review it by hand"
                ),
            });
        }
        if file.status() == "renamed" || file.status() == "copied" {
            notes.push(Note {
                code: NoteCode::SourcePathUnknown,
                path: Some(file.path().to_string()),
                detail: format!(
                    "{} from a path rank.v0 does not report, so rules cannot see where it came from; \
                     rank with `git diff --no-renames` to judge both paths",
                    file.status()
                ),
            });
        } else if !is_known_status(file.status()) {
            notes.push(Note {
                code: NoteCode::UnknownStatus,
                path: Some(file.path().to_string()),
                detail: format!(
                    "status `{}` is not in this forge-warden's vocabulary",
                    file.status()
                ),
            });
        }
    }
    notes
}
