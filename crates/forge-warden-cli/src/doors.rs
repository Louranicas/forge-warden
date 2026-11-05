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
fn read_bounded(mut reader: impl std::io::Read, what: &str) -> Result<Vec<u8>, DoorError> {
    let limit = u64::try_from(MAX_INPUT_BYTES)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(|e| DoorError::io(what, &e))?;
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(DoorError {
            message: format!("{what}: over the {MAX_INPUT_BYTES} byte budget"),
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
    Date::from_unix_days(since_epoch.as_secs() / 86_400).ok_or_else(|| DoorError {
        message: "system clock is outside 1970..=9999".to_owned(),
        over_budget: false,
    })
}

/// Write `text` to stdout. A closed pipe (`forge-warden … | head`) is not an error.
#[expect(clippy::disallowed_methods, reason = "door: emit")]
pub(crate) fn emit(text: &str) -> Result<(), DoorError> {
    let mut out = std::io::stdout().lock();
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
