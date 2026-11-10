//! Effect doors (ADR 0001): the only functions in forge-warden that reach outside the process.
//!
//! Each door is one small function carrying `#[expect(clippy::disallowed_methods, reason =
//! "door: …")]`. Adding an effect means adding a door here, in a reviewed diff; the workspace
//! `clippy.toml` makes the same call anywhere else a compile error, and `repo_invariants.rs`
//! fails if any other file opens a door.

use forge_warden_core::{Date, Exit};

/// Why a door could not be passed. It decides the registry exit, in one exhaustive `match`
/// (`Failure::from_door` in main.rs), so a new kind cannot ship unmapped (Q2-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DoorErrorKind {
    /// Reading or writing failed (exit 3, `check_input_output`).
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

/// A harness=false target runs as a no-op during ordinary cargo test discovery. Only fixture
/// commands explicitly supply this test-build-only marker; release builds do not read it.
#[cfg(test)]
#[expect(clippy::disallowed_methods, reason = "test door: fixture CLI dispatch")]
pub(crate) fn fixture_cli_requested() -> bool {
    std::env::var_os("FORGE_WARDEN_TEST_POLICY_ROOT").is_some()
}

mod file;
#[cfg(test)]
#[cfg(not(fw_fixture_cli))]
use file::{ELOOP, FOLLOW_FLAGS, PIN_FLAGS};
use file::{Symlinks, open_regular, read_within};
pub(crate) use file::{read_file, read_stdin};

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

mod diagnose;
pub(crate) use diagnose::{diagnose, diagnose_json, emit};

/// The process status for a registry [`Exit`]: the only place a status is made (R-20). `main`
/// returns a `Status`, so every way the binary ends goes through the exit-code registry.
pub(crate) struct Status(pub(crate) Exit);

impl std::process::Termination for Status {
    fn report(self) -> std::process::ExitCode {
        std::process::ExitCode::from(self.0.entry().code)
    }
}

/// Capture OS arguments once; parsing and machine-output selection remain pure in main.rs.
#[expect(clippy::disallowed_methods, reason = "door: arguments")]
pub(crate) fn arguments() -> Vec<std::ffi::OsString> {
    std::env::args_os().collect()
}

/// Child processes (ADR 0005 §2.4, §2.5): the ranker and Git run through these doors only.
pub(crate) mod child;

#[cfg(test)]
#[cfg(not(fw_fixture_cli))]
#[path = "../../tests/support/process_env.rs"]
mod test_environment;

