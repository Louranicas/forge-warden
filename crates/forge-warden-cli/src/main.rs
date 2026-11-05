//! `forge-warden`: change policy as code for deep-diff-forge reviews.
//!
//! ```text
//! git diff main... | tee change.patch | deep-diff-forge --stdin-patch --rank --json \
//!   | forge-warden check --policy warden.toml --patch change.patch
//! ```
//!
//! Results go to stdout, diagnostics to stderr, and on error stdout is empty. Every exit code is
//! in the registry (`forge-warden schema exit-codes`), with the next action to take.

mod doors;

use clap::{Parser, Subcommand};
use forge_warden_core::exit::{entry_by_code, entry_by_name};
use forge_warden_core::policy::{PolicyError, parse_policy};
use forge_warden_core::rank::{RankError, parse_rank};
use forge_warden_core::render::render_text;
use forge_warden_core::seal::sha256_hex;
use forge_warden_core::verdict::Seals;
use forge_warden_core::{Date, EXIT_REGISTRY, evaluate};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The JSON Schema of `forge-warden.verdict.v0`, committed and checked against the code.
const VERDICT_SCHEMA_JSON: &str = include_str!("../../../schemas/verdict.v0.schema.json");
/// The JSON Schema of `forge-warden.policy.v0` (TOML editors can validate against it).
const POLICY_SCHEMA_JSON: &str = include_str!("../../../schemas/policy.v0.schema.json");
/// The starter policy `forge-warden init` prints.
const STARTER_POLICY: &str = include_str!("../../../examples/starter.warden.toml");

#[derive(Parser)]
#[command(
    name = "forge-warden",
    version,
    about = "Change policy as code for deep-diff-forge reviews: a sealed, deterministic gate.",
    long_about = None,
    after_help = "Exit codes: forge-warden schema exit-codes · forge-warden explain <code>"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Judge a ranking (deep-diff-forge --rank --json) against a policy.
    Check {
        /// The policy file.
        #[arg(long, value_name = "FILE")]
        policy: PathBuf,
        /// The ranking document; `-` or absent reads stdin.
        #[arg(long, value_name = "FILE")]
        rank: Option<PathBuf>,
        /// The patch that was ranked. Its SHA-256 must equal the ranking's seal, binding the
        /// verdict to the exact bytes reviewed.
        #[arg(long, value_name = "FILE")]
        patch: Option<PathBuf>,
        /// Judge waivers as of this date (YYYY-MM-DD) instead of today (UTC).
        #[arg(long, value_name = "DATE")]
        today: Option<String>,
        /// Print the forge-warden.verdict.v0 document instead of the human summary.
        #[arg(long)]
        json: bool,
    },
    /// Validate a policy without judging anything; lists every issue at once.
    Lint {
        /// The policy file.
        policy: PathBuf,
    },
    /// Explain an exit code (by number or name) and the action it asks for.
    Explain {
        /// A code such as `7`, or a name such as `policy_hold`.
        code: String,
    },
    /// Print a JSON Schema or the exit-code registry.
    Schema {
        /// What to print.
        #[arg(value_parser = ["verdict", "policy", "exit-codes"])]
        what: String,
    },
    /// Print a starter policy: `forge-warden init > warden.toml`.
    Init,
}

/// A failure with its registry exit code.
struct Failure {
    code: u8,
    message: String,
}

impl Failure {
    fn new(name: &str, message: impl Into<String>) -> Self {
        let code = entry_by_name(name).map_or(101, |entry| entry.code);
        Self {
            code,
            message: message.into(),
        }
    }

    fn from_door(error: doors::DoorError) -> Self {
        let name = if error.over_budget {
            "invalid_input"
        } else {
            "io"
        };
        Self::new(name, error.message)
    }
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let text = error.render().to_string();
            return if error.use_stderr() {
                doors::diagnose(&text);
                ExitCode::from(2)
            } else {
                finish(doors::emit(&text).map(|()| 0).map_err(Failure::from_door))
            };
        }
    };
    finish(run(cli.command))
}

fn finish(result: Result<u8, Failure>) -> ExitCode {
    match result {
        Ok(code) => ExitCode::from(code),
        Err(failure) => {
            doors::diagnose(&format!("forge-warden: {}", failure.message));
            if let Some(entry) = entry_by_code(failure.code) {
                doors::diagnose(&format!(
                    "next: {} (forge-warden explain {})",
                    entry.action, entry.code
                ));
            }
            ExitCode::from(failure.code)
        }
    }
}

