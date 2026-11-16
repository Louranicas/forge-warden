//! Bound-mode door (ADR 0012 D2, D3): the diff of two commits, made from the objects of a repository
//! the caller names, without trusting that repository's object store.
//!
//! Git hashes an object when it receives it, not when it reads it, so a store can hold bytes other
//! than its ids name. The diff therefore never reads the caller's store. A staging repository with
//! the caller's objects as its one alternate only enumerates and packs; `index-pack` copies the pack
//! into a verified repository with no alternate, naming every object by the hash of its content; the
//! merge base is recomputed there and the diff runs there. forge-warden writes the verified
//! repository's own `shallow` file at the edge of the copy (never a caller's shallow file).

use super::git::AdmittedGit;
use super::{ChildFailure, ChildSpec, DoorError, Isolation, Program, run_child};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Wall-clock budget for one whole bound run (ADR 0012 D3.6; measured worst 1.8 s on a
/// 100k-commit repository with a 99,896-object copy, warm cache).
const BOUND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
/// The largest pack the copy may move: 256 MiB (measured worst 14 MB).
const PACK_CAP: usize = 268_435_456;
/// Object and commit lists Git prints while enumerating (one id per line): 64 MiB.
const LIST_CAP: usize = 67_108_864;
/// Git's stderr is diagnostic only: 64 KiB.
const STDERR_CAP: usize = 65_536;
/// `RLIMIT_DATA` of every bound-mode Git child (measured worst peak RSS 174 MB).
pub(in crate::doors) const DATA_CAP: u64 = 1 << 30;
/// `RLIMIT_AS` of every bound-mode Git child: Git's pack windows and thread stacks count here, so it
/// is wider than the data cap (measured: the worst case passes at 1 GiB and fails at 512 MiB with
/// "unable to create thread").
pub(in crate::doors) const ADDRESS_SPACE_CAP: u64 = 4 << 30;

/// The diff bound mode made, and the merge base it used.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct BoundDiff {
    pub(crate) patch: Vec<u8>,
    pub(crate) merge_base: String,
}

/// Why bound mode could not make the diff.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BoundFailure {
    /// A setup door failed (the private directory, a file it writes): an environment problem.
    Door(DoorError),
    /// The ids do not name what bound mode needs (not commits, no or several merge bases):
    /// a wrong input.
    Input(String),
    /// The object store does not hold what its ids name, or Git refused it: no verdict.
    Store(String),
    /// Git could not run within its bounds (time, output, a signal): no verdict.
    Unavailable(String),
}

/// The repository `dir` names, admitted for bound mode (ADR 0012 D2): its objects directory.
///
/// `dir` must canonicalize to an absolute UTF-8 path with no `:` and no control character (the
/// alternates file is line-based). It is a work tree with a `.git` directory, or a bare repository.
/// Refused: a `.git` file (a linked worktree or submodule pointer), a symlinked `.git` or objects
/// directory, a caller alternates file, and a caller shallow file.
#[expect(
    clippy::disallowed_methods,
    reason = "door: admit_repository (canonicalize and no-follow metadata of the repository the caller names)"
)]
pub(crate) fn admit_repository(dir: &Path) -> Result<PathBuf, DoorError> {
    let refuse = |why: &str| DoorError::environment(format!("--repo {}: {why}", dir.display()));
    let real = std::fs::canonicalize(dir)
        .map_err(|e| DoorError::io("canonicalizing the --repo directory", &e))?;
    let text = real
        .to_str()
        .ok_or_else(|| refuse("the path is not UTF-8"))?;
    if text.contains(':') || text.chars().any(char::is_control) {
        return Err(refuse(
            "the path contains ':' or a control character, which Git's object lists cannot carry",
        ));
    }
    let kind = |path: &Path| std::fs::symlink_metadata(path).ok().map(|m| m.file_type());
    let git_dir = match kind(&real.join(".git")) {
        Some(t) if t.is_dir() => real.join(".git"),
        Some(t) if t.is_file() => {
            return Err(refuse(
                "its .git is a file (a linked worktree or a submodule); pass the repository's git \
                 directory: --repo \"$(git rev-parse --git-common-dir)\"",
            ));
        }
        Some(_) => return Err(refuse("its .git is a symlink")),
        None => {
            let bare = kind(&real.join("objects")).is_some_and(|t| t.is_dir())
                && kind(&real.join("HEAD")).is_some_and(|t| t.is_file());
            if !bare {
                return Err(refuse("not a Git repository (no .git directory, not bare)"));
            }
            real.clone()
        }
    };
    let objects = git_dir.join("objects");
    if !kind(&objects).is_some_and(|t| t.is_dir()) {
        return Err(refuse("its objects directory is missing or a symlink"));
    }
    for name in ["info/alternates", "info/http-alternates"] {
        if kind(&objects.join(name)).is_some() {
            return Err(refuse(&format!(
                "objects/{name} exists: the store borrows objects from elsewhere"
            )));
        }
    }
    if kind(&git_dir.join("shallow")).is_some() {
        return Err(refuse(
            "it is a shallow repository: its history is incomplete",
        ));
    }
    Ok(objects)
}

