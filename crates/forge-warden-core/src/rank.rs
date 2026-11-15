//! Reading a `deep-diff-forge.rank.v0` document.
//!
//! The reader is **tolerant of additions and strict about meaning**. deep-diff-forge adds fields
//! without changing the schema name, so unknown members are ignored. Every member forge-warden
//! relies on is checked: the schema name, the seal, the tool, and each file's path, status, score,
//! signals and evidence. A document that fails any check is refused as a whole.

use crate::nonempty::NonEmpty;
use crate::path::{PathError, RepoPath};
use crate::seal::Sha256Hex;
use crate::vocab::{Side, Signal, Status};
use serde::Deserialize;
use std::collections::BTreeSet;

/// The only ranking schema this version reads.
pub const RANK_SCHEMA: &str = "deep-diff-forge.rank.v0";

/// A validated ranking. Only [`parse_rank`] and [`RankDoc::new`] make one, and both validate.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RankDoc {
    input_sha256: Sha256Hex,
    tool_version: String,
    files: Vec<RankedFile>,
}

/// What a ranking's file list is, before any rule reads it (Q1-12).
///
/// A ranking with no files, or with one path listed more than once, is valid input: a real producer
/// writes both (an empty patch; a file/symlink type change is an addition and a deletion of one path).
/// Neither may pass. With no files there is nothing to judge, and with a repeated path a rule cannot
/// tell which entry is the file. [`RankDoc::shape`] is the one place this is decided, and the enum is
/// exhaustive, so a consumer that reads the files has to say what it does with each shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RankShape<'a> {
    /// No files.
    Empty,
    /// At least one path is listed more than once: each such path once, in path order.
    Duplicates(NonEmpty<&'a RepoPath>),
    /// At least one file, and every path listed once.
    Clean,
}

/// One ranked file. Only [`RankedFile::new`] makes one, and it validates.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RankedFile {
    path: RepoPath,
    status: Status,
    score: u32,
    signals: Vec<Signal>,
    evidence: Vec<Evidence>,
    evidence_truncated: bool,
}

/// One piece of code evidence: a signal anchored to a hunk and a line. Only [`Evidence::new`] makes one.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub struct Evidence {
    signal: Signal,
    address: Sha256Hex,
    side: Side,
    line: u64,
}

impl RankDoc {
    /// A ranking, if the producer version is exact (the seal is a [`Sha256Hex`], so already well formed).
    ///
    /// # Errors
    ///
    /// [`RankInvalid`] naming the problem.
    pub fn new(
        input_sha256: Sha256Hex,
        tool_version: String,
        files: Vec<RankedFile>,
    ) -> Result<Self, RankInvalid> {
        if tool_version.trim().is_empty() {
            return Err(RankInvalid::ToolVersionEmpty);
        }
        if tool_version.trim() != tool_version {
            return Err(RankInvalid::ToolVersionWhitespace {
                version: tool_version,
            });
        }
        Ok(Self {
            input_sha256,
            tool_version,
            files,
        })
    }

    /// deep-diff-forge's seal: SHA-256 of the patch bytes it ranked.
    #[must_use]
    pub const fn input_sha256(&self) -> &Sha256Hex {
        &self.input_sha256
    }

    /// The deep-diff-forge version that wrote the ranking.
    #[must_use]
    pub fn tool_version(&self) -> &str {
        &self.tool_version
    }

    /// Files in deep-diff-forge's review order (highest risk first).
    #[must_use]
    pub fn files(&self) -> &[RankedFile] {
        &self.files
    }

    /// Whether this ranking is empty, repeats a path, or is clean. Derived from the files, which
    /// cannot change after construction, so it is the same every time it is asked.
    #[must_use]
    pub fn shape(&self) -> RankShape<'_> {
        if self.files.is_empty() {
            return RankShape::Empty;
        }
        let mut seen = BTreeSet::new();
        let repeated: BTreeSet<&RepoPath> = self
            .files
            .iter()
            .map(RankedFile::path)
            .filter(|path| !seen.insert(*path))
            .collect();
        match NonEmpty::from_vec(repeated.into_iter().collect()) {
            Some(paths) => RankShape::Duplicates(paths),
            None => RankShape::Clean,
        }
    }

    /// This ranking with each file replaced by `widen(file)`, keeping the seal and the version.
    /// Infallible: the seal and version were validated when this document was built, and `widen`
    /// returns files, which are valid by construction (Zen B4).
    pub(crate) fn map_files(&self, widen: impl FnMut(&RankedFile) -> RankedFile) -> Self {
        Self {
            input_sha256: self.input_sha256.clone(),
            tool_version: self.tool_version.clone(),
            files: self.files.iter().map(widen).collect(),
        }
    }
}

