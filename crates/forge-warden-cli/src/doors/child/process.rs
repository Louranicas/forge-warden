//! Child process doors: executable pinning, the bounded supervisor (`run_child`) and group kill.

use super::{
    ChildFailure, ChildOutput, ChildSpec, DoorError, Program, Symlinks, open_regular, read_within,
};
use forge_warden_core::defaults::NamedDefault as _;
use forge_warden_core::seal::{Sha256Hex, sha256_hex};
use std::io::{Read as _, Write as _};
use std::path::Path;
use std::process::ExitStatus;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// `O_NOFOLLOW` on Linux (`x86_64`, `aarch64`): open refuses a symlink in the final path component.
#[cfg(all(target_os = "linux", test, not(fw_fixture_cli)))]
pub(in crate::doors) const O_NOFOLLOW: i32 = 0o400_000;

/// The largest executable forge-warden will pin (hash) before running it.
pub(in crate::doors) const MAX_EXECUTABLE_BYTES: usize = 512 * 1024 * 1024;

/// How often the parent checks a running child for exit, overflow or timeout.
pub(super) const POLL: std::time::Duration = std::time::Duration::from_millis(5);

/// How [`wait_bounded`] sees time (Q2-14): the time since the child started, and one pause
/// between polls. The door builds the only production clock from `Instant::now` and a real
/// [`POLL`] sleep, inside [`run_child_killing_with`]. A test passes a fake clock, so every deadline
/// and precedence case of the wait runs without a process or a real sleep.
pub(super) struct Clock<E, P> {
    /// Time since the child was started.
    pub(super) elapsed: E,
    /// Wait one poll interval.
    pub(super) pause: P,
}

