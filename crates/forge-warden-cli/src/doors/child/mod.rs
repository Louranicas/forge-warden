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

mod entropy;
use entropy::private_suffix;
#[cfg(test)]
#[cfg(not(fw_fixture_cli))]
pub(in crate::doors) use entropy::uuid_suffix;

mod isolation;
pub(in crate::doors) use isolation::Isolation;
#[cfg(test)]
#[cfg(not(fw_fixture_cli))]
pub(in crate::doors) use isolation::{PrivateDir, admit_base};

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