/// A change-kind signal Git's facts imply (`new_file`, `deleted_file`, `binary_change`,
/// `symlink_change`, `mode_change`, …): a `&'static str` from the core's own vocabulary, never empty.
pub(crate) type DerivedSignal = Signal;

impl RankedFile {
    /// A ranked file, if its status and signals are non-empty and every piece of evidence cites a
    /// signal the file carries.
    ///
    /// # Errors
    ///
    /// [`RankInvalid`] naming the problem.
    pub fn new(
        path: RepoPath,
        status: Status,
        score: u32,
        signals: Vec<Signal>,
        evidence: Vec<Evidence>,
        evidence_truncated: bool,
    ) -> Result<Self, RankInvalid> {
        if status.name().is_empty() {
            return Err(RankInvalid::EmptyStatus);
        }
        if signals.iter().any(|s| s.name().is_empty()) {
            return Err(RankInvalid::EmptySignal);
        }
        if let Some((index, item)) = evidence
            .iter()
            .enumerate()
            .find(|(_, e)| !signals.contains(&e.signal))
        {
            return Err(RankInvalid::EvidenceSignalMissing {
                index,
                signal: item.signal.name().to_owned(),
            });
        }
        Ok(Self {
            path,
            status,
            score,
            signals,
            evidence,
            evidence_truncated,
        })
    }

    /// This file with every signal in `added` it does not already carry appended, in order, and
    /// nothing removed. Infallible by construction (Zen B4): the status and evidence are unchanged,
    /// evidence still cites signals the file carries (none is removed), and an added signal is a
    /// non-empty core constant (checked by `every_derived_signal_is_a_non_empty_constant`).
    pub(crate) fn with_added_signals<'a>(
        &self,
        added: impl IntoIterator<Item = &'a DerivedSignal>,
    ) -> Self {
        let mut signals = self.signals.clone();
        for signal in added {
            if !signals.contains(signal) {
                signals.push(signal.clone());
            }
        }
        Self {
            path: self.path.clone(),
            status: self.status.clone(),
            score: self.score,
            signals,
            evidence: self.evidence.clone(),
            evidence_truncated: self.evidence_truncated,
        }
    }

    /// The path (patch-author controlled: data, never instruction).
    #[must_use]
    pub const fn path(&self) -> &RepoPath {
        &self.path
    }

    /// The file status, such as `modified`.
    #[must_use]
    pub const fn status(&self) -> &Status {
        &self.status
    }

    /// The additive risk score.
    #[must_use]
    pub const fn score(&self) -> u32 {
        self.score
    }

    /// Every signal that contributed to the score.
    #[must_use]
    pub fn signals(&self) -> &[Signal] {
        &self.signals
    }

    /// The code evidence behind code signals (at most 32 per file upstream).
    #[must_use]
    pub fn evidence(&self) -> &[Evidence] {
        &self.evidence
    }

    /// deep-diff-forge found more evidence than it listed.
    #[must_use]
    pub const fn evidence_truncated(&self) -> bool {
        self.evidence_truncated
    }
}

impl Evidence {
    /// Evidence, if the line is 1-based (the address is a [`Sha256Hex`] and the side a [`Side`]).
    ///
    /// # Errors
    ///
    /// [`RankInvalid`] naming the problem.
    pub fn new(
        signal: Signal,
        address: Sha256Hex,
        side: Side,
        line: u64,
    ) -> Result<Self, RankInvalid> {
        if line == 0 {
            return Err(RankInvalid::ZeroLine);
        }
        Ok(Self {
            signal,
            address,
            side,
            line,
        })
    }

    /// The signal it supports.
    #[must_use]
    pub const fn signal(&self) -> &Signal {
        &self.signal
    }

    /// The hunk's content address.
    #[must_use]
    pub const fn address(&self) -> &Sha256Hex {
        &self.address
    }

