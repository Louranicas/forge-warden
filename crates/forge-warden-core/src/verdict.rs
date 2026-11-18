//! The verdict document, `forge-warden.verdict.v3`.
//!
//! Field order is the serialization order, and every list is in a defined order (rules in policy
//! order, violations in deep-diff-forge's risk order), so the same inputs always serialize to the
//! same bytes. `forge-warden schema verdict` prints the JSON Schema, which is committed at
//! `schemas/verdict.v3.schema.json` and checked against the code by a test.
//!
//! v1 (ADR 0005 §5) seals where the ranking came from (`provenance`, `ranked_by.sha256`), what Git
//! saw in the patch (`patch_facts_sha256`, `git`), and any `claims` about the change; v3 (ADR 0012)
//! adds verified claims with the merge base when bound mode made the diff.

use crate::date::Date;
use crate::defaults::NamedDefault as _;
use crate::exit::Outcome;
use crate::nonempty::NonEmpty;
use crate::seal::Sha256Hex;
use alloc::collections::BTreeSet;
use alloc::{borrow::ToOwned, format, string::String, string::ToString, vec::Vec};
use serde::Serialize;

/// The schema name every verdict carries.
pub const VERDICT_SCHEMA: &str = "forge-warden.verdict.v3";

/// A sealed, deterministic verdict. It cannot be cloned out of a shared judged view.
///
/// Q-TYPES-2 (Q1-03): every field is private. A verdict is assembled once, by
/// `Verdict::assemble` (crate-private), which derives the outcome (one strictest-wins fold over rules, notes and
/// expired waivers), the exit status (from the outcome) and the summary (from the rules), so no
/// holder can make them disagree (`verdict = pass` with `exit = 8` is unrepresentable). Serialized
/// member names and order are unchanged.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Verdict {
    schema: &'static str,
    tool: Tool,
    evaluated_on: Date,
    seals: Seals,
    #[expect(
        clippy::struct_field_names,
        reason = "the document member is named `verdict`; renaming it would break the schema"
    )]
    verdict: Outcome,
    exit: ExitInfo,
    summary: Summary,
    rules: Vec<RuleResult>,
    expired_waivers: Vec<WaiverNote>,
    unused_waivers: Vec<WaiverNote>,
    notes: Vec<Note>,
    /// Ledger Q80: the unused waivers that matched what an earlier waiver already covered. Only the
    /// text report says so; the verdict document is unchanged (they are in `unused_waivers`).
    #[serde(skip)]
    superseded_waivers: BTreeSet<usize>,
}

/// Tool identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Tool {
    /// `forge-warden`.
    pub name: &'static str,
    /// The crate version.
    pub version: &'static str,
}

/// The seals that bind a verdict to its inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Seals {
    /// SHA-256 of the policy bytes.
    pub policy_sha256: Sha256Hex,
    /// SHA-256 of the ranking document bytes (the produced ranking, in authority mode).
    pub rank_sha256: Sha256Hex,
    /// SHA-256 of the patch. forge-warden hashed the patch itself and found the ranking's own seal
    /// equal to it.
    pub patch_sha256: Sha256Hex,
    /// SHA-256 of Git's facts about the patch: `git apply --numstat -z` output, one 0x00 byte, then
    /// `git apply --summary` output. `null` when Git refused the patch.
    pub patch_facts_sha256: Option<Sha256Hex>,
    /// Whether forge-warden produced the ranking itself (`produced`, authority mode) or was given it
    /// (`supplied`, advisory mode; such a verdict is never a pass).
    pub provenance: Provenance,
    /// The ranker.
    pub ranked_by: RankedBy,
    /// The Git that read the patch, when one did.
    pub git: Option<GitOracle>,
    /// What the caller said about the change (`--base`, `--head`). Present only when given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claims: Option<Claims>,
}

