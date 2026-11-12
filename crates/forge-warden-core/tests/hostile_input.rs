//! Hostile-input harnesses for the three parsers that read untrusted bytes (finish condition 6,
//! WP-HOSTILE-INPUT): `rank::parse_rank` (the producer's ranking), `policy::parse_policy` (the
//! repository's policy file) and `facts::PatchFacts::from_git` (Git's own output for the patch).
//!
//! For every input, each parser must return. It must never panic and never hang past
//! [`TIME_BOUND`], and the same bytes must give the same answer twice. An over-budget input is
//! the only thing that may be refused as too large. Inputs are arbitrary bytes, plus the committed
//! fixtures mutated by byte flips, deletions, insertions, duplications and truncations.
//! One more property per parser keeps a real, valid input and makes ONE field hostile at a time, so
//! that field's validation is reached on every case (HOSTILE-INPUT.1: a hang behind a long path
//! escaped the default budget when every field was hostile at once).
//! Deterministic cases cover deep nesting and size edges. Every file under `fixtures/hostile/rank/`
//! and `fixtures/hostile/policy/` is replayed on every run through its parser; a crash, hang or
//! wrong refusal of the ranking or policy parser becomes one such file the same day. Patch facts
//! have no replay directory: a facts finding becomes a named test in this file instead.
//!
//! Budget (recorded here and in `fixtures/hostile/README.md`): [`CASES`] cases per property,
//! with proptest's default seeding; `HOSTILE_CASES=<n>` raises it for a longer campaign (a value
//! below the budget is ignored). A parse
//! that does not return within the bound fails the test from a watchdog thread instead of hanging
//! it, and shrinking is capped at [`SHRINK_MS`] so a slow input still ends the run.
#![expect(
    clippy::disallowed_methods,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test helpers read committed fixtures, run each parse on a watchdog thread and read HOSTILE_CASES; a panic here is a failed test"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the harness bounds each parse by wall time: Instant is the clock it measures with"
)]

use forge_warden_core::facts::{PatchFacts, Unverifiable};
use forge_warden_core::policy::parse_policy;
use forge_warden_core::rank::parse_rank;
use proptest::prelude::*;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Cases per property per run: the harness budget.
const CASES: u32 = 256;
/// The longest any one parse may take in a debug test build.
const TIME_BOUND: Duration = Duration::from_secs(2);
/// The longest proptest may spend shrinking one failure.
const SHRINK_MS: u32 = 60_000;

/// The case count: `HOSTILE_CASES` may raise it for a campaign, never lower it, so the environment
/// cannot switch the harness off (Keel R1: `HOSTILE_CASES=0` made every property vacuous).
fn cases(requested: Option<&str>) -> u32 {
    requested
        .and_then(|n| n.parse::<u32>().ok())
        .map_or(CASES, |n| n.max(CASES))
}