    /// `new` (an added line) or `old` (a removed line).
    #[must_use]
    pub const fn side(&self) -> Side {
        self.side
    }

    /// The line number on that side.
    #[must_use]
    pub const fn line(&self) -> u64 {
        self.line
    }
}

/// Why a ranking was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RankError {
    /// More than [`crate::MAX_INPUT_BYTES`].
    #[error("ranking is {0} bytes, over the {max} byte budget", max = crate::MAX_INPUT_BYTES)]
    TooLarge(usize),
    /// Not UTF-8 JSON of the expected shape.
    #[error("ranking is not a valid deep-diff-forge.rank.v0 document: {0}")]
    Malformed(String),
    /// Well-formed JSON whose content breaks the contract.
    #[error("ranking refused: {0}")]
    Invalid(RankInvalid),
}

/// What breaks the `rank.v0` contract, typed so a caller (or a test, or the CLI's exit mapping) can
/// match on it instead of reading prose (Q-TYPES-2, Q1-05). Each variant's message is exactly the
/// text the reader has always printed; locations are fields, not string prefixes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RankInvalid {
    /// `schema` names another document type.
    #[error(
        "schema is {found:?}; expected {RANK_SCHEMA:?} (pipe `deep-diff-forge --stdin-patch --rank --json`)"
    )]
    Schema {
        /// The schema name found.
        found: String,
    },
    /// `tool.name` is not `deep-diff-forge`.
    #[error("tool.name is {found:?}; expected \"deep-diff-forge\"")]
    ToolName {
        /// The tool name found.
        found: String,
    },
    /// `tool.version` is empty or only whitespace.
    #[error("tool.version is empty or whitespace")]
    ToolVersionEmpty,
    /// `tool.version` has leading or trailing whitespace.
    #[error("tool.version {version:?} has surrounding whitespace")]
    ToolVersionWhitespace {
        /// The version as written.
        version: String,
    },
    /// `input_sha256` is not a seal.
    #[error("input_sha256 is not 64 lowercase hex digits")]
    BadSeal,
    /// A file's path is not a canonical repository path.
    #[error("path {0}")]
    Path(PathError),
    /// A file's status is empty.
    #[error("status is empty")]
    EmptyStatus,
    /// A file carries an empty signal.
    #[error("a signal is empty")]
    EmptySignal,
    /// Evidence cites a signal its file does not carry.
    #[error("evidence[{index}]: cites signal {signal:?}, which the file does not carry")]
    EvidenceSignalMissing {
        /// The evidence item's index.
        index: usize,
        /// The cited signal's name.
        signal: String,
    },
    /// An evidence address is not a hunk seal.
    #[error("address is not 64 lowercase hex digits")]
    BadAddress,
    /// An evidence side is not `new` or `old`.
    #[error("side is {side:?}; expected \"new\" or \"old\"")]
    BadSide {
        /// The side as written.
        side: String,
    },
    /// An evidence line is 0 (lines are 1-based).
    #[error("line is 0; lines are 1-based")]
    ZeroLine,
    /// A problem in one evidence item of a file.
    #[error("evidence[{index}]: {problem}")]
    InEvidence {
        /// The evidence item's index.
        index: usize,
        /// What is wrong with it.
        problem: Box<RankInvalid>,
    },
    /// A problem in one ranked file.
    #[error("ranked[{index}]: {problem}")]
    InFile {
        /// The file's index in `ranked`.
        index: usize,
        /// What is wrong with it.
        problem: Box<RankInvalid>,
    },
}

#[derive(Deserialize)]
struct RawRank {
    schema: String,
    input_sha256: String,
    tool: RawTool,
    ranked: Vec<RawFile>,
}

#[derive(Deserialize)]
struct RawTool {
    name: String,
    version: String,
}

#[derive(Deserialize)]
struct RawFile {
    path: String,
    status: String,
    score: u32,
    signals: Vec<String>,
    #[serde(default)]
    evidence: Vec<RawEvidence>,
    #[serde(default)]
    evidence_truncated: bool,
}

#[derive(Deserialize)]
struct RawEvidence {
    signal: String,
    address: String,
    side: String,
    line: u64,
}

