//! The TOML shapes a policy is read into, and the table of kinds and fields.

use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawPolicy {
    pub(super) schema: String,
    #[serde(default, rename = "rule")]
    pub(super) rules: Vec<RawRule>,
    #[serde(default, rename = "waiver")]
    pub(super) waivers: Vec<RawWaiver>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawRule {
    pub(super) id: String,
    pub(super) why: String,
    pub(super) kind: String,
    pub(super) severity: String,
    pub(super) signals: Option<Vec<String>>,
    pub(super) paths: Option<Vec<String>>,
    pub(super) except: Option<Vec<String>>,
    pub(super) when: Option<Vec<String>>,
    pub(super) require: Option<Vec<String>>,
    pub(super) limit: Option<u32>,
    pub(super) per: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawWaiver {
    pub(super) rule: String,
    pub(super) paths: Vec<String>,
    pub(super) reason: String,
    pub(super) owner: String,
    pub(super) expires: String,
}

/// Every optional field a rule may carry, and the kinds that accept it. (`id`, `why`, `kind` and
/// `severity` are required on every rule.)
pub const RULE_FIELD_KINDS: [(&str, &[&str]); 7] = [
    ("signals", &["forbid_signal"]),
    (
        "paths",
        &["forbid_signal", "protect_paths", "max_score", "max_files"],
    ),
    (
        "except",
        &[
            "forbid_signal",
            "require_companion",
            "protect_paths",
            "max_score",
            "max_files",
        ],
    ),
    ("when", &["require_companion"]),
    ("require", &["require_companion"]),
    ("limit", &["max_score", "max_files"]),
    ("per", &["max_score"]),
];

/// The kinds a policy may use.
pub const KINDS: [&str; 5] = [
    "forbid_signal",
    "require_companion",
    "protect_paths",
    "max_score",
    "max_files",
];
