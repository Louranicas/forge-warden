//! What Git says a patch does, and whether a ranking agrees (ADR 0005 §2.2, §2.8, §2.9).
//!
//! The CLI runs `git apply --numstat -z` and `git apply --summary` on the patch bytes (ADR 0010's
//! isolated environment) and passes their output here with the patch itself. Parsing is pure and
//! strict: every form this module cannot read exactly is a refusal ([`Unverifiable`]), never a guess.
//! Facts derived this way are Git's, so they outrank the ranking on file identity and change kind;
//! the ranking still supplies scores and code-evidence signals.

use crate::path::RepoPath;
use crate::rank::{RankDoc, RankedFile};
use crate::vocab::{Signal, Status};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// What happened to a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
// Exhaustive on purpose: Git reports a file as added, deleted or modified. Adding a variant must break every
// consumer's match (Q-TYPES-1; recorded in the public-API snapshot).
pub enum ChangeKind {
    /// The file is new.
    Added,
    /// The file is removed.
    Deleted,
    /// The file exists before and after.
    Modified,
}

/// A Git file mode forge-warden supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
// Exhaustive on purpose: the file modes ADR 0005 §2.2 admits. Adding a variant must break every
// consumer's match (Q-TYPES-1; recorded in the public-API snapshot).
pub enum Mode {
    /// `100644`
    File,
    /// `100755`
    Exec,
    /// `120000`
    Symlink,
}

/// Git's facts about one file in a patch.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FileChange {
    path: RepoPath,
    kind: ChangeKind,
    old_mode: Option<Mode>,
    new_mode: Option<Mode>,
    binary: bool,
    content: bool,
}

/// Git's facts about a whole patch: one record per file, each path once.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PatchFacts {
    files: BTreeMap<RepoPath, FileChange>,
}

/// Why Git's view of a patch cannot be trusted as complete. Each one holds the verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Unverifiable {
    /// Git refused the patch (nonzero exit).
    GitRefused(String),
    /// Git found no file in the patch.
    Empty,
    /// A path is not UTF-8.
    NonUtf8Path,
    /// A path contains a newline (the summary could not be read unambiguously).
    NewlineInPath,
    /// A path is not canonical (`./x`, `/x`, `a//b`, `..`).
    NonCanonicalPath(String),
    /// Git lists one path twice (repeated operations; `--no-prefix` folding).
    RepeatedPath(String),
    /// A summary record names a path the numstat does not.
    SummaryPathUnknown(String),
    /// More than one summary record for one path (a file/symlink type change).
    TypeChange(String),
    /// A rename or a copy: rank `git diff --no-renames` instead.
    RenameOrCopy,
    /// A mode other than `100644`, `100755` or `120000` (a submodule is `160000`).
    UnsupportedMode(String),
    /// A mode-bearing header line (`index`, `new file mode`, `deleted file mode`, `old mode`,
    /// `new mode`) of a shape the grammar does not admit.
    UnreadableModeHeader(String),
    /// An existing symlink's target changes (`index a..b 120000`): Git's summary has no record
    /// for it, so the change kind cannot be tied to a path (Touchstone C1b-O3-01).
    SymlinkTargetEdit,
    /// Output this module does not recognise.
    UnreadableGitOutput(String),
}

impl fmt::Display for Unverifiable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GitRefused(why) => write!(f, "git refused the patch: {why}"),
            Self::Empty => f.write_str("git found no file in the patch"),
            Self::NonUtf8Path => f.write_str("a path is not UTF-8"),
            Self::NewlineInPath => f.write_str("a path contains a newline"),
            Self::NonCanonicalPath(p) => write!(f, "path {p:?} is not canonical"),
            Self::RepeatedPath(p) => write!(f, "git lists {p:?} more than once"),
            Self::SummaryPathUnknown(p) => {
                write!(f, "git's summary names {p:?}, which its numstat does not")
            }
            Self::TypeChange(p) => write!(f, "{p:?} changes type (more than one summary record)"),
            Self::RenameOrCopy => {
                f.write_str("the patch renames or copies a file; rank `git diff --no-renames`")
            }
            Self::UnsupportedMode(m) => {
                write!(
                    f,
                    "unsupported file mode {m} (a submodule or unknown file type)"
                )
            }
            Self::UnreadableModeHeader(line) => write!(f, "unreadable mode header {line:?}"),
            Self::SymlinkTargetEdit => f.write_str(
                "an existing symlink's target changes; that change cannot be verified per file",
            ),
            Self::UnreadableGitOutput(what) => write!(f, "unreadable git output: {what}"),
        }
    }
}

