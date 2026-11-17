//! What Git says a patch does, and whether a ranking agrees (ADR 0005 §2.2, §2.8, §2.9).
//!
//! The CLI runs `git apply --numstat -z` and `git apply --summary` on the patch bytes (ADR 0010's
//! isolated environment) and passes their output here with the patch itself. Parsing is pure and
//! strict: every form this module cannot read exactly is a refusal ([`Unverifiable`]), never a guess.
//! Facts derived this way are Git's, so they outrank the ranking on file identity and change kind;
//! the ranking still supplies scores and code-evidence signals.

use crate::defaults::NamedDefault;
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
    /// The mode on the `index` line of a binary edit that keeps its mode (`index A..B MODE`): Git's
    /// in bound mode, where the patch is Git's own diff (ADR 0012); the author's text otherwise.
    kept_mode: Option<Mode>,
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
    /// A file/symlink type change: Git deletes the path with one file type and creates it with the
    /// other (numstat lists the path twice; Q60), or a summary names one path more than once.
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
    /// A binary section whose file mode is not stated where Git states it (M7-Q64-MODE, ledger Q64):
    /// an edit that keeps the mode carries it on its `index` line, and a creation, deletion or mode
    /// change carries it on its own header line with no mode on the `index` line. Without it a binary
    /// edit of an executable reads as one of a plain file (Git applies such a patch and keeps the
    /// file executable). Carries the path.
    BinaryModeUnstated(String),
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
            Self::TypeChange(p) => write!(
                f,
                "{p:?} changes type: Git deletes it and creates it again as a symlink or a file; \
                 that change cannot be verified per file"
            ),
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
                 the patch with forge-warden's `scripts/make_patch.sh <base> <head>`"
            ),
            Self::BinaryModeUnstated(p) => write!(
                f,
                "{p:?} is a binary section whose file mode is not stated as Git states it (on the \
                 `index` line for an edit, on its own header line for a creation, deletion or mode \
                 change); make the patch with forge-warden's `scripts/make_patch.sh <base> <head>`"
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
/// and three octal digits for every other control byte. There is no arm for a line feed (`\n`
/// in Git's spelling): a reported path that contains one is refused before any header is read
/// ([`Unverifiable::NewlineInPath`]), so the arm could not be reached or tested. Bytes that are not ASCII are octal too,
/// unless `core.quotePath` is off, when Git leaves them as they are (`raw_high`).
fn c_quoted(path: &str, raw_high: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(path.len());
    for byte in path.bytes() {
        match byte {
            0x07 => out.extend_from_slice(b"\\a"),
            0x08 => out.extend_from_slice(b"\\b"),
            b'\t' => out.extend_from_slice(b"\\t"),
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
    let expected = header_spellings(files);
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

/// Every `diff --git ` header spelling Git writes for the reported paths (the text after
/// `diff --git `), with the path each one names: `a/PATH b/PATH` and the reverse, raw or quoted.
fn header_spellings(files: &BTreeMap<RepoPath, FileChange>) -> BTreeMap<Vec<u8>, &str> {
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
    expected
}

/// What one section's header lines say about its mode.
#[derive(Default)]
struct SectionModes {
    /// The mode of an `index A..B MODE` line (the mode an edit keeps), as written.
    index_mode: Option<String>,
    /// A `new file mode`, `deleted file mode`, `old mode` or `new mode` line.
    mode_header: bool,
    /// Past the section's `GIT binary patch` line: its body, which states no mode.
    in_body: bool,
}

/// A section with neither line (also a path with no section, which [`check_header_paths`] refuses first).
const NO_MODE_LINES: SectionModes = SectionModes {
    index_mode: None,
    mode_header: false,
    in_body: false,
};

/// Each binary section states its file mode where Git states it (M7-Q64-MODE, ledger Q64). Git writes
/// an edit that keeps the mode as `index A..B MODE`, and a creation, deletion or mode change with the
/// mode on its own header line and none on the `index` line (measured, Git 2.56.1,
/// `lanes/bastion/M7-Q64/git_forms.sh`). A patch an author wrote can drop that mode or state another,
/// and `git apply` still applies it and keeps the file's own mode, so a binary edit of an executable
/// would read as one of a plain file. Runs after [`check_header_paths`], so every column-0
/// `diff --git ` line names exactly one reported path. Lines are read as [`check_header_modes`] reads
/// them: hunk lines start with whitespace or `+`/`-`/`\`, and base85 lines have no whitespace. A
/// section is read up to its `GIT binary patch` line: Git writes every header line before the body,
/// so a line after it is not one (Touchstone, M7-Q64-MODE verify).
///
/// Returns the mode each binary edit that keeps its mode states on its `index` line: the
/// `kept_mode` the built-in rule `builtin:binary-on-executable` reads (M7-Q64).
fn check_binary_modes(
    patch: &[u8],
    files: &BTreeMap<RepoPath, FileChange>,
) -> Result<Vec<(RepoPath, Mode)>, Unverifiable> {
    let spellings = header_spellings(files);
    let mut sections: BTreeMap<&str, SectionModes> = BTreeMap::new();
    let mut current: Option<&str> = None;
    for raw in patch.split(|b| *b == b'\n') {
        if let Some(header) = raw.strip_prefix(b"diff --git ") {
            current = spellings.get(header).copied();
            if let Some(path) = current {
                sections.entry(path).or_default();
            }
            continue;
        }
        let Some(section) = current.and_then(|path| sections.get_mut(path)) else {
            continue;
        };
        if section.in_body {
            continue;
        }
        if raw == b"GIT binary patch" {
            section.in_body = true;
            continue;
        }
        let line = String::from_utf8_lossy(raw);
        if line.starts_with(|c: char| c.is_whitespace()) {
            continue;
        }
        match line.split_whitespace().collect::<Vec<_>>().as_slice() {
            ["index", _, stated] => section.index_mode = Some((*stated).to_owned()),
            ["new" | "deleted", "file", "mode", _] | ["old" | "new", "mode", _] => {
                section.mode_header = true;
            }
            _ => {}
        }
    }
    let mut kept = Vec::new();
    for change in files.values().filter(|change| change.binary) {
        let name = change.path.as_str();
        // A path with no `diff --git` section states no mode, which is what `NO_MODE_LINES` records: an
        // edit keeping its mode is then refused, so the default loosens nothing and hides nothing.
        let stated = sections.get(name).or_fail_safe(&NO_MODE_LINES);
        let edit_keeping_mode = change.kind == ChangeKind::Modified && change.new_mode.is_none();
        let as_git_writes = if edit_keeping_mode {
            stated.index_mode.is_some() && !stated.mode_header
        } else {
            stated.index_mode.is_none()
        };
        if !as_git_writes {
            return Err(Unverifiable::BinaryModeUnstated(name.to_owned()));
        }
        if let (true, Some(written)) = (edit_keeping_mode, &stated.index_mode) {
            kept.push((change.path.clone(), mode(written)?));
        }
    }
    Ok(kept)
}

/// Whether a numstat count column is a decimal number as Git writes one: 1 to 20 ASCII digits
/// (20 is the length of the largest 64-bit count).
fn decimal(column: &[u8]) -> bool {
    (1..=20).contains(&column.len()) && column.iter().all(u8::is_ascii_digit)
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
            // Git writes each count as a decimal, or `-` and `-` for a binary file (Q58).
            let binary = match (added, deleted) {
                (b"-", b"-") => true,
                _ if decimal(added) && decimal(deleted) => false,
                _ => {
                    return Err(Unverifiable::UnreadableGitOutput(
                        "numstat counts are not two decimals or `-` and `-`".to_owned(),
                    ));
                }
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
            let change = FileChange {
                path: repo_path.clone(),
                kind: ChangeKind::Modified,
                old_mode: None,
                new_mode: None,
                kept_mode: None,
                binary,
                content: binary || added != b"0" || deleted != b"0",
            };
            if files.insert(repo_path, change).is_some() {
                // Git writes a type change as a deletion and a creation of one path (Q60).
                if summary_type_change(summary, text) {
                    return Err(Unverifiable::TypeChange(text.to_owned()));
                }
                return Err(Unverifiable::RepeatedPath(text.to_owned()));
            }
        }
        if files.is_empty() {
            return Err(Unverifiable::Empty);
        }
        apply_summary(&mut files, summary)?;
        check_header_paths(patch, &files)?;
        for (path, stated) in check_binary_modes(patch, &files)? {
            if let Some(change) = files.get_mut(&path) {
                change.kept_mode = Some(stated);
            }
        }
        Ok(Self { files })
    }

    /// Every file Git sees, by path.
    pub fn files(&self) -> impl Iterator<Item = &FileChange> {
        self.files.values()
    }
}

/// Whether Git's summary deletes `path` with one file type and creates it with the other (a
/// symlink and a regular file), whole-line and byte for byte as [`apply_summary`] reads it: the
/// shape `git diff --no-renames` writes for a type change, whose numstat lists the path twice (Q60).
/// Two regular files of one path (prefixes folded by `--no-prefix`) are not a type change.
fn summary_type_change(summary: &[u8], path: &str) -> bool {
    let mut deleted: Option<Mode> = None;
    let mut created: Option<Mode> = None;
    for raw in summary.split(|b| *b == b'\n') {
        let Ok(line) = std::str::from_utf8(raw) else {
            continue;
        };
        let (slot, rest) = if let Some(rest) = line.strip_prefix(" delete mode ") {
            (&mut deleted, rest)
        } else if let Some(rest) = line.strip_prefix(" create mode ") {
            (&mut created, rest)
        } else {
            continue;
        };
        if let Some((stated, _)) = rest.split_once(' ').filter(|(_, named)| *named == path) {
            *slot = mode(stated).ok();
        }
    }
    match (deleted, created) {
        (Some(old), Some(new)) => (old == Mode::Symlink) != (new == Mode::Symlink),
        _ => false,
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
        kept_mode: _,
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
        kept_mode: _,
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

    /// The file's mode after the change: the mode Git's summary gives a creation or a mode change,
    /// or, for a binary edit that keeps its mode, the mode its `index` line states (Git's own in
    /// bound mode; the author's text otherwise, so a falsely stated `100644` is the limit only bound
    /// mode closes). `None` for a deletion, and for a text edit, whose mode the facts do not read.
    #[must_use]
    pub const fn resulting_mode(&self) -> Option<Mode> {
        match (self.kind, self.new_mode) {
            (ChangeKind::Deleted, _) => None,
            (_, Some(mode)) => Some(mode),
            (_, None) => self.kept_mode,
        }
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
                            kept_mode: None,
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

    /// [`default_headers`], plus the `index` line Git writes under the header of a binary edit that
    /// keeps its mode (a binary record the summary does not name): `check_binary_modes` wants it.
    fn headers_as_git_writes(numstat: &str, summary: &str) -> String {
        let mut out = String::new();
        for record in numstat.split('\0').filter(|r| !r.is_empty()) {
            let mut columns = record.splitn(3, '\t');
            let (Some(added), Some(deleted), Some(path)) =
                (columns.next(), columns.next(), columns.next())
            else {
                continue;
            };
            out.push_str(&default_headers(record));
            let named = summary
                .lines()
                .any(|line| line.ends_with(&format!(" {path}")));
            if added == "-" && deleted == "-" && !named {
                out.push_str("index 1111111..2222222 100644\n");
            }
        }
        out
    }

    fn facts(numstat: &str, summary: &str) -> Result<PatchFacts, Unverifiable> {
        PatchFacts::from_git(
            numstat.as_bytes(),
            summary.as_bytes(),
            headers_as_git_writes(numstat, summary).as_bytes(),
        )
    }

    /// A quoted header is believed only in the spelling Git itself writes for the reported path:
    /// each named escape, and three octal digits for a control byte that has none (the headers are
    /// what Git 2.56 wrote for these file names). A different spelling of the same byte, or two
    /// names that differ, is refused: nothing is unquoted, so nothing is normalised.
    #[test]
    fn a_quoted_header_is_believed_only_in_gits_own_spelling() {
        let with = |path: &str, header: &str| {
            PatchFacts::from_git(
                format!("1\t1\t{path}\0").as_bytes(),
                b"",
                format!("diff --git {header}\n").as_bytes(),
            )
        };
        for (path, header) in [
            ("x\u{7}y", "\"a/x\\ay\" \"b/x\\ay\""),
            ("x\u{8}y", "\"a/x\\by\" \"b/x\\by\""),
            ("x\ty", "\"a/x\\ty\" \"b/x\\ty\""),
            ("x\u{b}y", "\"a/x\\vy\" \"b/x\\vy\""),
            ("x\u{c}y", "\"a/x\\fy\" \"b/x\\fy\""),
            ("x\ry", "\"a/x\\ry\" \"b/x\\ry\""),
            ("x\u{1}y", "\"a/x\\001y\" \"b/x\\001y\""),
            ("x\u{1b}y", "\"a/x\\033y\" \"b/x\\033y\""),
            ("x\"y", "\"a/x\\\"y\" \"b/x\\\"y\""),
            ("x\\y", "\"a/x\\\\y\" \"b/x\\\\y\""),
        ] {
            assert!(with(path, header).is_ok(), "{header}");
        }
        for (path, header) in [
            ("x\u{7}y", "\"a/x\\ay\" \"b/x\\by\""),
            ("x\u{7}y", "\"a/x\\007y\" \"b/x\\007y\""),
            ("x\u{8}y", "\"a/x\\010y\" \"b/x\\010y\""),
            ("x\ty", "\"a/x\\011y\" \"b/x\\011y\""),
            ("x\u{b}y", "\"a/x\\013y\" \"b/x\\013y\""),
            ("x\u{c}y", "\"a/x\\014y\" \"b/x\\014y\""),
            ("x\ry", "\"a/x\\015y\" \"b/x\\015y\""),
            ("x\u{1}y", "\"a/x\\1y\" \"b/x\\1y\""),
            ("x\"y", "\"a/x\\042y\" \"b/x\\042y\""),
            ("x\\y", "\"a/x\\134y\" \"b/x\\134y\""),
            ("xy", "\"a/xy\" \"b/xy\" "),
        ] {
            assert_eq!(
                with(path, header),
                Err(Unverifiable::HeaderPaths(format!("diff --git {header}"))),
                "{header}"
            );
        }
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
        // One dash is not a binary file but an unreadable record (Q58): Git writes both or neither.
        for record in ["-\t3\ta.txt\0", "3\t-\ta.txt\0"] {
            assert!(matches!(
                facts(record, ""),
                Err(Unverifiable::UnreadableGitOutput(_))
            ));
        }
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
    fn a_type_change_is_named_as_one() {
        // Q60: Git's own output for a type change with `--no-renames` (measured, Git 2.56.1): the
        // path twice in numstat, a deletion and a creation in the summary.
        let twice = concat!("0\t1\tl\0", "1\t0\tl\0");
        for summary in [
            " delete mode 100644 l\n create mode 120000 l\n",
            " delete mode 100755 l\n create mode 120000 l\n",
            " delete mode 120000 l\n create mode 100644 l\n",
            " create mode 100644 l\n delete mode 120000 l\n",
        ] {
            assert_eq!(
                facts(twice, summary),
                Err(Unverifiable::TypeChange("l".to_owned())),
                "{summary:?}"
            );
        }
        // Anything else that lists a path twice stays a repeated path.
        for summary in [
            "",
            " delete mode 100644 l\n",
            " create mode 120000 l\n",
            " delete mode 100644 l\n create mode 100644 l\n",
            " delete mode 100644 l\n create mode 100755 l\n",
            " delete mode 120000 l\n create mode 120000 l\n",
            " delete mode 100644 l\n create mode 120000 xl\n",
            " delete mode 100644 xl\n create mode 120000 l\n",
            " delete mode 100644 l\n create mode 120000 l \n",
            " delete mode 100644 l\n create mode 160000 l\n",
            " delete mode 100644 l\n create  mode 120000 l\n",
        ] {
            assert_eq!(
                facts(twice, summary),
                Err(Unverifiable::RepeatedPath("l".to_owned())),
                "{summary:?}"
            );
        }
    }

    #[test]
    fn numstat_counts_are_two_decimals_or_two_dashes() {
        // Q58: Git writes `ADDED\tDELETED\tPATH`, each count a decimal, or `-` `-` for binary.
        for record in [
            "x\t1\ta\0",
            "1\tx\ta\0",
            "\t1\ta\0",
            "1\t\ta\0",
            "-\t1\ta\0",
            "1\t-\ta\0",
            "+1\t0\ta\0",
            "1 \t0\ta\0",
            " 1\t0\ta\0",
            "0x1\t0\ta\0",
            "\u{661}\t0\ta\0",
            "123456789012345678901\t0\ta\0",
            "0\t123456789012345678901\ta\0",
        ] {
            assert_eq!(
                facts(record, ""),
                Err(Unverifiable::UnreadableGitOutput(
                    "numstat counts are not two decimals or `-` and `-`".to_owned()
                )),
                "{record:?}"
            );
        }
        let read = |record: &str| {
            let facts = facts(record, "").unwrap();
            let change = facts.files().next().unwrap().clone();
            (change.binary, change.content)
        };
        assert_eq!(read("0\t0\ta\0"), (false, false));
        assert_eq!(read("12\t0\ta\0"), (false, true));
        assert_eq!(read("0\t3\ta\0"), (false, true));
        assert_eq!(read("-\t-\ta\0"), (true, true));
        assert_eq!(read("12345678901234567890\t0\ta\0"), (false, true));
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
    fn a_binary_section_states_its_mode_as_git_writes_it() {
        // M7-Q64-MODE (ledger Q64). Git 2.56.1's own forms for each kind of binary section
        // (lanes/bastion/M7-Q64/git_forms.sh): an edit keeps its mode on the `index` line; a
        // creation, deletion or mode change has its own mode line and none on the `index` line.
        let run = |path: &str, summary: &str, headers: &str| {
            let patch = format!(
                "diff --git a/{path} b/{path}\n{headers}GIT binary patch\nliteral 3\nKcmZQzXhjE0\n\n"
            );
            PatchFacts::from_git(
                format!("-\t-\t{path}\0").as_bytes(),
                summary.as_bytes(),
                patch.as_bytes(),
            )
        };
        let edit_exec = "index 5c080cc..edafd8e 100755\n";
        let edit_plain = "index fcb17fa..6a0c0ee 100644\n";
        let chmod = "old mode 100644\nnew mode 100755\nindex 0a2aa5b..2ffd657\n";
        let create = "new file mode 100755\nindex 0000000..2c53052\n";
        let delete = "deleted file mode 100644\nindex 0cb3d2f..0000000\n";
        let chmod_summary = " mode change 100644 => 100755 b.bin\n";
        let create_summary = " create mode 100755 b.bin\n";
        let delete_summary = " delete mode 100644 b.bin\n";
        for (summary, headers) in [
            ("", edit_exec),
            ("", edit_plain),
            (chmod_summary, chmod),
            (create_summary, create),
            (delete_summary, delete),
        ] {
            assert!(run("b.bin", summary, headers).is_ok(), "{headers:?}");
        }
        let refused = Err(Unverifiable::BinaryModeUnstated("b.bin".to_owned()));
        for (summary, headers) in [
            // An edit whose `index` line names no mode (Git applies it and keeps an executable
            // executable), or with no `index` line at all.
            ("", "index 5c080cc..edafd8e\n"),
            ("", ""),
            // A mode on the `index` line beside the section's own mode line.
            (
                chmod_summary,
                "old mode 100644\nnew mode 100755\nindex 0a2aa5b..2ffd657 100755\n",
            ),
            (
                create_summary,
                "new file mode 100755\nindex 0000000..2c53052 100755\n",
            ),
            (
                delete_summary,
                "deleted file mode 100644\nindex 0cb3d2f..0000000 100644\n",
            ),
            // An edit with a mode line but no mode change in Git's summary.
            ("", "new mode 100755\nindex 5c080cc..edafd8e 100755\n"),
        ] {
            assert_eq!(run("b.bin", summary, headers), refused, "{headers:?}");
        }
        // Not binary: a text edit is not read here (its mode is the mode grammar's business).
        let text =
            b"diff --git a/t.txt b/t.txt\nindex 1111111..2222222\n--- a/t.txt\n+++ b/t.txt\n";
        assert!(PatchFacts::from_git(b"1\t1\tt.txt\0", b"", text).is_ok());
        // Stated limit: an edit that states the wrong mode (`100644` for an executable) is a
        // well-formed patch; which mode the file really has is known only in bound mode (ADR 0012).
        assert!(run("b.bin", "", edit_plain).is_ok());
        // A base85 line of the binary body has no whitespace, so one that begins `index` is not a header.
        let body = "diff --git a/b.bin b/b.bin\nindex 5c080cc..edafd8e 100755\nGIT binary patch\nliteral 3\nindexKcmZQzXhj\n\n";
        assert!(PatchFacts::from_git(b"-\t-\tb.bin\0", b"", body.as_bytes()).is_ok());
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
                "\"a\" changes type: Git deletes it and creates it again as a symlink or a file; \
                 that change cannot be verified per file",
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
                Unverifiable::BinaryModeUnstated("b.bin".to_owned()),
                "\"b.bin\" is a binary section whose file mode is not stated as Git states it (on the \
                 `index` line for an edit, on its own header line for a creation, deletion or mode \
                 change); make the patch with forge-warden's `scripts/make_patch.sh <base> <head>`",
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
