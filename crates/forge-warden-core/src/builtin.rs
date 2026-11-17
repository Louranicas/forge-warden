//! Built-in rules: holds that no policy can express and none can remove (M7-Q64, ledger Q64,
//! ruling R-a).
//!
//! A policy glob names paths, never file modes. So a binary section on an executable whose name no
//! policy lists passes every signal rule, and the shell can still run it: bash runs a file as a
//! script unless its first line holds a NUL byte, and Git calls a file binary for a NUL anywhere.
//! The built-in rule `builtin:binary-on-executable` holds such a change for a person.
//!
//! It is a rule outcome, not a note. Notes are unknown evidence, which no acknowledgement may accept.
//! Here the evidence is known (Git's own facts) and a person can read the file, so the hold is
//! acknowledged like a policy hold (`scripts/hold_ack.py`, verdict v3). No waiver can name it:
//! policy ids are kebab-case, so a `builtin:` id is never a policy rule's, and a waiver's rule must
//! be one of the policy's own.

use crate::facts::{Mode, PatchFacts};
use crate::verdict::{RuleOutcome, RuleResult, Violation};

/// The id of the built-in rule (a `:` no policy id may contain).
pub(crate) const BINARY_ON_EXECUTABLE: &str = "builtin:binary-on-executable";
/// The `kind` every built-in rule result reports.
pub(crate) const BUILTIN_KIND: &str = "builtin";
/// Why it holds, shown to whoever it stops.
const WHY: &str = "an executable or a symlink sent as a binary section hides its lines from every \
                   signal rule, and the shell can still run it; a person reads it before it merges";

/// The built-in rule over Git's `facts`: a hold on every file that Git reads as binary, that the
/// change does not delete, and whose mode after the change is executable (`100755`) or a symlink
/// (`120000`). It is listed whatever the policy's own rules say about the path (Lodestar's ruling
/// on M7-Q64 LOW-1): a hold is acknowledged by naming its rules, so what this rule exists to show
/// is always named in the verdict a person signs, never hidden behind another hold. `None` when no
/// file qualifies: the rule is listed only when it holds.
///
/// The resulting mode comes from Git's summary for a creation or a mode change, and from the
/// `index` line for a binary edit that keeps its mode (Git's own in bound mode; in unbound mode
/// the author's text, so a falsely stated `100644` on an executable is the limit only bound mode
/// closes: ADR 0012).
pub(crate) fn binary_on_executable(facts: &PatchFacts) -> Option<RuleResult> {
    let violations: Vec<Violation> = facts
        .files()
        .filter(|change| change.binary())
        .filter_map(|change| {
            // `resulting_mode` is `None` for a deletion: a deleted file is not run.
            let what = match change.resulting_mode()? {
                Mode::Exec => "an executable file (100755)",
                Mode::Symlink => "a symlink (120000)",
                Mode::File => return None,
            };
            Some(Violation {
                path: Some(change.path().as_str().to_owned()),
                message: format!("a binary section on {what}"),
                signal: None,
                evidence: Vec::new(),
                evidence_truncated: false,
                waived_by: None,
            })
        })
        .collect();
    (!violations.is_empty()).then(|| RuleResult {
        id: BINARY_ON_EXECUTABLE.to_owned(),
        kind: BUILTIN_KIND,
        severity: "hold",
        outcome: RuleOutcome::Hold,
        why: WHY.to_owned(),
        violations,
    })
}