/// An executable opened once, checked and hashed through that one descriptor. It runs as
/// `/proc/self/fd/<n>` (the same open file), so swapping the path after hashing changes nothing.
#[derive(Debug)]
pub(crate) struct PinnedExecutable {
    pub(super) file: std::fs::File,
    /// SHA-256 of the bytes read through the descriptor.
    pub(crate) sha256: Sha256Hex,
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
pub(in crate::doors) fn pin_within(
    path: &Path,
    budget: usize,
) -> Result<PinnedExecutable, DoorError> {
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

pub(super) fn program_path(program: &Program<'_>) -> std::path::PathBuf {
    use std::os::fd::AsRawFd as _;
    match program {
        #[cfg(test)]
        #[cfg(not(fw_fixture_cli))]
        Program::Absolute(path) => path.to_path_buf(),
        Program::Pinned(pinned) => {
            std::path::PathBuf::from(format!("/proc/self/fd/{}", pinned.file.as_raw_fd()))
        }
    }
}

/// Run one child within its bounds. The child gets its own process group; on every way out
/// (exit, timeout, overflow, error) the whole group is killed and reaped, so a grandchild that
/// holds the pipes cannot outlive the call. This bounds children that stay in the group: it is
/// **not** a sandbox (a child that calls `setsid` escapes; ADR 0005 §2.5).
pub(crate) fn run_child(spec: &ChildSpec<'_>) -> Result<ChildOutput, ChildFailure> {
    run_child_killing_with(spec, Path::new(KILL))
}

/// The program that kills a child's process group.
pub(super) const KILL: &str = "/usr/bin/kill";

/// [`run_child`] with an explicit group-kill program (a test seam for a missing or failing kill).
#[expect(clippy::disallowed_methods, reason = "door: run_child")]
pub(in crate::doors) fn run_child_killing_with(
    spec: &ChildSpec<'_>,
    kill: &Path,
) -> Result<ChildOutput, ChildFailure> {
    use std::os::unix::process::CommandExt as _;
    use std::process::{Command, Stdio};

    let program = program_path(&spec.program);
    if !program.is_absolute() {
        return Err(ChildFailure::Spawn(format!(
            "{} is not an absolute path",
            program.display()
        )));
    }
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

    let clock = Clock {
        elapsed: || started.elapsed(),
        pause: || std::thread::sleep(POLL),
    };
    let stat_path = format!("/proc/{group}/stat");
    let exited = || read_exited(&stat_path);
    let ended = wait_bounded(exited, &out_over, &err_over, &clock, spec.timeout);
    // Whatever happened, nothing in the child's group outlives this call. The leader is not
    // reaped yet (exited means a zombie), so the group still exists and its id cannot have been
    // reused: the kill must succeed, and a failure is reported (Q2-04).
    let killed = kill_group(kill, group);
    // If the group kill failed, the direct child still dies here, so `wait` cannot hang.
    let _killed = child.kill();
    let reaped = child.wait();
    // What is left of the deadline, plus one poll, read afresh at each use.
    let remaining = || {
        spec.timeout
            .saturating_sub((clock.elapsed)())
            .saturating_add(POLL)
    };
    let out_bytes = out_rx.recv_timeout(remaining());
    let err_bytes = err_rx.recv_timeout(POLL.saturating_mul(20));
    let wrote = wrote_rx.recv_timeout(remaining());

    killed.map_err(ChildFailure::Io)?;
    ended?;
    let status = reaped.map_err(|e| ChildFailure::Io(e.to_string()))?;
    if out_over.load(Ordering::SeqCst) {
        return Err(ChildFailure::StdoutOverflow);
    }
    if err_over.load(Ordering::SeqCst) {
        return Err(ChildFailure::StderrOverflow);
    }
    let take = |got: Result<std::io::Result<Vec<u8>>, std::sync::mpsc::RecvTimeoutError>| {
        got.map_err(|_| ChildFailure::Timeout)?
            .map_err(|e| ChildFailure::Io(e.to_string()))
    };
    let captured_stderr = take(err_bytes)?;
    exited_cleanly(status, &captured_stderr)?;
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
    Ok(ChildOutput {
        stdout: take(out_bytes)?,
        stderr: captured_stderr,
    })
}

/// Wait for the child to exit, stopping early on an overflow or the wall-clock limit. `exited`
/// reads whether the child has exited WITHOUT reaping it (`try_wait` would reap), so the caller can
/// still kill the group by the leader's id before the kernel may reuse it (Q2-04). Each poll checks,
/// in this order: stdout overflow, stderr overflow, exit, then the deadline (`elapsed >= timeout`),
/// and only then pauses. The loop has no effect of its own: time and the exit probe come in
/// through `clock` and `exited` (Q2-14).
pub(super) fn wait_bounded<E, P>(
    mut exited: impl FnMut() -> Result<bool, ChildFailure>,
    out_over: &AtomicBool,
    err_over: &AtomicBool,
    clock: &Clock<E, P>,
    timeout: std::time::Duration,
) -> Result<(), ChildFailure>
where
    E: Fn() -> std::time::Duration,
    P: Fn(),
{
    loop {
        if out_over.load(Ordering::SeqCst) {
            return Err(ChildFailure::StdoutOverflow);
        }
        if err_over.load(Ordering::SeqCst) {
            return Err(ChildFailure::StderrOverflow);
        }
        if exited()? {
            return Ok(());
        }
        if (clock.elapsed)() >= timeout {
            return Err(ChildFailure::Timeout);
        }
        (clock.pause)();
    }
}

/// The production exit probe for [`wait_bounded`]: whether the process whose `/proc/<pid>/stat` is
/// at `stat_path` has exited, read without reaping it.
#[expect(
    clippy::disallowed_methods,
    reason = "door: run_child (/proc/<pid>/stat)"
)]
fn read_exited(stat_path: &str) -> Result<bool, ChildFailure> {
    let stat = std::fs::read(stat_path)
        .map_err(|e| ChildFailure::Io(format!("reading {stat_path}: {e}")))?;
    has_exited(&stat)
        .ok_or_else(|| ChildFailure::Io(format!("{stat_path} is not a process status")))
}

