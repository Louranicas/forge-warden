//! Path globs: compiled, `/`-aware, and validated when the policy loads.

use super::issues::{Issues, Rejected};
use globset::{Glob, GlobBuilder, GlobSet, GlobSetBuilder};

/// A compiled set of globs, keeping the patterns for reporting.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PathSet {
    patterns: Vec<String>,
    set: GlobSet,
}

impl PathSet {
    /// Whether `path` matches any pattern.
    #[must_use]
    pub fn matches(&self, path: &str) -> bool {
        self.set.is_match(path)
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
    let mut builder = GlobSetBuilder::new();
    for pattern in globs {
        match build_glob(pattern) {
            Ok(glob) => {
                builder.add(glob);
            }
            Err(message) => {
                issues.push(
                    location,
                    format!("`{field}` pattern {pattern:?}: {message}"),
                );
            }
        }
    }
    issues.since(start)?;
    match builder.build() {
        Ok(set) => Ok(PathSet {
            patterns: globs.to_vec(),
            set,
        }),
        Err(e) => Err(issues.reject(location, format!("`{field}`: {e}"))),
    }
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
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .backslash_escape(true)
        .build()
        .map_err(|e| e.kind().to_string())
}
