//! File admission doors: the one named-file open ([`open_regular`]) and the bounded readers.

use super::{DoorError, DoorErrorKind};
use forge_warden_core::MAX_INPUT_BYTES;
use forge_warden_core::defaults::NamedDefault as _;
use std::io::Read as _;
use std::path::Path;

/// Read at most [`MAX_INPUT_BYTES`] from `reader`; one byte more is refused, never truncated.
pub(super) fn read_bounded(reader: impl std::io::Read, what: &str) -> Result<Vec<u8>, DoorError> {
    read_within(reader, what, MAX_INPUT_BYTES)
}

/// Read at most `budget` bytes; one byte more is refused. Reads `budget + 1` at most, so an
/// endless stream cannot exhaust memory.
pub(super) fn read_within(
    mut reader: impl std::io::Read,
    what: &str,
    budget: usize,
) -> Result<Vec<u8>, DoorError> {
    // A budget that does not fit in u64 reads at most one byte, so any input is refused.
    let limit = u64::try_from(budget).or_fail_closed(0).saturating_add(1);
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(|e| DoorError::io(what, &e))?;
    if bytes.len() > budget {
        return Err(DoorError {
            message: format!("{what}: over the {budget} byte budget"),
            kind: DoorErrorKind::OverBudget,
        });
    }
    Ok(bytes)
}

/// Whether [`open_regular`] follows a symbolic link in the final path component.
#[derive(Clone, Copy)]
pub(super) enum Symlinks {
    /// Follow it (input files: `--policy`, `--patch`, `--rank`).
    Follow,
    /// Refuse it (`O_NOFOLLOW`: the executable pin).
    Refuse,
}

/// Open flags for input files: `O_NONBLOCK` on Linux (`x86_64`, `aarch64`), so opening a FIFO with
/// no writer returns at once and the non-regular file is refused by `fstat` rather than hanging
/// the open (Bastion B-C1a-01, Q2-02). Reads of a regular file are unaffected. (`O_RDONLY` is 0;
/// std adds `O_CLOEXEC`.) Each flag set is one literal, so no operator can be mutated into an
/// equal value; a test checks the compositions.
pub(super) const FOLLOW_FLAGS: i32 = 0o4_000;

/// Open flags for the executable pin: `O_NOFOLLOW | O_NONBLOCK`.
pub(super) const PIN_FLAGS: i32 = 0o404_000;

/// `ELOOP`: `O_NOFOLLOW` met a symbolic link in the final component (or a loop).
pub(super) const ELOOP: i32 = 40;

/// The one place a named file is opened (Q2-02): read-only, always non-blocking, and
/// only a regular file is admitted (a FIFO, socket, device or directory is refused before any
/// read). A procfs or sysfs file reports itself as regular and is read within the byte budget like
/// any other file; this door does not claim to refuse it. A symbolic link the flags do not follow is refused, not an I/O error.
#[expect(
    clippy::disallowed_types,
    reason = "door: open_regular (every named-file open)"
)]
pub(super) fn open_regular(
    path: &Path,
    symlinks: Symlinks,
    what: &str,
) -> Result<(std::fs::File, std::fs::Metadata), DoorError> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let flags = match symlinks {
        Symlinks::Follow => FOLLOW_FLAGS,
        Symlinks::Refuse => PIN_FLAGS,
    };
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(flags)
        .open(path)
        .map_err(|e| {
            if e.raw_os_error() == Some(ELOOP) {
                DoorError::refused(what, &e.to_string())
            } else {
                DoorError::io(what, &e)
            }
        })?;
    let meta = file.metadata().map_err(|e| DoorError::io(what, &e))?;
    if !meta.is_file() {
        return Err(DoorError::refused(what, "not a regular file"));
    }
    Ok((file, meta))
}

/// Read a whole regular file (symbolic links followed), within the byte budget.
pub(crate) fn read_file(path: &Path) -> Result<Vec<u8>, DoorError> {
    let what = format!("reading {}", path.display());
    let (file, _) = open_regular(path, Symlinks::Follow, &what)?;
    read_bounded(file, &what)
}

/// Read all of stdin, within the byte budget.
#[expect(clippy::disallowed_methods, reason = "door: read_stdin")]
pub(crate) fn read_stdin() -> Result<Vec<u8>, DoorError> {
    read_bounded(std::io::stdin().lock(), "reading stdin")
}

#[cfg(test)]
#[cfg(not(fw_fixture_cli))]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn the_budget_is_inclusive_and_one_byte_more_is_refused() {
        assert_eq!(read_within(Cursor::new(b"abcd"), "t", 4).unwrap(), b"abcd");
        let over = read_within(Cursor::new(b"abcde"), "t", 4).unwrap_err();
        assert_eq!(over.kind, DoorErrorKind::OverBudget);
        assert_eq!(read_within(Cursor::new(b"abc"), "t", 4).unwrap(), b"abc");
    }

    #[test]
    fn a_missing_file_is_an_io_failure_and_a_directory_is_refused() {
        // REFUSAL-MAP rows DoorErrorKind::Io and ::Refused, observed by kind.
        let missing = read_file(Path::new("/nonexistent/forge-warden/refusal-map")).unwrap_err();
        assert_eq!(missing.kind, DoorErrorKind::Io, "{}", missing.message);
        let directory = read_file(Path::new("/")).unwrap_err();
        assert_eq!(directory.kind, DoorErrorKind::Refused, "{}", directory.message);
        assert!(directory.message.contains("not a regular file"), "{}", directory.message);
    }
}