/// Where the ranking and Git disagree.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Disagreement {
    /// Git sees this path; the ranking does not list it.
    Missing(RepoPath),
    /// The ranking lists this path; Git does not see it.
    Extraneous(RepoPath),
    /// Both list it, but the ranking's status or change-kind signals contradict Git.
    Status(RepoPath),
}

const SUPPORTED_MODES: [(&str, Mode); 3] = [
    ("100644", Mode::File),
    ("100755", Mode::Exec),
    ("120000", Mode::Symlink),
];

fn mode(text: &str) -> Result<Mode, Unverifiable> {
    SUPPORTED_MODES
        .iter()
        .find(|(m, _)| *m == text)
        .map(|(_, mode)| *mode)
        .ok_or_else(|| Unverifiable::UnsupportedMode(text.to_owned()))
}

/// Admit every mode a git-format patch states in a header line, by one total grammar
/// (Touchstone C1b-O3-01, C1b-O3-02). A header line starts at column 0 with a keyword; hunk lines
/// start with ` `, `+`, `-` or `\`, and base85 binary-patch lines contain no whitespace, so they
/// never match. Tokens are whitespace runs (Git accepts extra spaces and tabs), and each form has an
/// exact token count:
///
/// - `index A..B` (mode unchanged and unstated) or `index A..B MODE` (mode unchanged);
/// - `new file mode MODE`, `deleted file mode MODE`, `old mode MODE`, `new mode MODE`.
///
/// A keyword line of any other shape is refused, never skipped. An unchanged submodule
/// (`index a..b 160000`) has no `--summary` record (T1); an unchanged symlink is a target edit.
fn check_header_modes(patch: &[u8]) -> Result<(), Unverifiable> {
    for raw in patch.split(|b| *b == b'\n') {
        let line = String::from_utf8_lossy(raw);
        if line.starts_with(|c: char| c.is_whitespace()) {
            continue;
        }
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let unreadable = || Unverifiable::UnreadableModeHeader(line.trim_end().to_owned());
        let stated = match tokens.as_slice() {
            ["index", _] => continue,
            ["index", _, stated] => {
                // A mode on the `index` line means it did not change (creations, deletions and
                // mode changes state theirs on their own header lines), so 120000 here is an edit
                // of an existing symlink's target: a change kind no summary record carries.
                if mode(stated)? == Mode::Symlink {
                    return Err(Unverifiable::SymlinkTargetEdit);
                }
                continue;
            }
            ["new" | "deleted", "file", "mode", stated] | ["old" | "new", "mode", stated] => stated,
            ["index", ..]
            | ["new" | "deleted", "file", "mode", ..]
            | ["old" | "new", "mode", ..] => {
                return Err(unreadable());
            }
            _ => continue,
        };
        mode(stated)?;
    }
    Ok(())
}

fn utf8(bytes: &[u8]) -> Result<&str, Unverifiable> {
    std::str::from_utf8(bytes).map_err(|_| Unverifiable::NonUtf8Path)
}

