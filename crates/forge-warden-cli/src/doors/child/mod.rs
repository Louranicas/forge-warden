use super::{DoorError, Symlinks, open_regular, read_within};
use forge_warden_core::Policy;
use forge_warden_core::authority::{Produced, Ranking};
use forge_warden_core::defaults::NamedDefault as _;
use forge_warden_core::rank::parse_rank;
use forge_warden_core::seal::{Sha256Hex, sha256_hex};
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
    pub(crate) sha256: Sha256Hex,
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
    allowlist_sha256: Option<Sha256Hex>,
}

impl AdmittedGit {
    #[cfg(all(test, not(fw_fixture_cli)))]
    pub(crate) fn sha256(&self) -> &Sha256Hex {
        &self.executable.sha256
    }

    #[cfg(all(test, not(fw_fixture_cli)))]
    pub(crate) fn allowlist_sha256(&self) -> Option<&Sha256Hex> {
        self.allowlist_sha256.as_ref()
    }

    /// The verdict's record of this Git: built only here, from this admission and the version
    /// its own descriptor printed (Q-TYPES-2, B2). The constructors are disallowed everywhere else
    /// in the binary (clippy.toml; Q-TYPES-2.1), so this door is the only record of a Git.
    #[expect(
        clippy::disallowed_methods,
        reason = "door: AdmittedGit::oracle records the admitted Git (the only place the binary may)"
    )]
    pub(crate) fn oracle(
        &self,
        path: String,
    ) -> Result<forge_warden_core::verdict::GitOracle, DoorError> {
        use forge_warden_core::verdict::GitOracle;
        let version = git_version(self)?;
        let sha256 = self.executable.sha256.clone();
        Ok(match &self.allowlist_sha256 {
            Some(policy) => GitOracle::approved(path, version, sha256, policy.clone()),
            None => GitOracle::unpinned(path, version, sha256),
        })
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
) -> Result<std::collections::BTreeSet<Sha256Hex>, DoorError> {
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
        let unique = Sha256Hex::parse(digest).is_ok_and(|seal| digests.insert(seal));
        if !unique {
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
) -> Result<Option<(std::collections::BTreeSet<Sha256Hex>, Sha256Hex)>, DoorError> {
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
    if !bytes.starts_with(b"\x7fELF") || file.metadata().map_err(git_refusal)?.mode() & 0o111 == 0 {
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
    let refuse =
        |why: &str| DoorError::environment(format!("temporary directory {}: {why}", raw.display()));
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
            ChildFailure::Timeout => GitFactsFailure::Unavailable("git apply timed out".to_owned()),
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
    let text = String::from_utf8(out.stdout).map_err(|_| refuse("printed non-UTF-8".to_owned()))?;
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
    let doc = parse_rank(&out.stdout)
        .map_err(|e| RankerFailure::Failed(format!("the ranker's output is not a ranking: {e}")))?;
    let patch_sha256 = sha256_hex(patch);
    if *doc.input_sha256() != patch_sha256 {
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
