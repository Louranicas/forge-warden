use super::{DoorError, DoorErrorKind, Symlinks, open_regular, read_within};
use forge_warden_core::Policy;
use forge_warden_core::authority::{Produced, Ranking};
use forge_warden_core::rank::parse_rank;
use forge_warden_core::seal::sha256_hex;
use std::path::Path;

mod process;
pub(in crate::doors) use process::MAX_EXECUTABLE_BYTES;
#[cfg(test)]
#[cfg(not(fw_fixture_cli))]
pub(in crate::doors) use process::{
    O_NOFOLLOW, has_exited, pin_within, run_child_killing_with, stderr_excerpt,
};
pub(crate) use process::{PinnedExecutable, pin_executable, run_child};

mod git;
pub(crate) use git::{
    AdmittedGit, GitFactsFailure, PinnedGit, admit_git_advisory, admit_git_pinned, git_facts,
};
#[cfg(test)]
#[cfg(not(fw_fixture_cli))]
pub(in crate::doors) use git::{
    GIT_STDERR_CAP, fixture_git, git_facts_in, git_version, parse_git_allowlist,
};

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
