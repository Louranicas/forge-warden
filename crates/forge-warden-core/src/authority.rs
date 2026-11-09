//! Who produced the ranking, whether it covers the patch, and what a verdict may conclude
//! (ADR 0005 §2.1, §2.7, §2.8).
//!
//! - A ranking is either **produced** (forge-warden ran the pinned ranker on the patch it holds) or
//!   **supplied** (it arrived as a file). The two are different types: [`Ranking<Produced>`] is made
//!   only by [`Ranking::attest`], which takes an [`AdmittedProducer`] that only
//!   [`Policy::admit_producer`] makes, so every produced ranking names a ranker some policy's
//!   allowlist admits (Q-TYPES-1). That a door actually ran it is the binary's guarantee (the
//!   CLI's clippy.toml lets only `doors.rs` call `attest`); the core cannot observe a process.
//! - [`check`] compares a ranking with Git's facts about the patch and keeps a borrow of that exact
//!   ranking: a [`Checked`] is the only thing the judges accept, so a coverage proof cannot be
//!   paired with another ranking (Zen B1). Every disagreement or refusal becomes a note, and a note
//!   holds the verdict.
//! - Rules judge an *effective* ranking: each file's signals plus the change-kind signals Git's facts
//!   imply (T2: Git only adds). So a binary deletion is a deletion even when the producer says
//!   only `binary_change`.
//! - [`evaluate_advisory`] returns an [`AdvisoryVerdict`], whose outcome type has no pass.

use crate::date::Date;
use crate::eval::evaluate;
use crate::exit::Outcome;
use crate::facts::{Disagreement, PatchFacts, Unverifiable, compare, derived_signals};
use crate::policy::{AdmittedProducer, Policy};
use crate::rank::RankDoc;
use crate::seal::Sha256Hex;
use crate::verdict::{Note, NoteCode, Provenance, RankedBy, Seals, Verdict};
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
#[non_exhaustive]
pub struct Ranking<P> {
    doc: RankDoc,
    producer_sha256: Option<Sha256Hex>,
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
    /// A ranking forge-warden produced by running the ranker `producer`, which a policy's
    /// `[producer] sha256` allowlist admitted ([`Policy::admit_producer`]). The binary calls this
    /// only from `doors.rs`, right after the run.
    #[must_use]
    pub fn attest(doc: RankDoc, producer: AdmittedProducer) -> Self {
        Self {
            doc,
            producer_sha256: Some(producer.into_sha256()),
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
    pub const fn producer_sha256(&self) -> Option<&Sha256Hex> {
        self.producer_sha256.as_ref()
    }
}

/// A ranking checked against Git's facts about the patch. It borrows the exact ranking it checked,
/// and the judges take only this, so a proof cannot be paired with another ranking (Zen B1).
#[derive(Debug)]
#[non_exhaustive]
pub struct Checked<'r, P> {
    ranking: &'r Ranking<P>,
    facts: Result<PatchFacts, Unverifiable>,
    disagreements: Vec<Disagreement>,
}

/// Compare `ranking` with Git's view of the patch (or the reason Git's view is unavailable).
#[must_use]
pub fn check<P>(ranking: &Ranking<P>, facts: Result<PatchFacts, Unverifiable>) -> Checked<'_, P> {
    let disagreements = match &facts {
        Ok(facts) => compare(&ranking.doc, facts),
        Err(_) => Vec::new(),
    };
    Checked {
        ranking,
        facts,
        disagreements,
    }
}

impl<P> Checked<'_, P> {
    /// The ranking this proof is about.
    #[must_use]
    pub const fn ranking(&self) -> &Ranking<P> {
        self.ranking
    }

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

    /// The checked ranking with each file's signals widened by Git's change-kind facts (T2: Git
    /// only adds). Infallible (Zen B4): `RankDoc::map_files` and `RankedFile::with_added_signals`
    /// keep every constructor invariant by construction, so there is no fallback that could judge
    /// an unwidened ranking.
    fn effective(&self) -> RankDoc {
        let rank = &self.ranking.doc;
        let Ok(facts) = &self.facts else {
            return rank.clone();
        };
        let derived: BTreeMap<&crate::path::RepoPath, _> = facts
            .files()
            .map(|change| (change.path(), derived_signals(change)))
            .collect();
        rank.map_files(|file| {
            file.with_added_signals(derived.get(file.path()).into_iter().flatten())
        })
    }
}

/// A verdict on a supplied ranking. Its outcome type has no pass: a supplied ranking can at best
/// hold a change for a person.
#[derive(Debug)]
#[non_exhaustive]
pub struct AdvisoryVerdict {
    verdict: Verdict,
}

/// The outcomes a supplied ranking can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// Exhaustive on purpose: a supplied ranking can only hold or fail (ADR 0005 §2.7); a pass variant must never appear. Adding a variant must break every
// consumer's match (Q-TYPES-1; recorded in the public-API snapshot).
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
        match self.verdict.outcome() {
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
// Exhaustive on purpose: a supplied copy is absent, identical or different. Adding a variant must break every
// consumer's match (Q-TYPES-1; recorded in the public-API snapshot).
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

/// Judge the ranking a [`Checked`] proof is about, produced by forge-warden itself. Only this can
/// pass. `provenance` and `ranked_by` in `seals` are set from that ranking, whatever the caller put
/// there.
#[must_use]
pub fn evaluate_authority(
    policy: &Policy,
    checked: &Checked<'_, Produced>,
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
    let seals = provenance(seals, checked.ranking, Provenance::Produced);
    judge(policy, checked, extra, seals, today)
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

/// Judge the supplied ranking a [`Checked`] proof is about. It is held at least, never passed.
/// `provenance` and `ranked_by` in `seals` are set from that ranking.
#[must_use]
pub fn evaluate_advisory(
    policy: &Policy,
    checked: &Checked<'_, Supplied>,
    seals: Seals,
    today: Date,
) -> AdvisoryVerdict {
    let unverified = advisory_note(&seals);
    let seals = provenance(seals, checked.ranking, Provenance::Supplied);
    // `judge` re-derives the outcome through the one fold: the note makes it at least a hold.
    let verdict = judge(policy, checked, vec![unverified], seals, today);
    AdvisoryVerdict { verdict }
}

fn advisory_note(seals: &Seals) -> Note {
    let mut detail =
        "the ranking was supplied, not produced by forge-warden; it can inform, never approve"
            .to_owned();
    if seals
        .git
        .as_ref()
        .is_some_and(|git| git.admission() == crate::verdict::Admission::Unpinned)
    {
        detail.push_str("; Git is unpinned: no owner allowlist enrolled; its recorded executable digest is not approved");
    }
    Note {
        code: NoteCode::RankUnverified,
        path: None,
        detail,
    }
}

fn judge<P>(
    policy: &Policy,
    checked: &Checked<'_, P>,
    mut extra: Vec<Note>,
    seals: Seals,
    today: Date,
) -> Verdict {
    extra.extend(checked.notes());
    evaluate(policy, &checked.effective(), seals, today).with_notes(extra)
}
