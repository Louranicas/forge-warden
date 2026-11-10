//! Entropy door: unpredictable private-directory names from the kernel's random UUID, with no weaker fallback.

use super::{DoorError, Symlinks, open_regular, read_within};
use std::path::Path;

/// Linux generates a fresh random UUID for each read. Reuse the fixed-path, no-follow,
/// nonblocking regular-file door (procfs reports regular metadata), with a 37-byte budget.
/// This CLI already requires Linux procfs for child supervision and descriptor pinning.
/// Missing/malformed entropy refuses; no clock/PID fallback weakens name unpredictability.
pub(super) fn private_suffix() -> Result<String, DoorError> {
    let what = "private directory randomness (/proc/sys/kernel/random/uuid)";
    let (file, _) = open_regular(
        Path::new("/proc/sys/kernel/random/uuid"),
        Symlinks::Refuse,
        what,
    )?;
    let bytes = read_within(file, what, 37)?;
    uuid_suffix(&bytes)
}

pub(in crate::doors) fn uuid_suffix(bytes: &[u8]) -> Result<String, DoorError> {
    let refuse = || {
        DoorError::environment(
            "private directory randomness: kernel UUID is not canonical v4".to_owned(),
        )
    };
    let text = std::str::from_utf8(bytes).map_err(|_| refuse())?;
    let uuid = text.strip_suffix('\n').ok_or_else(refuse)?;
    if uuid.len() != 36
        || !uuid.bytes().enumerate().all(|(at, byte)| match at {
            8 | 13 | 18 | 23 => byte == b'-',
            14 => byte == b'4',
            19 => matches!(byte, b'8' | b'9' | b'a' | b'b'),
            _ => byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte),
        })
    {
        return Err(refuse());
    }
    Ok(uuid.to_owned())
}
