//! Effect doors (ADR 0001): the only functions in forge-warden that reach outside the process.
//!
//! Each door is one small function carrying `#[expect(clippy::disallowed_methods, reason =
//! "door: …")]`. Adding an effect means adding a door here, in a reviewed diff; the workspace
//! `clippy.toml` makes the same call anywhere else a compile error, and `repo_invariants.rs`
//! fails if any other file opens a door.

use forge_warden_core::defaults::NamedDefault as _;
use forge_warden_core::{Date, Exit, MAX_INPUT_BYTES};
use std::io::{Read as _, Write as _};
use std::path::Path;

/// Why a door could not be passed. It decides the registry exit, in one exhaustive `match`
/// (`Failure::from_door` in main.rs), so a new kind cannot ship unmapped (Q2-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DoorErrorKind {
    /// Reading or writing failed (exit 3, `check_environment`).
    Io,
    /// The host is not set up as the door requires: an unsafe temporary directory, an unusable
    /// clock (exit 3, `check_environment`).
    Environment,
    /// The input is over its byte budget (exit 4, `change_input`).
    OverBudget,
    /// The door refuses the input itself: not a regular file, a symbolic link it does not follow,
    /// unsafe ownership or mode (exit 4, `change_input`).
    Refused,
}

/// A door that could not be passed.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DoorError {
    /// What failed, for stderr.
    pub(crate) message: String,
    /// Why, which decides the exit.
    pub(crate) kind: DoorErrorKind,
}

impl DoorError {
    fn io(what: &str, error: &std::io::Error) -> Self {
        Self {
            message: format!("{what}: {error}"),
            kind: DoorErrorKind::Io,
        }
    }

    fn refused(what: &str, why: &str) -> Self {
        Self {
            message: format!("{what}: {why}"),
            kind: DoorErrorKind::Refused,
        }
    }

