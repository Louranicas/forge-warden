//! Effect doors (ADR 0001): the only functions in forge-warden that reach outside the process.
//!
//! Each door is one small function carrying `#[expect(clippy::disallowed_methods, reason =
//! "door: …")]`. Adding an effect means adding a door here, in a reviewed diff; the workspace
//! `clippy.toml` makes the same call anywhere else a compile error, and `repo_invariants.rs`
//! fails if any other file opens a door.

use forge_warden_core::{Date, Exit, MAX_INPUT_BYTES};
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
    // fails closed: a budget that does not fit in u64 reads at most one byte, so any input is refused
    let limit = u64::try_from(budget).unwrap_or(0).saturating_add(1);
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

/// The process status for a registry [`Exit`]: the only place a status is made (R-20). `main`
/// returns a `Status`, so every way the binary ends goes through the exit-code registry.
pub(crate) struct Status(pub(crate) Exit);

impl std::process::Termination for Status {
    fn report(self) -> std::process::ExitCode {
        std::process::ExitCode::from(self.0.entry().code)
    }
}

/// Write a diagnostic line to stderr. Failure to write a diagnostic is ignored: there is nowhere
/// left to report it.
#[expect(clippy::disallowed_methods, reason = "door: diagnose")]
pub(crate) fn diagnose(text: &str) {
    let mut err = std::io::stderr().lock();
    let _ignored = writeln!(err, "{}", text.trim_end());
}

/// Child processes (ADR 0005 §2.4, §2.5): the ranker and Git run through these doors only.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "WP-C1a builds the doors; WP-C1c wires them into `check`"
    )
)]
pub(crate) mod child {
    use super::{DoorError, read_within};
    use std::io::{Read as _, Write as _};
    use std::path::Path;
    use std::process::ExitStatus;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// `O_NOFOLLOW` on Linux (`x86_64`, `aarch64`): open refuses a symlink in the final path component.
    #[cfg(target_os = "linux")]
    pub(super) const O_NOFOLLOW: i32 = 0o400_000;

    /// `O_NONBLOCK` on Linux (`x86_64`, `aarch64`): opening a FIFO with no writer returns at once
    /// instead of blocking, so a non-regular file is refused by `fstat` rather than hanging the
    /// open (Bastion B-C1a-01). Reads of a regular file are unaffected.
    #[cfg(target_os = "linux")]
    pub(super) const O_NONBLOCK: i32 = 0o4_000;

    /// The flags `pin_executable` opens with: `O_NOFOLLOW | O_NONBLOCK` (`O_RDONLY` is 0; std adds
    /// `O_CLOEXEC`). One literal, so the composition cannot be mutated into an equal value (the two
    /// bits are disjoint); a test checks it equals the composition.
    #[cfg(target_os = "linux")]
    pub(super) const PIN_FLAGS: i32 = 0o404_000;

    /// The largest executable forge-warden will pin (hash) before running it.
    pub(super) const MAX_EXECUTABLE_BYTES: usize = 512 * 1024 * 1024;

    /// How often the parent checks a running child for exit, overflow or timeout.
    const POLL: std::time::Duration = std::time::Duration::from_millis(5);

    /// An executable opened once, checked and hashed through that one descriptor. It runs as
    /// `/proc/self/fd/<n>` (the same open file), so swapping the path after hashing changes nothing.
    #[derive(Debug)]
    pub(crate) struct PinnedExecutable {
        file: std::fs::File,
        /// SHA-256 of the bytes read through the descriptor.
        pub(crate) sha256: String,
    }

    /// How a child ended, or why it was refused.
    #[derive(Debug, PartialEq, Eq)]
    pub(crate) enum ChildFailure {
        /// The program could not be started.
        Spawn(String),
        /// It did not finish within the wall-clock limit (stdin write and both drains included).
        Timeout,
        /// It wrote more than the cap to stdout.
        StdoutOverflow,
        /// It wrote more than the cap to stderr.
        StderrOverflow,
        /// It was ended by a signal.
        Signal(i32),
        /// It exited with a nonzero status.
        Exit(i32),
        /// Talking to it failed.
        Io(String),
    }

    /// What a child that exited 0 wrote.
    #[derive(Debug, PartialEq, Eq)]
    pub(crate) struct ChildOutput {
        pub(crate) stdout: Vec<u8>,
        pub(crate) stderr: Vec<u8>,
    }

