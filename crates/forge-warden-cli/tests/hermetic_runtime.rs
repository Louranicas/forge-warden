//! Named prerequisites and scratch ownership, including unwind cleanup.
mod common;

#[test]
fn locked_dependencies_probe() {
    common::require_locked_deps();
    common::require_locked_deps();
}

#[test]
fn empty_cargo_home_has_an_actionable_precondition() {
    let cold = common::scratch("cold-cargo");
    let executable = std::env::current_exe().unwrap();
    let output = common::command(executable)
        .args(["--exact", "locked_dependencies_probe", "--nocapture"])
        .env("CARGO_HOME", &cold)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(text.contains("TEST_LOCKED_DEPS_UNAVAILABLE"), "{text}");
    assert!(text.contains("cargo fetch --locked"), "{text}");
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "test fixture: looks at whether the scratch directory still exists"
)]
fn scratch_is_unique_and_cleans_on_drop_and_unwind() {
    let first = common::scratch("same-label");
    let second = common::scratch("same-label");
    assert_ne!(&*first, &*second);
    let path = first.to_path_buf();
    drop(first);
    assert!(!path.exists());
    let unwound_path = second.to_path_buf();
    let failure = std::panic::catch_unwind(move || {
        let _owned = second;
        assert_eq!(0, 1, "deliberate unwind cleanup probe");
    });
    assert!(failure.is_err());
    assert!(!unwound_path.exists());
}

#[test]
fn missing_tools_have_a_named_precondition() {
    let failure = std::panic::catch_unwind(|| common::command("/nonexistent/fw-test-tool"));
    let error = failure.expect_err("missing tool must refuse");
    let text = error.downcast_ref::<String>().expect("diagnostic string");
    assert!(text.contains("TEST_TOOL_UNAVAILABLE"));
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "test fixture: resolve scratch executable identity and child cwd independently of repository modes"
)]
fn relative_tools_keep_their_identity_after_changing_directory() {
    use std::os::unix::fs::PermissionsExt as _;
    let tools = common::scratch("relative-tool");
    // Own the executable and its mode; never rely on copied repository permission bits.
    // The existing helper writes out-of-process to avoid concurrent-fork ETXTBSY (C1c v4.4).
    let fixture = common::write_executable(
        &tools,
        "identity",
        b"#!/bin/sh\nprintf 'scratch-tool\\n'\npwd -P\n",
    );
    std::fs::set_permissions(&fixture, std::fs::Permissions::from_mode(0o700)).unwrap();
    let absolute = std::fs::canonicalize(&fixture).unwrap();
    let cwd = std::env::current_dir().unwrap();
    let mut relative = std::path::PathBuf::new();
    for _ in cwd.ancestors().skip(1) {
        relative.push("..");
    }
    relative.push(absolute.strip_prefix("/").unwrap());
    assert!(!relative.is_absolute());
    let elsewhere = common::scratch("tool-working-directory");
    let mut child = common::command(&relative);
    assert!(std::path::Path::new(child.get_program()).is_absolute());
    assert_eq!(
        std::fs::canonicalize(child.get_program()).unwrap(),
        absolute
    );
    let output = child.current_dir(&elsewhere).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        format!(
            "scratch-tool\n{}\n",
            std::fs::canonicalize(&elsewhere).unwrap().display()
        )
        .as_bytes()
    );
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "test fixture: makes a symlink to a scratch executable"
)]
fn executable_symlink_invocation_name_is_preserved() {
    let directory = common::scratch("tool-symlink");
    let link = directory.join("sh");
    std::os::unix::fs::symlink("/bin/sh", &link).unwrap();
    let output = common::command(&link)
        .args(["-c", "printf '%s' \"$0\""])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, link.as_os_str().as_encoded_bytes());
}