    fn environment(message: String) -> Self {
        Self {
            message,
            kind: DoorErrorKind::Environment,
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
enum Symlinks {
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
const FOLLOW_FLAGS: i32 = 0o4_000;

/// Open flags for the executable pin: `O_NOFOLLOW | O_NONBLOCK`.
const PIN_FLAGS: i32 = 0o404_000;

/// `ELOOP`: `O_NOFOLLOW` met a symbolic link in the final component (or a loop).
const ELOOP: i32 = 40;

/// The one place a named file is opened (Q2-02): read-only, always non-blocking, and
/// only a regular file is admitted (a FIFO, socket, device or directory is refused before any
/// read). A procfs or sysfs file reports itself as regular and is read within the byte budget like
/// any other file; this door does not claim to refuse it. A symbolic link the flags do not follow is refused, not an I/O error.
#[expect(
    clippy::disallowed_types,
    reason = "door: open_regular (every named-file open)"
)]
fn open_regular(
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

/// Today's date in UTC, from the system clock.
#[expect(clippy::disallowed_methods, reason = "door: today")]
pub(crate) fn today() -> Result<Date, DoorError> {
    let since_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| DoorError::environment(format!("system clock is before 1970: {e}")))?;
    date_of_unix_seconds(since_epoch.as_secs())
}

/// The UTC calendar date of a Unix timestamp.
fn date_of_unix_seconds(seconds: u64) -> Result<Date, DoorError> {
    Date::from_unix_days(seconds / 86_400)
        .ok_or_else(|| DoorError::environment("system clock is outside 1970..=9999".to_owned()))
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
pub(crate) mod child {
    use super::{DoorError, Symlinks, open_regular, read_within};
    use forge_warden_core::Policy;
    use forge_warden_core::authority::{Produced, Ranking};
    use forge_warden_core::defaults::NamedDefault as _;
    use forge_warden_core::rank::parse_rank;
    use forge_warden_core::seal::sha256_hex;
    use std::io::{Read as _, Write as _};
    use std::path::Path;
    use std::process::ExitStatus;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// `O_NOFOLLOW` on Linux (`x86_64`, `aarch64`): open refuses a symlink in the final path component.
    #[cfg(all(target_os = "linux", test))]
    pub(super) const O_NOFOLLOW: i32 = 0o400_000;

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
    #[expect(clippy::disallowed_methods, reason = "door: pin_executable")]
    pub(super) fn pin_within(path: &Path, budget: usize) -> Result<PinnedExecutable, DoorError> {
        use std::os::unix::fs::MetadataExt as _;
        let what = format!("pinning {}", path.display());
        let (mut file, meta) = open_regular(path, Symlinks::Refuse, &what)?;
        let own_uid = std::fs::metadata("/proc/self")
            .map_err(|e| DoorError::io("reading /proc/self", &e))?
            .uid();
        if meta.uid() != 0 && meta.uid() != own_uid {
            return Err(DoorError::refused(
                &what,
                "owned by neither root nor this user",
            ));
        }
        if meta.mode() & 0o022 != 0 {
            return Err(DoorError::refused(&what, "writable by group or others"));
        }
        let bytes = read_within(&mut file, &what, budget)?;
        Ok(PinnedExecutable {
            file,
            sha256: sha256_hex(&bytes),
        })
    }

    /// Run one child within its bounds. The child gets its own process group; on every way out
    /// (exit, timeout, overflow, error) the whole group is killed and reaped, so a grandchild that
    /// holds the pipes cannot outlive the call. This bounds children that stay in the group: it is
    /// **not** a sandbox (a child that calls `setsid` escapes; ADR 0005 §2.5).
    pub(crate) fn run_child(spec: &ChildSpec<'_>) -> Result<ChildOutput, ChildFailure> {
        run_child_killing_with(spec, Path::new(KILL))
    }

    /// The program that kills a child's process group.
    const KILL: &str = "/usr/bin/kill";

    /// [`run_child`] with an explicit group-kill program (a test seam for a missing or failing kill).
    #[expect(clippy::disallowed_methods, reason = "door: run_child")]
    pub(super) fn run_child_killing_with(
        spec: &ChildSpec<'_>,
        kill: &Path,
    ) -> Result<ChildOutput, ChildFailure> {
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
            let _group = kill_group(kill, group);
            let _killed = child.kill();
            let _reaped = child.wait();
            return Err(ChildFailure::Io("a child pipe was not created".to_owned()));
        };
        let input = spec.stdin.to_vec();
        // The writer reports through a channel read against the deadline, never joined (Q2-03): a
        // descendant that left the group can hold stdin open without reading, and a join would
        // then wait for it. A writer still blocked at the deadline is left behind (a one-shot CLI).
        let (wrote_tx, wrote_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _sent = wrote_tx.send(stdin.write_all(&input));
        });
        let drain = |pipe: Box<dyn std::io::Read + Send>, cap: usize| {
            let over = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&over);
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                // A cap that does not fit in u64 reads one byte, so any output overflows.
                let limit = u64::try_from(cap).or_fail_closed(0).saturating_add(1);
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

        let ended = wait_bounded(child.id(), &out_over, &err_over, started, spec.timeout);
        // Whatever happened, nothing in the child's group outlives this call. The leader is not
        // reaped yet (exited means a zombie), so the group still exists and its id cannot have been
        // reused: the kill must succeed, and a failure is reported (Q2-04).
        let killed = kill_group(kill, group);
        // If the group kill failed, the direct child still dies here, so `wait` cannot hang.
        let _killed = child.kill();
        let reaped = child.wait();
        let remaining = spec
            .timeout
            .saturating_sub(started.elapsed())
            .saturating_add(POLL);
        let out_bytes = out_rx.recv_timeout(remaining);
        let err_bytes = err_rx.recv_timeout(POLL.saturating_mul(20));
        let wrote = wrote_rx.recv_timeout(
            spec.timeout
                .saturating_sub(started.elapsed())
                .saturating_add(POLL),
        );

        killed.map_err(ChildFailure::Io)?;
        ended?;
        let status = reaped.map_err(|e| ChildFailure::Io(e.to_string()))?;
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
            // Still blocked at the deadline: something outside the group holds stdin unread.
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return Err(ChildFailure::Timeout),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err(ChildFailure::Io("the stdin writer panicked".to_owned()));
            }
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

    /// Wait for the child `pid` to exit, stopping early on an overflow or the wall-clock limit. Exit
    /// is read from `/proc/<pid>/stat` WITHOUT reaping (`try_wait` would reap), so the caller can
    /// still kill the group by the leader's id before the kernel may reuse it (Q2-04).
    #[expect(
        clippy::disallowed_methods,
        reason = "door: run_child (poll sleep, /proc/<pid>/stat)"
    )]
    fn wait_bounded(
        pid: u32,
        out_over: &AtomicBool,
        err_over: &AtomicBool,
        started: std::time::Instant,
        timeout: std::time::Duration,
    ) -> Result<(), ChildFailure> {
        let stat_path = format!("/proc/{pid}/stat");
        loop {
            if out_over.load(Ordering::SeqCst) {
                return Err(ChildFailure::StdoutOverflow);
            }
            if err_over.load(Ordering::SeqCst) {
                return Err(ChildFailure::StderrOverflow);
            }
            let stat = std::fs::read(&stat_path)
                .map_err(|e| ChildFailure::Io(format!("reading {stat_path}: {e}")))?;
            match has_exited(&stat) {
                Some(true) => return Ok(()),
                Some(false) => {}
                None => {
                    return Err(ChildFailure::Io(format!(
                        "{stat_path} is not a process status"
                    )));
                }
            }
            if started.elapsed() >= timeout {
                return Err(ChildFailure::Timeout);
            }
            std::thread::sleep(POLL);
        }
    }

    /// Whether a `/proc/<pid>/stat` line shows an exited, unreaped process (state `Z` or `X`). The
    /// state follows the last `)`, because the command name in parentheses may hold anything.
    pub(super) fn has_exited(stat: &[u8]) -> Option<bool> {
        let close = stat.iter().rposition(|&b| b == b')')?;
        let rest = stat.get(close.checked_add(1)?..)?;
        let state = rest.iter().find(|b| !b.is_ascii_whitespace())?;
        Some(matches!(state, b'Z' | b'X'))
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

    /// A private, empty, 0700 directory under the system temp directory, removed on drop. Git runs
    /// with it as both `HOME` and cwd, and with its parent as `GIT_CEILING_DIRECTORIES`, so no
    /// repository above it is ever discovered (ADR 0010).
    #[derive(Debug)]
    pub(crate) struct PrivateDir {
        path: std::path::PathBuf,
    }

    impl PrivateDir {
        /// A private directory under `base`, created exclusively: a name that already exists is
        /// never reused, and any other error is reported at once, not retried.
        pub(super) fn create_in(base: &Path) -> Result<Self, DoorError> {
            use std::os::unix::fs::DirBuilderExt as _;
            for attempt in 0..64_u32 {
                let path = base.join(format!("forge-warden-{}-{attempt}", std::process::id()));
                match std::fs::DirBuilder::new().mode(0o700).create(&path) {
                    Ok(()) => return Ok(Self { path }),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(DoorError::io("creating a private directory", &e)),
                }
            }
            Err(DoorError::environment(
                "no free private directory name after 64 attempts".to_owned(),
            ))
        }

        pub(crate) fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for PrivateDir {
        fn drop(&mut self) {
            // Git writes nothing here; an unexpected leftover makes removal fail, which is left
            // visible on disk rather than deleted recursively.
            let _removed = std::fs::remove_dir(&self.path);
        }
    }

    /// Why Git's facts could not be read.
    #[derive(Debug, PartialEq, Eq)]
    pub(crate) enum GitFactsFailure {
        /// Git ran and refused the patch (a fact about the patch: HOLD).
        Refused(String),
        /// Git could not be run within its bounds (an environment problem: no verdict).
        Unavailable(String),
    }

    /// `git apply --numstat -z` and `git apply --summary` on the same patch bytes, isolated per
    /// ADR 0010: an absolute Git, argv only, a cleared environment with exactly the contract's
    /// controls (no `PATH`), a private empty cwd and `HOME`, and the candidate-free config overrides.
    #[expect(
        clippy::disallowed_methods,
        reason = "door: git_facts (the system temp dir)"
    )]
    pub(crate) fn git_facts(
        git: &Path,
        patch: &[u8],
    ) -> Result<(Vec<u8>, Vec<u8>), GitFactsFailure> {
        git_facts_in(git, patch, &std::env::temp_dir())
    }

    /// The private directory's base, admitted only if it is safe for all three of its uses: a cwd,
    /// `HOME`, and Git's `:`-separated absolute `GIT_CEILING_DIRECTORIES` (Bastion B-C1b-01). It must
    /// be absolute; it is canonicalized (symlinks resolved); and the result must be UTF-8 without
    /// `:`. Anything else is refused: a relative or `:`-split ceiling is ignored by Git, which would
    /// then discover and read the candidate's repository.
    #[expect(clippy::disallowed_methods, reason = "door: admit_base (canonicalize)")]
    pub(super) fn admit_base(raw: &Path) -> Result<String, DoorError> {
        let refuse = |why: &str| {
            DoorError::environment(format!("temporary directory {}: {why}", raw.display()))
        };
        if !raw.is_absolute() {
            return Err(refuse("not absolute"));
        }
        let real = std::fs::canonicalize(raw)
            .map_err(|e| DoorError::io("canonicalizing the temporary directory", &e))?;
        let text = real.to_str().ok_or_else(|| refuse("not UTF-8"))?;
        if text.contains(':') {
            return Err(refuse("contains ':', which splits Git's ceiling list"));
        }
        Ok(text.to_owned())
    }

    /// A private directory under an admitted base, and the ADR 0010 environment that points at
    /// it. Every child (Git and the ranker) runs with exactly this environment and that cwd.
    pub(super) struct Isolation {
        pub(super) dir: PrivateDir,
        home: String,
        ceiling: String,
    }

    impl Isolation {
        /// Admit `base` ([`admit_base`]) and create the private directory under it.
        pub(super) fn create_in(base: &Path) -> Result<Self, DoorError> {
            let ceiling = admit_base(base)?;
            let dir = PrivateDir::create_in(Path::new(&ceiling))?;
            // The private dir is `<admitted base>/<ASCII name>`, so its path is UTF-8 without `:`
            // too; were it ever not, that is refused, never converted lossily.
            let home = dir
                .path()
                .to_str()
                .ok_or_else(|| {
                    DoorError::environment("private directory path is not UTF-8".to_owned())
                })?
                .to_owned();
            Ok(Self { dir, home, ceiling })
        }

        /// The whole child environment: nothing else is inherited (no `PATH`, no `LD_*`).
        pub(super) fn env(&self) -> [(&str, &str); 7] {
            [
                ("LC_ALL", "C"),
                ("HOME", self.home.as_str()),
                ("GIT_CONFIG_NOSYSTEM", "1"),
                ("GIT_CONFIG_GLOBAL", "/dev/null"),
                ("GIT_ATTR_NOSYSTEM", "1"),
                ("GIT_NO_REPLACE_OBJECTS", "1"),
                ("GIT_CEILING_DIRECTORIES", self.ceiling.as_str()),
            ]
        }
    }

    /// [`git_facts`] with an explicit temporary base (tests use this; `git_facts` passes the
    /// system temp dir).
    pub(super) fn git_facts_in(
        git: &Path,
        patch: &[u8],
        base: &Path,
    ) -> Result<(Vec<u8>, Vec<u8>), GitFactsFailure> {
        let isolation =
            Isolation::create_in(base).map_err(|e| GitFactsFailure::Unavailable(e.message))?;
        let env = isolation.env();
        let dir = &isolation.dir;
        let run = |mode: &[&str]| {
            let mut args = vec![
                "-c",
                "core.attributesFile=/dev/null",
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "core.fsmonitor=false",
                "apply",
            ];
            args.extend_from_slice(mode);
            args.push("-");
            run_child(&ChildSpec {
                program: Program::Absolute(git),
                args: &args,
                stdin: patch,
                env: &env,
                cwd: dir.path(),
                timeout: GIT_TIMEOUT,
                stdout_cap: forge_warden_core::MAX_INPUT_BYTES,
                stderr_cap: GIT_STDERR_CAP,
            })
            .map(|out| out.stdout)
            .map_err(|failure| match failure {
                ChildFailure::Exit(code) => {
                    GitFactsFailure::Refused(format!("git apply exited {code}"))
                }
                ChildFailure::Spawn(why) | ChildFailure::Io(why) => {
                    GitFactsFailure::Unavailable(format!("git apply: {why}"))
                }
                ChildFailure::Timeout => {
                    GitFactsFailure::Unavailable("git apply timed out".to_owned())
                }
                ChildFailure::StdoutOverflow => {
                    GitFactsFailure::Unavailable("git apply wrote more than the budget".to_owned())
                }
                ChildFailure::StderrOverflow => {
                    GitFactsFailure::Unavailable("git apply wrote too much to stderr".to_owned())
                }
                ChildFailure::Signal(signal) => {
                    GitFactsFailure::Unavailable(format!("git apply was killed by signal {signal}"))
                }
            })
        };
        let numstat = run(&["--numstat", "-z"])?;
        let summary = run(&["--summary"])?;
        Ok((numstat, summary))
    }

    /// `git --version`, run isolated like every Git call and trimmed. The verdict records it for an
    /// auditor; forge-warden does not attest it (ADR 0005 §2.4).
    #[expect(
        clippy::disallowed_methods,
        reason = "door: git_version (the system temp dir)"
    )]
    pub(crate) fn git_version(git: &Path) -> Result<String, DoorError> {
        let isolation = Isolation::create_in(&std::env::temp_dir())?;
        let env = isolation.env();
        let refuse = |why: String| DoorError {
            message: format!("{} --version {why}", git.display()),
            kind: super::DoorErrorKind::Io,
        };
        let out = run_child(&ChildSpec {
            program: Program::Absolute(git),
            args: &["--version"],
            stdin: b"",
            env: &env,
            cwd: isolation.dir.path(),
            timeout: GIT_TIMEOUT,
            stdout_cap: GIT_STDERR_CAP,
            stderr_cap: GIT_STDERR_CAP,
        })
        .map_err(|failure| refuse(describe(&failure)))?;
        let text =
            String::from_utf8(out.stdout).map_err(|_| refuse("printed non-UTF-8".to_owned()))?;
        Ok(text.trim().to_owned())
    }

    /// A child failure in words, for a diagnostic.
    fn describe(failure: &ChildFailure) -> String {
        match failure {
            ChildFailure::Spawn(why) => format!("could not be started: {why}"),
            ChildFailure::Timeout => "timed out".to_owned(),
            ChildFailure::StdoutOverflow => "wrote more than its stdout budget".to_owned(),
            ChildFailure::StderrOverflow => "wrote more than its stderr budget".to_owned(),
            ChildFailure::Signal(signal) => format!("was killed by signal {signal}"),
            ChildFailure::Exit(code) => format!("exited {code}"),
            ChildFailure::Io(why) => format!("failed: {why}"),
        }
    }

    /// Why authority mode produced no ranking. Neither case gives a verdict.
    #[derive(Debug, PartialEq, Eq)]
    pub(crate) enum RankerFailure {
        /// The policy does not admit this ranker (exit 4: change the policy or the ranker).
        NotAdmitted(String),
        /// A door refused or failed before the ranker ran (pinning it, isolating it); its kind
        /// decides the exit (Q2-05).
        Door(DoorError),
        /// The ranker ran and failed, or wrote no ranking of this patch (exit 3).
        Failed(String),
    }

    /// The arguments the ranker gets: deep-diff-forge reading the patch from stdin.
    pub(super) const RANKER_ARGS: [&str; 3] = ["--stdin-patch", "--rank", "--json"];
    /// Wall-clock limit for the ranker (ADR 0005 §2.5).
    const RANKER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
    /// The ranker's stderr is diagnostic only.
    pub(super) const RANKER_STDERR_CAP: usize = 64 * 1024;

    /// Authority mode's ranking (ADR 0005 §2.1-§2.5): pin `ranker` by descriptor, require the
    /// policy's `[producer] sha256` to list the pinned bytes, run exactly those bytes isolated on
    /// `patch` (the one buffer), and admit the output only if it is a ranking sealed to that patch.
    /// This is the only place the binary mints a [`Ranking<Produced>`]. Returns the ranking and its
    /// exact bytes.
    #[expect(
        clippy::disallowed_methods,
        reason = "door: produce_ranking (the system temp dir)"
    )]
    pub(crate) fn produce_ranking(
        ranker: &Path,
        policy: &Policy,
        patch: &[u8],
    ) -> Result<(Ranking<Produced>, Vec<u8>), RankerFailure> {
        produce_ranking_in(ranker, policy, patch, &std::env::temp_dir())
    }

    /// [`produce_ranking`] with an explicit temporary base.
    #[expect(
        clippy::disallowed_methods,
        reason = "door: produce_ranking mints Ranking<Produced> (the only place the binary may)"
    )]
    pub(super) fn produce_ranking_in(
        ranker: &Path,
        policy: &Policy,
        patch: &[u8],
        base: &Path,
    ) -> Result<(Ranking<Produced>, Vec<u8>), RankerFailure> {
        if policy.producers().is_empty() {
            return Err(RankerFailure::NotAdmitted(
                "the policy has no [producer] sha256 allowlist, so authority mode admits no ranker \
                 (add the ranker's sha256, or judge a supplied ranking with --advisory)"
                    .to_owned(),
            ));
        }
        if !Path::new("/proc/self/fd").is_dir() {
            return Err(RankerFailure::Failed(
                "authority mode needs /proc/self/fd to run a pinned ranker; it is not weakened"
                    .to_owned(),
            ));
        }
        let pinned = pin_executable(ranker).map_err(RankerFailure::Door)?;
        if !policy.admits_producer(&pinned.sha256) {
            return Err(RankerFailure::NotAdmitted(format!(
                "ranker {} has sha256 {}, which the policy's [producer] sha256 does not list",
                ranker.display(),
                pinned.sha256
            )));
        }
        let isolation = Isolation::create_in(base).map_err(RankerFailure::Door)?;
        let env = isolation.env();
        let out = run_child(&ChildSpec {
            program: Program::Pinned(&pinned),
            args: &RANKER_ARGS,
            stdin: patch,
            env: &env,
            cwd: isolation.dir.path(),
            timeout: RANKER_TIMEOUT,
            stdout_cap: forge_warden_core::MAX_INPUT_BYTES,
            stderr_cap: RANKER_STDERR_CAP,
        })
        .map_err(|failure| RankerFailure::Failed(format!("the ranker {}", describe(&failure))))?;
        let doc = parse_rank(&out.stdout).map_err(|e| {
            RankerFailure::Failed(format!("the ranker's output is not a ranking: {e}"))
        })?;
        let patch_sha256 = sha256_hex(patch);
        if doc.input_sha256() != patch_sha256 {
            return Err(RankerFailure::Failed(format!(
                "the ranker sealed patch {} but was given {patch_sha256}",
                doc.input_sha256()
            )));
        }
        Ok((Ranking::attest(doc, pinned.sha256), out.stdout))
    }

    /// Wall-clock limit for one Git call.
    const GIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
    /// Git's stderr is diagnostic only.
    pub(super) const GIT_STDERR_CAP: usize = 64 * 1024;

    /// The group kill's own wall-clock limit: it sends one signal and exits at once.
    const KILL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

    /// Kill every process in `group` with SIGKILL (`<kill> -KILL -- -<group>`); no `unsafe` and no
    /// new dependency. Called while the group's leader is unreaped, so the group exists: a kill
    /// that cannot start, exits nonzero, or does not finish within [`KILL_TIMEOUT`] (it is a child
    /// too, Bastion E3c-O4-01) is an error the caller reports (Q2-04).
    #[expect(
        clippy::disallowed_methods,
        reason = "door: kill_group (spawn, poll sleep)"
    )]
    fn kill_group(kill: &Path, group: u32) -> Result<(), String> {
        let failed = |why: String| {
            format!(
                "killing the child's process group: {} {why}",
                kill.display()
            )
        };
        let mut killer = std::process::Command::new(kill)
            .args(["-KILL", "--", &format!("-{group}")])
            .env_clear()
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| failed(format!("could not be started: {e}")))?;
        let started = std::time::Instant::now();
        loop {
            match killer.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(status)) => return Err(failed(status.to_string())),
                // Time is left while the limit minus the elapsed time is still defined.
                Ok(None) if KILL_TIMEOUT.checked_sub(started.elapsed()).is_some() => {
                    std::thread::sleep(POLL);
                }
                Ok(None) => {
                    let _killed = killer.kill();
                    let _reaped = killer.wait();
                    return Err(failed(format!("did not finish within {KILL_TIMEOUT:?}")));
                }
                Err(e) => {
                    let _killed = killer.kill();
                    let _reaped = killer.wait();
                    return Err(failed(format!("could not be waited on: {e}")));
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/support/process_env.rs"]
mod test_environment;

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

    fn command(program: &str) -> std::process::Command {
        static HOME: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        let home = HOME.get_or_init(|| scratch("process-home"));
        let mut cmd = std::process::Command::new(program);
        super::test_environment::configure(&mut cmd, home);
        cmd
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

    /// Create `to` as a copy of `from` by a child process, so this test process never holds a write
    /// descriptor on a file it later executes. With one open, another test thread's concurrent fork
    /// inherits it until that child's exec, and executing the file fails with ETXTBSY (MEASURED: 4 in
    /// 2000 runs of `a_pinned_executable_runs_the_hashed_bytes_after_its_path_is_swapped` on 9d57295).
    fn copy_out_of_process(from: &str, to: &Path) {
        let status = command("/usr/bin/cp")
            .arg("--")
            .arg(from)
            .arg(to)
            .status()
            .unwrap();
        assert!(status.success(), "cp {from} {}", to.display());
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fw-child-{}-{name}", std::process::id()));
        let _gone = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn every_open_is_nonblocking_and_the_pin_adds_nofollow_exactly() {
        // Reference values from the Linux headers (x86_64, aarch64).
        use super::child::O_NOFOLLOW;
        const O_NONBLOCK: i32 = 0o4_000;
        assert_eq!(O_NOFOLLOW & O_NONBLOCK, 0, "disjoint bits");
        assert_eq!(super::FOLLOW_FLAGS, O_NONBLOCK);
        assert_eq!(super::PIN_FLAGS, O_NOFOLLOW | O_NONBLOCK);
        assert_eq!(super::ELOOP, 40);
    }

    #[test]
    fn every_file_door_refuses_a_fifo_at_once_not_waited_on() {
        // Q2-02: `--policy`, `--patch` and `--rank` pointed at a FIFO with no writer must be refused,
        // not block forever. Every door that opens a named file is in this table.
        type Door = fn(&Path) -> Result<(), String>;
        let doors: [(&str, Door); 2] = [
            ("read_file", |p| {
                super::read_file(p).map(drop).map_err(|e| e.message)
            }),
            ("pin_executable", |p| {
                pin_executable(p).map(drop).map_err(|e| e.message)
            }),
        ];
        for (name, door) in doors {
            let dir = scratch(&format!("fifo-{name}"));
            let fifo = dir.join("input");
            let made = command("/usr/bin/mkfifo").arg(&fifo).status().unwrap();
            assert!(made.success());
            let (tx, rx) = std::sync::mpsc::channel();
            let target = fifo.clone();
            let opening = std::thread::spawn(move || {
                let _sent = tx.send(door(&target));
            });
            let outcome = rx.recv_timeout(Duration::from_millis(900));
            if outcome.is_err() {
                // Release a blocked open (the writer closes at once, so a blocked read sees EOF)
                // so this test fails instead of hanging the suite.
                let _writer = std::fs::OpenOptions::new().write(true).open(&fifo);
            }
            let _joined = opening.join();
            let Ok(result) = outcome else {
                panic!("{name} blocked on a FIFO for 900 ms");
            };
            let refused = result.unwrap_err();
            assert!(refused.ends_with("not a regular file"), "{name}: {refused}");
            let _cleaned = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn an_escaped_descendant_holding_stdin_cannot_stretch_the_deadline() {
        // Q2-03: the child starts a `setsid` descendant (it leaves the group, so the group kill
        // misses it) that keeps stdin open and never reads, then exits 0. With more input than a
        // pipe holds, the stdin writer blocks; the call must still end at its deadline.
        // The descendant touches a marker only after `setsid`, and the child waits for it before
        // exiting, so the group kill can never catch the descendant still in the group (measured:
        // without the wait, 1 run in ~100 killed it first and the writer saw EPIPE instead).
        // Q-DOOR.2: with job control off, POSIX gives an asynchronous list stdin = /dev/null
        // before its own redirections, so in dash (`/bin/sh` on Debian and Ubuntu) `<&0 &` hands
        // the descendant /dev/null, not the pipe. The pipe is therefore saved on fd 3 first and
        // passed as `<&3`, which holds in every POSIX shell. The descendant then writes what its
        // fd 0 really is into the marker (under a temporary name, renamed when complete), and the
        // test refuses to count a run in which it is not the pipe.
        let dir = scratch("escaped");
        let marker = dir.join("left-the-group");
        let script = format!(
            "exec 3<&0; /usr/bin/setsid /bin/sh -c '/usr/bin/readlink /proc/self/fd/0 > \"$1.tmp\" \
             && /usr/bin/mv \"$1.tmp\" \"$1\"; exec /usr/bin/sleep 6' x '{}' \
             <&3 3<&- >/dev/null 2>&1 & exec 3<&-; while [ ! -e '{}' ]; do :; done; exit 0",
            marker.display(),
            marker.display()
        );
        let input = vec![b'x'; 1024 * 1024];
        let args = sh(&script);
        let mut held = spec(Program::Absolute(Path::new("/bin/sh")), &args, &input);
        held.timeout = Duration::from_secs(1);
        let started = Instant::now();
        let got = run_child(&held);
        let took = started.elapsed();
        assert!(
            took < Duration::from_secs(4),
            "run_child took {took:?}: the stdin writer outlived the deadline"
        );
        let held_stdin = std::fs::read_to_string(&marker)
            .expect("the descendant never left the group, so this did not test the escaped case");
        assert!(
            held_stdin.starts_with("pipe:"),
            "the descendant's stdin was {held_stdin:?}, not the pipe, so this did not test the \
             escaped case (is this shell giving the asynchronous list /dev/null?)"
        );
        assert_eq!(got, Err(ChildFailure::Timeout));
        let _cleaned = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_group_kill_that_hangs_is_bounded_and_reported() {
        // Bastion E3c-O4-01 / Q2-04 (3): the kill is a third child; it has its own deadline. A
        // stand-in that never exits must not hold the call, and is reported, not ignored.
        use super::child::run_child_killing_with;
        use std::os::unix::fs::PermissionsExt as _;
        let dir = scratch("hung-kill");
        let source = dir.join("source");
        std::fs::write(&source, "#!/bin/sh\nexec /usr/bin/sleep 30\n").unwrap();
        let kill = dir.join("kill");
        copy_out_of_process(source.to_str().unwrap(), &kill);
        std::fs::set_permissions(&kill, std::fs::Permissions::from_mode(0o755)).unwrap();
        let args = sh("exit 0");
        let quick = spec(Program::Absolute(Path::new("/bin/sh")), &args, b"");
        let started = Instant::now();
        let got = run_child_killing_with(&quick, &kill);
        let took = started.elapsed();
        assert!(
            took < Duration::from_secs(5),
            "the hung kill held the call for {took:?}"
        );
        match got {
            Err(ChildFailure::Io(why)) => assert!(why.contains("did not finish"), "{why}"),
            other => panic!("{other:?}"),
        }
        let _cleaned = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn exit_is_read_from_the_state_after_the_last_parenthesis() {
        use super::child::has_exited;
        assert_eq!(has_exited(b"42 (sh) Z 1 42 42"), Some(true));
        assert_eq!(has_exited(b"42 (sh) X 1 42 42"), Some(true));
        assert_eq!(has_exited(b"42 (sh) S 1 42 42"), Some(false));
        assert_eq!(has_exited(b"42 (sh) R 1 42 42"), Some(false));
        // A command name may hold `)`, spaces and a fake state; only the last `)` counts.
        assert_eq!(has_exited(b"42 (a) Z (b) S 1 42 42"), Some(false));
        assert_eq!(has_exited(b"42 (a) S (b) Z 1 42 42"), Some(true));
        assert_eq!(has_exited(b"42 (sh)"), None);
        assert_eq!(has_exited(b"no parenthesis"), None);
    }

    #[test]
    fn a_group_kill_that_cannot_run_or_fails_is_reported() {
        // Q2-04: a kill that could not be started, or that failed, leaves grandchildren holding the
        // pipes; it is a failure of the call, never silence.
        use super::child::run_child_killing_with;
        let args = sh("exit 0");
        let quick = spec(Program::Absolute(Path::new("/bin/sh")), &args, b"");
        for kill in ["/nonexistent/kill", "/usr/bin/false"] {
            match run_child_killing_with(&quick, Path::new(kill)) {
                Err(ChildFailure::Io(why)) => assert!(
                    why.starts_with("killing the child's process group"),
                    "{kill}: {why}"
                ),
                other => panic!("{kill}: {other:?}"),
            }
        }
    }

    #[test]
    fn git_facts_reproduces_the_coverage_corpus_captures_byte_for_byte() {
        use super::child::{GitFactsFailure, git_facts};
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/coverage");
        let index: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("cases.json")).unwrap()).unwrap();
        let cases = index["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 57);
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let patch = std::fs::read(root.join(format!("{name}.patch"))).unwrap();
            let got = git_facts(Path::new("/usr/bin/git"), &patch);
            if case["git_rc"] == serde_json::json!([0, 0]) {
                let (numstat, summary) = got.unwrap_or_else(|e| panic!("{name}: {e:?}"));
                assert_eq!(
                    numstat,
                    std::fs::read(root.join(format!("{name}.numstat"))).unwrap(),
                    "{name}"
                );
                assert_eq!(
                    summary,
                    std::fs::read(root.join(format!("{name}.summary"))).unwrap(),
                    "{name}"
                );
            } else {
                assert!(
                    matches!(got, Err(GitFactsFailure::Refused(_))),
                    "{name}: {got:?}"
                );
            }
        }
    }

    #[test]
    fn a_private_dir_is_empty_0700_and_removed_on_drop_and_a_missing_git_is_unavailable() {
        use super::child::{GitFactsFailure, PrivateDir, git_facts};
        use std::os::unix::fs::PermissionsExt as _;
        // An exclusive base: in the shared temp dir another test's private dir may legitimately
        // reuse this process-scoped name after the drop (Bastion, C1c v4.2 receipt).
        let base = scratch("private-dir");
        let dir = PrivateDir::create_in(&base).unwrap();
        let path = dir.path().to_owned();
        let meta = std::fs::metadata(&path).unwrap();
        assert!(meta.is_dir());
        assert_eq!(meta.permissions().mode() & 0o777, 0o700);
        assert_eq!(std::fs::read_dir(&path).unwrap().count(), 0);
        drop(dir);
        assert!(!path.exists(), "{}", path.display());
        let tiny_patch = b"diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n";
        assert!(git_facts(Path::new("/usr/bin/git"), tiny_patch).is_ok());
        assert!(matches!(
            git_facts(Path::new("/nonexistent/git"), tiny_patch),
            Err(GitFactsFailure::Unavailable(_))
        ));
    }

    #[test]
    fn a_freed_private_dir_name_is_reused_so_absence_needs_an_exclusive_base() {
        // The C1c v4.2 race, measured: names are `forge-warden-<pid>-<attempt>` and attempts start
        // at 0, so after a drop the next private dir in the same base takes the freed name. Creation
        // stays exclusive (a live name is never reused), but a test asserting a dropped path is gone
        // must own its base; in the shared temp dir a concurrent test may legitimately recreate it.
        use super::child::PrivateDir;
        let base = scratch("name-reuse");
        let first = PrivateDir::create_in(&base).unwrap();
        let name = first.path().to_owned();
        drop(first);
        let second = PrivateDir::create_in(&base).unwrap();
        assert_eq!(second.path(), name);
        let live = PrivateDir::create_in(&base).unwrap();
        assert_ne!(live.path(), second.path(), "a live name is never reused");
    }

    #[test]
    fn a_private_dir_skips_taken_names_and_reports_other_errors_at_once() {
        use super::child::{GIT_STDERR_CAP, PrivateDir};
        let base = std::env::temp_dir().join(format!("fw-pdbase-{}", std::process::id()));
        let _gone = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join(format!("forge-warden-{}-0", std::process::id())))
            .unwrap();
        let dir = PrivateDir::create_in(&base).unwrap();
        assert!(
            dir.path()
                .ends_with(format!("forge-warden-{}-1", std::process::id()))
        );
        drop(dir);
        let missing = PrivateDir::create_in(&base.join("no-such-dir")).unwrap_err();
        assert!(
            missing.message.starts_with("creating a private directory"),
            "{}",
            missing.message
        );
        let _cleaned = std::fs::remove_dir_all(&base);
        assert_eq!(GIT_STDERR_CAP, 65_536);
    }

    #[test]
    fn the_ranker_runs_as_deep_diff_forge_with_its_stderr_capped_at_64_kib() {
        use super::child::{RANKER_ARGS, RANKER_STDERR_CAP};
        assert_eq!(RANKER_ARGS, ["--stdin-patch", "--rank", "--json"]);
        assert_eq!(RANKER_STDERR_CAP, 65_536);
    }

    #[test]
    fn the_private_base_is_admitted_only_if_safe_for_cwd_home_and_ceiling() {
        use super::child::admit_base;
        use std::os::unix::ffi::OsStrExt as _;
        let root = scratch("admit");
        let refused = |base: &Path| admit_base(base).unwrap_err().message;
        assert!(refused(Path::new("relative/dir")).ends_with("not absolute"));
        let colon = root.join("colon:base");
        std::fs::create_dir_all(&colon).unwrap();
        assert!(
            refused(&colon).contains("contains ':'"),
            "{}",
            refused(&colon)
        );
        let not_utf8 = root.join(std::ffi::OsStr::from_bytes(b"bad\xffname"));
        std::fs::create_dir_all(&not_utf8).unwrap();
        assert!(
            refused(&not_utf8).ends_with("not UTF-8"),
            "{}",
            refused(&not_utf8)
        );
        let real = root.join("real");
        std::fs::create_dir_all(&real).unwrap();
        std::os::unix::fs::symlink(&real, root.join("link")).unwrap();
        let canonical = std::fs::canonicalize(&real).unwrap();
        assert_eq!(
            admit_base(&root.join("link")).unwrap(),
            canonical.to_str().unwrap()
        );
        assert_eq!(admit_base(&real).unwrap(), canonical.to_str().unwrap());
        let _cleaned = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_candidate_repository_around_the_private_dir_is_never_consulted() {
        use super::child::git_facts_in;
        // Bastion B-C1b-01: a candidate repository whose own config breaks `git apply`.
        let candidate = scratch("candidate");
        let git = |args: &[&str], cwd: &Path| {
            command("/usr/bin/git")
                .args(args)
                .current_dir(cwd)
                .env_remove("GIT_DIR")
                .output()
                .unwrap()
        };
        assert!(git(&["init", "-q"], &candidate).status.success());
        assert!(
            git(
                &["config", "apply.ignoreWhitespace", "definitely-invalid"],
                &candidate
            )
            .status
            .success()
        );
        let inner = candidate.join("sub");
        std::fs::create_dir_all(&inner).unwrap();
        let patch = b"diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n";
        // The trap is live: Git started inside the candidate, with no ceiling, reads its config.
        let trapped = command("/usr/bin/git")
            .args(["apply", "--numstat", "-"])
            .current_dir(&inner)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write as _;
                // Git may refuse the config and exit before reading stdin (EPIPE): only its
                // exit status matters here.
                let _written = child.stdin.take().unwrap().write_all(patch);
                child.wait()
            })
            .unwrap();
        assert!(
            !trapped.success(),
            "the candidate config must break an unguarded git apply"
        );
        // Through the door, with the private base inside the candidate, the ceiling holds.
        let (numstat, _summary) = git_facts_in(Path::new("/usr/bin/git"), patch, &inner).unwrap();
        assert_eq!(numstat, b"1\t1\tx\0");
        let _cleaned = std::fs::remove_dir_all(&candidate);
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
        assert_eq!(
            refused.kind,
            super::DoorErrorKind::OverBudget,
            "{}",
            refused.message
        );
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
        copy_out_of_process("/usr/bin/true", &path);
        copy_out_of_process("/usr/bin/false", &dir.join("other"));
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
        copy_out_of_process("/usr/bin/true", &real);
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
        let source = dir.join("script.src");
        std::fs::write(&source, "#!/bin/sh\necho ran\n").unwrap();
        copy_out_of_process(source.to_str().unwrap(), &script);
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let pinned = pin_executable(&script).unwrap();
        // The interpreter starts but cannot reopen the closed descriptor: the script never runs.
        let out = run_child(&spec(Program::Pinned(&pinned), &[], b""));
        assert!(matches!(&out, Err(ChildFailure::Exit(_))), "{out:?}");
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
        assert_eq!(over.kind, DoorErrorKind::OverBudget);
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
