//! Repository paths: one canonical spelling per file.
//!
//! Policy globs are root-relative (`src/**`), so a ranked path must be too. `./src/lib.rs`, `/src/lib.rs`,
//! `src/../src/lib.rs` and `src//lib.rs` would each fall outside `src/**` while naming the same file (R-06). A
//! [`RepoPath`] can only be built by [`RepoPath::parse`], which refuses every such spelling instead of repairing it.
//! Other bytes (spaces, tabs, backslashes, quotes, non-ASCII) are kept exactly: they are legal in Git paths.

use std::fmt;

/// A canonical, root-relative repository path: `/`-separated, no empty, `.` or `..` segment.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RepoPath(String);

/// Why a path is not a canonical repository path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    /// The empty string.
    #[error("is empty")]
    Empty,
    /// A NUL byte, which no Git path contains.
    #[error("contains NUL")]
    Nul,
    /// A leading `/`.
    #[error("is absolute; paths are relative to the repository root")]
    Absolute,
    /// An empty segment (`a//b`, a trailing `/`), or a `.` or `..` segment.
    #[error("is not canonical: it has an empty, `.` or `..` segment")]
    NonCanonical,
}

impl RepoPath {
    /// The path, if `text` is already in canonical form.
    ///
    /// # Errors
    ///
    /// [`PathError`] naming the first problem found.
    pub fn parse(text: &str) -> Result<Self, PathError> {
        if text.is_empty() {
            return Err(PathError::Empty);
        }
        if text.contains('\0') {
            return Err(PathError::Nul);
        }
        if text.starts_with('/') {
            return Err(PathError::Absolute);
        }
        if text
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
        {
            return Err(PathError::NonCanonical);
        }
        Ok(Self(text.to_owned()))
    }

    /// The path as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RepoPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn every_non_canonical_spelling_is_refused_with_its_reason() {
        for (text, error) in [
            ("", PathError::Empty),
            ("a\0b", PathError::Nul),
            ("/src/lib.rs", PathError::Absolute),
            ("./src/lib.rs", PathError::NonCanonical),
            ("../src/lib.rs", PathError::NonCanonical),
            ("src/../src/lib.rs", PathError::NonCanonical),
            ("src//lib.rs", PathError::NonCanonical),
            ("src/lib.rs/", PathError::NonCanonical),
            ("src/./lib.rs", PathError::NonCanonical),
            (".", PathError::NonCanonical),
        ] {
            assert_eq!(RepoPath::parse(text), Err(error), "{text:?}");
        }
    }

    #[test]
    fn legal_git_bytes_are_kept_exactly() {
        for text in [
            "src/lib.rs",
            ".github/workflows/ci.yml",
            "dir sp/new\tü.rs",
            "back\\slash.bin",
            "quote\"name",
            "...",
            "a/..b/c.",
        ] {
            assert_eq!(
                RepoPath::parse(text).map(|p| p.as_str().to_owned()),
                Ok(text.to_owned())
            );
        }
    }

    proptest! {
        #[test]
        fn a_parsed_path_has_only_real_segments(text in "[a-z./]{0,12}") {
            if let Ok(path) = RepoPath::parse(&text) {
                prop_assert!(!path.as_str().starts_with('/'));
                prop_assert!(path.as_str().split('/').all(|s| !s.is_empty() && s != "." && s != ".."));
            }
        }
    }
}
