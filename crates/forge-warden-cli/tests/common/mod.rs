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

#[path = "../support/process_env.rs"]
mod process_env;

/// Clear ambient process configuration; admit only tool locations and an isolated HOME.
/// Tests needing hostile variables must add them explicitly after this boundary.
pub(crate) fn hermetic(command: &mut Command) -> &mut Command {
    static HOME: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    let home = HOME.get_or_init(|| scratch("process-home"));
    process_env::configure(command, home)
}

/// A test command with the shared explicit environment contract.
pub(crate) fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    hermetic(&mut command);
    command
}

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

/// [`run`] with explicit extra environment variables after clearing the ambient environment.
pub(crate) fn run_with_env(args: &[&str], stdin: &[u8], env: &[(&str, &str)]) -> Run {
    let mut child = command(env!("CARGO_BIN_EXE_forge-warden"))
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

/// A non-Rust production input: it must be on [`PRODUCTION_DATA`] with exactly the pinned bytes.
fn admit_data(name: &str) {
    let Some((_, pin)) = PRODUCTION_DATA.iter().find(|(path, _)| *path == name) else {
        panic!("unreviewed non-Rust input compiled into production: {name}");
    };
    let bytes = std::fs::read(root().join(name)).unwrap();
    assert_eq!(
        forge_warden_core::seal::sha256_hex(&bytes),
        *pin,
        "reviewed production data changed without its pin: {name}"
    );
}

/// Production and test-only Rust sources of the workspace, decided by the compiler.
pub(crate) struct Sources {
    /// Compiled into a `lib` or `bin` target built without `test` (what ships).
    pub(crate) production: std::collections::BTreeSet<String>,
    /// Compiled only by test, example or bench targets, or under `cfg(test)`.
    pub(crate) test_only: std::collections::BTreeSet<String>,
}

/// Every non-Rust file the production build reads, pinned by the SHA-256 of its exact bytes and sorted
/// by path: reviewed data (`include_str!` of a schema or the starter policy). Production code may not
/// `include!` anything (`tests/door_lints.rs`), so these can only ever be data, and a changed byte
/// fails until the pin is updated in a reviewed diff (B-C1c-04, Lodestar's v4.2 ruling).
pub(crate) const PRODUCTION_DATA: [(&str, &str); 3] = [
    (
        "examples/starter.warden.toml",
        "9a08b3feaf2b72d14908b774f50d45cb605ea0518bd667f15fb9ba89d85fa731",
    ),
    (
        "schemas/policy.v0.schema.json",
        "c519ee66ea998552723607a204926166963cfafa52231fb1848654d20ad59546",
    ),
    (
        "schemas/verdict.v1.schema.json",
        "215e5b0cd95a0af1f2008164780af7d464f1fcc1bc159e79ffba0ef0ff6316e2",
    ),
];
/// The workspace's members, from `cargo metadata --no-deps`: their exact package IDs and their
/// directories relative to the repository root.
pub(crate) struct Members {
    /// `workspace_members` package IDs.
    pub(crate) ids: std::collections::BTreeSet<String>,
    /// Each member's manifest directory (`crates/…`, or wherever it lives).
    pub(crate) dirs: Vec<String>,
}

/// The workspace members, by exact ID (Touchstone C1c-O3-04: never by name substring).
pub(crate) fn workspace_members() -> Members {
    let out = command(env!("CARGO"))
        .args([
            "metadata",
            "--no-deps",
            "--locked",
            "--offline",
            "--format-version=1",
        ])
        .current_dir(root())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let ids: std::collections::BTreeSet<String> = metadata["workspace_members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap().to_owned())
        .collect();
    let dirs = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| ids.contains(p["id"].as_str().unwrap()))
        .map(|p| {
            let manifest = PathBuf::from(p["manifest_path"].as_str().unwrap());
            relative(manifest.parent().unwrap())
        })
        .collect();
    Members { ids, dirs }
}

/// The nested `cargo check --workspace --all-targets --message-format=json` stream: its own
/// persistent discovery dir, inherited rustflags, wrappers and target dir cleared.
fn discovery_stream() -> (String, PathBuf) {
    let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("source-discovery");
    let out = command(env!("CARGO"))
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
    (String::from_utf8(out.stdout).unwrap(), target)
}

/// The evidence names the anchors, every crates/*/src source was compiled (no inactive source), and
/// the production data is exactly the reviewed ledger.
fn check_coverage(
    members: &Members,
    production: &std::collections::BTreeSet<String>,
    compiled: &std::collections::BTreeSet<String>,
    data: &std::collections::BTreeSet<String>,
) {
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
    // Every Rust file anywhere in every member's directory was compiled by some target, so no
    // source, test, example or extra root of any member is invisible to the checks.
    for dir in &members.dirs {
        for path in files_under(dir, ".rs") {
            if path.components().any(|c| c.as_os_str() == "target") {
                continue;
            }
            let name = relative(&path);
            assert!(
                compiled.contains(&name),
                "inactive Rust source (the compiler never compiled it): {name}"
            );
        }
    }
    assert_eq!(
        data.iter().map(String::as_str).collect::<Vec<_>>(),
        PRODUCTION_DATA.map(|(path, _)| path),
        "the reviewed production data ledger is stale"
    );
}

/// Every first-party Rust source, classified by **compilation context**, never by its path (Bastion
/// B-C1c-03; the contract of D6's `guardrail_sources.py`). A nested `cargo check --workspace
/// --all-targets --message-format=json` (persistent discovery dir, inherited rustflags and wrappers
/// cleared) reports every first-party artifact with its kind, its test profile and its dep-info.
/// Refused, never guessed: unparseable dep-info; a missing or ambiguous dep-info file; a Rust input
/// outside the repository; a source with a `tests` path component compiled into production; and
/// any `crates/*/src` source the compiler did not compile at all (inactive source).
pub(crate) fn sources() -> Sources {
    sources_with_evidence().0
}

/// [`sources`] together with the exact evidence it classified: the `cargo check
/// --message-format=json` stream and the discovery target directory holding its dep-info files, so a
/// second classifier (D6's `guardrail_sources.py`, Rulings #9 agreement check) judges the same build.
pub(crate) fn sources_with_evidence() -> (Sources, String, PathBuf) {
    let members = workspace_members();
    let (evidence, target) = discovery_stream();
    let mut production = std::collections::BTreeSet::new();
    let mut compiled = std::collections::BTreeSet::new();
    let mut data = std::collections::BTreeSet::new();
    for line in evidence.lines() {
        let message: serde_json::Value = serde_json::from_str(line).unwrap();
        if message["reason"] != "compiler-artifact" {
            continue;
        }
        // First party is exactly cargo metadata's workspace members (Touchstone C1c-O3-04); a
        // registry package is a dependency; any other package (an unlisted path crate) is refused.
        let package = message["package_id"].as_str().unwrap();
        if !members.ids.contains(package) {
            assert!(
                package.starts_with("registry+"),
                "a non-member, non-registry package was compiled: {package}"
            );
            continue;
        }
        let kinds: Vec<&str> = message["target"]["kind"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k.as_str().unwrap())
            .collect();
        // A build script's OUT_DIR output can be include!d as code no check reads (B-C1c-04); as in
        // D6, first-party build scripts are refused outright.
        assert!(
            !kinds.contains(&"custom-build"),
            "first-party build scripts are unsupported (their OUT_DIR output is never inspected): {package}"
        );
        // Rulings #9 (one rule for C1c and D6): production = any artifact of a shipping kind built
        // without test; test, example and bench targets never ship.
        let ships = kinds.iter().any(|kind| {
            matches!(
                *kind,
                "lib" | "rlib" | "bin" | "proc-macro" | "dylib" | "cdylib" | "staticlib"
            )
        }) && message["profile"]["test"] == serde_json::Value::Bool(false);
        for input in dep_info_inputs(&artifact_dep_info(&message, &kinds)) {
            let name = relative(&root().join(&input));
            if std::path::Path::new(&input)
                .extension()
                .is_none_or(|e| e != "rs")
            {
                // A non-Rust input is admitted only as reviewed data (B-C1c-04): `include!` is not
                // bound to `.rs`, so an unknown input could be code no check ever reads.
                if ships {
                    admit_data(&name);
                    data.insert(name);
                }
                continue;
            }
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
    check_coverage(&members, &production, &compiled, &data);
    let test_only = compiled.difference(&production).cloned().collect();
    let classes = Sources {
        production,
        test_only,
    };
    (classes, evidence, target)
}

/// The production sources only (see [`sources`]).
pub(crate) fn production_sources() -> std::collections::BTreeSet<String> {
    sources().production
}

/// Write `bytes` to `dir/name` through a `/usr/bin/cp` child, for a file that will be EXECUTED: this
/// test process then never holds a write descriptor on it, which a concurrent fork in another test
/// thread would inherit until its exec, making the exec fail with ETXTBSY (C1c v4.4). The bytes go
/// first to `dir/name.src`, which is never executed.
pub(crate) fn write_executable(dir: &std::path::Path, name: &str, bytes: &[u8]) -> PathBuf {
    let source = write(dir, &format!("{name}.src"), bytes);
    let path = dir.join(name);
    let status = command("/usr/bin/cp")
        .arg("--")
        .arg(&source)
        .arg(&path)
        .status()
        .unwrap();
    assert!(
        status.success(),
        "cp {} {}",
        source.display(),
        path.display()
    );
    path
}
