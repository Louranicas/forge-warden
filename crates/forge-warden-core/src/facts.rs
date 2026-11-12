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
    /// A path is not canonical (`./x`, `/x`, `a//b`, `..`), and why (Q-TYPES-2, Q1-05: the reason is
    /// kept as data, not dropped).
    NonCanonicalPath {
        /// The path as Git wrote it.
        path: String,
        /// What makes it non-canonical.
        why: crate::path::PathError,
    },
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
    /// The patch's `diff --git` headers and the paths Git reports are not one for one
    /// `a/PATH b/PATH` (a patch made with `diff.noprefix` or other prefixes, or without Git
    /// headers): Git strips one leading component whatever it is, so the path it reports for
    /// such a patch is not the path the patch was made for. Carries the header or the path.
    HeaderPaths(String),
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
            Self::NonCanonicalPath { path, .. } => write!(f, "path {path:?} is not canonical"),
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
            Self::HeaderPaths(what) => write!(
                f,
                "{what:?} is not named by exactly one `diff --git a/PATH b/PATH` header; make \
                 the patch with `git diff --src-prefix=a/ --dst-prefix=b/`"
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

/// `path` as Git writes it inside double quotes in a header (`quote_c_style`): the named escapes,
/// and three octal digits for every other control byte. Bytes that are not ASCII are octal too,
/// unless `core.quotePath` is off, when Git leaves them as they are (`raw_high`).
fn c_quoted(path: &str, raw_high: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(path.len());
    for byte in path.bytes() {
        match byte {
            0x07 => out.extend_from_slice(b"\\a"),
            0x08 => out.extend_from_slice(b"\\b"),
            b'\t' => out.extend_from_slice(b"\\t"),
            b'\n' => out.extend_from_slice(b"\\n"),
            0x0b => out.extend_from_slice(b"\\v"),
            0x0c => out.extend_from_slice(b"\\f"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b' '..=b'~' => out.push(byte),
            0x80.. if raw_high => out.push(byte),
            _ => out.extend_from_slice(format!("\\{byte:03o}").as_bytes()),
        }
    }
    out
}

/// Believe the paths Git reports only if the patch names each of them itself, with the prefixes
/// Git writes by default (freeze review N3b). `git apply` strips one leading path component
/// whatever it is: for a patch made with `diff.noprefix`, whose header is
/// `diff --git scripts/x scripts/x`, it reports `x`, the ranker agrees, and no path rule
/// matches. So the column-0 `diff --git ` lines and the reported paths must be one for one, and
/// each line must be exactly `a/PATH b/PATH` for its path (`b/PATH a/PATH` from a reverse diff):
/// raw, or with both names quoted as Git quotes them. Nothing is parsed out of a header, so nothing is guessed: a line is one of the
/// expected spellings or the patch is refused. Renames and copies never get here (refused above).
///
/// Not detectable here, by any reader: prefixes that themselves begin `a/` and `b/`
/// (`--src-prefix=a/x/`); such a patch is a well-formed patch for `x/PATH`.
fn check_header_paths(
    patch: &[u8],
    files: &BTreeMap<RepoPath, FileChange>,
) -> Result<(), Unverifiable> {
    let mut expected: BTreeMap<Vec<u8>, &str> = BTreeMap::new();
    for file in files.keys() {
        let reported = file.as_str();
        // `a/ b/`, and `b/ a/`: a reverse diff (`git diff -R`) swaps the two prefixes.
        for (old, new) in [("a/", "b/"), ("b/", "a/")] {
            expected.insert(
                format!("{old}{reported} {new}{reported}").into_bytes(),
                reported,
            );
            for raw_high in [false, true] {
                let quoted = c_quoted(reported, raw_high);
                let mut spelling = format!("\"{old}").into_bytes();
                spelling.extend_from_slice(&quoted);
                spelling.extend_from_slice(format!("\" \"{new}").as_bytes());
                spelling.extend_from_slice(&quoted);
                spelling.push(b'"');
                expected.insert(spelling, reported);
            }
        }
    }
    let mut seen = BTreeSet::new();
    for line in patch.split(|b| *b == b'\n') {
        let Some(header) = line.strip_prefix(b"diff --git ") else {
            continue;
        };
        let refused = || Unverifiable::HeaderPaths(String::from_utf8_lossy(line).into_owned());
        let reported = expected.get(header).ok_or_else(refused)?;
        if !seen.insert(*reported) {
            return Err(refused());
        }
    }
    match files.keys().find(|file| !seen.contains(file.as_str())) {
        Some(unnamed) => Err(Unverifiable::HeaderPaths(unnamed.as_str().to_owned())),
        None => Ok(()),
    }
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
            let repo_path =
                RepoPath::parse(text).map_err(|why| Unverifiable::NonCanonicalPath {
                    path: text.to_owned(),
                    why,
                })?;
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
        check_header_paths(patch, &files)?;
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

    /// The headers Git writes by default for the paths of a numstat, one per record.
    fn default_headers(numstat: &str) -> String {
        let mut headers = String::new();
        for path in numstat
            .split('\0')
            .filter_map(|record| record.splitn(3, '\t').nth(2))
        {
            headers.push_str("diff --git a/");
            headers.push_str(path);
            headers.push_str(" b/");
            headers.push_str(path);
            headers.push('\n');
        }
        headers
    }

    fn facts(numstat: &str, summary: &str) -> Result<PatchFacts, Unverifiable> {
        PatchFacts::from_git(
            numstat.as_bytes(),
            summary.as_bytes(),
            default_headers(numstat).as_bytes(),
        )
    }

    /// Freeze review N3b. Each header below is what Git 2.56 wrote for that setting (the
    /// repository `fixtures/hostile/patch-prefix/` holds whole patches); the numstat is what
    /// `git apply --numstat -z` then reported, or what it would report if it accepted the patch.
    #[test]
    fn a_path_is_believed_only_under_the_default_a_and_b_prefixes() {
        let with = |numstat: &str, patch: &str| {
            PatchFacts::from_git(numstat.as_bytes(), b"", patch.as_bytes())
        };
        let refused = |numstat: &str, patch: &str, what: &str| {
            assert_eq!(
                with(numstat, patch),
                Err(Unverifiable::HeaderPaths(what.to_owned())),
                "{patch:?}"
            );
        };
        // Default prefixes: plain, spaces, a directory literally named `a` or `b`, and each of
        // Git's three spellings of a name it quotes.
        for (path, header) in [
            (
                "scripts/ci_local.sh",
                "a/scripts/ci_local.sh b/scripts/ci_local.sh",
            ),
            ("a b", "a/a b b/a b"),
            ("a/x", "a/a/x b/a/x"),
            ("b/y", "a/b/y b/b/y"),
            ("plain.bin", "b/plain.bin a/plain.bin"),
            (
                "docs/\u{e9}.md",
                "\"b/docs/\\303\\251.md\" \"a/docs/\\303\\251.md\"",
            ),
            (
                "sp ace/my file.txt",
                "a/sp ace/my file.txt b/sp ace/my file.txt",
            ),
            (
                "docs/\u{e9}.md",
                "\"a/docs/\\303\\251.md\" \"b/docs/\\303\\251.md\"",
            ),
            ("docs/\u{e9}.md", "a/docs/\u{e9}.md b/docs/\u{e9}.md"),
            ("q\"uote.txt", "\"a/q\\\"uote.txt\" \"b/q\\\"uote.txt\""),
            (
                "back\\slash.txt",
                "\"a/back\\\\slash.txt\" \"b/back\\\\slash.txt\"",
            ),
            ("tab\there.txt", "\"a/tab\\there.txt\" \"b/tab\\there.txt\""),
            ("del\u{7f}.txt", "\"a/del\\177.txt\" \"b/del\\177.txt\""),
            ("t\t\u{e9}", "\"a/t\\t\u{e9}\" \"b/t\\t\u{e9}\""),
        ] {
            let patch = format!("diff --git {header}\n--- a\n+++ b\n@@ -1 +1 @@\n-x\n+y\n");
            let facts = with(&format!("1\t1\t{path}\0"), &patch).unwrap();
            assert_eq!(
                facts.files().map(|f| f.path().as_str()).collect::<Vec<_>>(),
                [path],
                "{header}"
            );
        }
        // `diff.noprefix`: Git strips `scripts/` and reports `ci_local.sh` (measured: authority
        // mode then passed a change to a protected path).
        refused(
            "2\t0\tci_local.sh\0",
            "diff --git scripts/ci_local.sh scripts/ci_local.sh\n",
            "diff --git scripts/ci_local.sh scripts/ci_local.sh",
        );
        // The same for a file under a directory named `a`: the header looks half right.
        refused("1\t0\tx\0", "diff --git a/x a/x\n", "diff --git a/x a/x");
        // Other prefixes, mnemonic prefixes, one side only, a quoted unprefixed name.
        for (reported, header) in [
            ("f", "old/f new/f"),
            ("f", "i/f w/f"),
            ("f", "c/f o/f"),
            ("f", "a/f f"),
            ("f", "f b/f"),
            ("f", "a/f a/f"),
            ("f", "b/f b/f"),
            ("\u{e9}", "\"docs/\\303\\251\" \"docs/\\303\\251\""),
            ("f", "\"a/f\" b/f"),
            ("f", "a/f b/f "),
            ("f", "a/f  b/f"),
            ("f", "a/f b/f\r"),
            ("f", "a/f\tb/f"),
        ] {
            let line = format!("diff --git {header}");
            refused(&format!("1\t0\t{reported}\0"), &format!("{line}\n"), &line);
        }
        // A header for a path Git does not report, and a header given twice.
        refused(
            "1\t0\tf\0",
            "diff --git a/f b/f\ndiff --git a/g b/g\n",
            "diff --git a/g b/g",
        );
        refused(
            "1\t0\tf\0",
            "diff --git a/f b/f\ndiff --git a/f b/f\n",
            "diff --git a/f b/f",
        );
        // A path Git reports with no Git header at all: a traditional patch, where `-p1` strips
        // whatever the first component is; and one prefixed section beside one unprefixed.
        refused("1\t0\tf\0", "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-x\n+y\n", "f");
        refused(
            concat!("1\t0\tf\0", "1\t0\tg\0"),
            "diff --git a/f b/f\ndiff --git scripts/g scripts/g\n",
            "diff --git scripts/g scripts/g",
        );
        // A `diff --git` line inside a hunk body is content, not a header: it starts with the
        // hunk's own first column.
        let body = "diff --git a/f b/f\n@@ -1,2 +1,3 @@\n diff --git x x\n+diff --git y y\n-diff --git a/f b/f\n";
        assert!(with("1\t1\tf\0", body).is_ok());
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
    fn each_unreadable_answer_from_git_is_refused_with_its_own_variant() {
        // REFUSAL-MAP rows (refusal_rows.rs): each variant observed through from_git, by pattern.
        assert_eq!(facts("", ""), Err(Unverifiable::Empty));
        assert_eq!(
            PatchFacts::from_git(b"1\t1\ta\xff\0", b"", b""),
            Err(Unverifiable::NonUtf8Path)
        );
        assert_eq!(facts("1\t1\ta\nb\0", ""), Err(Unverifiable::NewlineInPath));
        for (summary, path) in [
            (" create mode 100644 b\n", "b"),
            (" create mode 100644 ./a\n", "./a"),
        ] {
            assert_eq!(
                facts("1\t1\ta\0", summary),
                Err(Unverifiable::SummaryPathUnknown(path.to_owned())),
                "{summary}"
            );
        }
        assert_eq!(
            facts(
                "1\t1\ta\0",
                " delete mode 100644 a\n create mode 120000 a\n"
            ),
            Err(Unverifiable::TypeChange("a".to_owned()))
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
    fn a_non_canonical_git_path_keeps_its_reason() {
        // Q-TYPES-2 (Q1-05): the PathError is data, not dropped; the message is unchanged.
        for (path, why) in [
            ("/a", crate::path::PathError::Absolute),
            ("./a", crate::path::PathError::NonCanonical),
        ] {
            let error = facts(&format!("1\t0\t{path}\0"), "").unwrap_err();
            assert_eq!(
                error,
                Unverifiable::NonCanonicalPath {
                    path: path.to_owned(),
                    why
                }
            );
            assert_eq!(error.to_string(), format!("path {path:?} is not canonical"));
        }
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
                Unverifiable::NonCanonicalPath {
                    path: "./a".to_owned(),
                    why: crate::path::PathError::NonCanonical,
                },
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