/// `git diff merge-base..head` of `base` and `head`, made in a verified copy of the objects
/// `objects` holds (ADR 0012 D3), with every option `scripts/make_patch.sh` passes.
#[expect(
    clippy::disallowed_methods,
    reason = "door: bound_diff (the system temp dir)"
)]
pub(crate) fn bound_diff(
    git: &AdmittedGit,
    objects: &Path,
    base: &str,
    head: &str,
) -> Result<BoundDiff, BoundFailure> {
    bound_diff_in(git, objects, base, head, &std::env::temp_dir())
}

/// [`bound_diff`] under the temporary base `tmp` (tests use this).
#[expect(
    clippy::disallowed_methods,
    reason = "door: bound_diff_in (its one deadline for the whole bound run)"
)]
pub(in crate::doors) fn bound_diff_in(
    git: &AdmittedGit,
    objects: &Path,
    base: &str,
    head: &str,
    tmp: &Path,
) -> Result<BoundDiff, BoundFailure> {
    let format = object_format(base, head)?;
    let objects = objects
        .to_str()
        .ok_or_else(|| BoundFailure::Input("the objects path is not UTF-8".to_owned()))?;
    let isolation = Isolation::create_in(tmp).map_err(BoundFailure::Door)?;
    let staging = isolation.dir.path().join("staging.git");
    let verified = isolation.dir.path().join("verified.git");
    // Dropped before `isolation`, so the private directory is empty when it is removed.
    let _repositories = Remove(vec![staging.clone(), verified.clone()]);
    let deadline = std::time::Instant::now()
        .checked_add(BOUND_TIMEOUT)
        .ok_or_else(|| {
            BoundFailure::Unavailable("the clock cannot hold the bound run's deadline".to_owned())
        })?;
    let bound = Bound {
        run: Runner {
            git,
            isolation: &isolation,
            deadline,
        },
        staging: utf8(&staging)?,
        verified: utf8(&verified)?,
    };
    bound.init(format, objects, &staging)?;
    let mb = bound.check_and_merge_base(base, head)?;
    let rows = bound.copy(&mb, base, head)?;
    Bound::cut(&mb, &rows, &verified)?;
    bound.recheck(&mb, base, head)?;
    let patch = bound.diff(&mb, head)?;
    Ok(BoundDiff {
        patch,
        merge_base: mb,
    })
}

/// One bound run: its runner and its two private repositories.
struct Bound<'a> {
    run: Runner<'a>,
    staging: String,
    verified: String,
}

