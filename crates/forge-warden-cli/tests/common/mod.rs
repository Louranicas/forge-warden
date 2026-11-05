//! Shared helpers for the CLI's integration tests. Tests may spawn the binary and read repository
//! files, so this module opens those doors for tests only, in one reviewed place.
#![expect(
    clippy::disallowed_methods,
    reason = "test door: integration tests spawn the binary and read repository files"
)]
#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test helpers: a panic is a failed test"
)]
#![expect(
    dead_code,
    reason = "each test crate uses a different subset of these helpers"
)]

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// What a run of the binary produced.
pub(crate) struct Run {
    pub(crate) code: i32,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

/// The repository root.
pub(crate) fn root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// A file under the repository root, as text.
pub(crate) fn read(relative: &str) -> String {
    let path = root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

/// A file under the repository root, as bytes.
pub(crate) fn read_bytes(relative: &str) -> Vec<u8> {
    let path = root().join(relative);
    std::fs::read(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

/// Every file under `dir` (relative to the root) whose name ends with `suffix`, sorted.
pub(crate) fn files_under(dir: &str, suffix: &str) -> Vec<PathBuf> {
    fn walk(dir: &std::path::Path, suffix: &str, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, suffix, out);
            } else if path.to_string_lossy().ends_with(suffix) {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(&root().join(dir), suffix, &mut out);
    out.sort();
    out
}

/// Run `forge-warden` with `args` from the repository root, feeding `stdin`.
pub(crate) fn run(args: &[&str], stdin: &[u8]) -> Run {
    let mut child = Command::new(env!("CARGO_BIN_EXE_forge-warden"))
        .args(args)
        .current_dir(root())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    let output = child.wait_with_output().unwrap();
    Run {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}
