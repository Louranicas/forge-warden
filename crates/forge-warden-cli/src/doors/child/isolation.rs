//! Isolation doors: the private 0700 working directory and the admitted temporary base a child runs in.

use super::{DoorError, private_suffix};
use std::path::Path;

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
    pub(in crate::doors) fn create_in(base: &Path) -> Result<Self, DoorError> {
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
pub(in crate::doors) fn admit_base(raw: &Path) -> Result<String, DoorError> {
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
pub(in crate::doors) struct Isolation {
    pub(in crate::doors) dir: PrivateDir,
    home: String,
    ceiling: String,
}

impl Isolation {
    /// Admit `base` ([`admit_base`]) and create the private directory under it.
    pub(in crate::doors) fn create_in(base: &Path) -> Result<Self, DoorError> {
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
    pub(in crate::doors) fn env(&self) -> [(&str, &str); 7] {
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