fn config() -> ProptestConfig {
    ProptestConfig {
        cases: cases(std::env::var("HOSTILE_CASES").ok().as_deref()),
        max_shrink_time: SHRINK_MS,
        // A failure is turned into a corpus file by hand, with a ledger row: nothing is written
        // into the source tree by the test itself.
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn files(dir: &str, suffix: &str) -> Vec<(String, Vec<u8>)> {
    let mut found: Vec<(String, Vec<u8>)> = std::fs::read_dir(root().join(dir))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.to_string_lossy().ends_with(suffix))
        .map(|path| (path.display().to_string(), std::fs::read(&path).unwrap()))
        .collect();
    found.sort();
    found
}

fn rank_seeds() -> Vec<Vec<u8>> {
    let mut seeds: Vec<Vec<u8>> = files("fixtures", ".rank.json")
        .into_iter()
        .map(|(_, b)| b)
        .collect();
    seeds.extend(
        files("fixtures/coverage", ".rank.json")
            .into_iter()
            .map(|(_, b)| b),
    );
    assert!(seeds.len() > 3, "rank seeds");
    seeds
}

fn policy_seeds() -> Vec<Vec<u8>> {
    let mut seeds: Vec<Vec<u8>> = files("fixtures", ".toml")
        .into_iter()
        .map(|(_, b)| b)
        .collect();
    seeds.extend(files("examples", ".toml").into_iter().map(|(_, b)| b));
    assert!(seeds.len() > 2, "policy seeds");
    seeds
}

/// (numstat -z, summary, patch) triples Git really produced for the coverage corpus.
fn facts_seeds() -> Vec<[Vec<u8>; 3]> {
    let dir = root().join("fixtures/coverage");
    let seeds: Vec<[Vec<u8>; 3]> = files("fixtures/coverage", ".numstat")
        .into_iter()
        .map(|(name, numstat)| {
            let stem = Path::new(&name)
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let read = |ext: &str| std::fs::read(dir.join(format!("{stem}.{ext}"))).unwrap();
            [numstat, read("summary"), read("patch")]
        })
        .collect();
    assert!(seeds.len() > 3, "facts seeds");
    seeds
}

/// One edit to a seed: what a careless or hostile writer could do to a real file.
#[derive(Debug, Clone)]
enum Edit {
    Flip { at: usize, bits: u8 },
    Delete { at: usize, len: usize },
    Insert { at: usize, bytes: Vec<u8> },
    Duplicate { at: usize, len: usize },
    Truncate { at: usize },
}

fn edit() -> impl Strategy<Value = Edit> {
    prop_oneof![
        (any::<usize>(), 1u8..=255).prop_map(|(at, bits)| Edit::Flip { at, bits }),
        (any::<usize>(), 1usize..64).prop_map(|(at, len)| Edit::Delete { at, len }),
        (
            any::<usize>(),
            proptest::collection::vec(any::<u8>(), 1..16)
        )
            .prop_map(|(at, bytes)| Edit::Insert { at, bytes }),
        (any::<usize>(), 1usize..256).prop_map(|(at, len)| Edit::Duplicate { at, len }),
        any::<usize>().prop_map(|at| Edit::Truncate { at }),
    ]
}

fn apply(mut bytes: Vec<u8>, edits: &[Edit]) -> Vec<u8> {
    for edit in edits {
        let n = bytes.len().max(1);
        let at_of = |at: &usize| at.checked_rem(n).unwrap_or(0);
        match edit {
            Edit::Flip { at, bits } => {
                if let Some(byte) = bytes.get_mut(at_of(at)) {
                    *byte ^= bits;
                }
            }
            Edit::Delete { at, len } => {
                let from = at_of(at).min(bytes.len());
                let to = from.saturating_add(*len).min(bytes.len());
                bytes.drain(from..to);
            }
            Edit::Insert { at, bytes: new } => {
                let at = at_of(at).min(bytes.len());
                bytes.splice(at..at, new.iter().copied());
            }
            Edit::Duplicate { at, len } => {
                let from = at_of(at).min(bytes.len());
                let to = from.saturating_add(*len).min(bytes.len());
                let copy = bytes.get(from..to).map_or_else(Vec::new, <[u8]>::to_vec);
                bytes.splice(to..to, copy);
            }
            Edit::Truncate { at } => bytes.truncate(at_of(at)),
        }
    }
    bytes
}

/// A field value: mostly a value the parser knows, otherwise arbitrary Unicode, a short string with
/// non-ASCII and path punctuation, or a very long one. Keel R3: a bug behind valid JSON (a panic
/// on a non-ASCII path) was missed at the default budget by byte edits alone.
fn field(known: &'static [&'static str]) -> impl Strategy<Value = String> {
    prop_oneof![
        3 => proptest::sample::select(known).prop_map(str::to_owned),
        1 => any::<String>(),
        1 => "[a-z0-9/._ \\x{80}-\\x{10FFFF}-]{0,40}",
        1 => (1usize..20_000).prop_map(|n| "é/".repeat(n)),
    ]
}

/// A number field: small, at the edges of the parser's integer types, or past them.
fn number() -> impl Strategy<Value = serde_json::Value> {
    prop_oneof![
        3 => (0u64..100).prop_map(serde_json::Value::from),
        1 => prop_oneof![
            Just(u64::from(u32::MAX)),
            Just(u64::from(u32::MAX).saturating_add(1)),
            Just(u64::MAX),
        ]
        .prop_map(serde_json::Value::from),
        1 => any::<i64>().prop_map(serde_json::Value::from),
        1 => any::<f64>().prop_map(serde_json::Value::from),
    ]
}

/// Prefixes for a `diff --git` header: none (`diff.noprefix`), the default pair's halves, the
/// mnemonic ones, and arbitrary ones. Never one that itself begins `a/` or `b/` and goes on: that
/// is a well-formed patch for another path (see `fixtures/hostile/patch-prefix/README.md`).
fn prefix() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(String::new()),
        Just("a/".to_owned()),
        Just("b/".to_owned()),
        Just("i/".to_owned()),
        Just("w/".to_owned()),
        Just("c/".to_owned()),
        Just("o/".to_owned()),
        "[c-z]{1,3}/",
        "[c-z]{1,2}/[a-z]{1,2}/",
    ]
}

