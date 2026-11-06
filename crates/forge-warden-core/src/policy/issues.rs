//! Collecting every problem in a policy, and the proof that a refusal said why.
//!
//! A validator that refuses returns `Err(Rejected)`. Only [`Issues`] can make a [`Rejected`], and
//! only by recording an issue, so a declared rule or waiver can never be dropped without a reason
//! that refuses the whole policy (R-07: a rule kind without a checker used to vanish silently).

use super::PolicyIssue;

/// Every issue found so far, in the order found.
pub(super) struct Issues(Vec<PolicyIssue>);

/// Proof that at least one issue was recorded. Its field is private to this module.
#[derive(Debug)]
pub(super) struct Rejected(());

/// A position in the issue list, to ask whether anything went wrong since.
#[derive(Debug, Clone, Copy)]
pub(super) struct Mark(usize);

impl Issues {
    pub(super) const fn new() -> Self {
        Self(Vec::new())
    }

    /// Record a problem and keep validating, so the author sees every issue at once.
    pub(super) fn push(&mut self, location: &str, message: String) {
        self.0.push(PolicyIssue {
            location: location.to_owned(),
            message,
        });
    }

    /// Record a problem and refuse the item it belongs to.
    pub(super) fn reject(&mut self, location: &str, message: String) -> Rejected {
        self.push(location, message);
        Rejected(())
    }

    pub(super) fn mark(&self) -> Mark {
        Mark(self.0.len())
    }

    /// `Err(Rejected)` if any issue was recorded after `mark`.
    pub(super) fn since(&self, mark: Mark) -> Result<(), Rejected> {
        if self.0.len() > mark.0 {
            Err(Rejected(()))
        } else {
            Ok(())
        }
    }

    /// Every issue recorded so far.
    pub(super) fn into_vec(self) -> Vec<PolicyIssue> {
        self.0
    }

    /// `value` when no issue was recorded; every issue otherwise.
    pub(super) fn finish<T>(self, value: T) -> Result<T, Vec<PolicyIssue>> {
        if self.0.is_empty() {
            Ok(value)
        } else {
            Err(self.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rejection_always_leaves_an_issue_behind() {
        let mut issues = Issues::new();
        let start = issues.mark();
        assert!(issues.since(start).is_ok());
        let _rejected = issues.reject("rule[0]", "why".to_owned());
        assert!(issues.since(start).is_err());
        assert_eq!(issues.finish(()).map_err(|v| v.len()), Err(1));
    }

    #[test]
    fn no_issue_means_the_value_is_returned() {
        assert_eq!(Issues::new().finish(7), Ok(7));
    }
}
