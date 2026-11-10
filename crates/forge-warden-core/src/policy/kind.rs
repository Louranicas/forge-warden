//! Rule kinds and the optional fields each kind accepts.
//!
//! `kind` is an enum, not a string, so adding a kind is a compile error at every place that decides
//! something about kinds ([`Kind::name`], [`Kind::fields`] and the checker built for each kind). A
//! field a kind does not list is refused, so forgetting to list one fails closed.

/// What a rule checks. Each variant is one `kind = "…"` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Kind {
    /// `forbid_signal`: no file in scope may carry any of the listed signals.
    ForbidSignal,
    /// `require_companion`: if a `when` file changes, a `require` file must change too.
    RequireCompanion,
    /// `protect_paths`: any change to a file in scope is a violation.
    ProtectPaths,
    /// `max_score`: a risk-score budget, per file or per change.
    MaxScore,
    /// `max_files`: at most `limit` files in scope may change.
    MaxFiles,
}

/// An optional rule field. `id`, `why`, `kind` and `severity` are required on every rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Field {
    /// `signals`
    Signals,
    /// `paths`
    Paths,
    /// `except`
    Except,
    /// `when`
    When,
    /// `require`
    Require,
    /// `limit`
    Limit,
    /// `per`
    Per,
}

impl Kind {
    /// Every kind, in documentation order.
    pub const ALL: [Self; 5] = [
        Self::ForbidSignal,
        Self::RequireCompanion,
        Self::ProtectPaths,
        Self::MaxScore,
        Self::MaxFiles,
    ];

    /// The name a policy writes for this kind.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ForbidSignal => "forbid_signal",
            Self::RequireCompanion => "require_companion",
            Self::ProtectPaths => "protect_paths",
            Self::MaxScore => "max_score",
            Self::MaxFiles => "max_files",
        }
    }

    /// The kind a policy wrote as `name`, if there is one.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }

    /// The optional fields a rule of this kind may carry. Any other field is refused.
    #[must_use]
    pub const fn fields(self) -> &'static [Field] {
        match self {
            Self::ForbidSignal => &[Field::Signals, Field::Paths, Field::Except],
            Self::RequireCompanion => &[Field::When, Field::Require, Field::Except],
            Self::ProtectPaths => &[Field::Paths, Field::Except],
            Self::MaxScore => &[Field::Limit, Field::Per, Field::Paths, Field::Except],
            Self::MaxFiles => &[Field::Limit, Field::Paths, Field::Except],
        }
    }
}

impl Field {
    /// Every optional field, in documentation order.
    pub const ALL: [Self; 7] = [
        Self::Signals,
        Self::Paths,
        Self::Except,
        Self::When,
        Self::Require,
        Self::Limit,
        Self::Per,
    ];

    /// The key a policy writes for this field.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Signals => "signals",
            Self::Paths => "paths",
            Self::Except => "except",
            Self::When => "when",
            Self::Require => "require",
            Self::Limit => "limit",
            Self::Per => "per",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_are_distinct() {
        for kind in Kind::ALL {
            assert_eq!(Kind::from_name(kind.name()), Some(kind));
        }
        assert_eq!(Kind::from_name("require_review"), None);
        assert_eq!(Kind::from_name(""), None);
        let mut names: Vec<&str> = Kind::ALL.iter().map(|k| k.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Kind::ALL.len());
        let mut fields: Vec<&str> = Field::ALL.iter().map(|f| f.name()).collect();
        fields.sort_unstable();
        fields.dedup();
        assert_eq!(fields.len(), Field::ALL.len());
    }

    #[test]
    fn every_kind_accepts_except_and_only_its_own_fields() {
        for kind in Kind::ALL {
            assert!(kind.fields().contains(&Field::Except), "{kind:?}");
        }
        assert_eq!(
            Kind::ForbidSignal.fields(),
            [Field::Signals, Field::Paths, Field::Except]
        );
        assert_eq!(
            Kind::RequireCompanion.fields(),
            [Field::When, Field::Require, Field::Except]
        );
        assert_eq!(Kind::ProtectPaths.fields(), [Field::Paths, Field::Except]);
        assert_eq!(
            Kind::MaxScore.fields(),
            [Field::Limit, Field::Per, Field::Paths, Field::Except]
        );
        assert_eq!(
            Kind::MaxFiles.fields(),
            [Field::Limit, Field::Paths, Field::Except]
        );
    }
}
