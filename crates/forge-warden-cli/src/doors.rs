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

/// A harness=false target runs as a no-op during ordinary cargo test discovery. Only fixture
/// commands explicitly supply this test-build-only marker; release builds do not read it.
#[cfg(test)]
#[expect(clippy::disallowed_methods, reason = "test door: fixture CLI dispatch")]
pub(crate) fn fixture_cli_requested() -> bool {
    std::env::var_os("FORGE_WARDEN_TEST_POLICY_ROOT").is_some()
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
    let _ignored = writeln!(err, "{}", forge_warden_core::render::terminal_safe(text));
}

/// Capture OS arguments once; parsing and machine-output selection remain pure in main.rs.
#[expect(clippy::disallowed_methods, reason = "door: arguments")]
pub(crate) fn arguments() -> Vec<std::ffi::OsString> {
    std::env::args_os().collect()
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
    #[cfg(all(target_os = "linux", test, not(fw_fixture_cli)))]
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

    #[cfg(not(test))]
    const GIT_ALLOWLIST: &str = "/etc/forge-warden/git-allowlist.toml";
    const MAX_GIT_ALLOWLIST_BYTES: usize = 64 * 1024;

    /// One open executable and one admission snapshot. No path-based Git execution is exposed.
    /// An open descriptor pins the inode, not immutable bytes: the owner must not modify it.
    #[derive(Debug)]
    pub(crate) struct AdmittedGit {
        executable: PinnedExecutable,
        path: std::path::PathBuf,
        allowlist_sha256: Option<String>,
    }

    impl AdmittedGit {
        pub(crate) fn sha256(&self) -> &str {
            &self.executable.sha256
        }

        pub(crate) fn allowlist_sha256(&self) -> Option<&str> {
            self.allowlist_sha256.as_deref()
        }
    }

    /// Authority proof: only pinned admission constructs this private-field token.
    #[derive(Debug)]
    pub(crate) struct PinnedGit {
        admitted: AdmittedGit,
    }

    impl PinnedGit {
        pub(crate) fn admitted(&self) -> &AdmittedGit {
            &self.admitted
        }
    }

    #[derive(Clone, Copy)]
    enum GitAdmission {
        Pinned,
        Advisory,
    }

    /// Ownership boundary for a protected filesystem tree. Production always uses root at `/`.
    struct GitTrust<'a> {
        root: &'a Path,
        owner: u32,
    }

    fn git_refusal(why: impl std::fmt::Display) -> DoorError {
        DoorError::environment(format!("Git admission: {why}"))
    }

    /// A closed policy document. TOML rejects duplicate keys; the digest set rejects duplicates.
    pub(super) fn parse_git_allowlist(
        bytes: &[u8],
    ) -> Result<std::collections::BTreeSet<String>, DoorError> {
        let text = std::str::from_utf8(bytes).map_err(git_refusal)?;
        let value: toml::Value = toml::from_str(text).map_err(git_refusal)?;
        let table = value
            .as_table()
            .ok_or_else(|| git_refusal("policy must be a table"))?;
        if table.len() != 3
            || table.get("schema").and_then(toml::Value::as_str)
                != Some("forge-warden.git-allowlist.v1")
            || table
                .get("revision")
                .and_then(toml::Value::as_integer)
                .is_none_or(|r| r <= 0)
        {
            return Err(git_refusal(
                "expected schema, positive revision, and sha256 only",
            ));
        }
        let list = table
            .get("sha256")
            .and_then(toml::Value::as_array)
            .filter(|list| !list.is_empty())
            .ok_or_else(|| git_refusal("sha256 must be a nonempty list"))?;
        let mut digests = std::collections::BTreeSet::new();
        for entry in list {
            let digest = entry
                .as_str()
                .ok_or_else(|| git_refusal("digest must be a string"))?;
            if digest.len() != 64
                || !digest
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                || !digests.insert(digest.to_owned())
            {
                return Err(git_refusal("digests must be unique lowercase SHA-256"));
            }
        }
        Ok(digests)
    }

    fn git_metadata(meta: &std::fs::Metadata, trust: &GitTrust<'_>) -> Result<(), DoorError> {
        use std::os::unix::fs::MetadataExt as _;
        if meta.uid() != trust.owner || meta.mode() & 0o022 != 0 {
            return Err(git_refusal("wrong owner or group/other writable mode"));
        }
        Ok(())
    }

    /// Inspect each lexical parent, including the trust root. Missing protected descendants are
    /// reported separately from unsafe ones; symlinked or writable ancestors never mean absent.
    #[expect(
        clippy::disallowed_methods,
        reason = "door: git_parents (protected ancestors)"
    )]
    fn git_parents(path: &Path, trust: &GitTrust<'_>) -> Result<bool, DoorError> {
        use std::path::Component;
        if !path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            return Err(git_refusal(
                "path must be absolute without relative components",
            ));
        }
        let parent = path
            .parent()
            .ok_or_else(|| git_refusal("path has no parent"))?;
        let relative = parent.strip_prefix(trust.root).map_err(git_refusal)?;
        let mut current = trust.root.to_path_buf();
        let mut directories = vec![current.clone()];
        for component in relative.components() {
            current.push(component);
            directories.push(current.clone());
        }
        for directory in directories {
            let meta = match std::fs::symlink_metadata(&directory) {
                Ok(meta) => meta,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(e) => return Err(git_refusal(e)),
            };
            if !meta.is_dir() {
                return Err(git_refusal("parent is not a real directory"));
            }
            git_metadata(&meta, trust)?;
        }
        Ok(true)
    }

    fn git_file(
        path: &Path,
        trust: &GitTrust<'_>,
        budget: usize,
    ) -> Result<(std::fs::File, Vec<u8>), DoorError> {
        if !git_parents(path, trust)? {
            return Err(git_refusal("file parent does not exist"));
        }
        let (mut file, meta) = open_regular(path, Symlinks::Refuse, "Git admission")
            .map_err(|e| git_refusal(e.message))?;
        git_metadata(&meta, trust)?;
        if meta.len() > u64::try_from(budget).map_err(git_refusal)? {
            return Err(git_refusal("file exceeds byte budget"));
        }
        let bytes =
            read_within(&mut file, "Git admission", budget).map_err(|e| git_refusal(e.message))?;
        Ok((file, bytes))
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "door: git_policy (distinguish absent policy)"
    )]
    fn git_policy(
        path: &Path,
        trust: &GitTrust<'_>,
    ) -> Result<Option<(std::collections::BTreeSet<String>, String)>, DoorError> {
        if !git_parents(path, trust)? {
            return Ok(None);
        }
        match std::fs::symlink_metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(git_refusal(e)),
            Ok(_) => {}
        }
        let (_file, bytes) = git_file(path, trust, MAX_GIT_ALLOWLIST_BYTES)?;
        Ok(Some((parse_git_allowlist(&bytes)?, sha256_hex(&bytes))))
    }

    fn admit_git_with(
        path: &Path,
        policy: &Path,
        trust: &GitTrust<'_>,
        executable_trust: &GitTrust<'_>,
        admission: GitAdmission,
    ) -> Result<AdmittedGit, DoorError> {
        use std::os::unix::fs::MetadataExt as _;
        let snapshot = git_policy(policy, trust)?;
        if snapshot.is_none() && matches!(admission, GitAdmission::Pinned) {
            return Err(git_refusal("no Git allowlist enrolled"));
        }
        let (file, bytes) = git_file(path, executable_trust, MAX_EXECUTABLE_BYTES)?;
        if !bytes.starts_with(b"\x7fELF")
            || file.metadata().map_err(git_refusal)?.mode() & 0o111 == 0
        {
            return Err(git_refusal(
                "only executable ELF artifacts support descriptor execution",
            ));
        }
        let sha256 = sha256_hex(&bytes);
        let allowlist_sha256 = match snapshot {
            Some((digests, hash)) if digests.contains(&sha256) => Some(hash),
            Some(_) => return Err(git_refusal("executable digest is not enrolled")),
            None => None,
        };
        Ok(AdmittedGit {
            executable: PinnedExecutable { file, sha256 },
            path: path.to_path_buf(),
            allowlist_sha256,
        })
    }

    /// The production admission boundary. Its policy path, owner, and trust root are fixed.
    fn admit_git(path: &Path, admission: GitAdmission) -> Result<AdmittedGit, DoorError> {
        #[cfg(test)]
        {
            match fixture_git_from_environment(path, admission) {
                Some(result) => result,
                None => Err(git_refusal("test fixture policy root is required")),
            }
        }
        #[cfg(not(test))]
        {
            let trust = GitTrust {
                root: Path::new("/"),
                owner: 0,
            };
            admit_git_with(path, Path::new(GIT_ALLOWLIST), &trust, &trust, admission)
        }
    }

    /// Authority has no absent-policy path and receives an unforgeable proof token.
    pub(crate) fn admit_git_pinned(path: &Path) -> Result<PinnedGit, DoorError> {
        admit_git(path, GitAdmission::Pinned).map(|admitted| PinnedGit { admitted })
    }

    /// Advisory may record an absent policy, but cannot mint authority proof.
    pub(crate) fn admit_git_advisory(path: &Path) -> Result<AdmittedGit, DoorError> {
        admit_git(path, GitAdmission::Advisory)
    }

    #[cfg(test)]
    #[expect(
        clippy::disallowed_methods,
        reason = "test door: fixture Git policy context"
    )]
    fn fixture_git_from_environment(
        path: &Path,
        admission: GitAdmission,
    ) -> Option<Result<AdmittedGit, DoorError>> {
        use std::os::unix::fs::MetadataExt as _;
        let root = std::path::PathBuf::from(std::env::var_os("FORGE_WARDEN_TEST_POLICY_ROOT")?);
        Some(
            std::fs::metadata(&root)
                .map_err(git_refusal)
                .and_then(|meta| {
                    fixture_git(
                        path,
                        &root.join("allowlist.toml"),
                        &root,
                        meta.uid(),
                        matches!(admission, GitAdmission::Advisory),
                    )
                }),
        )
    }

    /// Only test compilations can supply a policy boundary. No production caller has this seam.
    #[cfg(test)]
    pub(super) fn fixture_git(
        path: &Path,
        policy: &Path,
        root: &Path,
        owner: u32,
        advisory: bool,
    ) -> Result<AdmittedGit, DoorError> {
        let trust = GitTrust { root, owner };
        let executable_trust = if path.starts_with(root) {
            GitTrust { root, owner }
        } else {
            GitTrust {
                root: Path::new("/"),
                owner: 0,
            }
        };
        admit_git_with(
            path,
            policy,
            &trust,
            &executable_trust,
            if advisory {
                GitAdmission::Advisory
            } else {
                GitAdmission::Pinned
            },
        )
    }

    #[test]
    fn git_metadata_requires_the_exact_owner_and_unwritable_ancestors() {
        use std::os::unix::fs::MetadataExt as _;
        let (_file, meta) = open_regular(
            Path::new("/usr/bin/git"),
            Symlinks::Refuse,
            "metadata fixture",
        )
        .unwrap();
        let wrong = GitTrust {
            root: Path::new("/"),
            owner: meta.uid().checked_add(1).unwrap(),
        };
        assert!(git_metadata(&meta, &wrong).is_err());
        let right = GitTrust {
            root: Path::new("/"),
            owner: meta.uid(),
        };
        assert!(git_metadata(&meta, &right).is_ok());
    }

    #[test]
    fn git_file_budget_accepts_exact_size_and_refuses_one_more_before_reading() {
        let path = Path::new("/usr/bin/git");
        let (_file, meta) = open_regular(path, Symlinks::Refuse, "budget fixture").unwrap();
        let budget = usize::try_from(meta.len()).unwrap();
        let trust = GitTrust {
            root: Path::new("/"),
            owner: 0,
        };
        assert!(git_file(path, &trust, budget).is_ok());
        assert!(
            git_file(path, &trust, budget.saturating_sub(1))
                .unwrap_err()
                .message
                .contains("file exceeds byte budget")
        );
    }

    #[test]
    fn git_paths_refuse_parent_traversal_before_metadata_walk() {
        let trust = GitTrust {
            root: Path::new("/"),
            owner: 0,
        };
        for path in [Path::new("/usr/bin/../bin/git"), Path::new("git")] {
            let error = git_parents(path, &trust).unwrap_err();
            assert!(
                error
                    .message
                    .contains("path must be absolute without relative components")
            );
        }
        assert_eq!(git_parents(Path::new("/usr/bin/git"), &trust), Ok(true));
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
        Exit {
            code: i32,
            /// Raw text whose escaped rendering fits 512 bytes. Escaped exactly once at the
            /// diagnostic/output boundary, not here (which would double-escape backslashes).
            stderr: String,
        },
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
        #[cfg(test)]
        #[cfg(not(fw_fixture_cli))]
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

    fn program_path(program: &Program<'_>) -> std::path::PathBuf {
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
    const KILL: &str = "/usr/bin/kill";

    /// [`run_child`] with an explicit group-kill program (a test seam for a missing or failing kill).
    #[expect(clippy::disallowed_methods, reason = "door: run_child")]
    pub(super) fn run_child_killing_with(
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
    fn exited_cleanly(status: ExitStatus, stderr: &[u8]) -> Result<(), ChildFailure> {
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
    pub(super) fn stderr_excerpt(bytes: &[u8]) -> String {
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

    /// Linux generates a fresh random UUID for each read. Reuse the fixed-path, no-follow,
    /// nonblocking regular-file door (procfs reports regular metadata), with a 37-byte budget.
    /// This CLI already requires Linux procfs for child supervision and descriptor pinning.
    /// Missing/malformed entropy refuses; no clock/PID fallback weakens name unpredictability.
    fn private_suffix() -> Result<String, DoorError> {
        let what = "private directory randomness (/proc/sys/kernel/random/uuid)";
        let (file, _) = open_regular(
            Path::new("/proc/sys/kernel/random/uuid"),
            Symlinks::Refuse,
            what,
        )?;
        let bytes = read_within(file, what, 37)?;
        uuid_suffix(&bytes)
    }

    pub(super) fn uuid_suffix(bytes: &[u8]) -> Result<String, DoorError> {
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
            for _attempt in 0..64_u32 {
                let suffix = private_suffix()?;
                let path = base.join(format!("forge-warden-{}-{suffix}", std::process::id()));
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
        /// A setup door failed; preserve its kind for the CLI action hint.
        Door(DoorError),
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
        git: &AdmittedGit,
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
        git: &AdmittedGit,
        patch: &[u8],
        base: &Path,
    ) -> Result<(Vec<u8>, Vec<u8>), GitFactsFailure> {
        let isolation = Isolation::create_in(base).map_err(GitFactsFailure::Door)?;
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
                program: Program::Pinned(&git.executable),
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
                ChildFailure::Exit { code, stderr } => {
                    GitFactsFailure::Refused(format!("git apply exited {code}: {stderr}"))
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
    pub(crate) fn git_version(git: &AdmittedGit) -> Result<String, DoorError> {
        let isolation = Isolation::create_in(&std::env::temp_dir())?;
        let env = isolation.env();
        let refuse = |why: String| DoorError {
            message: format!("{} --version {why}", git.path.display()),
            kind: super::DoorErrorKind::Io,
        };
        let out = run_child(&ChildSpec {
            program: Program::Pinned(&git.executable),
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
            ChildFailure::Exit { code, stderr } => format!("exited {code}: {stderr}"),
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
        // The policy's allowlist mints the token `attest` requires (Q-TYPES-1): no token, no
        // produced ranking, by type.
        let Some(admitted) = policy.admit_producer(&pinned.sha256) else {
            return Err(RankerFailure::NotAdmitted(format!(
                "ranker {} has sha256 {}, which the policy's [producer] sha256 does not list",
                ranker.display(),
                pinned.sha256
            )));
        };
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
        Ok((Ranking::attest(doc, admitted), out.stdout))
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
#[cfg(not(fw_fixture_cli))]
#[path = "../tests/support/process_env.rs"]
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

    fn test_git() -> super::child::AdmittedGit {
        use std::os::unix::fs::MetadataExt as _;
        let root = scratch("git-policy");
        let owner = std::fs::metadata(&root).unwrap().uid();
        let policy = root.join("allowlist.toml");
        let digest = forge_warden_core::seal::sha256_hex(&std::fs::read("/usr/bin/git").unwrap());
        std::fs::write(&policy, format!("schema = \"forge-warden.git-allowlist.v1\"\nrevision = 1\nsha256 = [\"{digest}\"]\n")).unwrap();
        super::child::fixture_git(Path::new("/usr/bin/git"), &policy, &root, owner, false).unwrap()
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
        assert_eq!(git.sha256(), digest);
        assert_eq!(
            git.allowlist_sha256(),
            Some(forge_warden_core::seal::sha256_hex(bytes.as_bytes()).as_str())
        );
        let version = git_version(&git).unwrap();
        std::fs::rename(&executable, root.join("old-git")).unwrap();
        std::fs::copy("/usr/bin/false", &executable).unwrap();
        std::fs::write(&policy, bytes.replace(&digest, &"0".repeat(64))).unwrap();
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
        std::fs::write(&policy, bytes.replace(&digest, &"0".repeat(64))).unwrap();
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
        let base = std::env::temp_dir().join(format!("fw-pdbase-{}", std::process::id()));
        let _gone = std::fs::remove_dir_all(&base);
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
