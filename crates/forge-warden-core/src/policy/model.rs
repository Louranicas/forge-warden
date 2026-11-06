//! The validated policy model: what a loaded policy means.

use super::kind::Kind;
use super::paths::{PathSet, Scope};
use crate::date::Date;
use crate::exit::Outcome;

/// A validated policy.
#[derive(Debug, Clone)]
pub struct Policy {
    /// Rules, in the order the file declares them (and the verdict reports them).
    pub rules: Vec<Rule>,
    /// Waivers, in file order.
    pub waivers: Vec<Waiver>,
}

/// One rule.
#[derive(Debug, Clone)]
pub struct Rule {
    /// Unique kebab-case id.
    pub id: String,
    /// Why the rule exists, for the person it stops.
    pub why: String,
    /// What a violation means.
    pub severity: Severity,
    /// What it checks.
    pub check: Check,
}

/// What a violated rule does to the verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The change must not merge as it is.
    Fail,
    /// A person must look before it merges.
    Hold,
}

impl Severity {
    /// The outcome a violation of this severity produces.
    #[must_use]
    pub const fn outcome(self) -> Outcome {
        match self {
            Self::Fail => Outcome::Fail,
            Self::Hold => Outcome::Hold,
        }
    }

    /// Lowercase name.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Fail => "fail",
            Self::Hold => "hold",
        }
    }
}

/// The check a rule performs. Each variant is one [`Kind`].
#[derive(Debug, Clone)]
pub enum Check {
    /// [`Kind::ForbidSignal`]: no file in scope may carry any of `signals`.
    ForbidSignal {
        /// Signals that are not allowed.
        signals: Vec<String>,
        /// Files the rule looks at.
        scope: Scope,
    },
    /// [`Kind::RequireCompanion`]: if a file in `when` changes, some file in `require` must change too.
    RequireCompanion {
        /// Files whose change triggers the rule.
        when: Scope,
        /// Files of which at least one must also be in the change.
        require: PathSet,
    },
    /// [`Kind::ProtectPaths`]: any change to a file in scope is a violation.
    ProtectPaths {
        /// Files that are protected.
        scope: Scope,
    },
    /// [`Kind::MaxScore`]: a risk-score budget.
    MaxScore {
        /// The largest allowed score.
        limit: u32,
        /// Per file, or summed over the change.
        per: ScorePer,
        /// Files the rule looks at.
        scope: Scope,
    },
    /// [`Kind::MaxFiles`]: at most `limit` files in scope may change.
    MaxFiles {
        /// The largest allowed number of files.
        limit: u32,
        /// Files the rule looks at.
        scope: Scope,
    },
}

impl Check {
    /// The kind of this check.
    #[must_use]
    pub const fn kind(&self) -> Kind {
        match self {
            Self::ForbidSignal { .. } => Kind::ForbidSignal,
            Self::RequireCompanion { .. } => Kind::RequireCompanion,
            Self::ProtectPaths { .. } => Kind::ProtectPaths,
            Self::MaxScore { .. } => Kind::MaxScore,
            Self::MaxFiles { .. } => Kind::MaxFiles,
        }
    }
}

/// Whether a score budget applies to each file or to the change as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScorePer {
    /// Each file's score.
    File,
    /// The sum of every in-scope file's score.
    Change,
}

/// A time-boxed, owned exemption from one rule for some paths.
#[derive(Debug, Clone)]
pub struct Waiver {
    /// The rule it exempts from.
    pub rule: String,
    /// Paths it covers.
    pub paths: PathSet,
    /// Why the exemption is acceptable.
    pub reason: String,
    /// Who answers for it.
    pub owner: String,
    /// The last day it applies. From the next day it is expired, and an expired waiver fails the
    /// verdict until it is removed or renewed.
    pub expires: Date,
}
