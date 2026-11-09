//! Reading a `deep-diff-forge.rank.v0` document.
//!
//! The reader is **tolerant of additions and strict about meaning**. deep-diff-forge adds fields
//! without changing the schema name, so unknown members are ignored. Every member forge-warden
//! relies on is checked: the schema name, the seal, the tool, and each file's path, status, score,
//! signals and evidence. A document that fails any check is refused as a whole.

use crate::path::RepoPath;
use crate::seal::Sha256Hex;
use serde::Deserialize;

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

/// One ranked file. Only [`RankedFile::new`] makes one, and it validates.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RankedFile {
    path: RepoPath,
    status: String,
    score: u32,
    signals: Vec<String>,
    evidence: Vec<Evidence>,
    evidence_truncated: bool,
}

/// One piece of code evidence: a signal anchored to a hunk and a line. Only [`Evidence::new`] makes one.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub struct Evidence {
    signal: String,
    address: Sha256Hex,
    side: String,
    line: u64,
}

impl RankDoc {
    /// A ranking, if the producer version is exact (the seal is a [`Sha256Hex`], so already well formed).
    ///
    /// # Errors
    ///
    /// [`RankError::Invalid`] naming the problem.
    pub fn new(
        input_sha256: Sha256Hex,
        tool_version: String,
        files: Vec<RankedFile>,
    ) -> Result<Self, RankError> {
        if tool_version.trim().is_empty() {
            return Err(RankError::Invalid(
                "tool.version is empty or whitespace".to_owned(),
            ));
        }
        if tool_version.trim() != tool_version {
            return Err(RankError::Invalid(format!(
                "tool.version {tool_version:?} has surrounding whitespace"
            )));
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
pub(crate) type DerivedSignal = &'static str;

impl RankedFile {
    /// A ranked file, if its status and signals are non-empty and every piece of evidence cites a
    /// signal the file carries.
    ///
    /// # Errors
    ///
    /// [`RankError::Invalid`] naming the problem.
    pub fn new(
        path: RepoPath,
        status: String,
        score: u32,
        signals: Vec<String>,
        evidence: Vec<Evidence>,
        evidence_truncated: bool,
    ) -> Result<Self, RankError> {
        if status.is_empty() {
            return Err(RankError::Invalid("status is empty".to_owned()));
        }
        if signals.iter().any(String::is_empty) {
            return Err(RankError::Invalid("a signal is empty".to_owned()));
        }
        if let Some((index, item)) = evidence
            .iter()
            .enumerate()
            .find(|(_, e)| !signals.contains(&e.signal))
        {
            return Err(RankError::Invalid(format!(
                "evidence[{index}]: cites signal {:?}, which the file does not carry",
                item.signal
            )));
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
            if !signals.iter().any(|s| s == signal) {
                signals.push((*signal).to_owned());
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
    pub fn status(&self) -> &str {
        &self.status
    }

    /// The additive risk score.
    #[must_use]
    pub const fn score(&self) -> u32 {
        self.score
    }

    /// Every signal that contributed to the score.
    #[must_use]
    pub fn signals(&self) -> &[String] {
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
    /// Evidence, if the side is `new` or `old` and the line is 1-based (the address is a [`Sha256Hex`]).
    ///
    /// # Errors
    ///
    /// [`RankError::Invalid`] naming the problem.
    pub fn new(
        signal: String,
        address: Sha256Hex,
        side: String,
        line: u64,
    ) -> Result<Self, RankError> {
        if side != "new" && side != "old" {
            return Err(RankError::Invalid(format!(
                "side is {side:?}; expected \"new\" or \"old\""
            )));
        }
        if line == 0 {
            return Err(RankError::Invalid(
                "line is 0; lines are 1-based".to_owned(),
            ));
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
    pub fn signal(&self) -> &str {
        &self.signal
    }

    /// The hunk's content address.
    #[must_use]
    pub const fn address(&self) -> &Sha256Hex {
        &self.address
    }

    /// `new` (an added line) or `old` (a removed line).
    #[must_use]
    pub fn side(&self) -> &str {
        &self.side
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
    Invalid(String),
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
        return Err(RankError::Invalid(format!(
            "schema is {:?}; expected {RANK_SCHEMA:?} (pipe `deep-diff-forge --stdin-patch --rank --json`)",
            raw.schema
        )));
    }
    if raw.tool.name != "deep-diff-forge" {
        return Err(RankError::Invalid(format!(
            "tool.name is {:?}; expected \"deep-diff-forge\"",
            raw.tool.name
        )));
    }
    let files = raw
        .ranked
        .into_iter()
        .enumerate()
        .map(|(index, file)| validate_file(index, file))
        .collect::<Result<Vec<_>, _>>()?;
    let input_sha256 = Sha256Hex::parse(&raw.input_sha256).map_err(|_| {
        RankError::Invalid("input_sha256 is not 64 lowercase hex digits".to_owned())
    })?;
    RankDoc::new(input_sha256, raw.tool.version, files)
}

fn validate_file(index: usize, raw: RawFile) -> Result<RankedFile, RankError> {
    let at = |what: &str| RankError::Invalid(format!("ranked[{index}]: {what}"));
    let path = RepoPath::parse(&raw.path).map_err(|e| at(&format!("path {e}")))?;
    let evidence = raw
        .evidence
        .into_iter()
        .enumerate()
        .map(|(e_index, item)| {
            let address = Sha256Hex::parse(&item.address).map_err(|_| {
                at(&format!(
                    "evidence[{e_index}]: address is not 64 lowercase hex digits"
                ))
            })?;
            Evidence::new(item.signal, address, item.side, item.line)
                .map_err(|e| at(&format!("evidence[{e_index}]: {}", reason(&e))))
        })
        .collect::<Result<Vec<_>, _>>()?;
    RankedFile::new(
        path,
        raw.status,
        raw.score,
        raw.signals,
        evidence,
        raw.evidence_truncated,
    )
    .map_err(|e| at(&reason(&e)))
}

/// The message inside a refusal, without its prefix, so a caller can add a location.
fn reason(error: &RankError) -> String {
    match error {
        RankError::Invalid(message) | RankError::Malformed(message) => message.clone(),
        RankError::TooLarge(bytes) => format!("{bytes} bytes"),
    }
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
        let wide = file.with_added_signals(&["binary_change", "deleted_file", "mode_change"]);
        assert_eq!(
            wide.signals,
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

        let mapped = rank.map_files(|f| f.with_added_signals(&["new_file"]));
        assert_eq!(mapped.input_sha256(), rank.input_sha256());
        assert_eq!(mapped.tool_version(), rank.tool_version());
        assert_eq!(mapped.files.len(), 1);
        assert_eq!(
            mapped.files[0].signals,
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