/// Whether a `/proc/<pid>/stat` line shows an exited, unreaped process (state `Z` or `X`). The
/// state follows the last `)`, because the command name in parentheses may hold anything.
pub(in crate::doors) fn has_exited(stat: &[u8]) -> Option<bool> {
    let close = stat.iter().rposition(|&b| b == b')')?;
    let rest = stat.get(close.checked_add(1)?..)?;
    let state = rest.iter().find(|b| !b.is_ascii_whitespace())?;
    Some(matches!(state, b'Z' | b'X'))
}

/// `Ok` only for a normal exit with status 0.
pub(super) fn exited_cleanly(status: ExitStatus, stderr: &[u8]) -> Result<(), ChildFailure> {
    use std::os::unix::process::ExitStatusExt as _;
    if let Some(signal) = status.signal() {
        return Err(ChildFailure::Signal(signal));
    }
    match status.code() {
        Some(0) => Ok(()),
        Some(code) => Err(ChildFailure::Exit {
            code,
            stderr: stderr_excerpt(stderr),
        }),
        None => Err(ChildFailure::Io("the child has no exit status".to_owned())),
    }
}

/// First whole characters whose escaped display fits the 512-printable-byte diagnostic
/// budget. Collection remains capped separately at 64 KiB. Lossy decoding is only for
/// diagnostics; it never feeds a verdict parser or a filesystem authority decision.
pub(in crate::doors) fn stderr_excerpt(bytes: &[u8]) -> String {
    let mut excerpt = String::new();
    let mut used = 0_usize;
    for ch in String::from_utf8_lossy(bytes).chars() {
        let cost = forge_warden_core::render::terminal_safe(&ch.to_string()).len();
        if used.saturating_add(cost) > 512 {
            break;
        }
        used = used.saturating_add(cost);
        excerpt.push(ch);
    }
    excerpt
}

