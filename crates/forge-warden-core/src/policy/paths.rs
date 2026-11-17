//! Path globs: compiled, `/`-aware, and validated when the policy loads.

use super::glob::{self, Glob};
use super::issues::{Issues, Rejected};
use alloc::{borrow::ToOwned, format, string::String, vec::Vec};

/// A compiled set of globs, keeping the patterns for reporting.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PathSet {
    patterns: Vec<String>,
    globs: Vec<Glob>,
}

impl PathSet {
    /// Whether `path` matches any pattern.
    #[must_use]
    pub fn matches(&self, path: &str) -> bool {
        self.globs.iter().any(|glob| glob.matches(path))
    }

    /// The patterns as written.
    #[must_use]
    pub fn patterns(&self) -> &[String] {
        &self.patterns
    }
}

/// Files a rule looks at: in `paths` and not in `except`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Scope {
    pub(super) paths: PathSet,
    pub(super) except: Option<PathSet>,
}

impl Scope {
    /// Whether `path` is in scope.
    #[must_use]
    pub fn contains(&self, path: &str) -> bool {
        self.paths.matches(path) && !self.except.as_ref().is_some_and(|ex| ex.matches(path))
    }
}

/// Compile globs with `/`-aware semantics: `*` stays inside one path segment, `**` crosses them.
pub(super) fn compile(
    location: &str,
    field: &str,
    globs: &[String],
    issues: &mut Issues,
) -> Result<PathSet, Rejected> {
    if globs.is_empty() {
        return Err(issues.reject(location, format!("`{field}` must not be empty")));
    }
    let start = issues.mark();
    let mut compiled = Vec::with_capacity(globs.len());
    for pattern in globs {
        match build_glob(pattern) {
            Ok(glob) => compiled.push(glob),
            Err(message) => {
                issues.push(
                    location,
                    format!("`{field}` pattern {pattern:?}: {message}"),
                );
            }
        }
    }
    issues.since(start)?;
    Ok(PathSet {
        patterns: globs.to_vec(),
        globs: compiled,
    })
}

fn build_glob(pattern: &str) -> Result<Glob, String> {
    if pattern.is_empty() {
        return Err("empty pattern".to_owned());
    }
    if pattern.starts_with('/') || pattern.starts_with("./") {
        return Err(
            "patterns are relative to the repository root; drop the leading `/` or `./`".to_owned(),
        );
    }
    glob::compile(pattern)
}