impl Seals {
    /// Seals for a supplied ranking with no Git facts and no claims; the `with_*` methods add the
    /// rest. The authority evaluators set `provenance` and `ranked_by` from the ranking itself.
    #[must_use]
    pub const fn new(
        policy_sha256: Sha256Hex,
        rank_sha256: Sha256Hex,
        patch_sha256: Sha256Hex,
        ranked_by_version: String,
    ) -> Self {
        Self {
            policy_sha256,
            rank_sha256,
            patch_sha256,
            patch_facts_sha256: None,
            provenance: Provenance::Supplied,
            ranked_by: RankedBy {
                version: ranked_by_version,
                sha256: None,
            },
            git: None,
            claims: None,
        }
    }

    /// These seals with Git's facts digest and the Git that produced them.
    #[must_use]
    pub fn with_git(mut self, facts_sha256: Option<Sha256Hex>, git: GitOracle) -> Self {
        self.patch_facts_sha256 = facts_sha256;
        self.git = Some(git);
        self
    }

    /// These seals with the caller's claims about the change.
    #[must_use]
    pub fn with_claims(mut self, claims: Claims) -> Self {
        self.claims = Some(claims);
        self
    }
}

/// Where a ranking came from (ADR 0005 §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Provenance {
    /// forge-warden ran the pinned ranker on the patch it holds.
    Produced,
    /// The ranking arrived as a file.
    Supplied,
}

/// The ranker that wrote the ranking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct RankedBy {
    /// The version the ranking reports.
    pub version: String,
    /// SHA-256 of the ranker binary forge-warden ran; `null` for a supplied ranking.
    pub sha256: Option<Sha256Hex>,
}

/// The Git whose facts the verdict used. Recorded for audit, not attested (ADR 0005 §2.4).
///
/// Q-TYPES-2 (B2 of the GIT-PIN boundary review): every field is private. The only constructors
/// are [`GitOracle::approved`] and [`GitOracle::unpinned`], so `admission` and `allowlist_sha256`
/// cannot disagree, and a recorded identity cannot be relabelled after it is built. The CLI builds
/// one only at its Git admission door, from the admitted descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct GitOracle {
    /// The absolute path that was run.
    path: String,
    /// What `git --version` printed, trimmed.
    version: String,
    /// SHA-256 of the executable read through the descriptor used for every Git invocation.
    sha256: Sha256Hex,
    /// `approved` when the executable digest occurs in the owner policy; `unpinned` otherwise.
    admission: Admission,
    /// SHA-256 of the exact admitted owner policy bytes; absent only for unpinned advisory Git.
    allowlist_sha256: Option<Sha256Hex>,
}

/// How the recorded Git was admitted. Serialized as `approved` or `unpinned`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Admission {
    /// The executable digest occurs in the owner's Git allowlist.
    Approved,
    /// No owner allowlist was enrolled (advisory output only).
    Unpinned,
}

/// What the caller said about the change (`--base`, `--head`), and whether forge-warden bound the
/// patch to it (ADR 0012). Claims never change an outcome.
///
/// Two forms. Asserted claims ([`Claims::new`]) serialize exactly as in verdict v2:
/// `{base?, head?, verified: false}`. Bound claims ([`Claims::bound`]) serialize as
/// `{base, head, merge_base, verified: true}` and can only be made from a [`BoundClaims`], whose
/// one constructor is the byte comparison [`BoundClaims::from_comparison`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Claims {
    form: ClaimsForm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ClaimsForm {
    Asserted {
        base: Option<String>,
        head: Option<String>,
    },
    Bound(BoundClaims),
}

/// The wire shape of [`Claims`]: member order is the serialization order (ADR 0004).
#[derive(Serialize)]
struct ClaimsWire<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    base: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    head: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    merge_base: Option<&'a str>,
    verified: bool,
}

impl Serialize for Claims {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ClaimsWire {
            base: self.base(),
            head: self.head(),
            merge_base: self.merge_base(),
            verified: self.verified(),
        }
        .serialize(serializer)
    }
}

/// Three full object ids of one length (40 or 64 lowercase hex digits): bound mode's base, head
/// and the merge base its diff was made from.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BoundIds {
    base: String,
    head: String,
    merge_base: String,
}

