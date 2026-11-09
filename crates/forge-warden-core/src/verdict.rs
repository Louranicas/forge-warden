//! The verdict document, `forge-warden.verdict.v2`.
//!
//! Field order is the serialization order, and every list is in a defined order (rules in policy
//! order, violations in deep-diff-forge's risk order), so the same inputs always serialize to the
//! same bytes. `forge-warden schema verdict` prints the JSON Schema, which is committed at
//! `schemas/verdict.v2.schema.json` and checked against the code by a test.
//!
//! v1 (ADR 0005 §5) seals where the ranking came from (`provenance`, `ranked_by.sha256`), what Git
//! saw in the patch (`patch_facts_sha256`, `git`), and any unverified `claims` about the change.

use crate::defaults::NamedDefault as _;
use crate::exit::Outcome;
use serde::Serialize;

/// The schema name every verdict carries.
pub const VERDICT_SCHEMA: &str = "forge-warden.verdict.v2";

/// A sealed, deterministic verdict. It cannot be cloned out of a shared judged view.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Verdict {
    /// Always [`VERDICT_SCHEMA`].
    pub schema: &'static str,
    /// The tool that wrote it.
    pub tool: Tool,
    /// The date waivers were judged against (`YYYY-MM-DD`).
    pub evaluated_on: String,
    /// Exactly which bytes this verdict is about.
    pub seals: Seals,
    /// The overall outcome: the strictest of every rule, note and waiver.
    #[expect(
        clippy::struct_field_names,
        reason = "the document member is named `verdict`; renaming it would break the schema"
    )]
    pub verdict: Outcome,
    /// The exit status this verdict produces, and what to do next.
    pub exit: ExitInfo,
    /// Counts, for dashboards and one-line summaries.
    pub summary: Summary,
    /// One result per rule, in policy order.
    pub rules: Vec<RuleResult>,
    /// Waivers past their date. Each one fails the verdict until it is renewed or removed.
    pub expired_waivers: Vec<WaiverNote>,
    /// Active waivers that matched no violation: candidates for deletion.
    pub unused_waivers: Vec<WaiverNote>,
    /// Things the policy cannot judge. Each one holds the verdict.
    pub notes: Vec<Note>,
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
    pub policy_sha256: String,
    /// SHA-256 of the ranking document bytes (the produced ranking, in authority mode).
    pub rank_sha256: String,
    /// SHA-256 of the patch. forge-warden hashed the patch itself and found the ranking's own seal
    /// equal to it.
    pub patch_sha256: String,
    /// SHA-256 of Git's facts about the patch: `git apply --numstat -z` output, one 0x00 byte, then
    /// `git apply --summary` output. `null` when Git refused the patch.
    pub patch_facts_sha256: Option<String>,
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
        policy_sha256: String,
        rank_sha256: String,
        patch_sha256: String,
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
    pub fn with_git(mut self, facts_sha256: Option<String>, git: GitOracle) -> Self {
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
    pub sha256: Option<String>,
}

/// The Git whose facts the verdict used. Recorded for audit, not attested (ADR 0005 §2.4). Build
/// one with [`GitOracle::new`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct GitOracle {
    /// The absolute path that was run.
    pub path: String,
    /// What `git --version` printed, trimmed.
    pub version: String,
    /// SHA-256 of the executable read through the descriptor used for every Git invocation.
    pub sha256: String,
    /// `approved` when the executable digest occurs in the owner policy; `unpinned` otherwise.
    pub admission: &'static str,
    /// SHA-256 of the exact admitted owner policy bytes; absent only for unpinned advisory Git.
    pub allowlist_sha256: Option<String>,
}

/// What the caller claimed about the change. Recorded, never checked, never used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Claims {
    /// The claimed base commit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// The claimed head commit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    /// Always `false`: forge-warden did not verify the claims.
    pub verified: Unverified,
}

impl RankedBy {
    /// The ranker that wrote a ranking: its reported `version`, and its SHA-256 if forge-warden ran it.
    #[must_use]
    pub const fn new(version: String, sha256: Option<String>) -> Self {
        Self { version, sha256 }
    }
}

impl GitOracle {
    /// The Git at the absolute `path` that printed `version` for `git --version`.
    #[must_use]
    pub fn new(
        path: String,
        version: String,
        sha256: String,
        allowlist_sha256: Option<String>,
    ) -> Self {
        let admission = if allowlist_sha256.is_some() {
            "approved"
        } else {
            "unpinned"
        };
        Self {
            path,
            version,
            sha256,
            admission,
            allowlist_sha256,
        }
    }
}

impl Claims {
    /// What the caller claimed about the change. Never verified: `verified` is always `false`,
    /// and no caller can say otherwise (the field's type has no other value and no public constructor).
    #[must_use]
    pub const fn new(base: Option<String>, head: Option<String>) -> Self {
        Self {
            base,
            head,
            verified: Unverified,
        }
    }
}

/// A value that serializes as `false` and has no other state: claims are never verified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Unverified;

impl Serialize for Unverified {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bool(false)
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
    #[serde(skip_serializing_if = "std::ops::Not::not")]
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
    pub address: String,
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
    pub expires: String,
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

impl Verdict {
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
