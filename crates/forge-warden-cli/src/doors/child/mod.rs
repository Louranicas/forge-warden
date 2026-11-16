use super::{DoorError, DoorErrorKind, Symlinks, open_regular, read_within};
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
// Bound mode (ADR 0012): the diff of two commits from a verified copy of a repository's objects.
mod bound;
#[cfg(test)]
#[cfg(not(fw_fixture_cli))]
pub(in crate::doors) use bound::{
    ADDRESS_SPACE_CAP, BoundDiff, DATA_CAP, INIT_FLAGS, bound_diff_in, bound_program, cut_in,
    recheck_in,
};
pub(crate) use bound::{BoundFailure, admit_repository, bound_diff};
// The admission refusals are named only in tests (the refusal map's rows and the door tests).
#[cfg(test)]
#[cfg(not(fw_fixture_cli))]
pub(crate) use bound::RepositoryRefusal;
pub(crate) use git::{
    AdmittedGit, GitFactsFailure, PinnedGit, admit_git_advisory, admit_git_pinned, git_facts,
    git_version,
};
#[cfg(test)]
#[cfg(not(fw_fixture_cli))]
pub(in crate::doors) use git::{GIT_STDERR_CAP, fixture_git, git_facts_in, parse_git_allowlist};

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
    /// A pinned executable run under `/usr/bin/prlimit` with these `RLIMIT_DATA` and `RLIMIT_AS` caps,
    /// set before the executable's first instruction (bound mode, ADR 0012 D3.6). The executable
    /// is reached as `/proc/<this process>/fd/<n>`: the descriptor is close-on-exec, so prlimit,
    /// once it runs, no longer has `/proc/self/fd/<n>`. A missing prlimit fails the spawn: never an
    /// uncapped run.
    Limited {
        pinned: &'a PinnedExecutable,
        data: u64,
        address_space: u64,
    },
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

mod ranker;
use ranker::describe;
#[cfg(test)]
#[cfg(not(fw_fixture_cli))]
pub(in crate::doors) use ranker::{RANKER_ARGS, RANKER_STDERR_CAP};
pub(crate) use ranker::{RankerFailure, produce_ranking};