/// Why three ids are not [`BoundIds`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum IdError {
    /// An id is not 40 or 64 characters long.
    #[error("{0} is not a full object id (40 or 64 lowercase hex digits)")]
    Length(&'static str),
    /// An id has a byte that is not a lowercase hex digit.
    #[error("{0} is not lowercase hex")]
    NotLowercaseHex(&'static str),
    /// The ids are full ids of different lengths (SHA-1 and SHA-256 mixed).
    #[error("base, head and merge base are not of one length")]
    MixedLengths,
}

/// A verified binding of a patch to a change: the supplied patch was byte-equal to the diff made
/// from `merge_base` to `head` (ADR 0012 D4). Its only constructor is
/// [`BoundClaims::from_comparison`].
///
/// What this type guarantees, and what it does not: a value exists only if a byte comparison of two
/// inputs passed and the ids are well formed. It cannot know where the second input came from; the
/// forge-warden binary passes the diff its door made from a verified copy of the repository's
/// objects, and a fence in the CLI's tests holds that it is the only call. A library caller can pass
/// any bytes, as it could already print any JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BoundClaims {
    ids: BoundIds,
}

/// The supplied patch is not the diff that was made: where they first differ, and both lengths.
/// No patch content is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct PatchMismatch {
    /// Offset of the first byte that differs; the shorter length when one is a prefix of the other.
    pub first_difference: usize,
    /// Length of the supplied patch.
    pub supplied_len: usize,
    /// Length of the diff that was made.
    pub made_len: usize,
}

impl RankedBy {
    /// The ranker that wrote a ranking: its reported `version`, and its SHA-256 if forge-warden ran it.
    #[must_use]
    pub const fn new(version: String, sha256: Option<Sha256Hex>) -> Self {
        Self { version, sha256 }
    }
}

impl GitOracle {
    /// Git at the absolute `path`, whose executable `sha256` occurs in the owner policy whose exact
    /// bytes hash to `allowlist_sha256`, and which printed `version` for `git --version`.
    #[must_use]
    pub const fn approved(
        path: String,
        version: String,
        sha256: Sha256Hex,
        allowlist_sha256: Sha256Hex,
    ) -> Self {
        Self {
            path,
            version,
            sha256,
            admission: Admission::Approved,
            allowlist_sha256: Some(allowlist_sha256),
        }
    }

    /// Git at the absolute `path`, measured as `sha256` but admitted with no owner policy enrolled.
    #[must_use]
    pub const fn unpinned(path: String, version: String, sha256: Sha256Hex) -> Self {
        Self {
            path,
            version,
            sha256,
            admission: Admission::Unpinned,
            allowlist_sha256: None,
        }
    }

    /// The absolute path that was run.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// What `git --version` printed, trimmed.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// SHA-256 of the executable read through the admitted descriptor.
    #[must_use]
    pub const fn sha256(&self) -> &Sha256Hex {
        &self.sha256
    }

    /// How this Git was admitted.
    #[must_use]
    pub const fn admission(&self) -> Admission {
        self.admission
    }

    /// SHA-256 of the exact owner policy bytes; `None` exactly when [`Admission::Unpinned`].
    #[must_use]
    pub const fn allowlist_sha256(&self) -> Option<&Sha256Hex> {
        self.allowlist_sha256.as_ref()
    }
}

impl Claims {
    /// What the caller claimed about the change, unverified (`verified` is `false`, no merge base).
    #[must_use]
    pub const fn new(base: Option<String>, head: Option<String>) -> Self {
        Self {
            form: ClaimsForm::Asserted { base, head },
        }
    }

    /// Verified claims: the binding a byte comparison produced.
    #[must_use]
    pub const fn bound(claims: BoundClaims) -> Self {
        Self {
            form: ClaimsForm::Bound(claims),
        }
    }

    /// The claimed base commit.
    #[must_use]
    pub fn base(&self) -> Option<&str> {
        match &self.form {
            ClaimsForm::Asserted { base, .. } => base.as_deref(),
            ClaimsForm::Bound(bound) => Some(bound.base()),
        }
    }

