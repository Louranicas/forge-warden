//! The validated policy model: what a loaded policy means.

use super::kind::Kind;
use super::paths::{PathSet, Scope};
use crate::date::Date;
use crate::exit::Outcome;
use crate::nonempty::NonEmpty;
use crate::seal::Sha256Hex;

/// A validated policy.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Policy {
    rules: NonEmpty<Rule>,
    waivers: Vec<Waiver>,
    producers: Vec<Sha256Hex>,
}

impl Policy {
    /// Only `parse_policy` builds a policy, so every one in existence was validated and has a rule.
    pub(super) const fn new(
        rules: NonEmpty<Rule>,
        waivers: Vec<Waiver>,
        producers: Vec<Sha256Hex>,
    ) -> Self {
        Self {
            rules,
            waivers,
            producers,
        }
    }

    /// Rules, in the order the file declares them (and the verdict reports them). Never empty.
    #[must_use]
    pub const fn rules(&self) -> &NonEmpty<Rule> {
        &self.rules
    }

    /// Waivers, in file order.
    #[must_use]
    pub fn waivers(&self) -> &[Waiver] {
        &self.waivers
    }

    /// `[producer] sha256`: the SHA-256 of every ranker binary authority mode may run, each 64
    /// lowercase hex digits, in file order. Empty when the policy has no `[producer]` table: such a
    /// policy can only be used in advisory mode (ADR 0005 §2.4).
    #[must_use]
    pub fn producers(&self) -> &[Sha256Hex] {
        &self.producers
    }

    /// Whether `sha256` is on the producer allowlist.
    #[must_use]
    pub fn admits_producer(&self, sha256: &Sha256Hex) -> bool {
        self.producers.contains(sha256)
    }

    /// The token that lets a ranking be attested as produced by the ranker whose bytes hash to
    /// `sha256`, if and only if this policy's `[producer] sha256` allowlist lists it. This is the
    /// only way to obtain an [`AdmittedProducer`] (Q-TYPES-1).
    #[must_use]
    pub fn admit_producer(&self, sha256: &Sha256Hex) -> Option<AdmittedProducer> {
        self.admits_producer(sha256).then(|| AdmittedProducer {
            sha256: sha256.clone(),
        })
    }
}

/// A ranker the policy's `[producer] sha256` allowlist admits, obtained only from
/// [`Policy::admit_producer`] (its field is private and it has no other constructor). Attesting a
/// produced ranking takes one by value, so the allowlist check is a precondition the compiler
/// enforces for every caller, the CLI and any library consumer alike (Q1-01).
#[derive(Debug)]
#[non_exhaustive]
pub struct AdmittedProducer {
    sha256: Sha256Hex,
}

impl AdmittedProducer {
    /// The admitted ranker's SHA-256 (64 lowercase hex digits, as the allowlist holds it).
    #[must_use]
    pub const fn sha256(&self) -> &Sha256Hex {
        &self.sha256
    }

    /// The SHA-256, consuming the token (an attestation uses it up).
    pub(crate) fn into_sha256(self) -> Sha256Hex {
        self.sha256
    }
}

/// One rule.
#[derive(Debug, Clone)]
#[non_exhaustive]
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
// Exhaustive on purpose: a rule's severity is hold or fail. Adding a variant must break every
// consumer's match (Q-TYPES-1; recorded in the public-API snapshot).
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
#[non_exhaustive]
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
// Exhaustive on purpose: a score budget is per file or per change. Adding a variant must break every
// consumer's match (Q-TYPES-1; recorded in the public-API snapshot).
pub enum ScorePer {
    /// Each file's score.
    File,
    /// The sum of every in-scope file's score.
    Change,
}

/// A time-boxed, owned exemption from one rule for some paths.
#[derive(Debug, Clone)]
#[non_exhaustive]
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