/// Parse and validate a ranking from its exact bytes.
///
/// # Errors
///
/// [`RankError`] if the bytes exceed the budget, are not a `rank.v0` document, or break its
/// contract (a bad seal, a foreign tool, an empty path, evidence that cites a signal the file
/// does not carry, …).
pub fn parse_rank(bytes: &[u8]) -> Result<RankDoc, RankError> {
    if bytes.len() > crate::MAX_INPUT_BYTES {
        return Err(RankError::TooLarge(bytes.len()));
    }
    let raw: RawRank =
        serde_json::from_slice(bytes).map_err(|e| RankError::Malformed(e.to_string()))?;
    if raw.schema != RANK_SCHEMA {
        return Err(RankError::Invalid(RankInvalid::Schema {
            found: raw.schema,
        }));
    }
    if raw.tool.name != "deep-diff-forge" {
        return Err(RankError::Invalid(RankInvalid::ToolName {
            found: raw.tool.name,
        }));
    }
    let files = raw
        .ranked
        .into_iter()
        .enumerate()
        .map(|(index, file)| validate_file(index, file))
        .collect::<Result<Vec<_>, _>>()?;
    let input_sha256 = Sha256Hex::parse(&raw.input_sha256)
        .map_err(|_| RankError::Invalid(RankInvalid::BadSeal))?;
    RankDoc::new(input_sha256, raw.tool.version, files).map_err(RankError::Invalid)
}

