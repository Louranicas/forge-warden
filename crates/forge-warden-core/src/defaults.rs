//! Named defaults (G-04, WP-B7): a missing value is replaced only by a method whose name says what
//! the replacement does to a decision.
//!
//! `.unwrap_or(Outcome::Pass)` once decided a verdict silently (R-19). The raw `unwrap_or`,
//! `unwrap_or_else` and `unwrap_or_default` of `Option` and `Result` are disallowed methods in both
//! `clippy.toml` files, so every default in forge-warden is one of the three intents below, chosen at
//! the call site and visible to a reviewer:
//!
//! - [`NamedDefault::or_fail_closed`]: the default is the **stricter** choice; a missing value can only
//!   refuse, hold or fail.
//! - [`NamedDefault::or_fail_safe`]: the default is **harmless**: it neither loosens a decision nor
//!   hides input.
//! - [`NamedDefault::or_display`]: the default changes **only human-readable text**, never a decision.
//!
//! The implementations use `match`, so this module needs no exemption from the lint.

/// A value that may be missing (`Option`) or failed (`Result`), replaced by a named default.
pub trait NamedDefault<T> {
    /// What the default is computed from when the value is missing: `()` for an `Option`, the error
    /// for a `Result`.
    type Missing;

    /// The value, or `strict`: the stricter choice (refuse, hold or fail).
    #[must_use]
    fn or_fail_closed(self, strict: T) -> T;

    /// The value, or the stricter choice computed from what is missing.
    #[must_use]
    fn or_fail_closed_with(self, strict: impl FnOnce(Self::Missing) -> T) -> T;

    /// The value, or `safe`: a default that neither loosens a decision nor hides input.
    #[must_use]
    fn or_fail_safe(self, safe: T) -> T;

    /// The value, or `shown`: a default that only changes human-readable text.
    #[must_use]
    fn or_display(self, shown: T) -> T;
}

impl<T> NamedDefault<T> for Option<T> {
    type Missing = ();

    fn or_fail_closed(self, strict: T) -> T {
        match self {
            Some(value) => value,
            None => strict,
        }
    }

    fn or_fail_closed_with(self, strict: impl FnOnce(()) -> T) -> T {
        match self {
            Some(value) => value,
            None => strict(()),
        }
    }

    fn or_fail_safe(self, safe: T) -> T {
        match self {
            Some(value) => value,
            None => safe,
        }
    }

    fn or_display(self, shown: T) -> T {
        match self {
            Some(value) => value,
            None => shown,
        }
    }
}

impl<T, E> NamedDefault<T> for Result<T, E> {
    type Missing = E;

    fn or_fail_closed(self, strict: T) -> T {
        match self {
            Ok(value) => value,
            Err(_) => strict,
        }
    }

    fn or_fail_closed_with(self, strict: impl FnOnce(E) -> T) -> T {
        match self {
            Ok(value) => value,
            Err(error) => strict(error),
        }
    }

    fn or_fail_safe(self, safe: T) -> T {
        match self {
            Ok(value) => value,
            Err(_) => safe,
        }
    }

    fn or_display(self, shown: T) -> T {
        match self {
            Ok(value) => value,
            Err(_) => shown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::NamedDefault;

    #[test]
    fn a_present_value_always_wins_and_a_missing_one_takes_the_named_default() {
        let present: Option<u8> = Some(1);
        let missing: Option<u8> = None;
        assert_eq!(present.or_fail_closed(9), 1);
        assert_eq!(present.or_fail_closed_with(|()| 9), 1);
        assert_eq!(present.or_fail_safe(9), 1);
        assert_eq!(present.or_display(9), 1);
        assert_eq!(missing.or_fail_closed(9), 9);
        assert_eq!(missing.or_fail_closed_with(|()| 9), 9);
        assert_eq!(missing.or_fail_safe(9), 9);
        assert_eq!(missing.or_display(9), 9);
        let ok: Result<u8, u8> = Ok(1);
        let err: Result<u8, u8> = Err(5);
        assert_eq!(ok.or_fail_closed(9), 1);
        assert_eq!(ok.or_fail_closed_with(|e| e), 1);
        assert_eq!(ok.or_fail_safe(9), 1);
        assert_eq!(ok.or_display(9), 1);
        assert_eq!(err.or_fail_closed(9), 9);
        assert_eq!(
            err.or_fail_closed_with(|e| e.saturating_mul(2)),
            10,
            "the error reaches the default"
        );
        assert_eq!(err.or_fail_safe(9), 9);
        assert_eq!(err.or_display(9), 9);
    }
}