const PATHS: &[&str] = &["src/lib.rs", "a", "tests/it.rs", "a//b", "../a", "/a", ""];
const SIDES: &[&str] = &["new", "old", "both", ""];
const SEAL: &str = "bbf580931c6739af491bb5e2258550631327ca07f1078135cb008797e22b836e";

fn evidence() -> impl Strategy<Value = serde_json::Value> {
    (
        field(&forge_warden_core::vocab::KNOWN_SIGNALS),
        field(&[SEAL]),
        field(SIDES),
        number(),
    )
        .prop_map(|(signal, address, side, line)| {
            serde_json::json!({"signal": signal, "address": address, "side": side, "line": line})
        })
}

fn ranked_file() -> impl Strategy<Value = serde_json::Value> {
    (
        field(PATHS),
        field(&forge_warden_core::vocab::KNOWN_STATUSES),
        number(),
        proptest::collection::vec(field(&forge_warden_core::vocab::KNOWN_SIGNALS), 0..6),
        proptest::collection::vec(evidence(), 0..4),
        any::<bool>(),
    )
        .prop_map(|(path, status, score, signals, evidence, truncated)| {
            serde_json::json!({
                "path": path, "status": status, "score": score, "signals": signals,
                "evidence": evidence, "evidence_truncated": truncated,
            })
        })
}

/// A ranking that is valid JSON with the right schema and tool name, so the parser reaches its
/// field validation, with hostile values in the fields (repeated entries included).
fn valid_shaped_ranking() -> impl Strategy<Value = Vec<u8>> {
    (
        field(&[SEAL]),
        field(&["0.8.2"]),
        proptest::collection::vec(ranked_file(), 0..12),
        any::<bool>(),
    )
        .prop_map(|(seal, version, mut ranked, repeat)| {
            if repeat && let Some(first) = ranked.first().cloned() {
                ranked.push(first);
            }
            serde_json::to_vec(&serde_json::json!({
                "schema": "deep-diff-forge.rank.v0",
                "input_sha256": seal,
                "tool": {"name": "deep-diff-forge", "version": version},
                "ranked": ranked,
            }))
            .unwrap()
        })
}

/// A hostile string with no known-good choice in it: arbitrary Unicode, short non-ASCII text with
/// path punctuation, or a very long one (HOSTILE-INPUT.1: a long path never met a well-typed
/// document in the all-fields-hostile property, so a hang behind it escaped the default budget).
fn hostile_text() -> impl Strategy<Value = String> {
    prop_oneof![
        any::<String>(),
        "[a-z0-9/._ \\x{80}-\\x{10FFFF}-]{0,40}",
        (1usize..20_000).prop_map(|n| "é/".repeat(n)),
    ]
}

/// A real ranking (`fixtures/demo.rank.json`) with ONE field replaced by a hostile value of the
/// same JSON type: the document still deserialises, so that field's validation is reached on every
/// case (HOSTILE-INPUT.1, ledger Q51).
fn one_hostile_field_ranking() -> impl Strategy<Value = Vec<u8>> {
    (0usize..10, hostile_text(), number()).prop_map(|(slot, text, num)| {
        let mut doc: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root().join("fixtures/demo.rank.json")).unwrap())
                .unwrap();
        // Each slot names one field by JSON pointer; slots 2 and 7 are numbers, the rest strings.
        let pointer = [
            "/ranked/0/path",
            "/ranked/0/status",
            "/ranked/0/score",
            "/ranked/0/signals/0",
            "/ranked/0/evidence/0/signal",
            "/ranked/0/evidence/0/address",
            "/ranked/0/evidence/0/side",
            "/ranked/0/evidence/0/line",
            "/tool/version",
            "/input_sha256",
        ]
        .get(slot)
        .copied()
        .unwrap();
        *doc.pointer_mut(pointer).unwrap() = if slot == 2 || slot == 7 {
            num
        } else {
            serde_json::Value::from(text)
        };
        serde_json::to_vec(&doc).unwrap()
    })
}

