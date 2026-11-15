//! Ranker door: run the pinned deep-diff-forge and mint the produced ranking (the binary's only mint).

use super::{ChildFailure, ChildSpec, DoorError, Isolation, Program, pin_executable, run_child};
use forge_warden_core::Policy;
use forge_warden_core::authority::{Produced, Ranking};
use forge_warden_core::rank::parse_rank;
use forge_warden_core::seal::sha256_hex;
use std::path::Path;

/// A child failure in words, for a diagnostic.
pub(super) fn describe(failure: &ChildFailure) -> String {
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
pub(in crate::doors) const RANKER_ARGS: [&str; 3] = ["--stdin-patch", "--rank", "--json"];
/// Wall-clock limit for the ranker (ADR 0005 §2.5).
pub(super) const RANKER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// The ranker's stderr is diagnostic only.
pub(in crate::doors) const RANKER_STDERR_CAP: usize = 64 * 1024;

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
pub(in crate::doors) fn produce_ranking_in(
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