fn run(command: Command) -> Result<u8, Failure> {
    match command {
        Command::Check {
            policy,
            rank,
            patch,
            today,
            json,
        } => check(&policy, rank, patch, today.as_deref(), json),
        Command::Lint { policy } => lint(&policy),
        Command::Explain { code } => explain(&code),
        Command::Schema { what } => schema(&what),
        Command::Init => emit(STARTER_POLICY),
    }
}

fn emit(text: &str) -> Result<u8, Failure> {
    doors::emit(text).map_err(Failure::from_door)?;
    Ok(0)
}

fn policy_failure(error: &PolicyError) -> Failure {
    Failure::new("invalid_input", error.to_string())
}

fn rank_failure(error: &RankError) -> Failure {
    Failure::new("invalid_input", error.to_string())
}

fn check(
    policy_path: &Path,
    rank_path: Option<PathBuf>,
    patch_path: Option<PathBuf>,
    today: Option<&str>,
    json: bool,
) -> Result<u8, Failure> {
    let today = match today {
        Some(text) => Date::parse(text).ok_or_else(|| {
            Failure::new(
                "usage",
                format!("--today {text:?} is not a YYYY-MM-DD date"),
            )
        })?,
        None => doors::today().map_err(Failure::from_door)?,
    };
    let policy_bytes = doors::read_file(policy_path).map_err(Failure::from_door)?;
    let policy = parse_policy(&policy_bytes).map_err(|e| policy_failure(&e))?;
    let rank_bytes = match rank_path {
        Some(path) if path.as_os_str() != "-" => doors::read_file(&path),
        _ => doors::read_stdin(),
    }
    .map_err(Failure::from_door)?;
    let rank = parse_rank(&rank_bytes).map_err(|e| rank_failure(&e))?;

    let patch_verified = match patch_path {
        Some(path) => {
            let patch_bytes = doors::read_file(&path).map_err(Failure::from_door)?;
            let actual = sha256_hex(&patch_bytes);
            if actual != rank.input_sha256 {
                return Err(Failure::new(
                    "invalid_input",
                    format!(
                        "the ranking describes patch {} but {} hashes to {actual}; re-rank the patch you mean to judge",
                        rank.input_sha256,
                        path.display()
                    ),
                ));
            }
            true
        }
        None => false,
    };

    let seals = Seals {
        policy_sha256: sha256_hex(&policy_bytes),
        rank_sha256: sha256_hex(&rank_bytes),
        patch_sha256: rank.input_sha256.clone(),
        patch_verified,
        ranked_by: rank.tool_version.clone(),
    };
    let verdict = evaluate(&policy, &rank, seals, today);
    let text = if json {
        verdict.to_json()
    } else {
        render_text(&verdict)
    };
    doors::emit(&text).map_err(Failure::from_door)?;
    Ok(verdict.exit.code)
}

fn lint(policy_path: &Path) -> Result<u8, Failure> {
    let bytes = doors::read_file(policy_path).map_err(Failure::from_door)?;
    let policy = parse_policy(&bytes).map_err(|e| policy_failure(&e))?;
    emit(&format!(
        "ok: {} rule(s), {} waiver(s), policy sha256 {}\n",
        policy.rules.len(),
        policy.waivers.len(),
        sha256_hex(&bytes)
    ))
}

fn explain(code: &str) -> Result<u8, Failure> {
    let entry = code
        .parse::<u8>()
        .ok()
        .and_then(entry_by_code)
        .or_else(|| entry_by_name(code))
        .ok_or_else(|| {
            Failure::new(
                "usage",
                format!("{code:?} is not a documented exit code or name"),
            )
        })?;
    let value = serde_json::to_string_pretty(entry)
        .map_err(|e| Failure::new("internal", format!("serializing the registry: {e}")))?;
    emit(&format!("{value}\n"))
}

fn schema(what: &str) -> Result<u8, Failure> {
    match what {
        "verdict" => emit(VERDICT_SCHEMA_JSON),
        "policy" => emit(POLICY_SCHEMA_JSON),
        _ => {
            let value = serde_json::to_string_pretty(&EXIT_REGISTRY)
                .map_err(|e| Failure::new("internal", format!("serializing the registry: {e}")))?;
            emit(&format!("{value}\n"))
        }
    }
}
