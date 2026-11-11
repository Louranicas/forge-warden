//! Git doors: owner-policy admission of the Git executable, and the Git facts and version it reports.

use super::{
    ChildFailure, ChildSpec, DoorError, Isolation, MAX_EXECUTABLE_BYTES, PinnedExecutable, Program,
    Symlinks, describe, open_regular, read_within, run_child,
};
use forge_warden_core::seal::{Sha256Hex, sha256_hex};
use std::path::Path;

#[cfg(not(test))]
pub(super) const GIT_ALLOWLIST: &str = "/etc/forge-warden/git-allowlist.toml";
pub(super) const MAX_GIT_ALLOWLIST_BYTES: usize = 64 * 1024;

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
pub(super) enum GitAdmission {
    Pinned,
    Advisory,
}

/// Ownership boundary for a protected filesystem tree. Production always uses root at `/`.
pub(super) struct GitTrust<'a> {
    root: &'a Path,
    owner: u32,
}

pub(super) fn git_refusal(why: impl std::fmt::Display) -> DoorError {
    DoorError::environment(format!("Git admission: {why}"))
}

/// A closed policy document. TOML rejects duplicate keys; the digest set rejects duplicates.
pub(in crate::doors) fn parse_git_allowlist(
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

pub(super) fn git_metadata(
    meta: &std::fs::Metadata,
    trust: &GitTrust<'_>,
) -> Result<(), DoorError> {
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
pub(super) fn git_parents(path: &Path, trust: &GitTrust<'_>) -> Result<bool, DoorError> {
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

pub(super) fn git_file(
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
pub(super) fn git_policy(
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

pub(super) fn admit_git_with(
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
pub(super) fn admit_git(path: &Path, admission: GitAdmission) -> Result<AdmittedGit, DoorError> {
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
pub(super) fn fixture_git_from_environment(
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
pub(in crate::doors) fn fixture_git(
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

/// [`git_facts`] with an explicit temporary base (tests use this; `git_facts` passes the
/// system temp dir).
pub(in crate::doors) fn git_facts_in(
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

/// Wall-clock limit for one Git call.
pub(super) const GIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// Git's stderr is diagnostic only.
pub(in crate::doors) const GIT_STDERR_CAP: usize = 64 * 1024;