    /// The claimed head commit.
    #[must_use]
    pub fn head(&self) -> Option<&str> {
        match &self.form {
            ClaimsForm::Asserted { head, .. } => head.as_deref(),
            ClaimsForm::Bound(bound) => Some(bound.head()),
        }
    }

    /// The merge base the bound diff was made from; `Some` exactly when [`Claims::verified`].
    #[must_use]
    pub fn merge_base(&self) -> Option<&str> {
        match &self.form {
            ClaimsForm::Asserted { .. } => None,
            ClaimsForm::Bound(bound) => Some(bound.merge_base()),
        }
    }

    /// Whether forge-warden bound the patch to these claims.
    #[must_use]
    pub const fn verified(&self) -> bool {
        matches!(self.form, ClaimsForm::Bound(_))
    }
}

impl BoundIds {
    /// `base`, `head` and `merge_base`, each 40 or 64 lowercase hex digits, all of one length.
    ///
    /// # Errors
    /// [`IdError`] names the first id that is not a full object id, or says the lengths differ.
    pub fn new(base: &str, head: &str, merge_base: &str) -> Result<Self, IdError> {
        for (name, id) in [("base", base), ("head", head), ("merge base", merge_base)] {
            if !matches!(id.len(), 40 | 64) {
                return Err(IdError::Length(name));
            }
            if !id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(IdError::NotLowercaseHex(name));
            }
        }
        if base.len() != head.len() || head.len() != merge_base.len() {
            return Err(IdError::MixedLengths);
        }
        Ok(Self {
            base: base.to_owned(),
            head: head.to_owned(),
            merge_base: merge_base.to_owned(),
        })
    }
}

impl BoundClaims {
    /// Binds `ids` to the change when `supplied` (the patch being judged) is byte-equal to `made`
    /// (the diff made from `ids`' merge base to head).
    ///
    /// # Errors
    /// [`PatchMismatch`] with the first differing offset and both lengths.
    pub fn from_comparison(
        ids: BoundIds,
        supplied: &[u8],
        made: &[u8],
    ) -> Result<Self, PatchMismatch> {
        if supplied == made {
            return Ok(Self { ids });
        }
        // Equal up to the shorter length: the first difference is where the shorter one ends.
        let first_difference = match supplied.iter().zip(made).position(|(a, b)| a != b) {
            Some(at) => at,
            None => supplied.len().min(made.len()),
        };
        Err(PatchMismatch {
            first_difference,
            supplied_len: supplied.len(),
            made_len: made.len(),
        })
    }

    /// The bound base commit.
    #[must_use]
    pub fn base(&self) -> &str {
        &self.ids.base
    }

    /// The bound head commit.
    #[must_use]
    pub fn head(&self) -> &str {
        &self.ids.head
    }

    /// The merge base the diff was made from.
    #[must_use]
    pub fn merge_base(&self) -> &str {
        &self.ids.merge_base
    }
}

/// The exit status and next action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct ExitInfo {
    /// Process exit status.
    pub code: u8,
    /// Registry name.
    pub name: &'static str,
    /// Next action, in deep-diff-forge's vocabulary.
    pub action: &'static str,
}

/// Counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Summary {
    /// Files in the ranking.
    pub files: usize,
    /// Rules evaluated.
    pub rules: usize,
    /// Rules with no violation.
    pub passed: usize,
    /// Rules whose every violation an active waiver covered.
    pub waived: usize,
    /// Rules that held the change.
    pub held: usize,
    /// Rules that failed the change.
    pub failed: usize,
}

/// How one rule came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
// Exhaustive on purpose: a rule's outcome lattice (ADR 0003). Adding a variant must break every
// consumer's match (Q-TYPES-1; recorded in the public-API snapshot).
pub enum RuleOutcome {
    /// No violation.
    Pass,
    /// Violations, each covered by an active waiver.
    Waived,
    /// Violated, severity `hold`.
    Hold,
    /// Violated, severity `fail`.
    Fail,
}