/// Well-formed `--numstat -z` and `--summary` output for one file, with ONE slot hostile: the
/// numstat path, either count, the summary path or the mode.
fn one_hostile_field_git_output() -> impl Strategy<Value = [Vec<u8>; 3]> {
    (0usize..5, hostile_text()).prop_map(|(slot, text)| {
        let mut slots = ["1", "1", "src/a.rs", "100644", "src/a.rs"].map(str::to_owned);
        if let Some(chosen) = slots.get_mut(slot) {
            *chosen = text;
        }
        let [added, deleted, path, mode, summary_path] = slots;
        [
            format!("{added}\t{deleted}\t{path}\0").into_bytes(),
            format!(" create mode {mode} {summary_path}\n").into_bytes(),
            Vec::new(),
        ]
    })
}

/// A TOML basic string holding `text` exactly.
fn toml_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
            out.push(c);
        } else if c.is_control() {
            write!(out, "\\u{:04X}", u32::from(c)).unwrap();
        } else {
            out.push(c);
        }
    }
    out.push('"');
    out
}

/// A valid policy (two rules and a waiver, as in `fixtures/rich.warden.toml`) with ONE value
/// hostile: a rule's id, why, kind, severity or signal, the integer limit, or the waiver's rule,
/// path glob, reason, owner or expiry date.
fn one_hostile_field_policy() -> impl Strategy<Value = Vec<u8>> {
    (0usize..11, hostile_text(), number()).prop_map(|(slot, text, num)| {
        let mut values = [
            "no-unsafe",
            "Unsafe code needs a dedicated safety review.",
            "forbid_signal",
            "fail",
            "unsafe_added",
            "",
            "no-unsafe",
            "src/simd/**",
            "Audited kernel.",
            "@louranicas",
            "2026-12-31",
        ]
        .map(toml_string);
        *values.get_mut(5).unwrap() = String::from("1");
        if let Some(chosen) = values.get_mut(slot) {
            *chosen = if slot == 5 { num.to_string() } else { toml_string(&text) };
        }
        let [id, why, kind, severity, signal, limit, rule, path, reason, owner, expires] = values;
        format!(
            "schema = \"forge-warden.policy.v0\"\n\
             [[rule]]\nid = {id}\nwhy = {why}\nkind = {kind}\nseverity = {severity}\nsignals = [{signal}]\n\
             [[rule]]\nid = \"small-changes\"\nwhy = \"Small changes get real review.\"\nkind = \"max_files\"\nseverity = \"hold\"\nlimit = {limit}\n\
             [[waiver]]\nrule = {rule}\npaths = [{path}]\nreason = {reason}\nowner = {owner}\nexpires = {expires}\n"
        )
        .into_bytes()
    })
}

/// Run `parse` twice on its own input, on a watchdog thread: it must return within the bound and
/// agree with itself. A parse that never returns fails here instead of hanging the test run.
fn holds<T>(what: &str, parse: impl Fn() -> T + Send + 'static) -> T
where
    T: std::fmt::Debug + Send + 'static,
{
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let start = Instant::now();
        let first = parse();
        let took = start.elapsed();
        let second = parse();
        drop(send.send((took, format!("{first:?}") == format!("{second:?}"), first)));
    });
    let (took, same, first) = match receive.recv_timeout(TIME_BOUND.saturating_mul(3)) {
        Ok(result) => result,
        // The parse thread ended without sending: it panicked (its message is printed above).
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => panic!("{what}: panicked"),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => panic!(
            "{what}: did not return within {:?}: a hang",
            TIME_BOUND.saturating_mul(3)
        ),
    };
    assert!(took < TIME_BOUND, "{what}: took {took:?}");
    assert!(same, "{what}: not deterministic");
    first
}

fn rank_holds(bytes: &[u8]) {
    let owned = bytes.to_vec();
    let result = holds("parse_rank", move || parse_rank(&owned));
    if let Err(forge_warden_core::rank::RankError::TooLarge(_)) = result {
        assert!(
            bytes.len() > forge_warden_core::MAX_INPUT_BYTES,
            "TooLarge under budget"
        );
    }
}

fn policy_holds(bytes: &[u8]) {
    let owned = bytes.to_vec();
    let result = holds("parse_policy", move || parse_policy(&owned));
    if let Err(forge_warden_core::policy::PolicyError::TooLarge(_)) = result {
        assert!(
            bytes.len() > forge_warden_core::MAX_POLICY_BYTES,
            "TooLarge under budget"
        );
    }
}

