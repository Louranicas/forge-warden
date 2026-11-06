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
    run_with_env(args, stdin, &[])
}

/// [`run`] with extra environment variables (the rest is inherited).
pub(crate) fn run_with_env(args: &[&str], stdin: &[u8], env: &[(&str, &str)]) -> Run {
    let mut child = Command::new(env!("CARGO_BIN_EXE_forge-warden"))
        .args(args)
        .envs(env.iter().copied())
        .current_dir(root())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // A run refused before reading stdin closes the pipe; that is not a test failure.
    let _written = child.stdin.take().unwrap().write_all(stdin);
    let output = child.wait_with_output().unwrap();
    Run {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

/// The fixture ranker (`tests/support/fixture_ranker.rs`), which `cargo test` builds as an example
/// next to the binary under test.
pub(crate) fn fixture_ranker() -> PathBuf {
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_forge-warden"));
    let ranker = binary
        .parent()
        .unwrap()
        .join("examples")
        .join("fixture-ranker");
    assert!(
        ranker.is_file(),
        "{} is missing: `cargo test` builds it (or run `cargo build --examples`)",
        ranker.display()
    );
    ranker
}

/// The fixture ranker's SHA-256, as a policy's `[producer] sha256` lists it.
pub(crate) fn fixture_ranker_sha256() -> String {
    forge_warden_core::seal::sha256_hex(&std::fs::read(fixture_ranker()).unwrap())
}

/// A fresh, empty directory for one test, under Cargo's per-target scratch directory.
pub(crate) fn scratch(name: &str) -> PathBuf {
    let dir =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}-{}", std::process::id()));
    let _gone = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write `bytes` to `dir/name` and return the path.
pub(crate) fn write(dir: &std::path::Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

/// The policy at `relative` plus a `[producer]` table admitting `digests`, written into `dir`.
pub(crate) fn policy_admitting(
    dir: &std::path::Path,
    relative: &str,
    digests: &[String],
) -> PathBuf {
    let list: Vec<String> = digests.iter().map(|d| format!("{d:?}")).collect();
    let text = format!(
        "{}\n[producer]\nsha256 = [{}]\n",
        read(relative),
        list.join(", ")
    );
    write(dir, "policy.toml", text.as_bytes())
}

/// A path as `&str` (every test path is UTF-8).
pub(crate) fn text(path: &std::path::Path) -> &str {
    path.to_str().unwrap()
}

/// Whether `scripts/bless.sh` is regenerating the golden files (`FORGE_WARDEN_BLESS=1`).
pub(crate) fn blessing() -> bool {
    std::env::var_os("FORGE_WARDEN_BLESS").is_some_and(|v| v == "1")
}

/// Overwrite the golden file at `relative` (bless mode only).
pub(crate) fn bless(relative: &str, contents: &str) {
    std::fs::write(root().join(relative), contents).unwrap();
}