impl PatchFacts {
    /// Git's facts, from `git apply --numstat -z`, `git apply --summary` and the patch bytes they
    /// were run on.
    ///
    /// # Errors
    ///
    /// [`Unverifiable`] for every form that cannot be read exactly.
    pub fn from_git(numstat_z: &[u8], summary: &[u8], patch: &[u8]) -> Result<Self, Unverifiable> {
        check_header_modes(patch)?;
        let mut files = BTreeMap::new();
        let mut fields = numstat_z.split(|b| *b == 0).peekable();
        while let Some(record) = fields.next() {
            if record.is_empty() {
                if fields.peek().is_none() {
                    break;
                }
                return Err(Unverifiable::UnreadableGitOutput(
                    "empty numstat record".to_owned(),
                ));
            }
            let mut columns = record.splitn(3, |b| *b == b'\t');
            let (Some(added), Some(deleted), Some(path_bytes)) =
                (columns.next(), columns.next(), columns.next())
            else {
                return Err(Unverifiable::UnreadableGitOutput(
                    "short numstat record".to_owned(),
                ));
            };
            if path_bytes.is_empty() {
                // `add\tdel\t\0old\0new\0`: the rename/copy form.
                return Err(Unverifiable::RenameOrCopy);
            }
            let text = utf8(path_bytes)?;
            if text.contains('\n') {
                return Err(Unverifiable::NewlineInPath);
            }
            let repo_path = RepoPath::parse(text)
                .map_err(|_| Unverifiable::NonCanonicalPath(text.to_owned()))?;
            let binary = added == b"-" && deleted == b"-";
            let change = FileChange {
                path: repo_path.clone(),
                kind: ChangeKind::Modified,
                old_mode: None,
                new_mode: None,
                binary,
                content: binary || added != b"0" || deleted != b"0",
            };
            if files.insert(repo_path, change).is_some() {
                return Err(Unverifiable::RepeatedPath(text.to_owned()));
            }
        }
        if files.is_empty() {
            return Err(Unverifiable::Empty);
        }
        apply_summary(&mut files, summary)?;
        Ok(Self { files })
    }

    /// Every file Git sees, by path.
    pub fn files(&self) -> impl Iterator<Item = &FileChange> {
        self.files.values()
    }
}

/// Read `--summary` whole-line by fixed format: `" create mode " MODE " " PATH`,
/// `" delete mode " MODE " " PATH`, `" mode change " MODE " => " MODE " " PATH`, where PATH is the
/// entire rest of the line, compared byte for byte with a numstat path. Never a suffix match.
fn apply_summary(
    files: &mut BTreeMap<RepoPath, FileChange>,
    summary: &[u8],
) -> Result<(), Unverifiable> {
    let mut seen = BTreeSet::new();
    for raw in summary.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        let line = utf8(raw)?;
        let unreadable = || Unverifiable::UnreadableGitOutput(line.to_owned());
        let (kind, old, new, path) = if let Some(rest) = line.strip_prefix(" create mode ") {
            let (stated, path) = rest.split_once(' ').ok_or_else(unreadable)?;
            (Some(ChangeKind::Added), None, Some(mode(stated)?), path)
        } else if let Some(rest) = line.strip_prefix(" delete mode ") {
            let (stated, path) = rest.split_once(' ').ok_or_else(unreadable)?;
            (Some(ChangeKind::Deleted), Some(mode(stated)?), None, path)
        } else if let Some(rest) = line.strip_prefix(" mode change ") {
            // `OLD => NEW PATH`
            let (from, rest) = rest.split_once(" => ").ok_or_else(unreadable)?;
            let (to, path) = rest.split_once(' ').ok_or_else(unreadable)?;
            (None, Some(mode(from)?), Some(mode(to)?), path)
        } else if line.starts_with(" rename ") || line.starts_with(" copy ") {
            return Err(Unverifiable::RenameOrCopy);
        } else {
            return Err(unreadable());
        };
        let repo_path =
            RepoPath::parse(path).map_err(|_| Unverifiable::SummaryPathUnknown(path.to_owned()))?;
        let Some(change) = files.get_mut(&repo_path) else {
            return Err(Unverifiable::SummaryPathUnknown(path.to_owned()));
        };
        if !seen.insert(repo_path) {
            return Err(Unverifiable::TypeChange(path.to_owned()));
        }
        if let Some(kind) = kind {
            change.kind = kind;
        }
        change.old_mode = old;
        change.new_mode = new;
    }
    Ok(())
}