impl Bound<'_> {
    /// Staging and verified, both bare with no template; staging's one alternate is `objects`.
    fn init(&self, format: &str, objects: &str, staging: &Path) -> Result<(), BoundFailure> {
        let object_format = format!("--object-format={format}");
        for dir in [&self.staging, &self.verified] {
            self.run
                .git(
                    &[
                        "init",
                        "--quiet",
                        "--bare",
                        "--template=",
                        &object_format,
                        dir,
                    ],
                    b"",
                    LIST_CAP,
                )
                .map_err(store("git init"))?;
        }
        write_private(
            &staging.join("objects/info/alternates"),
            format!("{objects}\n").as_bytes(),
        )
    }

    /// A staging command: the caller's commit-graph and multi-pack index are never read.
    fn staging(&self, args: &[&str]) -> Vec<String> {
        let mut all = vec![
            format!("--git-dir={}", self.staging),
            "-c".to_owned(),
            "core.commitGraph=false".to_owned(),
            "-c".to_owned(),
            "core.multiPackIndex=false".to_owned(),
        ];
        all.extend(args.iter().map(|a| (*a).to_owned()));
        all
    }

    fn verified(&self, args: &[&str]) -> Vec<String> {
        let mut all = vec![format!("--git-dir={}", self.verified)];
        all.extend(args.iter().map(|a| (*a).to_owned()));
        all
    }

    /// Both ids name commits in staging, and they have exactly one merge base: its id.
    fn check_and_merge_base(&self, base: &str, head: &str) -> Result<String, BoundFailure> {
        for (name, id) in [("base", base), ("head", head)] {
            let kind = self
                .run
                .git_owned(&self.staging(&["cat-file", "-t", id]), b"", LIST_CAP);
            if kind.as_deref() != Ok(b"commit\n".as_slice()) {
                return Err(BoundFailure::Input(format!(
                    "{name} {id} is not a commit in the repository"
                )));
            }
        }
        merge_base(
            &self.run,
            &self.staging(&["merge-base", "--all", base, head]),
        )
    }

    /// Packs the commits of `^mb^@ base head` and the complete trees of `mb` and `head` in staging
    /// and copies them into verified with `index-pack`. Returns the copied commits with their
    /// parents (`rev-list --parents` rows).
    fn copy(&self, mb: &str, base: &str, head: &str) -> Result<Vec<Vec<String>>, BoundFailure> {
        let parents = lines(
            &self
                .run
                .git_owned(
                    &self.staging(&["rev-parse", &format!("{mb}^@")]),
                    b"",
                    LIST_CAP,
                )
                .map_err(store("rev-parse"))?,
        );
        // A root merge base has no parents: `--not` with nothing after it excludes nothing.
        let mut rev_list = self.staging(&["rev-list", "--parents", base, head, "--not"]);
        rev_list.extend(parents);
        let rows: Vec<Vec<String>> = lines(
            &self
                .run
                .git_owned(&rev_list, b"", LIST_CAP)
                .map_err(store("rev-list"))?,
        )
        .iter()
        .map(|row| row.split(' ').map(str::to_owned).collect())
        .collect();
        let trees = lines(
            &self
                .run
                .git_owned(
                    &self.staging(&[
                        "rev-list",
                        "--objects",
                        "--no-object-names",
                        &format!("{mb}^{{tree}}"),
                        &format!("{head}^{{tree}}"),
                    ]),
                    b"",
                    LIST_CAP,
                )
                .map_err(store("rev-list --objects"))?,
        );
        let wanted: BTreeSet<&str> = rows
            .iter()
            .filter_map(|row| row.first().map(String::as_str))
            .chain(trees.iter().map(String::as_str))
            .collect();
        let mut list = wanted.into_iter().collect::<Vec<_>>().join("\n");
        list.push('\n');
        let pack = self
            .run
            .git_owned(
                &self.staging(&["-c", "pack.threads=1", "pack-objects", "--stdout", "-q"]),
                list.as_bytes(),
                PACK_CAP,
            )
            .map_err(store("pack-objects"))?;
        self.run
            .git_owned(&self.verified(&["index-pack", "--stdin"]), &pack, LIST_CAP)
            .map_err(store("index-pack"))?;
        Ok(rows)
    }

    /// forge-warden's own `shallow` file in verified: `mb` and every copied commit with a parent
    /// outside the copy. Every edge left is a re-hashed, real edge; a cut only removes ancestry.
    fn cut(mb: &str, rows: &[Vec<String>], verified: &Path) -> Result<(), BoundFailure> {
        let copied: BTreeSet<&str> = rows
            .iter()
            .filter_map(|row| row.first().map(String::as_str))
            .collect();
        let mut cut: BTreeSet<&str> = rows
            .iter()
            .filter(|row| row.iter().skip(1).any(|p| !copied.contains(p.as_str())))
            .filter_map(|row| row.first().map(String::as_str))
            .collect();
        cut.insert(mb);
        let mut shallow = cut.into_iter().collect::<Vec<_>>().join("\n");
        shallow.push('\n');
        write_private(&verified.join("shallow"), shallow.as_bytes())
    }

    /// The merge base the re-hashed objects give must be the one staging read.
    fn recheck(&self, mb: &str, base: &str, head: &str) -> Result<(), BoundFailure> {
        let again = merge_base(
            &self.run,
            &self.verified(&["merge-base", "--all", base, head]),
        )
        .map_err(|e| match e {
            BoundFailure::Input(why) => BoundFailure::Store(why),
            other @ (BoundFailure::Door(_)
            | BoundFailure::Store(_)
            | BoundFailure::Unavailable(_)) => other,
        })?;
        if again == mb {
            Ok(())
        } else {
            Err(BoundFailure::Store(format!(
                "the merge base read from the repository ({mb}) is not the one its re-hashed objects give ({again})"
            )))
        }
    }

    /// The diff in verified, with every option `scripts/make_patch.sh` passes and `--full-index`;
    /// attributes come from the empty tree of verified's object format.
    fn diff(&self, mb: &str, head: &str) -> Result<Vec<u8>, BoundFailure> {
        let empty = lines(
            &self
                .run
                .git_owned(
                    &self.verified(&["hash-object", "-t", "tree", "--stdin"]),
                    b"",
                    LIST_CAP,
                )
                .map_err(store("hash-object"))?,
        );
        let empty = empty
            .first()
            .ok_or_else(|| BoundFailure::Store("hash-object printed nothing".to_owned()))?;
        let mut args = vec![
            format!("--git-dir={}", self.verified),
            format!("--attr-source={empty}"),
        ];
        for config in [
            "core.attributesFile=/dev/null",
            "core.quotePath=true",
            "core.hooksPath=/dev/null",
            "core.fsmonitor=false",
        ] {
            args.push("-c".to_owned());
            args.push(config.to_owned());
        }
        args.extend(
            [
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
                "--submodule=short",
                "--ignore-submodules=none",
                mb,
                head,
            ]
            .map(str::to_owned),
        );
        self.run
            .git_owned(&args, b"", forge_warden_core::MAX_INPUT_BYTES)
            .map_err(store("git diff"))
    }
}

