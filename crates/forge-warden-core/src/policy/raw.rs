//! The TOML shapes a policy is read into.

use super::kind::Field;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawPolicy {
    pub(super) schema: String,
    #[serde(default, rename = "rule")]
    pub(super) rules: Vec<RawRule>,
    #[serde(default, rename = "waiver")]
    pub(super) waivers: Vec<RawWaiver>,
    pub(super) producer: Option<RawProducer>,
}

/// `[producer]`: the rankers authority mode admits, by the SHA-256 of their bytes.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawProducer {
    pub(super) sha256: Vec<String>,
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

impl RawRule {
    /// Whether the rule carries `field`. One arm per field, so a new field must be wired here.
    pub(super) const fn has(&self, field: Field) -> bool {
        match field {
            Field::Signals => self.signals.is_some(),
            Field::Paths => self.paths.is_some(),
            Field::Except => self.except.is_some(),
            Field::When => self.when.is_some(),
            Field::Require => self.require.is_some(),
            Field::Limit => self.limit.is_some(),
            Field::Per => self.per.is_some(),
        }
    }
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
