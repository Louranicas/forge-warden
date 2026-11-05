//! Reading a `deep-diff-forge.rank.v0` document.
//!
//! The reader is **tolerant of additions and strict about meaning**. deep-diff-forge adds fields
//! without changing the schema name, so unknown members are ignored. Every member forge-warden
//! relies on is checked: the schema name, the seal, the tool, and each file's path, status, score,
//! signals and evidence. A document that fails any check is refused as a whole.

use crate::seal::is_sha256_hex;
use serde::Deserialize;

/// The only ranking schema this version reads.
pub const RANK_SCHEMA: &str = "deep-diff-forge.rank.v0";

/// A validated ranking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankDoc {
    /// deep-diff-forge's seal: SHA-256 of the patch bytes it ranked.
    pub input_sha256: String,
    /// The deep-diff-forge version that wrote the ranking.
    pub tool_version: String,
    /// Files in deep-diff-forge's review order (highest risk first).
    pub files: Vec<RankedFile>,
}

/// One ranked file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankedFile {
    /// The path as deep-diff-forge reports it (patch-author controlled: data, never instruction).
    pub path: String,
    /// The file status, such as `modified`.
    pub status: String,
    /// The additive risk score.
    pub score: u32,
    /// Every signal that contributed to the score.
    pub signals: Vec<String>,
    /// The code evidence behind code signals (at most 32 per file upstream).
    pub evidence: Vec<Evidence>,
    /// deep-diff-forge found more evidence than it listed.
    pub evidence_truncated: bool,
}

/// One piece of code evidence: a signal anchored to a hunk and a line.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Evidence {
    /// The signal it supports.
    pub signal: String,
    /// The hunk's content address.
    pub address: String,
    /// `new` (an added line) or `old` (a removed line).
    pub side: String,
    /// The line number on that side.
    pub line: u64,
}

/// Why a ranking was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
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
    if !is_sha256_hex(&raw.input_sha256) {
        return Err(RankError::Invalid(
            "input_sha256 is not 64 lowercase hex digits".to_owned(),
        ));
    }
    if raw.tool.name != "deep-diff-forge" {
        return Err(RankError::Invalid(format!(
            "tool.name is {:?}; expected \"deep-diff-forge\"",
            raw.tool.name
        )));
    }
    if raw.tool.version.is_empty() {
        return Err(RankError::Invalid("tool.version is empty".to_owned()));
    }
    let files = raw
        .ranked
        .into_iter()
        .enumerate()
        .map(|(index, file)| validate_file(index, file))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(RankDoc {
        input_sha256: raw.input_sha256,
        tool_version: raw.tool.version,
        files,
    })
}

fn validate_file(index: usize, raw: RawFile) -> Result<RankedFile, RankError> {
    let at = |what: &str| RankError::Invalid(format!("ranked[{index}]: {what}"));
    if raw.path.is_empty() {
        return Err(at("path is empty"));
    }
    if raw.path.contains('\0') {
        return Err(at("path contains NUL"));
    }
    if raw.status.is_empty() {
        return Err(at("status is empty"));
    }
    if raw.signals.iter().any(String::is_empty) {
        return Err(at("a signal is empty"));
    }
    let mut evidence = Vec::with_capacity(raw.evidence.len());
    for (e_index, item) in raw.evidence.into_iter().enumerate() {
        let at_e = |what: &str| at(&format!("evidence[{e_index}]: {what}"));
        if !raw.signals.contains(&item.signal) {
            return Err(at_e(&format!(
                "cites signal {:?}, which the file does not carry",
                item.signal
            )));
        }
        if !is_sha256_hex(&item.address) {
            return Err(at_e("address is not 64 lowercase hex digits"));
        }
        if item.side != "new" && item.side != "old" {
            return Err(at_e(&format!(
                "side is {:?}; expected \"new\" or \"old\"",
                item.side
            )));
        }
        if item.line == 0 {
            return Err(at_e("line is 0; lines are 1-based"));
        }
        evidence.push(Evidence {
            signal: item.signal,
            address: item.address,
            side: item.side,
            line: item.line,
        });
    }
    Ok(RankedFile {
        path: raw.path,
        status: raw.status,
        score: raw.score,
        signals: raw.signals,
        evidence,
        evidence_truncated: raw.evidence_truncated,
    })
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
    fn reads_a_real_shape_and_ignores_additive_members() {
        let text = doc(&format!(
            r#"{{"path":"src/lib.rs","status":"modified","score":9,"signals":["public_api_surface","unsafe_added"],"future_member":1,
               "evidence":[{{"signal":"unsafe_added","address":"{SEAL}","side":"new","line":3,"basis":"lexical"}}]}}"#
        ));
        let rank = parse_rank(text.as_bytes()).unwrap();
        assert_eq!(rank.files.len(), 1);
        assert_eq!(rank.files[0].evidence[0].line, 3);
        assert_eq!(rank.tool_version, "0.7.0");
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
