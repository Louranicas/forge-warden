//! Output doors: stdout ([`emit`]) and the terminal-safe stderr diagnostics.

use super::DoorError;
use forge_warden_core::Exit;
use std::io::Write as _;

/// Write `text` to stdout. A closed pipe (`forge-warden … | head`) is not an error.
#[expect(clippy::disallowed_methods, reason = "door: emit")]
pub(crate) fn emit(text: &str) -> Result<(), DoorError> {
    write_tolerating_closed_pipe(std::io::stdout().lock(), text)
}

pub(super) fn write_tolerating_closed_pipe(
    mut out: impl std::io::Write,
    text: &str,
) -> Result<(), DoorError> {
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
    let _ignored = writeln!(err, "{}", forge_warden_core::render::terminal_safe(text));
}

/// Write exactly one JSON error envelope. Fields are terminal-safe before encoding, so JSON
/// escapes are not double-escaped by the human diagnostic door. Stderr write failure is final.
#[expect(clippy::disallowed_methods, reason = "door: diagnose_json")]
pub(crate) fn diagnose_json(exit: Exit, message: &str, next: &str) {
    let entry = exit.entry();
    let value = serde_json::json!({
        "exit": entry.code,
        "name": entry.name,
        "message": forge_warden_core::render::terminal_safe(message),
        "next": forge_warden_core::render::terminal_safe(next),
    });
    let mut err = std::io::stderr().lock();
    let _ignored = writeln!(err, "{value}");
}

#[cfg(test)]
#[cfg(not(fw_fixture_cli))]
mod tests {
    use super::*;
    use std::io::ErrorKind;

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