/// The change-kind signals Git's facts imply. A rule sees these **added** to the ranking's
/// signals; Git never removes one (ADR 0005 §2.8, T2).
#[must_use]
pub fn derived_signals(change: &FileChange) -> BTreeSet<Signal> {
    let FileChange {
        path: _,
        kind,
        old_mode,
        new_mode,
        binary,
        content: _,
    } = change;
    let mut out = BTreeSet::new();
    match kind {
        ChangeKind::Added => {
            out.insert(Signal::NewFile);
        }
        ChangeKind::Deleted => {
            out.insert(Signal::DeletedFile);
        }
        ChangeKind::Modified => {}
    }
    if *binary {
        out.insert(Signal::BinaryChange);
    }
    if *old_mode == Some(Mode::Symlink) || *new_mode == Some(Mode::Symlink) {
        out.insert(Signal::SymlinkChange);
    }
    let gained_exec = *new_mode == Some(Mode::Exec)
        && match kind {
            ChangeKind::Added => true,
            ChangeKind::Modified => old_mode.is_some_and(|m| m != Mode::Exec),
            ChangeKind::Deleted => false,
        };
    if gained_exec {
        out.insert(Signal::ExecutableBit);
    }
    out
}

/// Does the ranking's entry agree with Git's facts? The status must be the one the measured
/// producer writes for this kind of change (ADR 0005 §2.9), every derived signal must be present,
/// and no change-kind signal may contradict Git.
fn agrees(change: &FileChange, ranked: &RankedFile) -> bool {
    let FileChange {
        path: _,
        kind,
        old_mode,
        new_mode: _,
        binary,
        content,
    } = change;
    let signals: BTreeSet<&Signal> = ranked.signals().iter().collect();
    let forbidden: &[Signal] = match kind {
        ChangeKind::Added => &[Signal::DeletedFile],
        ChangeKind::Deleted => &[Signal::NewFile],
        ChangeKind::Modified => &[Signal::NewFile, Signal::DeletedFile],
    };
    if forbidden.iter().any(|s| signals.contains(s)) {
        return false;
    }
    if !derived_signals(change).iter().all(|s| signals.contains(s)) {
        return false;
    }
    // For a Modified file, `old_mode` is set only by a `mode change` record, which Git writes only
    // when the two modes differ.
    let mode_changed = old_mode.is_some();
    let status = match (kind, *binary, mode_changed, *content) {
        (ChangeKind::Added | ChangeKind::Deleted | ChangeKind::Modified, true, _, _) => {
            Status::BinaryChanged
        }
        (ChangeKind::Added, false, _, _) => Status::Added,
        (ChangeKind::Deleted, false, _, _) => Status::Deleted,
        (ChangeKind::Modified, false, true, false) => Status::ModeChanged,
        (ChangeKind::Modified, false, _, _) => Status::Modified,
    };
    *ranked.status() == status
}

/// Where `rank` and Git's `facts` disagree, in path order. Empty means the ranking covers the
/// patch exactly and agrees with Git on every file it lists.
#[must_use]
pub fn compare(rank: &RankDoc, facts: &PatchFacts) -> Vec<Disagreement> {
    let mut ranked: BTreeMap<&RepoPath, &RankedFile> = BTreeMap::new();
    for file in rank.files() {
        ranked.entry(file.path()).or_insert(file);
    }
    let mut out = Vec::new();
    for (path, change) in &facts.files {
        match ranked.get(path) {
            None => out.push(Disagreement::Missing(path.clone())),
            Some(file) if !agrees(change, file) => out.push(Disagreement::Status(path.clone())),
            Some(_) => {}
        }
    }
    for path in ranked.keys() {
        if !facts.files.contains_key(*path) {
            out.push(Disagreement::Extraneous((*path).clone()));
        }
    }
    out.sort();
    out
}

impl FileChange {
    /// The file's path.
    #[must_use]
    pub const fn path(&self) -> &RepoPath {
        &self.path
    }

    /// What happened to it.
    #[must_use]
    pub const fn kind(&self) -> ChangeKind {
        self.kind
    }

