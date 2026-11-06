//! Who produced the ranking, whether it covers the patch, and what a verdict may conclude
//! (ADR 0005 §2.1, §2.7, §2.8).
//!
//! - A ranking is either **produced** (forge-warden ran the pinned ranker on the patch it holds) or
//!   **supplied** (it arrived as a file). The two are different types: [`Ranking<Produced>`] is made
//!   only by [`Ranking::attest`], which the binary calls only from its doors (a repository invariant
//!   checks this; a library caller can still call it, and that is stated, not hidden).
//! - [`check`] compares a ranking with Git's facts about the patch. Every disagreement or refusal
//!   becomes a note, and a note holds the verdict.
//! - Rules judge an *effective* ranking: each file's signals plus the change-kind signals Git's facts
//!   imply (T2: Git only adds). So a binary deletion is a deletion even when the producer says
//!   only `binary_change`.
//! - [`evaluate_advisory`] returns an [`AdvisoryVerdict`], whose outcome type has no pass.

use crate::date::Date;
use crate::defaults::NamedDefault as _;
use crate::eval::evaluate;
use crate::exit::Outcome;
use crate::facts::{Disagreement, PatchFacts, Unverifiable, compare, derived_signals};
use crate::policy::Policy;
use crate::rank::{RankDoc, RankedFile};
use crate::verdict::{ExitInfo, Note, NoteCode, Provenance, RankedBy, Seals, Verdict};
use std::collections::BTreeMap;
use std::marker::PhantomData;

/// Marker: forge-warden ran the pinned ranker itself.
#[derive(Debug)]
pub enum Produced {}

/// Marker: the ranking was supplied by the caller.
#[derive(Debug)]
pub enum Supplied {}

/// A ranking and where it came from.
#[derive(Debug)]
pub struct Ranking<P> {
    doc: RankDoc,
    producer_sha256: Option<String>,
    provenance: PhantomData<P>,
}

impl Ranking<Supplied> {
    /// A ranking the caller supplied. It can never approve a change.
    #[must_use]
    pub const fn supplied(doc: RankDoc) -> Self {
        Self {
            doc,
            producer_sha256: None,
            provenance: PhantomData,
        }
    }
}

impl Ranking<Produced> {
    /// A ranking forge-warden produced by running the ranker whose bytes hash to
    /// `producer_sha256`. The binary calls this only from `doors.rs`, right after the run.
    #[must_use]
    pub const fn attest(doc: RankDoc, producer_sha256: String) -> Self {
        Self {
            doc,
            producer_sha256: Some(producer_sha256),
            provenance: PhantomData,
        }
    }
}

impl<P> Ranking<P> {
    /// The ranking document.
    #[must_use]
    pub const fn doc(&self) -> &RankDoc {
        &self.doc
    }

    /// The pinned ranker's SHA-256, for a produced ranking.
    #[must_use]
    pub fn producer_sha256(&self) -> Option<&str> {
        self.producer_sha256.as_deref()
    }
}

/// A ranking checked against Git's facts about the patch.
#[derive(Debug)]
pub struct Checked {
    facts: Result<PatchFacts, Unverifiable>,
    disagreements: Vec<Disagreement>,
}

/// Compare `rank` with Git's view of the patch (or the reason Git's view is unavailable).
#[must_use]
pub fn check(rank: &RankDoc, facts: Result<PatchFacts, Unverifiable>) -> Checked {
    let disagreements = match &facts {
        Ok(facts) => compare(rank, facts),
        Err(_) => Vec::new(),
    };
    Checked {
        facts,
        disagreements,
    }
}

impl Checked {
    /// Whether the ranking covers the patch exactly and agrees with Git on every file.
    #[must_use]
    pub const fn is_covered(&self) -> bool {
        self.facts.is_ok() && self.disagreements.is_empty()
    }

    /// One note per disagreement, or the refusal; each holds the verdict.
    fn notes(&self) -> Vec<Note> {
        let note = |code: NoteCode, path: &crate::path::RepoPath, detail: &str| Note {
            code,
            path: Some(path.to_string()),
            detail: detail.to_owned(),
        };
        match &self.facts {
            Err(why) => vec![Note {
                code: NoteCode::PatchUnverifiable,
                path: None,
                detail: format!("{why}; coverage cannot be verified, so nothing passes by default"),
            }],
            Ok(_) => self
                .disagreements
                .iter()
                .map(|d| match d {
                    Disagreement::Missing(path) => note(
                        NoteCode::RankIncomplete,
                        path,
                        "git sees this file in the patch, but the ranking does not list it",
                    ),
                    Disagreement::Extraneous(path) => note(
                        NoteCode::RankExtraneous,
                        path,
                        "the ranking lists this file, but the patch does not change it",
                    ),
                    Disagreement::Status(path) => note(
                        NoteCode::StatusMismatch,
                        path,
                        "the ranking's status or change-kind signals contradict git",
                    ),
                })
                .collect(),
        }
    }

    /// The ranking with each file's signals widened by Git's change-kind facts (T2: Git only adds).
    fn effective(&self, rank: &RankDoc) -> RankDoc {
        let Ok(facts) = &self.facts else {
            return rank.clone();
        };
        let derived: BTreeMap<&crate::path::RepoPath, _> = facts
            .files()
            .map(|change| (change.path(), derived_signals(change)))
            .collect();
        let files: Vec<RankedFile> = rank
            .files()
            .iter()
            .map(|file| {
                let mut signals = file.signals().to_vec();
                for signal in derived.get(file.path()).into_iter().flatten() {
                    if !signals.iter().any(|s| s == signal) {
                        signals.push((*signal).to_owned());
                    }
                }
                widened(file, signals)
            })
            .collect();
        let widened = RankDoc::new(
            rank.input_sha256().to_owned(),
            rank.tool_version().to_owned(),
            files,
        );
        // The seal and version were already valid, so this cannot be refused; were it ever, the
        // unwidened ranking is judged and Git's facts still hold it via status_mismatch.
        widened.or_fail_safe(rank.clone())
    }
}