/// A private path as UTF-8 (the private directory's own name is ASCII under an admitted base).
fn utf8(path: &Path) -> Result<String, BoundFailure> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        BoundFailure::Door(DoorError::environment(
            "private path is not UTF-8".to_owned(),
        ))
    })
}

/// `sha1` or `sha256` from the ids' length: 40 or 64 lowercase hex digits, both the same.
fn object_format(base: &str, head: &str) -> Result<&'static str, BoundFailure> {
    let hex = |id: &str| {
        id.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    };
    match (base.len(), head.len()) {
        (40, 40) if hex(base) && hex(head) => Ok("sha1"),
        (64, 64) if hex(base) && hex(head) => Ok("sha256"),
        _ => Err(BoundFailure::Input(
            "--base and --head must be full object ids of one length (40 or 64 lowercase hex digits)".to_owned(),
        )),
    }
}

/// The single merge base Git prints for `args` (`merge-base --all`), or why there is not one.
fn merge_base(run: &Runner<'_>, args: &[String]) -> Result<String, BoundFailure> {
    let out = match run.git_owned(args, b"", LIST_CAP) {
        Ok(out) => out,
        // `merge-base` exits 1 when the commits share no history.
        Err(ChildFailure::Exit { code: 1, .. }) => {
            return Err(BoundFailure::Input(
                "base and head have no common history".to_owned(),
            ));
        }
        Err(other) => return Err(store("merge-base")(other)),
    };
    match lines(&out).as_slice() {
        [one] => Ok(one.clone()),
        many => Err(BoundFailure::Input(format!(
            "base and head have {} merge bases; name the merge base as --base",
            many.len()
        ))),
    }
}

