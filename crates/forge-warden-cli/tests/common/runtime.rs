//! Explicit test prerequisites and owned per-run scratch directories.
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

pub(crate) struct Scratch(PathBuf);
impl Scratch {
    pub(crate) fn as_path(&self) -> &Path {
        &self.0
    }
}
impl std::ops::Deref for Scratch {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}
impl AsRef<Path> for Scratch {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}
impl AsRef<std::ffi::OsStr> for Scratch {
    fn as_ref(&self) -> &std::ffi::OsStr {
        self.0.as_os_str()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        // Cleanup must not double-panic during an assertion unwind.
        let _cleanup = std::fs::remove_dir_all(&self.0);
    }
}

pub(crate) fn scratch(name: &str) -> Scratch {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    assert!(
        !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')),
        "TEST_SCRATCH_NAME: use an ASCII label"
    );
    for _ in 0..128 {
        let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("fw-{name}-{}-{serial}", std::process::id()));
        match std::fs::create_dir(&path) {
            Ok(()) => return Scratch(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => panic!(
                "TEST_SCRATCH_UNAVAILABLE: cannot create {}: {error}",
                path.display()
            ),
        }
    }
    panic!("TEST_SCRATCH_UNAVAILABLE: unique directory attempts exhausted");
}

pub(crate) fn tool(program: &std::ffi::OsStr) -> PathBuf {
    let executable = |p: &Path| {
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    let path = Path::new(program);
    if path.components().count() > 1 && executable(path) {
        return std::path::absolute(path).unwrap_or_else(|error| {
            panic!(
                "TEST_TOOL_UNAVAILABLE: cannot resolve {}: {error}",
                path.display()
            )
        });
    }
    if path.components().count() == 1
        && let Some(search) = std::env::var_os("PATH")
    {
        for dir in std::env::split_paths(&search) {
            let candidate = dir.join(path);
            if executable(&candidate) {
                return std::path::absolute(&candidate).unwrap_or_else(|error| {
                    panic!(
                        "TEST_TOOL_UNAVAILABLE: cannot resolve {}: {error}",
                        candidate.display()
                    )
                });
            }
        }
    }
    panic!(
        "TEST_TOOL_UNAVAILABLE: executable {} is missing or not executable; install the documented test prerequisites and set PATH before running tests",
        program.to_string_lossy().escape_default()
    );
}

pub(crate) fn require_locked_deps() {
    static READY: std::sync::Once = std::sync::Once::new();
    static RESULT: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
    READY.call_once(|| {
        let output = super::command(env!("CARGO"))
            .args(["fetch", "--locked", "--offline"])
            .current_dir(super::root())
            .output();
        let result = match output {
            Ok(out) if out.status.success() => Ok(()),
            Ok(out) => Err(String::from_utf8_lossy(&out.stderr).into_owned()),
            Err(error) => Err(error.to_string()),
        };
        assert!(
            RESULT.set(result).is_ok(),
            "dependency precondition initialized twice"
        );
    });
    if let Some(Err(error)) = RESULT.get() {
        panic!(
            "TEST_LOCKED_DEPS_UNAVAILABLE: run cargo fetch --locked in this workspace with the intended CARGO_HOME before offline tests; preflight failed: {error}"
        );
    }
}