fn facts_holds(numstat: &[u8], summary: &[u8], patch: &[u8]) {
    let (numstat, summary, patch) = (numstat.to_vec(), summary.to_vec(), patch.to_vec());
    drop(holds("PatchFacts::from_git", move || {
        PatchFacts::from_git(&numstat, &summary, &patch)
    }));
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn rank_survives_arbitrary_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
        rank_holds(&bytes);
    }

    #[test]
    fn rank_survives_edited_real_rankings(
        pick in any::<usize>(),
        edits in proptest::collection::vec(edit(), 1..8),
    ) {
        let seeds = rank_seeds();
        let seed = seeds.get(pick.checked_rem(seeds.len()).unwrap_or(0)).unwrap().clone();
        rank_holds(&apply(seed, &edits));
    }

    #[test]
    fn rank_survives_hostile_values_in_valid_rankings(bytes in valid_shaped_ranking()) {
        rank_holds(&bytes);
    }

    #[test]
    fn rank_survives_one_hostile_field_in_a_real_ranking(bytes in one_hostile_field_ranking()) {
        rank_holds(&bytes);
    }

    #[test]
    fn policy_survives_one_hostile_value_in_a_valid_policy(bytes in one_hostile_field_policy()) {
        policy_holds(&bytes);
    }

    /// Header shapes Git can be configured to write, not only hostile values inside the default
    /// shape (the gap N3b came through). The numstat is what `git apply -p1` reports: the header's
    /// first name without its first component. Whenever the facts are believed, the path believed
    /// is the path the patch was made for and the prefixes were the default pair.
    #[test]
    fn a_path_is_believed_only_with_the_default_prefixes(
        old in prefix(),
        new in prefix(),
        path in "[a-z]{1,3}(/[a-z]{1,4}){0,3}",
    ) {
        let first = format!("{old}{path}");
        let Some((_, reported)) = first.split_once('/') else { return Ok(()); };
        let numstat = format!("1\t0\t{reported}\0");
        let patch = format!("diff --git {old}{path} {new}{path}\n--- {old}{path}\n+++ {new}{path}\n@@ -1 +1,2 @@\n x\n+y\n");
        match PatchFacts::from_git(numstat.as_bytes(), b"", patch.as_bytes()) {
            Ok(facts) => {
                let believed: Vec<&str> = facts.files().map(|file| file.path().as_str()).collect();
                prop_assert_eq!(believed, [path.as_str()], "{}", &patch);
                prop_assert!(
                    matches!((old.as_str(), new.as_str()), ("a/", "b/") | ("b/", "a/")),
                    "{}",
                    &patch
                );
            }
            Err(_) => prop_assert!(
                !matches!((old.as_str(), new.as_str()), ("a/", "b/") | ("b/", "a/")),
                "a default-prefixed header was refused: {}",
                &patch
            ),
        }
    }

    #[test]
    fn facts_survive_one_hostile_slot_in_real_git_output(parts in one_hostile_field_git_output()) {
        let [numstat, summary, patch] = &parts;
        facts_holds(numstat, summary, patch);
    }

    #[test]
    fn policy_survives_arbitrary_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
        policy_holds(&bytes);
    }

    #[test]
    fn policy_survives_edited_real_policies(
        pick in any::<usize>(),
        edits in proptest::collection::vec(edit(), 1..8),
    ) {
        let seeds = policy_seeds();
        let seed = seeds.get(pick.checked_rem(seeds.len()).unwrap_or(0)).unwrap().clone();
        policy_holds(&apply(seed, &edits));
    }

    #[test]
    fn facts_survive_arbitrary_bytes(
        numstat in proptest::collection::vec(any::<u8>(), 0..1024),
        summary in proptest::collection::vec(any::<u8>(), 0..1024),
        patch in proptest::collection::vec(any::<u8>(), 0..4096),
    ) {
        facts_holds(&numstat, &summary, &patch);
    }

    #[test]
    fn facts_survive_edited_real_git_output(
        pick in any::<usize>(),
        which in 0usize..3,
        edits in proptest::collection::vec(edit(), 1..8),
    ) {
        let seeds = facts_seeds();
        let mut seed = seeds.get(pick.checked_rem(seeds.len()).unwrap_or(0)).unwrap().clone();
        let part = seed.get_mut(which).unwrap();
        *part = apply(std::mem::take(part), &edits);
        let [numstat, summary, patch] = &seed;
        facts_holds(numstat, summary, patch);
    }
}