/// The group kill's own wall-clock limit: it sends one signal and exits at once.
pub(super) const KILL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Kill every process in `group` with SIGKILL (`<kill> -KILL -- -<group>`); no `unsafe` and no
/// new dependency. Called while the group's leader is unreaped, so the group exists: a kill
/// that cannot start, exits nonzero, or does not finish within [`KILL_TIMEOUT`] (it is a child
/// too, Bastion E3c-O4-01) is an error the caller reports (Q2-04).
#[expect(
    clippy::disallowed_methods,
    reason = "door: kill_group (spawn, poll sleep)"
)]
pub(super) fn kill_group(kill: &Path, group: u32) -> Result<(), String> {
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

/// Q2-14: the wait's deadline and precedence, driven by a fake clock and a scripted exit probe (no
/// process, no real sleep). Each poll checks stdout overflow, then stderr overflow, then exit, then
/// the deadline, and only then pauses one [`POLL`].
#[cfg(test)]
#[cfg(not(fw_fixture_cli))]
mod tests {
    use super::{Clock, POLL, wait_bounded};
    use crate::doors::child::ChildFailure;
    use std::cell::Cell;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    /// A clock that advances exactly one [`POLL`] per pause, counting the pauses.
    struct Fake {
        now: Cell<Duration>,
        pauses: Cell<u32>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                now: Cell::new(Duration::ZERO),
                pauses: Cell::new(0),
            }
        }

        fn clock(&self) -> Clock<impl Fn() -> Duration + '_, impl Fn() + '_> {
            Clock {
                elapsed: || self.now.get(),
                pause: || {
                    self.now.set(self.now.get().saturating_add(POLL));
                    self.pauses.set(self.pauses.get().saturating_add(1));
                },
            }
        }
    }

    /// Run the wait with a probe that reports exit on poll `exit_on` (1-based; 0 never exits).
    /// Returns the outcome, the number of probes and the number of pauses.
    fn run(
        exit_on: u32,
        timeout: Duration,
        stdout_over: &AtomicBool,
        stderr_over: &AtomicBool,
    ) -> (Result<(), ChildFailure>, u32, u32) {
        let fake = Fake::new();
        let probes = Cell::new(0_u32);
        let exited = || {
            probes.set(probes.get().saturating_add(1));
            // A wait that never stops (no deadline, or a clock that never moves) fails fast here
            // instead of hanging the test.
            if probes.get() > 1_000 {
                return Err(ChildFailure::Io("the wait did not stop".to_owned()));
            }
            Ok(exit_on != 0 && probes.get() >= exit_on)
        };
        let ended = wait_bounded(exited, stdout_over, stderr_over, &fake.clock(), timeout);
        (ended, probes.get(), fake.pauses.get())
    }

    fn flags(out: bool, err: bool) -> (AtomicBool, AtomicBool) {
        (AtomicBool::new(out), AtomicBool::new(err))
    }

    #[test]
    fn an_exited_child_ends_the_wait_without_a_pause() {
        let (out, err) = flags(false, false);
        assert_eq!(run(1, POLL.saturating_mul(4), &out, &err), (Ok(()), 1, 0));
    }

    #[test]
    fn a_running_child_is_polled_once_per_pause_until_it_exits() {
        let (out, err) = flags(false, false);
        assert_eq!(run(3, POLL.saturating_mul(4), &out, &err), (Ok(()), 3, 2));
    }

    #[test]
    fn the_deadline_is_reached_when_elapsed_equals_the_timeout() {
        // Polls at 0, 1, 2, 3 and 4 POLL; at 4 POLL elapsed == timeout, so the wait times out
        // there (`>=`, not `>`), after exactly four pauses.
        let (out, err) = flags(false, false);
        assert_eq!(
            run(0, POLL.saturating_mul(4), &out, &err),
            (Err(ChildFailure::Timeout), 5, 4)
        );
        // A zero timeout probes once and then times out without pausing.
        assert_eq!(
            run(0, Duration::ZERO, &out, &err),
            (Err(ChildFailure::Timeout), 1, 0)
        );
    }

    #[test]
    fn exit_on_the_deadline_poll_is_an_exit_not_a_timeout() {
        let (out, err) = flags(false, false);
        assert_eq!(run(5, POLL.saturating_mul(4), &out, &err), (Ok(()), 5, 4));
    }

    #[test]
    fn overflow_beats_exit_and_stdout_overflow_beats_stderr_overflow() {
        let cases = [
            (true, false, ChildFailure::StdoutOverflow),
            (false, true, ChildFailure::StderrOverflow),
            (true, true, ChildFailure::StdoutOverflow),
        ];
        for (stdout, stderr, expected) in cases {
            let (out, err) = flags(stdout, stderr);
            assert_eq!(run(1, POLL, &out, &err), (Err(expected), 0, 0));
        }
    }

    #[test]
    fn an_overflow_during_the_wait_is_seen_on_the_next_poll_before_the_probe() {
        let fake = Fake::new();
        let (out, err) = flags(false, false);
        let probes = Cell::new(0_u32);
        let exited = || {
            probes.set(probes.get().saturating_add(1));
            if probes.get() == 2 {
                err.store(true, Ordering::SeqCst);
            }
            Ok(probes.get() >= 3)
        };
        let ended = wait_bounded(exited, &out, &err, &fake.clock(), POLL.saturating_mul(10));
        assert_eq!(
            (ended, probes.get(), fake.pauses.get()),
            (Err(ChildFailure::StderrOverflow), 2, 2)
        );
    }

    #[test]
    fn a_probe_failure_ends_the_wait_before_the_deadline_check() {
        let fake = Fake::new();
        let (out, err) = flags(false, false);
        let failure = || Err(ChildFailure::Io("reading /proc/1/stat: gone".to_owned()));
        assert_eq!(
            wait_bounded(failure, &out, &err, &fake.clock(), Duration::ZERO),
            Err(ChildFailure::Io("reading /proc/1/stat: gone".to_owned()))
        );
        assert_eq!(fake.pauses.get(), 0);
    }
}