impl RuleOutcome {
    /// The overall outcome this rule contributes.
    #[must_use]
    pub const fn overall(self) -> Outcome {
        match self {
            Self::Pass | Self::Waived => Outcome::Pass,
            Self::Hold => Outcome::Hold,
            Self::Fail => Outcome::Fail,
        }
    }
}

/// One rule's result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct RuleResult {
    /// The rule id.
    pub id: String,
    /// The rule kind.
    pub kind: &'static str,
    /// The rule severity.
    pub severity: &'static str,
    /// How it came out.
    pub outcome: RuleOutcome,
    /// The rule's rationale, shown to whoever it stops.
    pub why: String,
    /// What violated it, in risk order. Waived violations stay listed, marked `waived_by`.
    pub violations: Vec<Violation>,
}

/// One violation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Violation {
    /// The file, when the violation is about one file (budgets over a whole change have none).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// What is wrong, in one sentence.
    pub message: String,
    /// The signal involved, for `forbid_signal`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<String>,
    /// Where deep-diff-forge found the evidence, so a person or agent can check it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Citation>,
    /// deep-diff-forge found more evidence for this file than it listed (it lists at most 32), so
    /// `evidence` is incomplete. Present only when true.
    #[serde(skip_serializing_if = "core::ops::Not::not")]
    pub evidence_truncated: bool,
    /// The index of the `[[waiver]]` that covers it, if one does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waived_by: Option<usize>,
}

/// A pointer into the patch: a hunk address, a side and a line.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[non_exhaustive]
pub struct Citation {
    /// The hunk's content address (`deep-diff-forge --hunks --address <A>` shows it).
    pub address: Sha256Hex,
    /// `new` or `old`.
    pub side: String,
    /// The line on that side.
    pub line: u64,
}

/// A waiver, reported because it expired or matched nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct WaiverNote {
    /// Its index in the policy.
    pub index: usize,
    /// The rule it waives.
    pub rule: String,
    /// Who owns it.
    pub owner: String,
    /// Its last day.
    pub expires: Date,
}

/// Something the policy cannot judge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Note {
    /// Why the verdict holds; see [`NoteCode`].
    pub code: NoteCode,
    /// The file involved, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// What and why.
    pub detail: String,
}

/// Declares [`NoteCode`] from one list, so its variants, [`NoteCode::name`] and [`NoteCode::ALL`]
/// cannot disagree: a code exists in all three or in none.
macro_rules! note_codes {
    ($($doc:literal $variant:ident => $name:literal,)*) => {
        /// Why a verdict holds when the policy alone cannot decide. Every note holds the verdict at
        /// least (ADR 0003); `docs/POLICY_REFERENCE.md` gives each code's trigger and outcome, and
        /// the schema's enum lists exactly these names (tests check both).
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        #[non_exhaustive]
        pub enum NoteCode {
            $(#[doc = $doc] $variant,)*
        }

        impl NoteCode {
            /// Every code, in declaration order.
            pub const ALL: &'static [Self] = &[$(Self::$variant,)*];

            /// The name the verdict writes.
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name,)*
                }
            }
        }
    };
}

note_codes! {
    "The ranking lists no file: there is nothing to judge."
    EmptyRanking => "empty_ranking",
    "A signal this forge-warden does not know."
    UnknownSignal => "unknown_signal",
    "A status this forge-warden does not know (including deep-diff-forge's `unknown`)."
    UnknownStatus => "unknown_status",
    "A rename or copy: rank.v0 does not report the source path."
    SourcePathUnknown => "source_path_unknown",
    "One path is listed more than once (deep-diff-forge does this for a type change)."
    DuplicatePaths => "duplicate_paths",
    "Git sees a file in the patch that the ranking does not list."
    RankIncomplete => "rank_incomplete",
    "The ranking lists a file the patch does not change."
    RankExtraneous => "rank_extraneous",
    "The ranking's status or change-kind signals contradict Git."
    StatusMismatch => "status_mismatch",
    "Git refused the patch, or its facts cannot be read; coverage cannot be verified."
    PatchUnverifiable => "patch_unverifiable",
    "The ranking was supplied, not produced (advisory mode)."
    RankUnverified => "rank_unverified",
    "The ranking supplied with `--rank` differs from the one forge-warden produced."
    RankMismatch => "rank_mismatch",
    "The policy requires verified claims (`[claims] require_verified`) and bound mode did not verify them."
    ClaimsUnverified => "claims_unverified",
}