/// `file` with `signals`. Adding signals keeps every constructor check true (status, non-empty
/// signals, evidence citing a carried signal), so this cannot be refused; if it ever were, the
/// original file is kept, which can only miss an added signal, and Git's facts still hold the
/// verdict through `status_mismatch`.
fn widened(file: &RankedFile, signals: Vec<String>) -> RankedFile {
    let wide = RankedFile::new(
        file.path().clone(),
        file.status().to_owned(),
        file.score(),
        signals,
        file.evidence().to_vec(),
        file.evidence_truncated(),
    );
    // See above: the unwidened file can only miss an added signal.
    wide.or_fail_safe(file.clone())
}

/// A verdict on a supplied ranking. Its outcome type has no pass: a supplied ranking can at best
/// hold a change for a person.
#[derive(Debug)]
pub struct AdvisoryVerdict {
    verdict: Verdict,
}

/// The outcomes a supplied ranking can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdvisoryOutcome {
    /// A person must decide.
    Hold,
    /// The change does not meet the policy.
    Fail,
}

impl AdvisoryVerdict {
    /// The outcome: hold or fail, never pass.
    #[must_use]
    pub const fn outcome(&self) -> AdvisoryOutcome {
        match self.verdict.verdict {
            Outcome::Fail => AdvisoryOutcome::Fail,
            // The `rank_unverified` note already holds every advisory verdict; were it ever
            // missing, the type still reads a pass as a hold (fails closed).
            Outcome::Hold | Outcome::Pass => AdvisoryOutcome::Hold,
        }
    }

    /// The full verdict document (its `verdict` is `hold` or `fail`).
    #[must_use]
    pub const fn verdict(&self) -> &Verdict {
        &self.verdict
    }
}

/// A ranking the caller also supplied in authority mode (`--rank`), compared byte for byte with
/// the produced one (ADR 0005 §2.1). The produced ranking is always the one judged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppliedCopy {
    /// No ranking was supplied.
    Absent,
    /// The supplied bytes equal the produced bytes.
    Identical,
    /// They differ: the verdict holds with `rank_mismatch`.
    Different,
}

impl SuppliedCopy {
    /// Compare a supplied ranking's bytes (if any) with the produced ranking's bytes.
    #[must_use]
    pub fn compare(supplied: Option<&[u8]>, produced: &[u8]) -> Self {
        match supplied {
            None => Self::Absent,
            Some(bytes) if bytes == produced => Self::Identical,
            Some(_) => Self::Different,
        }
    }
}

/// Judge a ranking forge-warden produced itself. Only this can pass. `provenance` and `ranked_by`
/// in `seals` are set from `ranking`, whatever the caller put there.
#[must_use]
pub fn evaluate_authority(
    policy: &Policy,
    ranking: &Ranking<Produced>,
    checked: &Checked,
    supplied: SuppliedCopy,
    seals: Seals,
    today: Date,
) -> Verdict {
    let extra = match supplied {
        SuppliedCopy::Absent | SuppliedCopy::Identical => Vec::new(),
        SuppliedCopy::Different => vec![Note {
            code: NoteCode::RankMismatch,
            path: None,
            detail:
                "the supplied ranking differs from the one forge-warden produced; the produced \
                     ranking is the one judged and sealed"
                    .to_owned(),
        }],
    };
    let seals = provenance(seals, ranking, Provenance::Produced);
    judge(policy, &ranking.doc, checked, extra, seals, today)
}

/// `seals` with `provenance` and `ranked_by` taken from the ranking itself.
fn provenance<P>(mut seals: Seals, ranking: &Ranking<P>, kind: Provenance) -> Seals {
    seals.provenance = kind;
    seals.ranked_by = RankedBy {
        version: ranking.doc.tool_version().to_owned(),
        sha256: ranking.producer_sha256.clone(),
    };
    seals
}

/// Judge a ranking the caller supplied. It is held at least, never passed. `provenance` and
/// `ranked_by` in `seals` are set from `ranking`.
#[must_use]
pub fn evaluate_advisory(
    policy: &Policy,
    ranking: &Ranking<Supplied>,
    checked: &Checked,
    seals: Seals,
    today: Date,
) -> AdvisoryVerdict {
    let unverified = Note {
        code: NoteCode::RankUnverified,
        path: None,
        detail:
            "the ranking was supplied, not produced by forge-warden; it can inform, never approve"
                .to_owned(),
    };
    let seals = provenance(seals, ranking, Provenance::Supplied);
    let mut verdict = judge(
        policy,
        &ranking.doc,
        checked,
        vec![unverified],
        seals,
        today,
    );
    hold_at_least(&mut verdict);
    AdvisoryVerdict { verdict }
}

fn judge(
    policy: &Policy,
    rank: &RankDoc,
    checked: &Checked,
    mut extra: Vec<Note>,
    seals: Seals,
    today: Date,
) -> Verdict {
    let mut verdict = evaluate(policy, &checked.effective(rank), seals, today);
    extra.extend(checked.notes());
    if !extra.is_empty() {
        verdict.notes.extend(extra);
        hold_at_least(&mut verdict);
    }
    verdict
}

/// A verdict with notes is at least a hold.
fn hold_at_least(verdict: &mut Verdict) {
    verdict.verdict = verdict.verdict.max(Outcome::Hold);
    let entry = verdict.verdict.exit_entry();
    verdict.exit = ExitInfo {
        code: entry.code,
        name: entry.name,
        action: entry.action,
    };
}
