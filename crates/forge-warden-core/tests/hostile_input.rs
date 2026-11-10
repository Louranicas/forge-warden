//! Hostile-input harnesses for the three parsers that read untrusted bytes (finish condition 6,
//! WP-HOSTILE-INPUT): `rank::parse_rank` (the producer's ranking), `policy::parse_policy` (the
//! repository's policy file) and `facts::PatchFacts::from_git` (Git's own output for the patch).
//!
//! For every input, each parser must return. It must never panic and never hang past
//! [`TIME_BOUND`], and the same bytes must give the same answer twice. An over-budget input is
//! the only thing that may be refused as too large. Inputs are arbitrary bytes, plus the committed
//! fixtures mutated by byte flips, deletions, insertions, duplications and truncations.
//! Deterministic cases cover deep nesting and size edges. Every file under
//! `fixtures/hostile/` is replayed on every run; a crash, hang or wrong refusal found here becomes
//! one such file the same day.
//!
//! Budget (recorded here and in `fixtures/hostile/README.md`): [`CASES`] cases per property,
//! with proptest's default seeding; `HOSTILE_CASES=<n>` raises it for a longer campaign. A parse
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

use forge_warden_core::facts::PatchFacts;
use forge_warden_core::policy::parse_policy;
use forge_warden_core::rank::parse_rank;
use proptest::prelude::*;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Cases per property per run: the harness budget.
const CASES: u32 = 256;
/// The longest any one parse may take in a debug test build.
const TIME_BOUND: Duration = Duration::from_secs(2);
/// The longest proptest may spend shrinking one failure.
const SHRINK_MS: u32 = 60_000;

fn config() -> ProptestConfig {
    let cases = std::env::var("HOSTILE_CASES")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(CASES);
    ProptestConfig {
        cases,
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
    let mut seeds: Vec<Vec<u8>> = files("fixtures", ".rank.json").into_iter().map(|(_, b)| b).collect();
    seeds.extend(files("fixtures/coverage", ".rank.json").into_iter().map(|(_, b)| b));
    assert!(seeds.len() > 3, "rank seeds");
    seeds
}

fn policy_seeds() -> Vec<Vec<u8>> {
    let mut seeds: Vec<Vec<u8>> = files("fixtures", ".toml").into_iter().map(|(_, b)| b).collect();
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
            let stem = Path::new(&name).file_stem().unwrap().to_string_lossy().into_owned();
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
        (any::<usize>(), proptest::collection::vec(any::<u8>(), 1..16))
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
    let Ok((took, same, first)) = receive.recv_timeout(TIME_BOUND.saturating_mul(3)) else {
        panic!("{what}: did not return within {:?}: a hang", TIME_BOUND.saturating_mul(3));
    };
    assert!(took < TIME_BOUND, "{what}: took {took:?}");
    assert!(same, "{what}: not deterministic");
    first
}

fn rank_holds(bytes: &[u8]) {
    let owned = bytes.to_vec();
    let result = holds("parse_rank", move || parse_rank(&owned));
    if let Err(forge_warden_core::rank::RankError::TooLarge(_)) = result {
        assert!(bytes.len() > forge_warden_core::MAX_INPUT_BYTES, "TooLarge under budget");
    }
}

fn policy_holds(bytes: &[u8]) {
    let owned = bytes.to_vec();
    let result = holds("parse_policy", move || parse_policy(&owned));
    if let Err(forge_warden_core::policy::PolicyError::TooLarge(_)) = result {
        assert!(bytes.len() > forge_warden_core::MAX_INPUT_BYTES, "TooLarge under budget");
    }
}

fn facts_holds(numstat: &[u8], summary: &[u8], patch: &[u8]) {
    let (numstat, summary, patch) = (numstat.to_vec(), summary.to_vec(), patch.to_vec());
    drop(holds("PatchFacts::from_git", move || PatchFacts::from_git(&numstat, &summary, &patch)));
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
    assert!(matches!(parse_rank(&over), Err(forge_warden_core::rank::RankError::TooLarge(_))));
    assert!(matches!(parse_policy(&over), Err(forge_warden_core::policy::PolicyError::TooLarge(_))));
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
