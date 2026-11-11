//! Why a policy was refused: every issue, each with a location.

use std::fmt;

/// A policy that could not be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PolicyError {
    /// More than [`crate::MAX_POLICY_BYTES`].
    #[error("policy is {0} bytes, over the {max} byte budget", max = crate::MAX_POLICY_BYTES)]
    TooLarge(usize),
    /// Not UTF-8, or not TOML of the policy shape (including unknown keys). `at` is the 1-based
    /// line and column of the problem when the TOML parser located it (Q-TYPES-2, Q1-05: the span
    /// is kept as data; the message text is unchanged).
    #[error("policy is not valid TOML of the forge-warden.policy.v0 shape: {message}")]
    Malformed {
        /// The parser's message.
        message: String,
        /// Where, as (line, column), both 1-based; `None` when the parser gave no span.
        at: Option<(usize, usize)>,
    },
    /// Well-formed, but wrong. Every issue found is listed.
    #[error("policy refused with {} issue(s):\n{}", .0.len(), IssueList(.0))]
    Invalid(Vec<PolicyIssue>),
}

/// One problem in a policy.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PolicyIssue {
    /// Where: `rule[2] "no-secrets"`, `waiver[0]`, or `policy`.
    pub location: String,
    /// What is wrong, and how to fix it when that is knowable.
    pub message: String,
}

impl fmt::Display for PolicyIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.location, self.message)
    }
}

struct IssueList<'a>(&'a [PolicyIssue]);

impl fmt::Display for IssueList<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, issue) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "  - {issue}")?;
        }
        Ok(())
    }
}
