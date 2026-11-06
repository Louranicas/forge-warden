//! Shared helpers for the CLI's integration tests. Tests may spawn the binary and read repository
//! files, so this module opens those doors for tests only, in one reviewed place.
#![expect(
    clippy::disallowed_methods,
    reason = "test door: integration tests spawn the binary and read repository files"
)]
#![expect(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
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

/// `path` relative to the repository root, canonicalized, with `/` separators (`crates/…`).
pub(crate) fn relative(path: &std::path::Path) -> String {
    let base = std::fs::canonicalize(root()).unwrap();
    let full = std::fs::canonicalize(path).unwrap();
    full.strip_prefix(&base)
        .unwrap_or_else(|_| panic!("{} is outside the repository", full.display()))
        .to_str()
        .unwrap()
        .to_owned()
}

/// Parse one restricted rustc dep-info file: one `target:` rule per line, inputs separated by single
/// spaces, only `[A-Za-z0-9_./+-]` (no escapes), `# env-dep:` comments ignored. Anything else is
/// refused, never guessed (the same language as D6's `guardrail_sources.py`).
fn dep_info_inputs(depfile: &std::path::Path) -> Vec<String> {
    let text = std::fs::read_to_string(depfile)
        .unwrap_or_else(|e| panic!("no compiler evidence at {}: {e}", depfile.display()));
    let mut inputs = Vec::new();
    for line in text.lines() {
        if line.is_empty() || line.starts_with("# env-dep:") {
            continue;
        }
        let admitted = line.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '+' | '-' | ':' | ' ')
        }) && line.matches(':').count() == 1
            && !line.contains("  ");
        assert!(admitted, "unsupported rustc dep-info syntax: {line:?}");
        let (target, rest) = line.split_once(':').unwrap();
        assert!(!target.is_empty(), "empty dep-info target");
        inputs.extend(rest.split_whitespace().map(str::to_owned));
    }
    assert!(
        !inputs.is_empty(),
        "empty compiler dep-info: {}",
        depfile.display()
    );
    inputs
}

/// The one dep-info file of a compiler artifact message; missing or ambiguous is refused.
fn artifact_dep_info(message: &serde_json::Value, kinds: &[&str]) -> PathBuf {
    let depfiles: std::collections::BTreeSet<PathBuf> = message["filenames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| PathBuf::from(f.as_str().unwrap()))
        .flat_map(|artifact| {
            let stem = artifact.file_stem().unwrap().to_str().unwrap().to_owned();
            [
                artifact.with_extension("d"),
                artifact.with_file_name(format!("{}.d", stem.trim_start_matches("lib"))),
            ]
        })
        .filter(|d| d.is_file())
        .collect();
    assert_eq!(
        depfiles.len(),
        1,
        "ambiguous or missing dep-info for {kinds:?}: {depfiles:?}"
    );
    depfiles.into_iter().next().unwrap()
}

/// Production and test-only Rust sources of the workspace, decided by the compiler.
pub(crate) struct Sources {
    /// Compiled into a `lib` or `bin` target built without `test` (what ships).
    pub(crate) production: std::collections::BTreeSet<String>,
    /// Compiled only by test, example or bench targets, or under `cfg(test)`.
    pub(crate) test_only: std::collections::BTreeSet<String>,
}

/// Every first-party Rust source, classified by **compilation context**, never by its path (Bastion
/// B-C1c-03; the contract of D6's `guardrail_sources.py`). A nested `cargo check --workspace
/// --all-targets --message-format=json` (persistent discovery dir, inherited rustflags and wrappers
/// cleared) reports every first-party artifact with its kind, its test profile and its dep-info.
/// Refused, never guessed: unparseable dep-info; a missing or ambiguous dep-info file; a Rust input
/// outside the repository; a source with a `tests` path component compiled into production; and
/// any `crates/*/src` source the compiler did not compile at all (inactive source).
pub(crate) fn sources() -> Sources {
    let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("source-discovery");
    let out = Command::new(env!("CARGO"))
        .args([
            "check",
            "--workspace",
            "--all-targets",
            "--locked",
            "--offline",
            "--message-format=json",
        ])
        .arg("--target-dir")
        .arg(&target)
        .current_dir(root())
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_BUILD_RUSTFLAGS")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "source discovery failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut production = std::collections::BTreeSet::new();
    let mut compiled = std::collections::BTreeSet::new();
    for line in String::from_utf8(out.stdout).unwrap().lines() {
        let message: serde_json::Value = serde_json::from_str(line).unwrap();
        if message["reason"] != "compiler-artifact"
            || !message["package_id"]
                .as_str()
                .unwrap()
                .contains("forge-warden")
        {
            continue;
        }
        let kinds: Vec<&str> = message["target"]["kind"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k.as_str().unwrap())
            .collect();
        // Rulings #9 (one rule for C1c and D6): production = any artifact of a shipping kind built
        // without test; test, example and bench targets never ship.
        let ships = kinds.iter().any(|kind| {
            matches!(
                *kind,
                "lib" | "rlib" | "bin" | "proc-macro" | "dylib" | "cdylib" | "staticlib"
            )
        }) && message["profile"]["test"] == serde_json::Value::Bool(false);
        for input in dep_info_inputs(&artifact_dep_info(&message, &kinds)) {
            if std::path::Path::new(&input)
                .extension()
                .is_none_or(|e| e != "rs")
            {
                continue;
            }
            let name = relative(&root().join(&input));
            if ships {
                assert!(
                    !std::path::Path::new(&name)
                        .components()
                        .any(|c| c.as_os_str() == "tests"),
                    "test source compiled into production: {name}"
                );
                production.insert(name.clone());
            }
            compiled.insert(name);
        }
    }
    for anchor in [
        "crates/forge-warden-cli/src/main.rs",
        "crates/forge-warden-cli/src/doors.rs",
        "crates/forge-warden-core/src/lib.rs",
    ] {
        assert!(
            production.contains(anchor),
            "compiler evidence lacks {anchor}"
        );
    }
    for member in [
        "crates/forge-warden-cli/src",
        "crates/forge-warden-core/src",
    ] {
        for path in files_under(member, ".rs") {
            let name = relative(&path);
            assert!(
                compiled.contains(&name),
                "inactive Rust source (the compiler never compiled it): {name}"
            );
        }
    }
    let test_only = compiled.difference(&production).cloned().collect();
    Sources {
        production,
        test_only,
    }
}

/// The production sources only (see [`sources`]).
pub(crate) fn production_sources() -> std::collections::BTreeSet<String> {
    sources().production
}