/// Inputs a fuzzer reaches slowly: very deep nesting, and sizes on both sides of the budget.
#[test]
fn deep_nesting_and_size_edges_are_refused_not_crashed() {
    for depth in [64, 1_000, 100_000] {
        let open = "[".repeat(depth);
        rank_holds(format!("{{\"schema\":{open}").as_bytes());
        policy_holds(format!("a = {open}").as_bytes());
        policy_holds(format!("{}x = 1", "[a.".repeat(depth.min(10_000))).as_bytes());
        policy_holds("{a = ".repeat(depth).as_bytes());
    }
    let over = vec![b' '; forge_warden_core::MAX_INPUT_BYTES.saturating_add(1)];
    assert!(matches!(
        parse_rank(&over),
        Err(forge_warden_core::rank::RankError::TooLarge(_))
    ));
    assert!(matches!(
        parse_policy(&over),
        Err(forge_warden_core::policy::PolicyError::TooLarge(_))
    ));
}

#[test]
fn the_environment_may_raise_the_budget_but_never_lower_it() {
    assert_eq!(cases(None), CASES);
    assert_eq!(cases(Some("0")), CASES);
    assert_eq!(cases(Some("1")), CASES);
    assert_eq!(cases(Some("not a number")), CASES);
    assert_eq!(cases(Some("20000")), 20_000);
}

/// The patches Git 2.56 writes for one change to `scripts/gate.sh` under each prefix setting, with
/// what `git apply --numstat -z` reported for each (`fixtures/hostile/patch-prefix/`, freeze review
/// N3b). A path is believed only where the header carries the default prefixes.
#[test]
fn the_patch_prefix_corpus_is_believed_only_under_the_default_prefixes() {
    let believed = |name: &str| {
        let read = |suffix: &str| {
            std::fs::read(root().join(format!("fixtures/hostile/patch-prefix/{name}.{suffix}")))
                .unwrap()
        };
        PatchFacts::from_git(&read("numstat"), b"", &read("patch")).map(|facts| {
            facts
                .files()
                .map(|file| file.path().as_str().to_owned())
                .collect::<Vec<_>>()
        })
    };
    let refused = |header: &str| Err(Unverifiable::HeaderPaths(header.to_owned()));
    assert_eq!(believed("default"), Ok(vec!["scripts/gate.sh".to_owned()]));
    assert_eq!(believed("reverse"), Ok(vec!["scripts/gate.sh".to_owned()]));
    // Git reports `gate.sh`: the path the measured authority-mode PASS was given for.
    assert_eq!(
        believed("noprefix"),
        refused("diff --git scripts/gate.sh scripts/gate.sh")
    );
    assert_eq!(
        believed("prefix-old-new"),
        refused("diff --git old/scripts/gate.sh new/scripts/gate.sh")
    );
    assert_eq!(
        believed("mnemonic-worktree"),
        refused("diff --git i/scripts/gate.sh w/scripts/gate.sh")
    );
    // The stated limit: prefixes that themselves begin `a/` and `b/` give a well-formed patch for
    // another path, `x/scripts/gate.sh`. No reader of the patch can tell; the path believed is the
    // one the patch names, and the documented commands fix their prefixes.
    assert_eq!(
        believed("prefix-a-x"),
        Ok(vec!["x/scripts/gate.sh".to_owned()])
    );
    let expected: Vec<String> = [
        "default",
        "mnemonic-worktree",
        "noprefix",
        "prefix-a-x",
        "prefix-old-new",
        "reverse",
    ]
    .iter()
    .flat_map(|name| [format!("{name}.numstat"), format!("{name}.patch")])
    .collect();
    let found: Vec<String> = files("fixtures/hostile/patch-prefix", "")
        .into_iter()
        .filter_map(|(name, _)| Some(Path::new(&name).file_name()?.to_str()?.to_owned()))
        .filter(|name| name != "README.md")
        .collect();
    assert_eq!(found, expected, "every corpus file is replayed above");
}

/// Every committed corpus file is replayed: a past crash, hang or wrong refusal stays fixed.
#[test]
fn the_hostile_corpus_replays() {
    for (name, bytes) in files("fixtures/hostile/rank", "") {
        println!("{name}");
        rank_holds(&bytes);
    }
    for (name, bytes) in files("fixtures/hostile/policy", "") {
        println!("{name}");
        policy_holds(&bytes);
    }
}
