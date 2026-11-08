//! A stand-in for deep-diff-forge, for authority-mode tests only (built as the `fixture-ranker`
//! example; never shipped). It is a native executable, so the door can pin and run it.
//!
//! Given `--stdin-patch --rank --json` and a patch on stdin, it:
//!
//! 1. obeys a directive, if the patch has an added line `+fixture-ranker: <directive>`:
//!    `exit <n>` (print the synthetic ranking, then exit `n`), `garbage` (print non-JSON),
//!    `wrong-seal` (seal another patch), `flood` (write past any stdout budget), `omit <path>` (leave
//!    a file out), or `signal <name>` (add a signal to the file the line is in);
//! 2. otherwise replays deep-diff-forge 0.7.0's recorded ranking when the patch is one of the
//!    committed fixtures (`fixtures/demo.patch`, `fixtures/coverage/*.patch`), byte for byte;
//! 3. otherwise prints a synthetic ranking read from the patch headers: each file once, status
//!    `added`, `deleted` or `modified`, with `new_file` or `deleted_file` where Git would derive it.
#![expect(
    clippy::disallowed_methods,
    reason = "test door: the fixture ranker reads stdin and the committed fixtures, and writes stdout/stderr"
)]

use forge_warden_core::seal::sha256_hex;
use std::io::{Read as _, Write as _};
use std::path::PathBuf;
use std::process::ExitCode;

/// One file in a synthetic ranking.
struct File {
    path: String,
    status: &'static str,
    signals: Vec<String>,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args == ["--version"] {
        let _ignored = std::io::stderr().write_all(b"git fixture \x1b\0\n\xe2\x80\xae");
        return ExitCode::from(2);
    }
    if args != ["--stdin-patch", "--rank", "--json"] {
        return ExitCode::from(2);
    }
    let mut input = Vec::new();
    if std::io::stdin().read_to_end(&mut input).is_err() {
        return ExitCode::from(3);
    }
    let text = String::from_utf8_lossy(&input).into_owned();
    let mut files = synthetic(&text);
    let mut seal = sha256_hex(&input);
    let mut code = 0_u8;
    let directives: Vec<&str> = text
        .lines()
        .filter_map(|line| line.strip_prefix("+fixture-ranker: "))
        .collect();
    for directive in &directives {
        let words: Vec<&str> = directive.split_whitespace().collect();
        match words.as_slice() {
            ["stderr-hex" | "schema-hex", hex] => {
                let bytes: Vec<u8> = hex
                    .as_bytes()
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .filter_map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
                    .collect();
                if words.first() == Some(&"stderr-hex") {
                    let _ignored = std::io::stderr().write_all(&bytes);
                    return ExitCode::from(2);
                }
                let doc = serde_json::json!({"schema": String::from_utf8_lossy(&bytes), "input_sha256": "a".repeat(64), "tool": {"name":"deep-diff-forge","version":"0.7.0"}, "ranked":[]});
                return emit(doc.to_string().as_bytes(), 0);
            }
            ["exit", n] => code = n.parse().unwrap_or(1),
            ["garbage"] => return emit(b"this is not a ranking\n", 0),
            ["wrong-seal"] => seal = "a".repeat(64),
            ["flood"] => return flood(),
            ["omit", path] => files.retain(|f| f.path != *path),
            ["signal", _] => {}
            _ => return ExitCode::from(4),
        }
    }
    if let (true, Some(replay)) = (directives.is_empty(), recorded(&input)) {
        return emit(&replay, 0);
    }
    let ranked: Vec<serde_json::Value> = files
        .iter()
        .map(|f| {
            serde_json::json!({"path": f.path, "status": f.status, "score": 1, "signals": f.signals})
        })
        .collect();
    let doc = serde_json::json!({
        "schema": "deep-diff-forge.rank.v0",
        "input_sha256": seal,
        "tool": {"name": "deep-diff-forge", "version": "0.7.0"},
        "ranked": ranked,
    });
    let mut bytes = serde_json::to_vec_pretty(&doc).unwrap_or_default();
    bytes.push(b'\n');
    emit(&bytes, code)
}

/// The files in the patch, from its `diff --git` headers.
fn synthetic(text: &str) -> Vec<File> {
    let mut files: Vec<File> = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("diff --git a/") {
            let path = rest.rsplit_once(" b/").map_or(rest, |(_, b)| b);
            files.push(File {
                path: path.to_owned(),
                status: "modified",
                signals: Vec::new(),
            });
        } else if let Some(file) = files.last_mut() {
            if line.starts_with("new file mode ") {
                file.status = "added";
                file.signals.push("new_file".to_owned());
            } else if line.starts_with("deleted file mode ") {
                file.status = "deleted";
                file.signals.push("deleted_file".to_owned());
            } else if let Some(signal) = line.strip_prefix("+fixture-ranker: signal ") {
                file.signals.push(signal.trim().to_owned());
            }
        }
    }
    files
}

/// deep-diff-forge's recorded ranking of `patch`, if it is a committed fixture.
fn recorded(patch: &[u8]) -> Option<Vec<u8>> {
    let fixtures = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures"));
    let mut candidates = vec![(fixtures.join("demo.patch"), fixtures.join("demo.rank.json"))];
    for entry in std::fs::read_dir(fixtures.join("coverage")).ok()?.flatten() {
        let fixture = entry.path();
        if fixture.extension().is_some_and(|e| e == "patch") {
            candidates.push((fixture.clone(), fixture.with_extension("rank.json")));
        }
    }
    candidates.sort();
    candidates
        .into_iter()
        .find(|(candidate, _)| std::fs::read(candidate).is_ok_and(|bytes| bytes == patch))
        .and_then(|(_, ranking)| std::fs::read(ranking).ok())
}

fn emit(bytes: &[u8], code: u8) -> ExitCode {
    let mut out = std::io::stdout().lock();
    if out.write_all(bytes).and_then(|()| out.flush()).is_err() {
        return ExitCode::from(5);
    }
    ExitCode::from(code)
}

/// Write until the reader stops reading (the door's stdout cap is 64 MiB).
fn flood() -> ExitCode {
    let block = vec![b'{'; 1 << 20];
    let mut out = std::io::stdout().lock();
    for _ in 0..80 {
        if out.write_all(&block).is_err() {
            break;
        }
    }
    ExitCode::SUCCESS
}