    /// How to run one child: an absolute program or a pinned executable, an argument vector (never a
    /// shell), stdin bytes, the exact environment (nothing is inherited), a working directory, and
    /// bounds.
    pub(crate) struct ChildSpec<'a> {
        pub(crate) program: Program<'a>,
        pub(crate) args: &'a [&'a str],
        pub(crate) stdin: &'a [u8],
        pub(crate) env: &'a [(&'a str, &'a str)],
        pub(crate) cwd: &'a Path,
        pub(crate) timeout: std::time::Duration,
        pub(crate) stdout_cap: usize,
        pub(crate) stderr_cap: usize,
    }

    /// The program a child runs.
    pub(crate) enum Program<'a> {
        /// A system tool named by an absolute path (e.g. `/usr/bin/git`).
        Absolute(&'a Path),
        /// An executable pinned by content.
        Pinned(&'a PinnedExecutable),
    }

    /// Open `path` once without following a final symlink, require a regular file owned by root or
    /// by this process's user and writable by neither group nor others, and hash its bytes through
    /// that descriptor. Linux only: elsewhere authority mode is refused, never weakened.
    #[cfg(target_os = "linux")]
    pub(crate) fn pin_executable(path: &Path) -> Result<PinnedExecutable, DoorError> {
        pin_within(path, MAX_EXECUTABLE_BYTES)
    }

    /// [`pin_executable`] with an explicit byte budget.
    #[cfg(target_os = "linux")]
    #[expect(
        clippy::disallowed_methods,
        clippy::disallowed_types,
        reason = "door: pin_executable"
    )]
    pub(super) fn pin_within(path: &Path, budget: usize) -> Result<PinnedExecutable, DoorError> {
        use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
        let what = format!("pinning {}", path.display());
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(PIN_FLAGS)
            .open(path)
            .map_err(|e| DoorError::io(&what, &e))?;
        let meta = file.metadata().map_err(|e| DoorError::io(&what, &e))?;
        let own_uid = std::fs::metadata("/proc/self")
            .map_err(|e| DoorError::io("reading /proc/self", &e))?
            .uid();
        let refuse = |why: &str| DoorError {
            message: format!("{what}: {why}"),
            over_budget: false,
        };
        if !meta.is_file() {
            return Err(refuse("not a regular file"));
        }
        if meta.uid() != 0 && meta.uid() != own_uid {
            return Err(refuse("owned by neither root nor this user"));
        }
        if meta.mode() & 0o022 != 0 {
            return Err(refuse("writable by group or others"));
        }
        let bytes = read_within(&mut file, &what, budget)?;
        Ok(PinnedExecutable {
            file,
            sha256: forge_warden_core::seal::sha256_hex(&bytes),
        })
    }

    /// Run one child within its bounds. The child gets its own process group; on every way out
    /// (exit, timeout, overflow, error) the whole group is killed and reaped, so a grandchild that
    /// holds the pipes cannot outlive the call. This bounds children that stay in the group: it is
    /// **not** a sandbox (a child that calls `setsid` escapes; ADR 0005 §2.5).
    #[expect(clippy::disallowed_methods, reason = "door: run_child")]
    pub(crate) fn run_child(spec: &ChildSpec<'_>) -> Result<ChildOutput, ChildFailure> {
        use std::os::fd::AsRawFd as _;
        use std::os::unix::process::CommandExt as _;
        use std::process::{Command, Stdio};

        let program = match spec.program {
            Program::Absolute(path) if path.is_absolute() => path.to_path_buf(),
            Program::Absolute(path) => {
                return Err(ChildFailure::Spawn(format!(
                    "{} is not an absolute path",
                    path.display()
                )));
            }
            Program::Pinned(pinned) => {
                std::path::PathBuf::from(format!("/proc/self/fd/{}", pinned.file.as_raw_fd()))
            }
        };
        let mut child = Command::new(&program)
            .args(spec.args)
            .env_clear()
            .envs(spec.env.iter().copied())
            .current_dir(spec.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .map_err(|e| ChildFailure::Spawn(e.to_string()))?;
        let group = child.id();
        let started = std::time::Instant::now();

        let (Some(mut stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            kill_group(group);
            let _killed = child.kill();
            let _reaped = child.wait();
            return Err(ChildFailure::Io("a child pipe was not created".to_owned()));
        };
        let input = spec.stdin.to_vec();
        let writer = std::thread::spawn(move || stdin.write_all(&input));
        let drain = |pipe: Box<dyn std::io::Read + Send>, cap: usize| {
            let over = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&over);
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                // fails closed: a cap that does not fit in u64 reads one byte, so any output overflows
                let limit = u64::try_from(cap).unwrap_or(0).saturating_add(1);
                let mut bytes = Vec::new();
                let read = pipe.take(limit).read_to_end(&mut bytes);
                if bytes.len() > cap {
                    flag.store(true, Ordering::SeqCst);
                }
                let _sent = tx.send(read.map(|_| bytes));
            });
            (over, rx)
        };
        let (out_over, out_rx) = drain(Box::new(stdout), spec.stdout_cap);
        let (err_over, err_rx) = drain(Box::new(stderr), spec.stderr_cap);

        let ended = wait_bounded(&mut child, &out_over, &err_over, started, spec.timeout);
        // Whatever happened, nothing in the child's group outlives this call.
        kill_group(group);
        // If the group kill failed, the direct child still dies here, so `wait` cannot hang.
        let _killed = child.kill();
        let _reaped = child.wait();
        let remaining = spec
            .timeout
            .saturating_sub(started.elapsed())
            .saturating_add(POLL);
        let out_bytes = out_rx.recv_timeout(remaining);
        let err_bytes = err_rx.recv_timeout(POLL.saturating_mul(20));
        let wrote = writer.join();

        let status = ended?;
        if out_over.load(Ordering::SeqCst) {
            return Err(ChildFailure::StdoutOverflow);
        }
        if err_over.load(Ordering::SeqCst) {
            return Err(ChildFailure::StderrOverflow);
        }
        exited_cleanly(status)?;
        match wrote {
            Ok(Ok(())) => {}
            // A child that exits 0 without reading all of stdin did not judge the whole input.
            Ok(Err(e)) => return Err(ChildFailure::Io(format!("writing the child's stdin: {e}"))),
            Err(_) => return Err(ChildFailure::Io("the stdin writer panicked".to_owned())),
        }
        let take = |got: Result<std::io::Result<Vec<u8>>, std::sync::mpsc::RecvTimeoutError>| {
            got.map_err(|_| ChildFailure::Timeout)?
                .map_err(|e| ChildFailure::Io(e.to_string()))
        };
        Ok(ChildOutput {
            stdout: take(out_bytes)?,
            stderr: take(err_bytes)?,
        })
    }

    /// Wait for the child to exit, stopping early on an overflow or the wall-clock limit.
    #[expect(clippy::disallowed_methods, reason = "door: run_child (poll sleep)")]
    fn wait_bounded(
        child: &mut std::process::Child,
        out_over: &AtomicBool,
        err_over: &AtomicBool,
        started: std::time::Instant,
        timeout: std::time::Duration,
    ) -> Result<ExitStatus, ChildFailure> {
        loop {
            if out_over.load(Ordering::SeqCst) {
                return Err(ChildFailure::StdoutOverflow);
            }
            if err_over.load(Ordering::SeqCst) {
                return Err(ChildFailure::StderrOverflow);
            }
            match child.try_wait() {
                Ok(Some(status)) => return Ok(status),
                Ok(None) => {}
                Err(e) => return Err(ChildFailure::Io(e.to_string())),
            }
            if started.elapsed() >= timeout {
                return Err(ChildFailure::Timeout);
            }
            std::thread::sleep(POLL);
        }
    }

    /// `Ok` only for a normal exit with status 0.
    fn exited_cleanly(status: ExitStatus) -> Result<(), ChildFailure> {
        use std::os::unix::process::ExitStatusExt as _;
        if let Some(signal) = status.signal() {
            return Err(ChildFailure::Signal(signal));
        }
        match status.code() {
            Some(0) => Ok(()),
            Some(code) => Err(ChildFailure::Exit(code)),
            None => Err(ChildFailure::Io("the child has no exit status".to_owned())),
        }
    }

    /// Kill every process in `group` with SIGKILL (`/usr/bin/kill -KILL -- -<group>`); no `unsafe`
    /// and no new dependency. Failure to kill is ignored here: the caller has already decided the
    /// outcome, and a group that is already gone is the common case.
    #[expect(clippy::disallowed_methods, reason = "door: kill_group")]
    fn kill_group(group: u32) {
        let _status = std::process::Command::new("/usr/bin/kill")
            .args(["-KILL", "--", &format!("-{group}")])
            .env_clear()
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

#[cfg(test)]
#[expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "tests build fixtures and inspect processes; a panic is a failed test"
)]
mod child_tests {
    use super::child::{
        ChildFailure, ChildSpec, MAX_EXECUTABLE_BYTES, Program, pin_executable, pin_within,
        run_child,
    };
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    fn spec<'a>(program: Program<'a>, args: &'a [&'a str], stdin: &'a [u8]) -> ChildSpec<'a> {
        ChildSpec {
            program,
            args,
            stdin,
            env: &[],
            cwd: Path::new("/"),
            timeout: Duration::from_secs(5),
            stdout_cap: 8 * 1024 * 1024,
            stderr_cap: 64 * 1024,
        }
    }

    fn sh(script: &str) -> Vec<&str> {
        vec!["-c", script]
    }

    /// No live process has `marker` in its command line, read straight from `/proc` (no `pgrep`:
    /// the offline verification image has none).
    fn none_left(marker: &str) -> bool {
        std::fs::read_dir("/proc")
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .bytes()
                    .all(|b| b.is_ascii_digit())
            })
            .filter_map(|e| std::fs::read(e.path().join("cmdline")).ok())
            .map(|raw| String::from_utf8_lossy(&raw).replace('\0', " "))
            .all(|cmdline| !cmdline.contains(marker))
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fw-child-{}-{name}", std::process::id()));
        let _gone = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_pin_flags_are_nofollow_and_nonblock_exactly() {
        use super::child::{O_NOFOLLOW, O_NONBLOCK, PIN_FLAGS};
        assert_eq!(O_NOFOLLOW & O_NONBLOCK, 0, "disjoint bits");
        assert_eq!(PIN_FLAGS, O_NOFOLLOW | O_NONBLOCK);
    }

    #[test]
    fn a_fifo_is_refused_at_once_not_waited_on() {
        let dir = scratch("fifo");
        let fifo = dir.join("ranker");
        let made = std::process::Command::new("/usr/bin/mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        let (tx, rx) = std::sync::mpsc::channel();
        let target = fifo.clone();
        let pinning = std::thread::spawn(move || {
            let _sent = tx.send(pin_executable(&target).map(|p| p.sha256));
        });
        let outcome = rx.recv_timeout(Duration::from_millis(900));
        if outcome.is_err() {
            // Release a blocked open so this test fails instead of hanging the suite.
            let _writer = std::fs::OpenOptions::new().write(true).open(&fifo);
        }
        let _joined = pinning.join();
        let refused = outcome
            .expect("pin_executable blocked on a FIFO for 900 ms")
            .unwrap_err();
        assert!(
            refused.message.ends_with("not a regular file"),
            "{}",
            refused.message
        );
        let _cleaned = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_large_input_round_trips_without_deadlock() {
        let input = vec![b'x'; 4 * 1024 * 1024];
        let out = run_child(&spec(Program::Absolute(Path::new("/bin/cat")), &[], &input)).unwrap();
        assert_eq!(out.stdout.len(), input.len());
        assert_eq!(out.stdout, input);
    }

    #[test]
    fn a_timeout_kills_the_whole_group() {
        // The marker carries this test process's id, so a concurrent run (or a mutant that leaves
        // its own grandchildren behind) can never be mistaken for a leak from this test.
        let marker = format!("sleep 31.{}", std::process::id());
        let script = format!("{marker} & {marker}; wait");
        let args = sh(&script);
        let mut s = spec(Program::Absolute(Path::new("/bin/sh")), &args, b"");
        s.timeout = Duration::from_millis(300);
        let started = Instant::now();
        assert_eq!(run_child(&s), Err(ChildFailure::Timeout));
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "{:?}",
            started.elapsed()
        );
        assert!(none_left(&marker));
    }

    #[test]
    fn a_grandchild_holding_the_pipes_is_reaped() {
        let marker = format!("sleep 32.{}", std::process::id());
        let script = format!("{marker} & echo started");
        let args = sh(&script);
        let started = Instant::now();
        let out = run_child(&spec(Program::Absolute(Path::new("/bin/sh")), &args, b"")).unwrap();
        assert_eq!(out.stdout, b"started\n");
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "{:?}",
            started.elapsed()
        );
        assert!(none_left(&marker));
    }

    #[test]
    fn output_over_either_cap_is_a_failure_not_a_truncation() {
        let flood_out = sh("head -c 2000 /dev/zero");
        let mut over_out = spec(Program::Absolute(Path::new("/bin/sh")), &flood_out, b"");
        over_out.stdout_cap = 1000;
        assert_eq!(run_child(&over_out), Err(ChildFailure::StdoutOverflow));
        let flood_err = sh("head -c 2000 /dev/zero >&2");
        let mut over_err = spec(Program::Absolute(Path::new("/bin/sh")), &flood_err, b"");
        over_err.stderr_cap = 1000;
        assert_eq!(run_child(&over_err), Err(ChildFailure::StderrOverflow));
        let exact = sh("head -c 1000 /dev/zero");
        let mut at_cap = spec(Program::Absolute(Path::new("/bin/sh")), &exact, b"");
        at_cap.stdout_cap = 1000;
        assert_eq!(
            run_child(&at_cap).map(|o| o.stdout.len()),
            Ok(1000),
            "exactly the cap is allowed"
        );
    }

    #[test]
    fn a_nonzero_exit_or_a_signal_fails_even_with_valid_looking_output() {
        let exits = sh("echo '{\"ranked\":[]}'; exit 3");
        assert_eq!(
            run_child(&spec(Program::Absolute(Path::new("/bin/sh")), &exits, b"")),
            Err(ChildFailure::Exit(3))
        );
        let killed = sh("echo ok; kill -9 $$");
        assert_eq!(
            run_child(&spec(Program::Absolute(Path::new("/bin/sh")), &killed, b"")),
            Err(ChildFailure::Signal(9))
        );
    }

    #[test]
    fn the_environment_is_exactly_the_allowlist() {
        let mut s = spec(Program::Absolute(Path::new("/usr/bin/env")), &[], b"");
        s.env = &[("LC_ALL", "C"), ("GIT_CONFIG_NOSYSTEM", "1")];
        let out = run_child(&s).unwrap();
        let mut lines: Vec<&[u8]> = out
            .stdout
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
            .collect();
        lines.sort_unstable();
        assert_eq!(lines, [&b"GIT_CONFIG_NOSYSTEM=1"[..], &b"LC_ALL=C"[..]]);
    }

    #[test]
    fn a_relative_program_is_refused() {
        assert!(matches!(
            run_child(&spec(Program::Absolute(Path::new("sh")), &[], b"")),
            Err(ChildFailure::Spawn(_))
        ));
    }

    #[test]
    fn the_pin_budget_is_512_mib_and_an_executable_over_it_is_refused_as_input() {
        assert_eq!(MAX_EXECUTABLE_BYTES, 536_870_912);
        let refused = pin_within(Path::new("/usr/bin/true"), 16).unwrap_err();
        assert!(refused.over_budget, "{}", refused.message);
        assert!(pin_within(Path::new("/usr/bin/true"), MAX_EXECUTABLE_BYTES).is_ok());
    }

    #[test]
    fn a_root_owned_system_binary_can_be_pinned_and_run() {
        let pinned = pin_executable(Path::new("/usr/bin/true")).unwrap();
        assert_eq!(
            run_child(&spec(Program::Pinned(&pinned), &[], b"")).map(|o| o.stdout),
            Ok(Vec::new())
        );
    }

    #[test]
    fn a_pinned_executable_runs_the_hashed_bytes_after_its_path_is_swapped() {
        let dir = scratch("swap");
        let path = dir.join("ranker");
        std::fs::copy("/usr/bin/true", &path).unwrap();
        std::fs::copy("/usr/bin/false", dir.join("other")).unwrap();
        let pinned = pin_executable(&path).unwrap();
        let expected =
            forge_warden_core::seal::sha256_hex(&std::fs::read("/usr/bin/true").unwrap());
        assert_eq!(pinned.sha256, expected);
        std::fs::rename(dir.join("other"), &path).unwrap();
        // The path now holds `false` (exit 1); the pinned descriptor still runs `true`.
        let out = run_child(&spec(Program::Pinned(&pinned), &[], b""));
        assert_eq!(out.map(|o| o.stdout), Ok(Vec::new()));
        let swapped = run_child(&spec(Program::Absolute(&path), &[], b""));
        assert_eq!(swapped, Err(ChildFailure::Exit(1)));
    }

    #[test]
    fn pinning_refuses_symlinks_and_writable_files_and_scripts_do_not_run() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = scratch("refuse");
        let real = dir.join("real");
        std::fs::copy("/usr/bin/true", &real).unwrap();
        std::os::unix::fs::symlink(&real, dir.join("link")).unwrap();
        assert!(pin_executable(&dir.join("link")).is_err(), "symlink");
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o775)).unwrap();
        let writable = pin_executable(&real).unwrap_err();
        assert!(
            writable.message.contains("writable by group or others"),
            "{}",
            writable.message
        );
        let script = dir.join("script");
        std::fs::write(&script, "#!/bin/sh\necho ran\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let pinned = pin_executable(&script).unwrap();
        // The interpreter starts but cannot reopen the closed descriptor: the script never runs.
        let out = run_child(&spec(Program::Pinned(&pinned), &[], b""));
        assert!(matches!(out, Err(ChildFailure::Exit(_))), "{out:?}");
    }
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
