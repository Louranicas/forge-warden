//! Calendar dates, used only for waiver expiry.
//!
//! The core never reads a clock: the CLI reads one through a door (or takes `--today`) and passes
//! a [`Date`] in. Dates are proleptic Gregorian and compare in calendar order.

use std::fmt;

/// Why text or numbers are not a [`Date`] (Q-TYPES-2, Q1-12: a caller learns the reason instead of
/// inventing one for a bare `None`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DateError {
    /// Not exactly `YYYY-MM-DD` (a time, an offset, a short or non-digit field).
    #[error("is not YYYY-MM-DD")]
    BadShape,
    /// The year is outside 1970..=9999.
    #[error("is outside 1970-01-01..=9999-12-31")]
    OutOfRange,
    /// The month or day does not exist (month 13, day 0, February 30, ...).
    #[error("is not a calendar date")]
    NotACalendarDate,
}

/// A calendar date between 1970-01-01 and 9999-12-31.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub struct Date {
    year: u16,
    month: u8,
    day: u8,
}

impl Date {
    /// The date, if it exists in the calendar and lies within the supported range.
    ///
    /// # Errors
    ///
    /// [`DateError::OutOfRange`] for a year outside 1970..=9999, [`DateError::NotACalendarDate`]
    /// for a month or day that does not exist.
    pub fn new(year: u16, month: u8, day: u8) -> Result<Self, DateError> {
        if !(1970..=9999).contains(&year) {
            return Err(DateError::OutOfRange);
        }
        if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
            return Err(DateError::NotACalendarDate);
        }
        Ok(Self { year, month, day })
    }

    /// Parse exactly `YYYY-MM-DD`. Anything else (a time, an offset, a short field) is refused.
    ///
    /// # Errors
    ///
    /// [`DateError::BadShape`] for anything not `YYYY-MM-DD`, otherwise as [`Date::new`].
    pub fn parse(text: &str) -> Result<Self, DateError> {
        let bytes = text.as_bytes();
        if bytes.len() != 10 || bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
            return Err(DateError::BadShape);
        }
        let field = |range: std::ops::Range<usize>| -> Option<u16> {
            let part = text.get(range)?;
            if !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            part.parse().ok()
        };
        let shape = |value: Option<u16>| value.ok_or(DateError::BadShape);
        let year = shape(field(0..4))?;
        let month = u8::try_from(shape(field(5..7))?).map_err(|_| DateError::NotACalendarDate)?;
        let day = u8::try_from(shape(field(8..10))?).map_err(|_| DateError::NotACalendarDate)?;
        Self::new(year, month, day)
    }

    /// The date `days` days after 1970-01-01 (the Unix epoch), if in range.
    ///
    /// This is Howard Hinnant's `civil_from_days`, restricted to non-negative day counts.
    #[must_use]
    pub fn from_unix_days(days: u64) -> Option<Self> {
        let z = days.checked_add(719_468)?;
        let era = z.checked_div(146_097)?;
        let doe = z.checked_sub(era.checked_mul(146_097)?)?;
        let yoe = doe
            .checked_sub(doe.checked_div(1_460)?)?
            .checked_add(doe.checked_div(36_524)?)?
            .checked_sub(doe.checked_div(146_096)?)?
            .checked_div(365)?;
        let year = yoe.checked_add(era.checked_mul(400)?)?;
        let doy = doe.checked_sub(
            yoe.checked_mul(365)?
                .checked_add(yoe.checked_div(4)?)?
                .checked_sub(yoe.checked_div(100)?)?,
        )?;
        let mp = doy.checked_mul(5)?.checked_add(2)?.checked_div(153)?;
        let day = doy
            .checked_sub(mp.checked_mul(153)?.checked_add(2)?.checked_div(5)?)?
            .checked_add(1)?;
        let month = if mp < 10 {
            mp.checked_add(3)?
        } else {
            mp.checked_sub(9)?
        };
        let year = if month <= 2 {
            year.checked_add(1)?
        } else {
            year
        };
        Self::new(
            u16::try_from(year).ok()?,
            u8::try_from(month).ok()?,
            u8::try_from(day).ok()?,
        )
        .ok()
    }

    /// Days since 1970-01-01 (Hinnant's `days_from_civil`); the inverse of
    /// [`Date::from_unix_days`].
    #[must_use]
    pub fn to_unix_days(self) -> u64 {
        let (year, month, day) = (
            u64::from(self.year),
            u64::from(self.month),
            u64::from(self.day),
        );
        // Every intermediate is bounded by the 1970..=9999 year range, so saturating arithmetic
        // never actually saturates; it only keeps the arithmetic lint-clean.
        let year = if month <= 2 {
            year.saturating_sub(1)
        } else {
            year
        };
        let era = year / 400;
        let yoe = year.saturating_sub(era.saturating_mul(400));
        let mp = if month > 2 {
            month.saturating_sub(3)
        } else {
            month.saturating_add(9)
        };
        let doy = (mp.saturating_mul(153).saturating_add(2) / 5)
            .saturating_add(day)
            .saturating_sub(1);
        let doe = yoe
            .saturating_mul(365)
            .saturating_add(yoe / 4)
            .saturating_sub(yoe / 100)
            .saturating_add(doy);
        era.saturating_mul(146_097)
            .saturating_add(doe)
            .saturating_sub(719_468)
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

const fn is_leap(year: u16) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

const fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refused_date_names_its_reason() {
        // Q-TYPES-2 (Q1-12): the caller gets the reason, not a bare None.
        for (text, why) in [
            ("2026-11-5", DateError::BadShape),
            ("2026-11-05T00:00", DateError::BadShape),
            ("2026/11/05", DateError::BadShape),
            ("2026-1x-05", DateError::BadShape),
            ("1969-12-31", DateError::OutOfRange),
            ("2026-13-01", DateError::NotACalendarDate),
            ("2026-02-30", DateError::NotACalendarDate),
            ("2026-11-00", DateError::NotACalendarDate),
        ] {
            assert_eq!(Date::parse(text), Err(why), "{text}");
        }
        assert_eq!(Date::new(10_000, 1, 1), Err(DateError::OutOfRange));
    }
    use proptest::prelude::*;

    #[test]
    fn epoch_and_known_dates() {
        assert_eq!(Date::from_unix_days(0), Date::new(1970, 1, 1).ok());
        // 2026-11-05 is day 20_762 (python: (date(2026,11,5) - date(1970,1,1)).days).
        assert_eq!(Date::from_unix_days(20_762), Date::new(2026, 11, 5).ok());
        assert_eq!(Date::from_unix_days(11_016), Date::new(2000, 2, 29).ok());
    }

    #[test]
    fn parse_is_exact() {
        assert_eq!(Date::parse("2026-11-05"), Date::new(2026, 11, 5));
        for bad in [
            "2026-11-5",
            "2026/11/05",
            "26-11-05",
            "2026-13-01",
            "2026-02-29",
            "2026-11-05T00:00",
            "+026-11-05",
            "2026-1a-05",
            "",
            "1969-12-31",
            "2026-11/05",
            "2026/11-05",
        ] {
            assert!(Date::parse(bad).is_err(), "{bad:?} must be refused");
        }
    }

    /// The calendar is checked against independent facts, not against itself: `parse`,
    /// `from_unix_days` and the round-trip property all share `Date::new`, so a bug there would
    /// pass them all (cargo-mutants found exactly that).
    #[test]
    fn the_calendar_matches_known_facts() {
        for (text, exists) in [
            ("2024-02-29", true),  // divisible by 4
            ("2026-02-29", false), // not divisible by 4
            ("2000-02-29", true),  // divisible by 400
            ("2100-02-29", false), // divisible by 100, not 400
            ("2026-02-28", true),
            ("2026-04-30", true),
            ("2026-04-31", false), // 30 days hath September, April, June and November
            ("2026-06-31", false),
            ("2026-09-31", false),
            ("2026-11-31", false),
            ("2026-12-31", true),
            ("2026-01-31", true),
            ("9999-12-31", true),
        ] {
            assert_eq!(Date::parse(text).is_ok(), exists, "{text}");
        }
    }

    #[test]
    fn display_round_trips() {
        let date = Date::new(2027, 3, 9).unwrap();
        assert_eq!(date.to_string(), "2027-03-09");
        assert_eq!(Date::parse(&date.to_string()), Ok(date));
    }

    proptest! {
        #[test]
        fn unix_days_round_trip(days in 0_u64..2_932_896) {
            let date = Date::from_unix_days(days).expect("in range");
            prop_assert_eq!(date.to_unix_days(), days);
            prop_assert_eq!(Date::parse(&date.to_string()), Ok(date));
        }

        #[test]
        fn calendar_order_matches_day_order(a in 0_u64..2_932_896, b in 0_u64..2_932_896) {
            let (da, db) = (Date::from_unix_days(a).unwrap(), Date::from_unix_days(b).unwrap());
            prop_assert_eq!(da.cmp(&db), a.cmp(&b));
        }
    }
}