    /// Whether Git treats it as binary.
    #[must_use]
    pub const fn binary(&self) -> bool {
        self.binary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_derived_signal_is_a_non_empty_constant() {
        // Zen B4: `RankedFile::with_added_signals` is infallible because an added signal can never
        // be empty. Every combination of kind, modes and binary is enumerated here.
        let kinds = [ChangeKind::Added, ChangeKind::Deleted, ChangeKind::Modified];
        let modes = [
            None,
            Some(Mode::File),
            Some(Mode::Exec),
            Some(Mode::Symlink),
        ];
        let mut seen = BTreeSet::new();
        for kind in kinds {
            for old_mode in modes {
                for new_mode in modes {
                    for binary in [false, true] {
                        let change = FileChange {
                            path: RepoPath::parse("a").unwrap(),
                            kind,
                            old_mode,
                            new_mode,
                            binary,
                            content: true,
                        };
                        for signal in derived_signals(&change) {
                            // Q-TYPES-2: a derived signal is a known vocabulary value, by type.
                            assert!(signal.is_known(), "{change:?}");
                            assert!(!signal.name().is_empty(), "{change:?}");
                            seen.insert(signal);
                        }
                    }
                }
            }
        }
        assert!(seen.len() >= 4, "too few derived signals reached: {seen:?}");
    }

    fn facts(numstat: &str, summary: &str) -> Result<PatchFacts, Unverifiable> {
        PatchFacts::from_git(numstat.as_bytes(), summary.as_bytes(), b"")
    }

    fn one(numstat: &str, summary: &str) -> FileChange {
        facts(numstat, summary)
            .unwrap()
            .files()
            .next()
            .unwrap()
            .clone()
    }

    #[test]
    fn binary_means_both_columns_are_dashes() {
        assert!(one("-\t-\ta.bin\0", "").binary());
        assert!(!one("-\t3\ta.txt\0", "").binary());
        assert!(!one("3\t-\ta.txt\0", "").binary());
        let change = one("2\t1\tsrc/a.rs\0", "");
        assert_eq!(
            (change.path().as_str(), change.kind()),
            ("src/a.rs", ChangeKind::Modified)
        );
    }

    #[test]
    fn renames_and_copies_refuse_in_either_output() {
        assert_eq!(facts("1\t1\t\0a\0b\0", ""), Err(Unverifiable::RenameOrCopy));
        for line in [" rename a => b (100%)\n", " copy a => b (100%)\n"] {
            assert_eq!(
                facts("1\t1\ta\0", line),
                Err(Unverifiable::RenameOrCopy),
                "{line}"
            );
        }
        assert!(matches!(
            facts("1\t1\ta\0", " chmod a\n"),
            Err(Unverifiable::UnreadableGitOutput(_))
        ));
    }

    #[test]
    fn derived_signals_follow_git() {
        let symlink = one("1\t0\tlink\0", " create mode 120000 link\n");
        assert_eq!(
            derived_signals(&symlink)
                .iter()
                .map(Signal::name)
                .collect::<Vec<_>>(),
            ["new_file", "symlink_change"]
        );
        let gained = one("0\t0\trun.sh\0", " mode change 100644 => 100755 run.sh\n");
        assert_eq!(
            derived_signals(&gained)
                .iter()
                .map(Signal::name)
                .collect::<Vec<_>>(),
            ["executable_bit"]
        );
        let lost = one("0\t0\trun.sh\0", " mode change 100755 => 100644 run.sh\n");
        assert_eq!(derived_signals(&lost).len(), 0);
        let deleted_link = one("0\t1\tl\0", " delete mode 120000 l\n");
        assert_eq!(
            derived_signals(&deleted_link)
                .iter()
                .map(Signal::name)
                .collect::<Vec<_>>(),
            ["deleted_file", "symlink_change"]
        );
    }

    #[test]
    fn an_existing_symlink_target_edit_is_refused_not_called_covered() {
        // Touchstone C1b-O3-01: numstat 1/1, empty summary, the mode only on the index line.
        let patch =
            b"diff --git a/link b/link\nindex 094df80..c907aac 120000\n--- a/link\n+++ b/link\n";
        assert_eq!(
            PatchFacts::from_git(b"1\t1\tlink\0", b"", patch),
            Err(Unverifiable::SymlinkTargetEdit)
        );
        // Siblings stay modelled: an unchanged executable edited in place (no change-kind signal
        // depends on it) and a mode-only change (its mode is on old/new mode lines).
        let exec = b"diff --git a/run.sh b/run.sh\nindex 094df80..c907aac 100755\n";
        let edited = PatchFacts::from_git(b"1\t1\trun.sh\0", b"", exec).unwrap();
        let change = edited.files().next().unwrap();
        assert_eq!(derived_signals(change).len(), 0);
        let mode_only = b"diff --git a/run.sh b/run.sh\nold mode 100644\nnew mode 100755\n";
        let changed = PatchFacts::from_git(
            b"0\t0\trun.sh\0",
            b" mode change 100644 => 100755 run.sh\n",
            mode_only,
        )
        .unwrap();
        assert_eq!(
            derived_signals(changed.files().next().unwrap())
                .iter()
                .map(Signal::name)
                .collect::<Vec<_>>(),
            ["executable_bit"]
        );
    }

    #[test]
    fn every_mode_bearing_header_is_admitted_by_one_total_grammar() {
        let check = |header: &str| check_header_modes(header.as_bytes());
        // Git-accepted whitespace never skips admission (Touchstone C1b-O3-02).
        for spelling in [
            "index 3fcd8c9..905671c  160000",
            "index 3fcd8c9..905671c\t160000",
            "index 3fcd8c9..905671c 160000 ",
            "new file mode\t160000",
            "deleted  file mode 160000",
            "old mode 160000",
        ] {
            assert_eq!(
                check(spelling),
                Err(Unverifiable::UnsupportedMode("160000".to_owned())),
                "{spelling:?}"
            );
        }
        assert_eq!(
            check("index a..b  120000"),
            Err(Unverifiable::SymlinkTargetEdit)
        );
        // Positive controls: supported modes in any Git spelling are admitted.
        for ok in [
            "index a..b",
            "index a..b  100644",
            "index a..b\t100755 ",
            "new file mode\t100755",
            "new mode 100644",
            " index a..b 160000",
            "+index a..b 160000",
            "Some text that is not a header",
        ] {
            assert_eq!(check(ok), Ok(()), "{ok:?}");
        }
        // A keyword line of any other shape is refused, never skipped.
        for odd in [
            "index",
            "index a..b 100644 extra",
            "new file mode",
            "old mode 100644 100755",
        ] {
            assert!(
                matches!(check(odd), Err(Unverifiable::UnreadableModeHeader(_))),
                "{odd:?}"
            );
        }
    }

    #[test]
    fn every_refusal_says_why() {
        let cases = [
            (
                Unverifiable::GitRefused("x".to_owned()),
                "git refused the patch: x",
            ),
            (Unverifiable::Empty, "git found no file in the patch"),
            (Unverifiable::NonUtf8Path, "a path is not UTF-8"),
            (Unverifiable::NewlineInPath, "a path contains a newline"),
            (
                Unverifiable::NonCanonicalPath("./a".to_owned()),
                "path \"./a\" is not canonical",
            ),
            (
                Unverifiable::RepeatedPath("a".to_owned()),
                "git lists \"a\" more than once",
            ),
            (
                Unverifiable::SummaryPathUnknown("a".to_owned()),
                "git's summary names \"a\", which its numstat does not",
            ),
            (
                Unverifiable::TypeChange("a".to_owned()),
                "\"a\" changes type (more than one summary record)",
            ),
            (
                Unverifiable::RenameOrCopy,
                "the patch renames or copies a file; rank `git diff --no-renames`",
            ),
            (
                Unverifiable::UnsupportedMode("160000".to_owned()),
                "unsupported file mode 160000 (a submodule or unknown file type)",
            ),
            (
                Unverifiable::SymlinkTargetEdit,
                "an existing symlink's target changes; that change cannot be verified per file",
            ),
            (
                Unverifiable::UnreadableModeHeader("index a b c d".to_owned()),
                "unreadable mode header \"index a b c d\"",
            ),
            (
                Unverifiable::UnreadableGitOutput("q".to_owned()),
                "unreadable git output: q",
            ),
        ];
        for (refusal, text) in cases {
            assert_eq!(refusal.to_string(), text);
        }
    }
}