fn validate_file(index: usize, raw: RawFile) -> Result<RankedFile, RankError> {
    let in_file = |problem: RankInvalid| {
        RankError::Invalid(RankInvalid::InFile {
            index,
            problem: Box::new(problem),
        })
    };
    let path = RepoPath::parse(&raw.path).map_err(|e| in_file(RankInvalid::Path(e)))?;
    let evidence = raw
        .evidence
        .into_iter()
        .enumerate()
        .map(|(e_index, item)| {
            let in_evidence = |problem: RankInvalid| {
                in_file(RankInvalid::InEvidence {
                    index: e_index,
                    problem: Box::new(problem),
                })
            };
            let address = Sha256Hex::parse(&item.address)
                .map_err(|_| in_evidence(RankInvalid::BadAddress))?;
            let side = Side::parse(&item.side).ok_or_else(|| {
                in_evidence(RankInvalid::BadSide {
                    side: item.side.clone(),
                })
            })?;
            Evidence::new(Signal::parse(&item.signal), address, side, item.line)
                .map_err(in_evidence)
        })
        .collect::<Result<Vec<_>, _>>()?;
    RankedFile::new(
        path,
        Status::parse(&raw.status),
        raw.score,
        raw.signals.iter().map(|s| Signal::parse(s)).collect(),
        evidence,
        raw.evidence_truncated,
    )
    .map_err(in_file)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEAL: &str = "9317219840467a00fbb7bc3ff905b0ae26a999582c12c8d02f49d380b398bbb3";

    fn doc(ranked: &str) -> String {
        format!(
            r#"{{"schema":"deep-diff-forge.rank.v0","input_sha256":"{SEAL}","tool":{{"name":"deep-diff-forge","version":"0.7.0"}},"ranked":[{ranked}]}}"#
        )
    }

    fn ranking(paths: &[&str]) -> RankDoc {
        let files = paths
            .iter()
            .map(|p| {
                RankedFile::new(
                    RepoPath::parse(p).unwrap(),
                    Status::parse("added"),
                    1,
                    vec![],
                    vec![],
                    false,
                )
                .unwrap()
            })
            .collect();
        RankDoc::new(Sha256Hex::parse(SEAL).unwrap(), "0.7.0".to_owned(), files).unwrap()
    }

    fn repeated(doc: &RankDoc) -> Option<Vec<&str>> {
        match doc.shape() {
            RankShape::Duplicates(paths) => Some(paths.iter().map(|p| p.as_str()).collect()),
            RankShape::Empty | RankShape::Clean => None,
        }
    }

    #[test]
    fn a_ranking_is_empty_clean_or_names_each_repeated_path_once_in_order() {
        assert_eq!(ranking(&[]).shape(), RankShape::Empty);
        assert_eq!(ranking(&["a"]).shape(), RankShape::Clean);
        assert_eq!(ranking(&["b", "a"]).shape(), RankShape::Clean);
        assert_eq!(repeated(&ranking(&["a", "a"])), Some(vec!["a"]));
        assert_eq!(
            repeated(&ranking(&["z", "b", "z", "b", "z", "c"])),
            Some(vec!["b", "z"])
        );
    }

    proptest::proptest! {
        #[test]
        fn the_shape_is_exactly_what_the_paths_say(
            picks in proptest::collection::vec(0usize..4, 0..12)
        ) {
            let names = ["a", "b", "c/d", "e"];
            let paths: Vec<&str> = picks.iter().filter_map(|i| names.get(*i).copied()).collect();
            let doc = ranking(&paths);
            let mut counts = std::collections::BTreeMap::new();
            for p in &paths {
                let n = counts.entry(*p).or_insert(0u32);
                *n = n.saturating_add(1);
            }
            let twice: Vec<&str> = counts.iter().filter(|(_, n)| **n > 1).map(|(p, _)| *p).collect();
            let expected = if paths.is_empty() {
                "empty"
            } else if twice.is_empty() {
                "clean"
            } else {
                "duplicates"
            };
            let found = match doc.shape() {
                RankShape::Empty => "empty",
                RankShape::Clean => "clean",
                RankShape::Duplicates(_) => "duplicates",
            };
            proptest::prop_assert_eq!(found, expected);
            if !twice.is_empty() {
                proptest::prop_assert_eq!(repeated(&doc), Some(twice));
            }
        }
    }

    #[test]
    fn a_contract_refusal_is_typed_with_its_location_and_keeps_its_message() {
        // Q-TYPES-2 (Q1-05): a caller matches on the problem and its indices, not on prose.
        let text = doc(&format!(
            r#"{{"path":"a","status":"added","score":1,"signals":["secret_like"],"evidence":[{{"signal":"secret_like","address":"{SEAL}","side":"new","line":1}},{{"signal":"secret_like","address":"{SEAL}","side":"both","line":1}}]}}"#
        ));
        let error = parse_rank(text.as_bytes()).unwrap_err();
        assert_eq!(
            error,
            RankError::Invalid(RankInvalid::InFile {
                index: 0,
                problem: Box::new(RankInvalid::InEvidence {
                    index: 1,
                    problem: Box::new(RankInvalid::BadSide {
                        side: "both".to_owned()
                    }),
                }),
            })
        );
        assert_eq!(
            error.to_string(),
            "ranking refused: ranked[0]: evidence[1]: side is \"both\"; expected \"new\" or \"old\""
        );
        let bad_path = doc(r#"{"path":"a//b","status":"added","score":1,"signals":[]}"#);
        assert!(matches!(
            parse_rank(bad_path.as_bytes()),
            Err(RankError::Invalid(RankInvalid::InFile { index: 0, problem }))
                if *problem == RankInvalid::Path(PathError::NonCanonical)
        ));
    }

    #[test]
    fn added_signals_are_appended_once_in_order_and_nothing_is_lost() {
        // Zen B4: widening keeps every original signal, evidence item and field, adds only the
        // missing signals in the order given, and never duplicates one.
        let text = doc(&format!(
            r#"{{"path":"src/lib.rs","status":"modified","score":9,"signals":["unsafe_added","binary_change"],
               "evidence":[{{"signal":"unsafe_added","address":"{SEAL}","side":"new","line":3,"basis":"lexical"}}],"evidence_truncated":true}}"#
        ));
        let rank = parse_rank(text.as_bytes()).unwrap();
        let file = &rank.files[0];
        let wide = file.with_added_signals(
            &["binary_change", "deleted_file", "mode_change"].map(Signal::parse),
        );
        assert_eq!(
            wide.signals.iter().map(Signal::name).collect::<Vec<_>>(),
            [
                "unsafe_added",
                "binary_change",
                "deleted_file",
                "mode_change"
            ]
        );
        assert_eq!(wide.evidence, file.evidence);
        assert_eq!(wide.path, file.path);
        assert_eq!(wide.status, file.status);
        assert_eq!(wide.score, file.score);
        assert!(wide.evidence_truncated);
        assert_eq!(
            &file.with_added_signals(&[]),
            file,
            "adding nothing changes nothing"
        );

        let mapped = rank.map_files(|f| f.with_added_signals(&[Signal::NewFile]));
        assert_eq!(mapped.input_sha256(), rank.input_sha256());
        assert_eq!(mapped.tool_version(), rank.tool_version());
        assert_eq!(mapped.files.len(), 1);
        assert_eq!(
            mapped.files[0]
                .signals
                .iter()
                .map(Signal::name)
                .collect::<Vec<_>>(),
            ["unsafe_added", "binary_change", "new_file"]
        );
    }

    #[test]
    fn reads_a_real_shape_and_ignores_additive_members() {
        let text = doc(&format!(
            r#"{{"path":"src/lib.rs","status":"modified","score":9,"signals":["public_api_surface","unsafe_added"],"future_member":1,
               "evidence":[{{"signal":"unsafe_added","address":"{SEAL}","side":"new","line":3,"basis":"lexical"}}]}}"#
        ));
        let rank = parse_rank(text.as_bytes()).unwrap();
        assert_eq!(rank.files.len(), 1);
        assert_eq!(rank.files[0].evidence[0].line, 3);
        assert_eq!(rank.tool_version(), "0.7.0");
        assert_eq!(rank.input_sha256().as_str(), SEAL);
    }

    #[test]
    fn refusals_name_the_problem() {
        let cases = [
            (doc("").replace("rank.v0", "review.v0"), "schema"),
            (doc("").replace(SEAL, "abc"), "input_sha256"),
            (
                doc("").replace("\"name\":\"deep-diff-forge\"", "\"name\":\"other\""),
                "tool.name",
            ),
            (
                doc("").replace("\"version\":\"0.7.0\"", "\"version\":\" 0.7.0\""),
                "surrounding whitespace",
            ),
            (
                doc("").replace("\"version\":\"0.7.0\"", "\"version\":\" \""),
                "empty or whitespace",
            ),
            (
                doc(r#"{"path":"","status":"added","score":1,"signals":[]}"#),
                "path is empty",
            ),
            (
                doc(r#"{"path":"a","status":"added","score":1,"signals":[""]}"#),
                "signal is empty",
            ),
            (
                doc(&format!(
                    r#"{{"path":"a","status":"added","score":1,"signals":[],"evidence":[{{"signal":"secret_like","address":"{SEAL}","side":"new","line":1}}]}}"#
                )),
                "does not carry",
            ),
            (
                doc(&format!(
                    r#"{{"path":"a","status":"added","score":1,"signals":["secret_like"],"evidence":[{{"signal":"secret_like","address":"{SEAL}","side":"both","line":1}}]}}"#
                )),
                "side",
            ),
            (
                doc(&format!(
                    r#"{{"path":"a","status":"added","score":1,"signals":["secret_like"],"evidence":[{{"signal":"secret_like","address":"{SEAL}","side":"new","line":0}}]}}"#
                )),
                "1-based",
            ),
        ];
        for (text, needle) in cases {
            let error = parse_rank(text.as_bytes()).unwrap_err().to_string();
            assert!(
                error.contains(needle),
                "{error:?} should mention {needle:?}"
            );
        }
    }

    #[test]
    fn non_json_is_malformed_and_never_panics() {
        for bytes in [&b""[..], b"{", b"\xff\xfe", b"[]", b"null"] {
            assert!(matches!(parse_rank(bytes), Err(RankError::Malformed(_))));
        }
    }

    #[test]
    fn the_budget_is_64_mib_and_one_byte_more_is_refused_unparsed() {
        assert_eq!(crate::MAX_INPUT_BYTES, 67_108_864);
        let over = vec![b' '; crate::MAX_INPUT_BYTES + 1];
        assert_eq!(
            parse_rank(&over),
            Err(RankError::TooLarge(crate::MAX_INPUT_BYTES + 1))
        );
        let at = vec![b' '; crate::MAX_INPUT_BYTES];
        assert!(
            matches!(parse_rank(&at), Err(RankError::Malformed(_))),
            "at the budget it is parsed"
        );
    }

    #[test]
    fn negative_scores_are_refused() {
        let text = doc(r#"{"path":"a","status":"added","score":-1,"signals":[]}"#);
        assert!(matches!(
            parse_rank(text.as_bytes()),
            Err(RankError::Malformed(_))
        ));
    }
}