impl Serialize for NoteCode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

impl PartialEq<&str> for NoteCode {
    fn eq(&self, other: &&str) -> bool {
        self.name() == *other
    }
}

/// The strictest outcome of a judgement: every rule's own outcome, a hold for every note and a fail
/// for every expired waiver. The single place the verdict lattice is folded (Q1-03); there is no
/// empty case, because a judgement always has at least one rule.
pub(crate) fn strictest(
    rules: &NonEmpty<RuleResult>,
    notes: &[Note],
    expired: &[WaiverNote],
) -> Outcome {
    rules
        .iter()
        .map(|r| r.outcome.overall())
        .chain(notes.iter().map(|_| Outcome::Hold))
        .chain(expired.iter().map(|_| Outcome::Fail))
        .fold(rules.first().outcome.overall(), Outcome::max)
}

impl From<Outcome> for ExitInfo {
    fn from(outcome: Outcome) -> Self {
        let entry = outcome.exit_entry();
        Self {
            code: entry.code,
            name: entry.name,
            action: entry.action,
        }
    }
}

impl Verdict {
    /// The only way to make a verdict: the outcome, exit status and summary are derived here from
    /// the parts, never supplied.
    pub(crate) fn assemble(
        evaluated_on: Date,
        seals: Seals,
        files: usize,
        rules: NonEmpty<RuleResult>,
        expired_waivers: Vec<WaiverNote>,
        unused_waivers: Vec<WaiverNote>,
        notes: Vec<Note>,
    ) -> Self {
        let verdict = strictest(&rules, &notes, &expired_waivers);
        let rules = rules.into_vec();
        let count = |outcome: RuleOutcome| rules.iter().filter(|r| r.outcome == outcome).count();
        let summary = Summary {
            files,
            rules: rules.len(),
            passed: count(RuleOutcome::Pass),
            waived: count(RuleOutcome::Waived),
            held: count(RuleOutcome::Hold),
            failed: count(RuleOutcome::Fail),
        };
        Self {
            schema: VERDICT_SCHEMA,
            tool: Tool {
                name: crate::TOOL_NAME,
                version: crate::TOOL_VERSION,
            },
            evaluated_on,
            seals,
            verdict,
            exit: ExitInfo::from(verdict),
            summary,
            rules,
            expired_waivers,
            unused_waivers,
            notes,
            superseded_waivers: BTreeSet::new(),
        }
    }

    /// This verdict knowing which unused waivers were superseded by an earlier one (Q80).
    #[must_use]
    pub(crate) fn with_superseded_waivers(mut self, indices: BTreeSet<usize>) -> Self {
        self.superseded_waivers = indices;
        self
    }

    /// Whether the waiver at `index` matched only what an earlier waiver already covered (Q80).
    pub(crate) fn waiver_was_superseded(&self, index: usize) -> bool {
        self.superseded_waivers.contains(&index)
    }

    /// This verdict with `extra` notes appended, re-derived through the same fold: a note can only
    /// hold, never loosen (the outcome and exit follow from the parts, not from a patch).
    #[must_use]
    pub(crate) fn with_notes(mut self, extra: Vec<Note>) -> Self {
        if extra.is_empty() {
            return self;
        }
        self.notes.extend(extra);
        let verdict = self
            .notes
            .iter()
            .map(|_| Outcome::Hold)
            .fold(self.verdict, Outcome::max);
        self.verdict = verdict;
        self.exit = ExitInfo::from(verdict);
        self
    }

