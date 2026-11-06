//! Rendering a verdict for a terminal.
//!
//! Paths and signals come from whoever wrote the patch, so nothing from a document reaches the
//! terminal raw: every character outside printable ASCII is written as `\u{…}`. That makes
//! terminal-escape injection and Trojan-Source bidi reordering impossible by construction, at the
//! cost of escaping legitimate non-ASCII paths (the JSON output keeps them exact).

use crate::verdict::{Note, RuleOutcome, Verdict};
use std::fmt::Write as _;

/// `text` with every character outside printable ASCII (0x20..=0x7e) escaped as `\u{XXXX}`, and
/// backslashes doubled so the escaping is unambiguous.
#[must_use]
pub fn terminal_safe(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            ' '..='~' => out.push(ch),
            other => {
                let _ignored = write!(out, "\\u{{{:04x}}}", u32::from(other));
            }
        }
    }
    out
}

/// A human summary of `verdict`, one line per rule and per violation.
#[must_use]
pub fn render_text(verdict: &Verdict) -> String {
    let mut out = String::new();
    let label = verdict.verdict.label().to_ascii_uppercase();
    let _ = writeln!(
        out,
        "forge-warden {}: {label} (exit {} {}, next: {})",
        verdict.tool.version, verdict.exit.code, verdict.exit.name, verdict.exit.action
    );
    let ranker = match &verdict.seals.ranked_by.sha256 {
        Some(sha256) => format!("produced by ranker {}", terminal_safe(&short(sha256))),
        None => "supplied, unverified".to_owned(),
    };
    let _ = writeln!(
        out,
        "  patch {}, policy {}, ranked by deep-diff-forge {} ({ranker}), on {}",
        short(&verdict.seals.patch_sha256),
        short(&verdict.seals.policy_sha256),
        terminal_safe(&verdict.seals.ranked_by.version),
        verdict.evaluated_on,
    );
    let s = &verdict.summary;
    let _ = writeln!(
        out,
        "  {} file(s), {} rule(s): {} pass, {} waived, {} hold, {} fail",
        s.files, s.rules, s.passed, s.waived, s.held, s.failed
    );
    for rule in &verdict.rules {
        let mark = match rule.outcome {
            RuleOutcome::Pass => "PASS  ",
            RuleOutcome::Waived => "WAIVED",
            RuleOutcome::Hold => "HOLD  ",
            RuleOutcome::Fail => "FAIL  ",
        };
        let _ = writeln!(
            out,
            "  [{mark}] {} ({}, {})",
            rule.id, rule.kind, rule.severity
        );
        if matches!(rule.outcome, RuleOutcome::Hold | RuleOutcome::Fail) {
            let _ = writeln!(out, "           why: {}", terminal_safe(&rule.why));
        }
        for v in &rule.violations {
            let path = v
                .path
                .as_deref()
                .map_or_else(|| "(whole change)".to_owned(), terminal_safe);
            // display only: an unwaived violation carries no waiver annotation
            let waived = v
                .waived_by
                .map(|i| format!(" [waived by waiver[{i}]]"))
                .unwrap_or_default();
            let _ = writeln!(
                out,
                "           - {path}: {}{waived}",
                terminal_safe(&v.message)
            );
            for c in &v.evidence {
                let _ = writeln!(
                    out,
                    "               at {} line {} (hunk {})",
                    c.side,
                    c.line,
                    short(&c.address)
                );
            }
        }
    }
    for w in &verdict.expired_waivers {
        let _ = writeln!(
            out,
            "  [EXPIRED] waiver[{}] for {} (owner {}) expired {}: renew or delete it",
            w.index,
            terminal_safe(&w.rule),
            terminal_safe(&w.owner),
            w.expires
        );
    }
    for w in &verdict.unused_waivers {
        let _ = writeln!(
            out,
            "  [UNUSED] waiver[{}] for {} matched nothing in this change",
            w.index,
            terminal_safe(&w.rule)
        );
    }
    render_notes(&mut out, &verdict.notes);
    out
}

/// The first 12 characters of a seal, for a one-line summary.
fn short(seal: &str) -> String {
    // fails safe: a seal shorter than 12 characters is shown whole, never hidden
    seal.get(..12).unwrap_or(seal).to_owned()
}

fn render_notes(out: &mut String, notes: &[Note]) {
    for n in notes {
        // display only: a change-wide note has no path to show
        let path = n
            .path
            .as_deref()
            .map(|p| format!(" {}:", terminal_safe(p)))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "  [NOTE] {}{path} {}",
            n.code.name(),
            terminal_safe(&n.detail)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn hostile_text_is_neutralised() {
        assert_eq!(terminal_safe("a\u{1b}[31mred"), "a\\u{001b}[31mred");
        assert_eq!(terminal_safe("x\u{202e}y"), "x\\u{202e}y");
        assert_eq!(terminal_safe("tab\there"), "tab\\u{0009}here");
        assert_eq!(terminal_safe("back\\slash"), "back\\\\slash");
        assert_eq!(terminal_safe("src/lib.rs"), "src/lib.rs");
    }

    proptest! {
        #[test]
        fn output_is_always_printable_ascii(text in any::<String>()) {
            let safe = terminal_safe(&text);
            prop_assert!(safe.bytes().all(|b| (0x20..=0x7e).contains(&b)));
        }

        #[test]
        fn printable_ascii_without_backslash_is_unchanged(text in "[ -\\[\\]-~]*") {
            prop_assert_eq!(terminal_safe(&text), text);
        }
    }
}
