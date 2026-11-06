//! # forge-warden-core
//!
//! The pure engine behind `forge-warden`: a repository-owned change policy, evaluated over the
//! sealed risk ranking that [deep-diff-forge] produces (`deep-diff-forge.rank.v0`), giving a
//! sealed, deterministic verdict (`forge-warden.verdict.v0`).
//!
//! deep-diff-forge answers *"what does this change do, and where is the risk?"*. forge-warden
//! answers the question that follows: *"does this change meet the standards this repository has
//! written down?"*
//!
//! ## Guarantees (each one is a test, not a promise)
//!
//! - **Pure.** No I/O, no clock, no environment, no threads (`clippy.toml` in this crate makes
//!   them compile errors). Bytes and a date go in; a [`Verdict`] comes out.
//! - **Deterministic.** The same policy bytes, ranking bytes and date give byte-identical output.
//!   No hash-ordered collection is reachable from this crate.
//! - **Unknown is never pass.** A signal or status the policy vocabulary does not know, an empty
//!   ranking, or an expired waiver can only make a verdict stricter.
//! - **Policies are checked like code.** Unknown keys, unknown signals, invalid globs, missing
//!   rationale, duplicate ids and waivers for rules that do not exist are refused, all at once,
//!   with a location for each.
//!
//! [deep-diff-forge]: https://github.com/Louranicas/deep-diff-forge

pub mod date;
pub mod eval;
pub mod exit;
pub mod nonempty;
pub mod path;
pub mod policy;
pub mod rank;
pub mod render;
pub mod seal;
pub mod verdict;
pub mod vocab;

pub use date::Date;
pub use eval::evaluate;
pub use exit::{EXIT_REGISTRY, Exit, ExitEntry, Outcome};
pub use policy::{Policy, PolicyError, PolicyIssue};
pub use rank::{RankDoc, RankError};
pub use verdict::Verdict;

/// The name every forge-warden document carries in its `tool` member.
pub const TOOL_NAME: &str = "forge-warden";

/// The version every forge-warden document carries in its `tool` member.
pub const TOOL_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The largest ranking or policy document forge-warden will read, in bytes (64 MiB, the same
/// budget deep-diff-forge applies to a patch). Larger input is refused, never truncated.
pub const MAX_INPUT_BYTES: usize = 64 * 1024 * 1024;