    /// This verdict with built-in rule results appended (`crate::builtin`), re-derived through the
    /// same parts: the summary is counted from the rules again, and the outcome is the strictest of
    /// the verdict's and each added rule's. A built-in rule only holds, so it can never loosen.
    #[must_use]
    pub(crate) fn with_builtin_rules(mut self, extra: Vec<RuleResult>) -> Self {
        if extra.is_empty() {
            return self;
        }
        let verdict = extra
            .iter()
            .map(|rule| rule.outcome.overall())
            .fold(self.verdict, Outcome::max);
        self.rules.extend(extra);
        let count =
            |outcome: RuleOutcome| self.rules.iter().filter(|r| r.outcome == outcome).count();
        let summary = Summary {
            files: self.summary.files,
            rules: self.rules.len(),
            passed: count(RuleOutcome::Pass),
            waived: count(RuleOutcome::Waived),
            held: count(RuleOutcome::Hold),
            failed: count(RuleOutcome::Fail),
        };
        self.summary = summary;
        self.verdict = verdict;
        self.exit = ExitInfo::from(verdict);
        self
    }

    /// This verdict with `extra` notes and then `because` appended. `because` is not optional, so
    /// the result always carries a note and is at least a hold (Q25: the refusal arm of
    /// `authority::judge` has no other way to finish).
    #[must_use]
    pub(crate) fn held_by(self, mut extra: Vec<Note>, because: Note) -> Self {
        extra.push(because);
        self.with_notes(extra)
    }

    /// Always [`VERDICT_SCHEMA`].
    #[must_use]
    pub const fn schema(&self) -> &'static str {
        self.schema
    }

    /// The tool that wrote it.
    #[must_use]
    pub const fn tool(&self) -> &Tool {
        &self.tool
    }

    /// The date waivers were judged against (`YYYY-MM-DD`).
    #[must_use]
    pub const fn evaluated_on(&self) -> Date {
        self.evaluated_on
    }

    /// Exactly which bytes this verdict is about.
    #[must_use]
    pub const fn seals(&self) -> &Seals {
        &self.seals
    }

    /// The overall outcome: the strictest of every rule, note and waiver.
    #[must_use]
    pub const fn outcome(&self) -> Outcome {
        self.verdict
    }

    /// The exit status this verdict produces, and what to do next (derived from the outcome).
    #[must_use]
    pub const fn exit(&self) -> &ExitInfo {
        &self.exit
    }

    /// Counts, for dashboards and one-line summaries (derived from the rules).
    #[must_use]
    pub const fn summary(&self) -> &Summary {
        &self.summary
    }

    /// One result per rule, in policy order.
    #[must_use]
    pub fn rules(&self) -> &[RuleResult] {
        &self.rules
    }

    /// Waivers past their date. Each one fails the verdict until it is renewed or removed.
    #[must_use]
    pub fn expired_waivers(&self) -> &[WaiverNote] {
        &self.expired_waivers
    }

    /// Active waivers that matched no violation: candidates for deletion.
    #[must_use]
    pub fn unused_waivers(&self) -> &[WaiverNote] {
        &self.unused_waivers
    }

    /// Things the policy cannot judge. Each one holds the verdict.
    #[must_use]
    pub fn notes(&self) -> &[Note] {
        &self.notes
    }

    /// The verdict as pretty JSON with a trailing newline. Serializing plain data to a `String`
    /// cannot fail, so this does not return a `Result`.
    #[must_use]
    pub fn to_json(&self) -> String {
        // The fallback document has no `verdict` member, so no consumer can read it as a pass.
        let mut text = serde_json::to_string_pretty(self).or_fail_closed_with(|e| {
            // Unreachable in practice: every field is a string, number, bool or list of them.
            format!(
                "{{\"schema\":\"{VERDICT_SCHEMA}\",\"serialization_error\":{:?}}}",
                e.to_string()
            )
        });
        text.push('\n');
        text
    }
}

#[cfg(test)]
mod tests {
    use super::NoteCode;

    #[test]
    fn a_note_code_equals_its_own_name_and_no_other() {
        for code in NoteCode::ALL {
            for other in NoteCode::ALL {
                assert_eq!(
                    *code == other.name(),
                    code == other,
                    "{code:?} vs {other:?}"
                );
            }
            assert!(*code != "", "{code:?}");
        }
    }
}
