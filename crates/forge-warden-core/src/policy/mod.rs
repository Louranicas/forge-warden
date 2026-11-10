//! Reading and validating a policy (`warden.toml`, schema `forge-warden.policy.v0`).
//!
//! A policy is held to the same standard as code. Loading it either yields a fully typed
//! [`Policy`] or refuses with **every** problem found, each with a location, so an author fixes
//! them in one pass. Required fields have no defaults: a rule says what it means.
//!
//! ```toml
//! schema = "forge-warden.policy.v0"
//!
//! [[rule]]
//! id = "no-secrets"
//! why = "A credential committed once is compromised forever."
//! kind = "forbid_signal"
//! severity = "fail"
//! signals = ["secret_like"]
//! ```
//!
//! The module is split by reason to change: `model` holds the validated types, `paths` the glob
//! semantics, `raw` the TOML shapes, `kind` the rule kinds and their fields, `validate` the
//! parsing and validation, `issues` the issue collector (a refused rule always records why), and
//! `error` the refusal types.
//! A rule or waiver is only ever left out of a [`Policy`] together with an issue that refuses the
//! whole policy, so a declared rule cannot vanish.

mod error;
mod issues;
mod kind;
mod model;
mod paths;
mod raw;
mod validate;

pub use error::{PolicyError, PolicyIssue};
pub use kind::{Field, Kind};
pub use model::{AdmittedProducer, Check, Policy, Rule, ScorePer, Severity, Waiver};
pub use paths::{PathSet, Scope};
pub use validate::{MAX_ID_CHARS, MIN_WHY_CHARS, POLICY_SCHEMA, parse_policy};

#[cfg(test)]
mod tests;
