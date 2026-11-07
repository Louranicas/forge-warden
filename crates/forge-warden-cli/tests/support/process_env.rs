//! Shared explicit environment contract for integration and inline test processes.
#![expect(
    clippy::disallowed_methods,
    reason = "test environment door: reads only explicit tool locations before clearing child environments"
)]
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn configure<'a>(command: &'a mut Command, home: &Path) -> &'a mut Command {
    command.env_clear();
    for key in ["PATH", "CARGO_HOME", "RUSTUP_HOME", "RUSTUP_TOOLCHAIN"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    if let Some(original_home) = std::env::var_os("HOME") {
        for (key, directory) in [("CARGO_HOME", ".cargo"), ("RUSTUP_HOME", ".rustup")] {
            if std::env::var_os(key).is_none() {
                command.env(key, PathBuf::from(&original_home).join(directory));
            }
        }
    }
    command
        .env("HOME", home)
        .env("TMPDIR", home)
        .env("LC_ALL", "C")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("CARGO_NET_OFFLINE", "true")
        .env("RUSTUP_AUTO_INSTALL", "0")
}