#[cfg(test)]
#[cfg(not(fw_fixture_cli))]
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

    #[test]
    fn git_allowlist_is_closed_nonempty_and_canonical() {
        let digest = "a".repeat(64);
        let valid = format!(
            "schema = \"forge-warden.git-allowlist.v1\"\nrevision = 1\nsha256 = [\"{digest}\"]\n"
        );
        assert!(super::child::parse_git_allowlist(valid.as_bytes()).is_ok());
        for bad in [
            valid.replace("revision = 1", "revision = 0"),
            valid.replace("revision = 1", "revision = -1"),
            valid.replace("revision = 1", "revision = 1.0"),
            valid.replace("git-allowlist.v1", "git-allowlist.v2"),
            valid.replace(&digest, &"A".repeat(64)),
            valid.replace(&digest, "abcd"),
            valid.replace(&format!("[\"{digest}\"]"), "[]"),
            valid.replace(
                &format!("[\"{digest}\"]"),
                &format!("[\"{digest}\", \"{digest}\"]"),
            ),
            format!("{valid}unexpected = true\n"),
            format!("{valid}revision = 2\n"),
            String::new(),
        ] {
            assert!(
                super::child::parse_git_allowlist(bad.as_bytes()).is_err(),
                "accepted {bad:?}"
            );
        }
        assert!(super::child::parse_git_allowlist(&[255]).is_err());
    }

    #[test]
    fn scratch_calls_never_replace_a_live_directory() {
        let first = scratch("same-label");
        let sentinel = first.join("owned");
        std::fs::write(&sentinel, b"first caller").unwrap();
        let second = scratch("same-label");
        assert_ne!(first, second, "each scratch call owns a distinct path");
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"first caller");
        std::fs::remove_dir_all(first).unwrap();
        std::fs::remove_dir_all(second).unwrap();
    }

    fn concurrent_git_policy_callers(callers: usize) {
        use std::sync::{Arc, Barrier};
        for iteration in 0..100 {
            let start = Arc::new(Barrier::new(callers.checked_add(1).unwrap()));
            let mut workers = Vec::new();
            for _ in 0..callers {
                let start = Arc::clone(&start);
                workers.push(std::thread::spawn(move || {
                    start.wait();
                    test_git()
                }));
            }
            start.wait();
            let results: Vec<_> = workers
                .into_iter()
                .map(std::thread::JoinHandle::join)
                .collect();
            assert!(
                results.iter().all(Result::is_ok),
                "{callers} test_git callers raced at iteration {iteration}"
            );
        }
    }

    #[test]
    fn three_concurrent_git_policy_callers_keep_snapshots_separate() {
        concurrent_git_policy_callers(3);
    }

    #[test]
    fn four_concurrent_git_policy_callers_keep_snapshots_separate() {
        concurrent_git_policy_callers(4);
    }

    fn test_git() -> super::child::AdmittedGit {
        use std::os::unix::fs::MetadataExt as _;
        let root = scratch("git-policy");
        let owner = std::fs::metadata(&root).unwrap().uid();
        let policy = root.join("allowlist.toml");
        let digest = forge_warden_core::seal::sha256_hex(&std::fs::read("/usr/bin/git").unwrap());
        std::fs::write(&policy, format!("schema = \"forge-warden.git-allowlist.v1\"\nrevision = 1\nsha256 = [\"{digest}\"]\n")).unwrap();
        let git =
            super::child::fixture_git(Path::new("/usr/bin/git"), &policy, &root, owner, false)
                .unwrap();
        // Admission owns its descriptor and snapshot; only this call owns the fixture path.
        std::fs::remove_dir_all(&root).unwrap();
        git
    }

    #[test]
    fn git_admission_binds_descriptor_and_exact_policy_bytes() {
        use super::child::{fixture_git, git_version};
        use std::os::unix::fs::MetadataExt as _;
        let root = scratch("git-admission");
        let owner = std::fs::metadata(&root).unwrap().uid();
        let executable = root.join("git");
        std::fs::copy("/usr/bin/git", &executable).unwrap();
        let digest = forge_warden_core::seal::sha256_hex(&std::fs::read(&executable).unwrap());
        let policy = root.join("allowlist.toml");
        let bytes = format!(
            "schema = \"forge-warden.git-allowlist.v1\"\nrevision = 1\nsha256 = [\"{digest}\"]\n"
        );
        std::fs::write(&policy, &bytes).unwrap();
        let git = fixture_git(&executable, &policy, &root, owner, false).unwrap();
        assert_eq!(git.sha256(), &digest);
        assert_eq!(
            git.allowlist_sha256(),
            Some(&forge_warden_core::seal::sha256_hex(bytes.as_bytes()))
        );
        let version = git_version(&git).unwrap();
        std::fs::rename(&executable, root.join("old-git")).unwrap();
        std::fs::copy("/usr/bin/false", &executable).unwrap();
        std::fs::write(&policy, bytes.replace(digest.as_str(), &"0".repeat(64))).unwrap();
        assert_eq!(git_version(&git).unwrap(), version);
        assert!(fixture_git(&executable, &policy, &root, owner, false).is_err());
        // Restoring the executable does not undo owner-policy revocation.
        std::fs::copy("/usr/bin/git", &executable).unwrap();
        assert!(fixture_git(&executable, &policy, &root, owner, false).is_err());
    }

    #[test]
    fn git_admission_refuses_unsafe_inputs_and_authority_never_falls_back() {
        use super::child::fixture_git;
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _, symlink};
        let root = scratch("git-refusals");
        let owner = std::fs::metadata(&root).unwrap().uid();
        let policy = root.join("allowlist.toml");
        let git = Path::new("/usr/bin/git");
        assert!(
            fixture_git(git, &policy, &root, owner, false)
                .unwrap_err()
                .message
                .contains("no Git allowlist enrolled")
        );
        assert!(
            fixture_git(git, &policy, &root, owner, true)
                .unwrap()
                .allowlist_sha256()
                .is_none()
        );
        std::fs::write(&policy, b"not a policy").unwrap();
        assert!(fixture_git(git, &policy, &root, owner, true).is_err());
        let digest = forge_warden_core::seal::sha256_hex(&std::fs::read(git).unwrap());
        let bytes = format!(
            "schema = \"forge-warden.git-allowlist.v1\"\nrevision = 1\nsha256 = [\"{digest}\"]\n"
        );
        std::fs::write(&policy, &bytes).unwrap();
        assert!(fixture_git(git, &policy, &root, owner.checked_add(1).unwrap(), false).is_err());
        std::fs::set_permissions(&policy, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(fixture_git(git, &policy, &root, owner, true).is_err());
        std::fs::set_permissions(&policy, std::fs::Permissions::from_mode(0o600)).unwrap();
        let linked = root.join("linked.toml");
        symlink(&policy, &linked).unwrap();
        assert!(fixture_git(git, &linked, &root, owner, true).is_err());
        std::fs::write(&policy, vec![b' '; 65 * 1024]).unwrap();
        assert!(fixture_git(git, &policy, &root, owner, true).is_err());
        std::fs::write(&policy, bytes.replace(digest.as_str(), &"0".repeat(64))).unwrap();
        assert!(fixture_git(git, &policy, &root, owner, true).is_err());
        let script = root.join("script");
        std::fs::write(&script, b"#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::remove_file(&policy).unwrap();
        assert!(fixture_git(&script, &policy, &root, owner, true).is_err());
    }

    #[test]
    fn git_admission_refuses_fifo_parent_links_writable_executable_and_oversize() {
        use super::child::{MAX_EXECUTABLE_BYTES, fixture_git};
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _, symlink};
        let root = scratch("git-special-files");
        let owner = std::fs::metadata(&root).unwrap().uid();
        let absent = root.join("absent.toml");
        let fifo = root.join("fifo");
        assert!(
            command("/usr/bin/mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        let started = Instant::now();
        assert!(fixture_git(Path::new("/usr/bin/git"), &fifo, &root, owner, true).is_err());
        assert!(
            fixture_git(&fifo, &absent, &root, owner, true)
                .unwrap_err()
                .message
                .contains("not a regular file")
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        let executable = root.join("git");
        std::fs::copy("/usr/bin/git", &executable).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(fixture_git(&executable, &absent, &root, owner, true).is_err());
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(fixture_git(&executable, &absent, &root, owner, true).is_err());
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        let linked = root.join("linked-git");
        symlink(&executable, &linked).unwrap();
        assert!(fixture_git(&linked, &absent, &root, owner, true).is_err());
        let parent = root.join("linked-parent");
        symlink(&root, &parent).unwrap();
        assert!(
            fixture_git(&parent.join("git"), &absent, &root, owner, true)
                .unwrap_err()
                .message
                .contains("parent is not a real directory")
        );
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&executable)
            .unwrap();
        file.set_len(
            u64::try_from(MAX_EXECUTABLE_BYTES)
                .unwrap()
                .saturating_add(1),
        )
        .unwrap();
        drop(file);
        assert!(
            fixture_git(&executable, &absent, &root, owner, true)
                .unwrap_err()
                .message
                .contains("file exceeds byte budget")
        );
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(fixture_git(Path::new("/usr/bin/git"), &absent, &root, owner, true).is_err());
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn git_parent_absence_is_distinct_from_metadata_failure() {
        use super::child::fixture_git;
        use std::os::unix::fs::MetadataExt as _;
        let root = scratch("git-parent-errors");
        let owner = std::fs::metadata(&root).unwrap().uid();
        let executable = Path::new("/usr/bin/git");
        let missing = root.join("missing-parent").join("allowlist.toml");
        let unpinned = fixture_git(executable, &missing, &root, owner, true).unwrap();
        assert_eq!(unpinned.allowlist_sha256(), None);
        assert!(
            fixture_git(executable, &missing, &root, owner, false)
                .unwrap_err()
                .message
                .contains("no Git allowlist enrolled")
        );

        // ENAMETOOLONG is reproducible even as root; permission-denial fixtures are not.
        let impossible_parent = root.join("x".repeat(4096));
        let metadata_error = std::fs::symlink_metadata(&impossible_parent).unwrap_err();
        assert_ne!(metadata_error.kind(), std::io::ErrorKind::NotFound);
        let unreadable = impossible_parent.join("allowlist.toml");
        for advisory in [false, true] {
            let error = fixture_git(executable, &unreadable, &root, owner, advisory).unwrap_err();
            assert!(
                !error.message.contains("no Git allowlist enrolled"),
                "metadata errors must not become absent-policy fallback"
            );
        }
    }

    #[test]
    fn git_policy_budget_is_exactly_64_kib() {
        use super::child::fixture_git;
        use std::os::unix::fs::MetadataExt as _;
        let root = scratch("git-policy-budget");
        let owner = std::fs::metadata(&root).unwrap().uid();
        let path = root.join("allowlist.toml");
        let executable = Path::new("/usr/bin/git");
        let digest = forge_warden_core::seal::sha256_hex(&std::fs::read(executable).unwrap());
        let mut bytes = format!(
            "schema = \"forge-warden.git-allowlist.v1\"\nrevision = 1\nsha256 = [\"{digest}\"]\n#"
        )
        .into_bytes();
        bytes.resize(65536, b' ');
        std::fs::write(&path, &bytes).unwrap();
        assert!(fixture_git(executable, &path, &root, owner, false).is_ok());
        bytes.push(b' ');
        std::fs::write(&path, bytes).unwrap();
        assert!(
            fixture_git(executable, &path, &root, owner, false)
                .unwrap_err()
                .message
                .contains("file exceeds byte budget")
        );
    }

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
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        for _ in 0..128 {
            let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir = std::env::temp_dir()
                .join(format!("fw-child-{}-{name}-{serial}", std::process::id()));
            match std::fs::create_dir(&dir) {
                Ok(()) => return dir,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("creating test scratch {}: {error}", dir.display()),
            }
        }
        panic!("unique test scratch attempts exhausted");
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
            let got = git_facts(&test_git(), &patch);
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
        use super::child::{PrivateDir, git_facts};
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
        assert!(git_facts(&test_git(), tiny_patch).is_ok());
        assert!(
            super::child::admit_git_advisory(Path::new("/nonexistent/git"))
                .unwrap_err()
                .message
                .contains("test fixture policy root is required")
        );
    }

    #[test]
    fn child_exit_keeps_a_bounded_stderr_prefix_and_overflow_still_refuses() {
        use super::child::stderr_excerpt;
        let command = sh("printf 'reason\\033\\000\\n' >&2; exit 7");
        let out = run_child(&spec(
            Program::Absolute(Path::new("/bin/sh")),
            &command,
            b"",
        ));
        assert_eq!(
            out,
            Err(ChildFailure::Exit {
                code: 7,
                stderr: "reason\x1b\0\n".to_owned()
            })
        );
        assert_eq!(stderr_excerpt(&vec![b'x'; 513]), "x".repeat(512));
        assert_eq!(stderr_excerpt(&vec![b'x'; 512]), "x".repeat(512));
        assert_eq!(stderr_excerpt(b""), "");
        for bytes in [
            vec![0; 1024],
            "\u{202e}".repeat(1024).into_bytes(),
            vec![255; 1024],
            vec![b'\\'; 1024],
        ] {
            let safe = forge_warden_core::render::terminal_safe(&stderr_excerpt(&bytes));
            assert!(safe.len() <= 512);
            assert!(safe.bytes().all(|b| (0x20..=0x7e).contains(&b)));
        }
    }

    #[test]
    fn kernel_uuid_admission_refuses_malformed_entropy() {
        use super::child::uuid_suffix;
        let valid = b"12345678-1234-4234-9234-123456789abc\n";
        assert_eq!(
            uuid_suffix(valid).unwrap(),
            "12345678-1234-4234-9234-123456789abc"
        );
        // Exercise both hex boundaries and every RFC variant deterministically; runtime UUIDs
        // alone would make these admission boundaries depend on a random sample.
        for variant in ['8', '9', 'a', 'b'] {
            let value = format!("00000000-ffff-4fff-{variant}fff-ffffffffffff\n");
            assert_eq!(uuid_suffix(value.as_bytes()).unwrap(), value.trim_end());
        }
        for position in [8, 13, 18, 23] {
            let mut malformed = valid.to_vec();
            malformed[position] = b'0';
            assert!(uuid_suffix(&malformed).is_err());
        }
        for bad in [
            b"".as_slice(),
            b"\n",
            b"\xff\n",
            b"12345678-1234-4234-9234-123456789ab\n",
            b"12345678-1234-4234-9234-123456789abcd\n",
            b"12345678-1234-4234-9234-123456789abF\n",
            b"../predictable\n",
            b"12345678-1234-1234-9234-123456789abc\n",
            b"12345678-1234-4234-7234-123456789abc\n",
            b"12345678-1234-4234-9234-123456789abc",
            b"12345678-1234-4234-9234-123456789abg\n",
        ] {
            assert!(uuid_suffix(bad).is_err(), "{bad:?}");
        }
    }

    // ADR0010 temporary-base bullet: reject before creating anything or running a child.
    #[test]
    fn adr0010_relative_tmpdir_refuses_without_fallback() {
        assert!(
            super::child::Isolation::create_in(Path::new("relative/base"))
                .err()
                .unwrap()
                .message
                .contains("not absolute")
        );
    }
    #[test]
    fn adr0010_colon_tmpdir_refuses_without_fallback() {
        let base = scratch("adr0010-colon");
        let path = base.join("a:b");
        std::fs::create_dir(&path).unwrap();
        assert!(
            super::child::Isolation::create_in(&path)
                .err()
                .unwrap()
                .message
                .contains("contains ':'")
        );
        assert_eq!(std::fs::read_dir(&path).unwrap().count(), 0);
    }
    #[test]
    fn adr0010_non_utf8_tmpdir_refuses_without_fallback() {
        use std::os::unix::ffi::OsStrExt as _;
        let base = scratch("adr0010-nonutf8");
        let path = base.join(std::ffi::OsStr::from_bytes(b"bad\xff"));
        std::fs::create_dir(&path).unwrap();
        assert!(
            super::child::Isolation::create_in(&path)
                .err()
                .unwrap()
                .message
                .contains("not UTF-8")
        );
        assert_eq!(std::fs::read_dir(&path).unwrap().count(), 0);
    }
    #[test]
    fn adr0010_absolute_tmpdir_preserves_home_cwd_and_ceiling() {
        let base = scratch("adr0010-positive");
        let isolation = super::child::Isolation::create_in(&base).unwrap();
        let env = isolation.env();
        let home = env.iter().find(|(k, _)| *k == "HOME").unwrap().1;
        let ceiling = env
            .iter()
            .find(|(k, _)| *k == "GIT_CEILING_DIRECTORIES")
            .unwrap()
            .1;
        assert_eq!(isolation.dir.path(), Path::new(home));
        assert_eq!(Path::new(home).parent().unwrap(), Path::new(ceiling));
        assert_eq!(Path::new(ceiling), std::fs::canonicalize(base).unwrap());
    }

    #[test]
    fn predicted_private_names_do_not_exhaust_creation() {
        use super::child::PrivateDir;
        let base = scratch("predicted-names");
        for attempt in 0..64 {
            std::fs::create_dir(
                base.join(format!("forge-warden-{}-{attempt}", std::process::id())),
            )
            .unwrap();
        }
        let dir =
            PrivateDir::create_in(&base).expect("old predictable names must not exhaust creation");
        assert!(dir.path().is_dir());
    }

    #[test]
    fn a_freed_private_dir_gets_a_new_random_name() {
        // Fresh entropy is read for every creation, including after an earlier Drop.
        use super::child::PrivateDir;
        let base = scratch("name-reuse");
        let first = PrivateDir::create_in(&base).unwrap();
        let name = first.path().to_owned();
        drop(first);
        let second = PrivateDir::create_in(&base).unwrap();
        assert_ne!(second.path(), name);
        let live = PrivateDir::create_in(&base).unwrap();
        assert_ne!(live.path(), second.path(), "a live name is never reused");
    }

    #[test]
    fn a_private_dir_skips_taken_names_and_reports_other_errors_at_once() {
        use super::child::{GIT_STDERR_CAP, PrivateDir};
        let base = scratch("private-dir-collision");
        std::fs::create_dir_all(base.join(format!("forge-warden-{}-0", std::process::id())))
            .unwrap();
        let dir = PrivateDir::create_in(&base).unwrap();
        assert!(dir.path().is_dir());
        assert_ne!(
            dir.path(),
            base.join(format!("forge-warden-{}-0", std::process::id()))
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
        let (numstat, _summary) = git_facts_in(&test_git(), patch, &inner).unwrap();
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
            Err(ChildFailure::Exit {
                code: 3,
                stderr: String::new()
            })
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
        assert!(matches!(swapped, Err(ChildFailure::Exit { code: 1, .. })));
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
        assert!(matches!(&out, Err(ChildFailure::Exit { .. })), "{out:?}");
    }
}

#[cfg(test)]
#[cfg(not(fw_fixture_cli))]
mod tests {
    use super::*;

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
}
