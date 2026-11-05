//! The verdict document, `forge-warden.verdict.v0`.
//!
//! Field order is the serialization order, and every list is in a defined order (rules in policy
//! order, violations in deep-diff-forge's risk order), so the same inputs always serialize to the
//! same bytes. `forge-warden schema verdict` prints the JSON Schema, which is committed at
//! `schemas/verdict.v0.schema.json` and checked against the code by a test.

use crate::exit::Outcome;
use serde::Serialize;

/// The schema name every verdict carries.
pub const VERDICT_SCHEMA: &str = "forge-warden.verdict.v0";

/// A sealed, deterministic verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
pub struct Tool {
    /// `forge-warden`.
    pub name: &'static str,
    /// The crate version.
    pub version: &'static str,
}

/// The seals that bind a verdict to its inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Seals {
    /// SHA-256 of the policy bytes.
    pub policy_sha256: String,
    /// SHA-256 of the ranking document bytes.
    pub rank_sha256: String,
    /// deep-diff-forge's seal: SHA-256 of the patch the ranking describes.
    pub patch_sha256: String,
    /// Whether forge-warden hashed the patch itself (`--patch`) and found it equal to
    /// `patch_sha256`. `false` means the ranking's claim was taken as given.
    pub patch_verified: bool,
    /// The deep-diff-forge version that ranked the patch.
    pub ranked_by: String,
}

/// The exit status and next action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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

    /// Lowercase name.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Waived => "waived",
            Self::Hold => "hold",
            Self::Fail => "fail",
        }
    }
}

/// One rule's result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
    /// The index of the `[[waiver]]` that covers it, if one does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waived_by: Option<usize>,
}

/// A pointer into the patch: a hunk address, a side and a line.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
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
pub struct Note {
    /// `empty_ranking`, `unknown_signal` or `unknown_status`.
    pub code: &'static str,
    /// The file involved, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// What and why.
    pub detail: String,
}

impl Verdict {
    /// The verdict as pretty JSON with a trailing newline. Serializing plain data to a `String`
    /// cannot fail, so this does not return a `Result`.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).unwrap_or_else(|e| {
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
