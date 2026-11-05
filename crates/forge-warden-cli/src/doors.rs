//! Effect doors (ADR 0001): the only functions in forge-warden that reach outside the process.
//!
//! Each door is one small function carrying `#[expect(clippy::disallowed_methods, reason =
//! "door: …")]`. Adding an effect means adding a door here, in a reviewed diff; the workspace
//! `clippy.toml` makes the same call anywhere else a compile error, and `repo_invariants.rs`
//! fails if any other file opens a door.

use forge_warden_core::{Date, MAX_INPUT_BYTES};
use std::io::{Read as _, Write as _};
use std::path::Path;

/// A door that could not be passed.
#[derive(Debug)]
pub(crate) struct DoorError {
    /// What failed, for stderr.
    pub(crate) message: String,
    /// Whether the input was over budget (an input problem, not an environment problem).
    pub(crate) over_budget: bool,
}

impl DoorError {
    fn io(what: &str, error: &std::io::Error) -> Self {
        Self {
            message: format!("{what}: {error}"),
            over_budget: false,
        }
    }
}

/// Read at most [`MAX_INPUT_BYTES`] from `reader`; one byte more is refused, never truncated.
fn read_bounded(reader: impl std::io::Read, what: &str) -> Result<Vec<u8>, DoorError> {
    read_within(reader, what, MAX_INPUT_BYTES)
}

/// Read at most `budget` bytes; one byte more is refused. Reads `budget + 1` at most, so an
/// endless stream cannot exhaust memory.
fn read_within(
    mut reader: impl std::io::Read,
    what: &str,
    budget: usize,
) -> Result<Vec<u8>, DoorError> {
    let limit = u64::try_from(budget).unwrap_or(u64::MAX).saturating_add(1);
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(|e| DoorError::io(what, &e))?;
    if bytes.len() > budget {
        return Err(DoorError {
            message: format!("{what}: over the {budget} byte budget"),
            over_budget: true,
        });
    }
    Ok(bytes)
}

/// Read a whole file, within the byte budget.
#[expect(clippy::disallowed_methods, reason = "door: read_file")]
pub(crate) fn read_file(path: &Path) -> Result<Vec<u8>, DoorError> {
    let what = format!("reading {}", path.display());
    let file = std::fs::File::open(path).map_err(|e| DoorError::io(&what, &e))?;
    read_bounded(file, &what)
}

/// Read all of stdin, within the byte budget.
#[expect(clippy::disallowed_methods, reason = "door: read_stdin")]
pub(crate) fn read_stdin() -> Result<Vec<u8>, DoorError> {
    read_bounded(std::io::stdin().lock(), "reading stdin")
}

/// Today's date in UTC, from the system clock.
#[expect(clippy::disallowed_methods, reason = "door: today")]
pub(crate) fn today() -> Result<Date, DoorError> {
    let since_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| DoorError {
            message: format!("system clock is before 1970: {e}"),
            over_budget: false,
        })?;
    date_of_unix_seconds(since_epoch.as_secs())
}

/// The UTC calendar date of a Unix timestamp.
fn date_of_unix_seconds(seconds: u64) -> Result<Date, DoorError> {
    Date::from_unix_days(seconds / 86_400).ok_or_else(|| DoorError {
        message: "system clock is outside 1970..=9999".to_owned(),
        over_budget: false,
    })
}

/// Write `text` to stdout. A closed pipe (`forge-warden … | head`) is not an error.
#[expect(clippy::disallowed_methods, reason = "door: emit")]
pub(crate) fn emit(text: &str) -> Result<(), DoorError> {
    write_tolerating_closed_pipe(std::io::stdout().lock(), text)
}

fn write_tolerating_closed_pipe(mut out: impl std::io::Write, text: &str) -> Result<(), DoorError> {
    match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
        Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => {
            Err(DoorError::io("writing stdout", &e))
        }
        Ok(()) | Err(_) => Ok(()),
    }
}

/// Write a diagnostic line to stderr. Failure to write a diagnostic is ignored: there is nowhere
/// left to report it.
#[expect(clippy::disallowed_methods, reason = "door: diagnose")]
pub(crate) fn diagnose(text: &str) {
    let mut err = std::io::stderr().lock();
    let _ignored = writeln!(err, "{}", text.trim_end());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, ErrorKind};

    #[test]
    fn the_budget_is_inclusive_and_one_byte_more_is_refused() {
        assert_eq!(read_within(Cursor::new(b"abcd"), "t", 4).unwrap(), b"abcd");
        let over = read_within(Cursor::new(b"abcde"), "t", 4).unwrap_err();
        assert!(over.over_budget);
        assert_eq!(read_within(Cursor::new(b"abc"), "t", 4).unwrap(), b"abc");
    }

    #[test]
    fn unix_seconds_map_to_utc_dates() {
        assert_eq!(
            date_of_unix_seconds(0).unwrap(),
            Date::new(1970, 1, 1).unwrap()
        );
        assert_eq!(
            date_of_unix_seconds(86_399).unwrap(),
            Date::new(1970, 1, 1).unwrap()
        );
        assert_eq!(
            date_of_unix_seconds(86_400).unwrap(),
            Date::new(1970, 1, 2).unwrap()
        );
        // 2026-11-05T12:00:00Z
        assert_eq!(
            date_of_unix_seconds(1_793_880_000).unwrap(),
            Date::new(2026, 11, 5).unwrap()
        );
    }

    struct Failing(ErrorKind);

    impl std::io::Write for Failing {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(self.0.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_closed_pipe_is_not_an_error_but_other_write_failures_are() {
        assert!(write_tolerating_closed_pipe(Failing(ErrorKind::BrokenPipe), "x").is_ok());
        assert!(write_tolerating_closed_pipe(Failing(ErrorKind::PermissionDenied), "x").is_err());
        let mut sink = Vec::new();
        write_tolerating_closed_pipe(&mut sink, "hello").unwrap();
        assert_eq!(sink, b"hello");
    }
}