/// The non-empty lines of Git's output.
fn lines(out: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(out)
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

/// How a failed Git step is reported: a nonzero exit is the store (or Git) refusing; anything
/// else is Git not running within its bounds.
fn store(step: &'static str) -> impl Fn(ChildFailure) -> BoundFailure {
    move |failure| match failure {
        ChildFailure::Exit { code, stderr } => {
            BoundFailure::Store(format!("{step} exited {code}: {stderr}"))
        }
        ChildFailure::Timeout => {
            BoundFailure::Unavailable(format!("{step}: the bound run's time limit passed"))
        }
        ChildFailure::StdoutOverflow => {
            BoundFailure::Unavailable(format!("{step} wrote more than its budget"))
        }
        ChildFailure::StderrOverflow => {
            BoundFailure::Unavailable(format!("{step} wrote too much to stderr"))
        }
        ChildFailure::Signal(signal) => {
            BoundFailure::Unavailable(format!("{step} was killed by signal {signal}"))
        }
        ChildFailure::Spawn(why) | ChildFailure::Io(why) => {
            BoundFailure::Unavailable(format!("{step}: {why}"))
        }
    }
}

/// How every bound-mode Git child runs: the admitted Git under `/usr/bin/prlimit` with the data
/// and address-space caps (Lodestar's ruling on review S2).
pub(in crate::doors) fn bound_program(git: &AdmittedGit) -> Program<'_> {
    Program::Limited {
        pinned: git.executable(),
        data: DATA_CAP,
        address_space: ADDRESS_SPACE_CAP,
    }
}

/// One admitted Git, one private directory, one deadline.
struct Runner<'a> {
    git: &'a AdmittedGit,
    isolation: &'a Isolation,
    deadline: std::time::Instant,
}

impl Runner<'_> {
    fn git(&self, args: &[&str], stdin: &[u8], stdout_cap: usize) -> Result<Vec<u8>, ChildFailure> {
        let env = self.isolation.env();
        run_child(&ChildSpec {
            program: bound_program(self.git),
            args,
            stdin,
            env: &env,
            cwd: self.isolation.dir.path(),
            timeout: self.remaining()?,
            stdout_cap,
            stderr_cap: STDERR_CAP,
        })
        .map(|out| out.stdout)
    }

    fn git_owned(
        &self,
        args: &[String],
        stdin: &[u8],
        stdout_cap: usize,
    ) -> Result<Vec<u8>, ChildFailure> {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        self.git(&args, stdin, stdout_cap)
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "door: Runner::remaining (the time left in the bound run's one deadline)"
    )]
    fn remaining(&self) -> Result<std::time::Duration, ChildFailure> {
        self.deadline
            .checked_duration_since(std::time::Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or(ChildFailure::Timeout)
    }
}

/// Writes `bytes` to `path`, a file inside the run's own private directory (the staging alternates
/// and the verified shallow file). The directory was created exclusively with mode 0700 for this
/// run, and Git created neither file, so nothing that already existed is replaced.
#[expect(
    clippy::disallowed_methods,
    reason = "door: write_private (two files bound mode writes in its own private directory)"
)]
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), BoundFailure> {
    std::fs::write(path, bytes).map_err(|e| {
        BoundFailure::Door(DoorError::io(
            "writing a bound-mode file in the private directory",
            &e,
        ))
    })
}

/// Removes the two repositories bound mode made, when the run ends however it ends. `remove_dir_all`
/// does not follow symlinks (it removes a link, not its target).
struct Remove(Vec<PathBuf>);

impl Drop for Remove {
    #[expect(
        clippy::disallowed_methods,
        reason = "door: Remove::drop (removes the two repositories bound mode created in its private directory)"
    )]
    fn drop(&mut self) {
        for path in &self.0 {
            let _removed = std::fs::remove_dir_all(path);
        }
    }
}
