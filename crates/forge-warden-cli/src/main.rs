//! `forge-warden`: change policy as code for deep-diff-forge reviews.
//!
//! ```text
//! git diff --binary --no-renames main... > change.patch
//! forge-warden check --policy warden.toml --patch change.patch --ranker /path/to/deep-diff-forge
//! ```
//!
//! Authority mode (`--ranker`, ADR 0005) runs the pinned ranker on the patch itself and is the only
//! mode that can pass. Advisory mode (`--advisory --rank FILE`) judges a supplied ranking and can at
//! best hold.
//!
//! Results go to stdout, diagnostics to stderr, and on error stdout is empty. Every exit code is
//! in the registry (`forge-warden schema exit-codes`), with the next action to take.

mod doors;

#[cfg(test)]
#[cfg(not(fw_fixture_cli))]
mod docs_contract;

use clap::{ArgGroup, Parser, Subcommand};
use doors::DoorErrorKind;
use doors::child::{GitFactsFailure, RankerFailure};
use forge_warden_core::authority::{
    Ranking, SuppliedCopy, check as check_coverage, evaluate_advisory, evaluate_authority,
};
use forge_warden_core::exit::{Exit, entry_by_code, entry_by_name};
use forge_warden_core::facts::{PatchFacts, Unverifiable};
use forge_warden_core::policy::{PolicyError, parse_policy};
use forge_warden_core::rank::{RankError, parse_rank};
use forge_warden_core::render::render_text;
use forge_warden_core::seal::sha256_hex;
use forge_warden_core::verdict::{Claims, GitOracle, Seals, Verdict};
use forge_warden_core::{Date, EXIT_REGISTRY};
use std::path::{Path, PathBuf};

/// The JSON Schema of `forge-warden.verdict.v2`, committed and checked against the code.
const VERDICT_SCHEMA_JSON: &str = include_str!("../../../schemas/verdict.v2.schema.json");
/// The JSON Schema of `forge-warden.policy.v0` (TOML editors can validate against it).
const POLICY_SCHEMA_JSON: &str = include_str!("../../../schemas/policy.v0.schema.json");
/// The starter policy `forge-warden init` prints.
const STARTER_POLICY: &str = include_str!("../../../examples/starter.warden.toml");
const LINT_SCHEMA_JSON: &str = include_str!("../../../schemas/lint.v1.schema.json");
const ERROR_SCHEMA_JSON: &str = include_str!("../../../schemas/error.v1.schema.json");
const EXIT_ENTRY_SCHEMA_JSON: &str = include_str!("../../../schemas/exit-entry.v1.schema.json");
const EXIT_CODES_SCHEMA_JSON: &str = include_str!("../../../schemas/exit-codes.v1.schema.json");

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
    /// Judge a patch against a policy. Authority mode (`--ranker`) runs the ranker itself and is
    /// the only mode that can pass; advisory mode (`--advisory --rank`) can at best hold.
    #[command(group(ArgGroup::new("mode").required(true).args(["ranker", "advisory"])))]
    Check {
        /// The policy file.
        #[arg(long, value_name = "FILE")]
        policy: PathBuf,
        /// The patch to judge (`git diff --binary --no-renames`). Read once; the ranker and Git
        /// both read that one buffer. `--patch -` is refused: stdin is reserved for `--rank -`.
        #[arg(long, value_name = "FILE")]
        patch: PathBuf,
        /// Authority mode: the deep-diff-forge binary to run. Its SHA-256 must be listed in the
        /// policy's `[producer] sha256`.
        #[arg(long, value_name = "FILE")]
        ranker: Option<PathBuf>,
        /// Advisory mode: judge the ranking given with `--rank`. The verdict is hold or fail,
        /// never pass.
        #[arg(long, requires = "rank")]
        advisory: bool,
        /// A ranking document (`-` reads stdin). Advisory mode judges it; authority mode compares
        /// it with the produced ranking and holds (`rank_mismatch`) if they differ.
        #[arg(long, value_name = "FILE")]
        rank: Option<PathBuf>,
        /// The Git that reads the patch's facts (absolute; recorded in the verdict).
        #[arg(long, value_name = "FILE", default_value = "/usr/bin/git")]
        git: PathBuf,
        /// A claimed base commit (hex), recorded unverified in the verdict.
        #[arg(long, value_name = "REV")]
        base: Option<String>,
        /// A claimed head commit (hex), recorded unverified in the verdict.
        #[arg(long, value_name = "REV")]
        head: Option<String>,
        /// Judge waivers as of this date (YYYY-MM-DD) instead of today (UTC).
        #[arg(long, value_name = "DATE")]
        today: Option<String>,
        /// Print the forge-warden.verdict.v2 document instead of the human summary.
        #[arg(long)]
        json: bool,
    },
    /// Validate a policy without judging anything; lists every issue at once.
    Lint {
        /// The policy file.
        policy: PathBuf,
        /// Print the forge-warden.lint.v1 document; failures are one JSON line on stderr.
        #[arg(long)]
        json: bool,
    },
    /// Explain an exit code (by number or name) and the action it asks for.
    Explain {
        /// A code such as `7`, or a name such as `policy_hold`.
        code: String,
        /// Keep the JSON result; also format failures as one JSON line on stderr.
        #[arg(long)]
        json: bool,
    },
    /// Print a JSON Schema or the exit-code registry.
    Schema {
        /// What to print.
        #[arg(value_parser = ["verdict", "policy", "exit-codes", "lint", "error", "exit-entry", "exit-codes-schema"])]
        what: String,
        /// Keep the JSON result; also format failures as one JSON line on stderr.
        #[arg(long)]
        json: bool,
    },
    /// Print a starter policy: `forge-warden init > warden.toml`.
    Init,
}

/// A failure with the registry exit it ends the process with.
struct Failure {
    exit: Exit,
    message: String,
    next: &'static str,
}

impl Failure {
    fn new(exit: Exit, message: impl Into<String>) -> Self {
        Self {
            exit,
            message: message.into(),
            next: exit.entry().action,
        }
    }

    /// The one mapping from a door's kind to the registry (Q2-05). Exhaustive: a new kind is a
    /// compile error here until someone decides its exit.
    fn from_door(error: doors::DoorError) -> Self {
        let (exit, next) = match error.kind {
            DoorErrorKind::Io => (Exit::Io, "check_input_output"),
            DoorErrorKind::Environment => (Exit::Io, "check_environment"),
            DoorErrorKind::OverBudget => (Exit::InvalidInput, "reduce_input_size"),
            DoorErrorKind::Refused => (Exit::InvalidInput, "change_input"),
        };
        Self {
            exit,
            message: error.message,
            next,
        }
    }
}

fn main() -> doors::Status {
    #[cfg(test)]
    if !doors::fixture_cli_requested() {
        return doors::Status(Exit::Pass);
    }
    doors::Status(run_cli())
}

fn run_cli() -> Exit {
    let arguments = doors::arguments();
    let json = machine_requested(&arguments);
    let cli = match Cli::try_parse_from(arguments) {
        Ok(cli) => cli,
        Err(error) => {
            let text = error.render().to_string();
            return if error.use_stderr() {
                if json {
                    finish(Err(Failure::new(Exit::Usage, text)), true)
                } else {
                    doors::diagnose(&text);
                    Exit::Usage
                }
            } else {
                finish(
                    doors::emit(&text)
                        .map(|()| Exit::Pass)
                        .map_err(Failure::from_door),
                    json,
                )
            };
        }
    };
    finish(run(cli.command), json)
}

/// An exact flag before the option terminator requests machine diagnostics, even on parse errors.
/// A positional file named `--json` after `--` does not change the output contract.
fn machine_requested(arguments: &[std::ffi::OsString]) -> bool {
    arguments
        .iter()
        .skip(1)
        .take_while(|arg| *arg != "--")
        .any(|arg| arg == "--json")
}

fn finish(result: Result<Exit, Failure>, json: bool) -> Exit {
    match result {
        Ok(exit) => exit,
        Err(failure) => {
            let entry = failure.exit.entry();
            if json {
                doors::diagnose_json(failure.exit, &failure.message, failure.next);
            } else {
                doors::diagnose(&format!("forge-warden: {}", failure.message));
                doors::diagnose(&format!(
                    "next: {} (forge-warden explain {})",
                    failure.next, entry.code
                ));
            }
            failure.exit
        }
    }
}

fn run(command: Command) -> Result<Exit, Failure> {
    match command {
        Command::Check {
            policy,
            patch,
            ranker,
            advisory: _,
            rank,
            git,
            base,
            head,
            today,
            json,
        } => check(&CheckArgs {
            policy,
            patch,
            ranker,
            rank,
            git,
            base,
            head,
            today,
            json,
        }),
        Command::Lint { policy, json } => lint(&policy, json),
        Command::Explain { code, json: _ } => explain(&code),
        Command::Schema { what, json: _ } => schema(&what),
        Command::Init => emit(STARTER_POLICY),
    }
}

fn emit(text: &str) -> Result<Exit, Failure> {
    doors::emit(text).map_err(Failure::from_door)?;
    Ok(Exit::Pass)
}

fn policy_failure(error: &PolicyError) -> Failure {
    Failure::new(Exit::InvalidInput, error.to_string())
}

fn rank_failure(error: &RankError) -> Failure {
    Failure::new(Exit::InvalidInput, error.to_string())
}

/// `check`'s arguments. The mode is `ranker`: `Some` is authority mode; `None` is advisory mode,
/// which clap admits only with `--advisory --rank`.
struct CheckArgs {
    policy: PathBuf,
    patch: PathBuf,
    ranker: Option<PathBuf>,
    rank: Option<PathBuf>,
    git: PathBuf,
    base: Option<String>,
    head: Option<String>,
    today: Option<String>,
    json: bool,
}

fn check(args: &CheckArgs) -> Result<Exit, Failure> {
    if args.patch.as_os_str() == "-" {
        return Err(Failure::new(
            Exit::InvalidInput,
            "patch_stdin_unsupported: --patch - is not supported; save the patch to a file (stdin is reserved for --rank -)",
        ));
    }
    let today = match &args.today {
        Some(text) => Date::parse(text).ok_or_else(|| {
            Failure::new(
                Exit::Usage,
                format!("--today {text:?} is not a YYYY-MM-DD date"),
            )
        })?,
        None => doors::today().map_err(Failure::from_door)?,
    };
    let claims = claims(args.base.as_deref(), args.head.as_deref())?;
    let git_path = git_path(&args.git)?;
    let policy_bytes = doors::read_file(&args.policy).map_err(Failure::from_door)?;
    let policy = parse_policy(&policy_bytes).map_err(|e| policy_failure(&e))?;
    // The one patch buffer (ADR 0005 §2.3): hashed, given to the ranker and to Git; never reread.
    let patch = doors::read_file(&args.patch).map_err(Failure::from_door)?;
    let patch_sha256 = sha256_hex(&patch);
    let supplied = args.rank.as_deref().map(read_rank).transpose()?;

    let admitted_git =
        doors::child::admit_git(&args.git, args.ranker.is_none()).map_err(Failure::from_door)?;
    let (facts, facts_sha256) = git_facts(&admitted_git, &patch)?;
    let git = GitOracle::new(
        git_path,
        doors::child::git_version(&admitted_git).map_err(Failure::from_door)?,
        admitted_git.sha256().to_owned(),
        admitted_git.allowlist_sha256().map(str::to_owned),
    );

    let mut verdict = if let Some(ranker) = &args.ranker {
        let (ranking, bytes) = doors::child::produce_ranking(ranker, &policy, &patch).map_err(
            |failure| match failure {
                RankerFailure::NotAdmitted(why) => Failure::new(Exit::InvalidInput, why),
                RankerFailure::Door(error) => Failure::from_door(error),
                RankerFailure::Failed(why) => Failure::new(Exit::Io, why),
            },
        )?;
        let copy = SuppliedCopy::compare(supplied.as_deref(), &bytes);
        let checked = check_coverage(&ranking, facts);
        let seals = seals(
            &policy_bytes,
            &bytes,
            patch_sha256,
            &ranking,
            facts_sha256,
            git,
            claims,
        );
        evaluate_authority(&policy, &checked, copy, seals, today)
    } else {
        // clap admits advisory mode only with `--rank`.
        let bytes = supplied.ok_or_else(|| Failure::new(Exit::Usage, "--advisory needs --rank"))?;
        let doc = parse_rank(&bytes).map_err(|e| rank_failure(&e))?;
        if doc.input_sha256() != patch_sha256 {
            return Err(Failure::new(
                Exit::InvalidInput,
                format!(
                    "the ranking describes patch {} but {} hashes to {patch_sha256}; re-rank the patch you mean to judge",
                    doc.input_sha256(),
                    args.patch.display()
                ),
            ));
        }
        let ranking = Ranking::supplied(doc);
        let checked = check_coverage(&ranking, facts);
        let seals = seals(
            &policy_bytes,
            &bytes,
            patch_sha256,
            &ranking,
            facts_sha256,
            git,
            claims,
        );
        evaluate_advisory(&policy, &checked, seals, today)
            .verdict()
            .clone()
    };
    if admitted_git.allowlist_sha256().is_none() {
        for note in &mut verdict.notes {
            if note.code == forge_warden_core::verdict::NoteCode::RankUnverified {
                note.detail.push_str("; Git is unpinned: no owner allowlist enrolled; its recorded executable digest is not approved");
            }
        }
    }
    report(&verdict, args.json)
}

fn report(verdict: &Verdict, json: bool) -> Result<Exit, Failure> {
    let text = if json {
        verdict.to_json()
    } else {
        render_text(verdict)
    };
    doors::emit(&text).map_err(Failure::from_door)?;
    Ok(verdict.verdict.exit())
}

/// A ranking file, or stdin for `-`.
fn read_rank(path: &Path) -> Result<Vec<u8>, Failure> {
    if path.as_os_str() == "-" {
        doors::read_stdin()
    } else {
        doors::read_file(path)
    }
    .map_err(Failure::from_door)
}

/// `--git` must be absolute and UTF-8: it is run without `PATH` and recorded in the verdict.
fn git_path(git: &Path) -> Result<String, Failure> {
    if !git.is_absolute() {
        return Err(Failure::new(
            Exit::Usage,
            format!("--git {} must be an absolute path", git.display()),
        ));
    }
    git.to_str()
        .map(str::to_owned)
        .ok_or_else(|| Failure::new(Exit::Usage, format!("--git {} is not UTF-8", git.display())))
}

/// Git's facts about the patch, and their digest. A refusal is a fact about the patch (it holds
/// the verdict); Git that cannot run gives no verdict.
fn git_facts(
    git: &doors::child::AdmittedGit,
    patch: &[u8],
) -> Result<(Result<PatchFacts, Unverifiable>, Option<String>), Failure> {
    match doors::child::git_facts(git, patch) {
        Ok((numstat, summary)) => {
            let mut sealed = numstat.clone();
            sealed.push(0);
            sealed.extend_from_slice(&summary);
            Ok((
                PatchFacts::from_git(&numstat, &summary, patch),
                Some(sha256_hex(&sealed)),
            ))
        }
        Err(GitFactsFailure::Door(error)) => Err(Failure::from_door(error)),
        Err(GitFactsFailure::Refused(why)) => Ok((Err(Unverifiable::GitRefused(why)), None)),
        Err(GitFactsFailure::Unavailable(why)) => Err(Failure::new(Exit::Io, why)),
    }
}

/// The largest claimed revision accepted: a SHA-256 object id.
const MAX_REV_CHARS: usize = 64;

/// `--base`/`--head` as claims. Each must be 4 to 64 lowercase hex digits (an object id or an
/// abbreviation), so nothing else reaches the verdict.
fn claims(base: Option<&str>, head: Option<&str>) -> Result<Option<Claims>, Failure> {
    let admit = |flag: &str, rev: Option<&str>| -> Result<Option<String>, Failure> {
        let Some(rev) = rev else { return Ok(None) };
        let hex = rev
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if hex && (4..=MAX_REV_CHARS).contains(&rev.len()) {
            Ok(Some(rev.to_owned()))
        } else {
            Err(Failure::new(
                Exit::Usage,
                format!("{flag} {rev:?} is not 4 to {MAX_REV_CHARS} lowercase hex digits"),
            ))
        }
    };
    let base = admit("--base", base)?;
    let head = admit("--head", head)?;
    Ok((base.is_some() || head.is_some()).then(|| Claims::new(base, head)))
}

/// The seals for a verdict. `provenance` and `ranked_by` are set again by the evaluator from the
/// ranking itself.
fn seals<P>(
    policy_bytes: &[u8],
    rank_bytes: &[u8],
    patch_sha256: String,
    ranking: &Ranking<P>,
    facts_sha256: Option<String>,
    git: GitOracle,
    claims: Option<Claims>,
) -> Seals {
    let seals = Seals::new(
        sha256_hex(policy_bytes),
        sha256_hex(rank_bytes),
        patch_sha256,
        ranking.doc().tool_version().to_owned(),
    )
    .with_git(facts_sha256, git);
    match claims {
        Some(claims) => seals.with_claims(claims),
        None => seals,
    }
}

fn lint(policy_path: &Path, json: bool) -> Result<Exit, Failure> {
    let bytes = doors::read_file(policy_path).map_err(Failure::from_door)?;
    let policy = parse_policy(&bytes).map_err(|e| policy_failure(&e))?;
    if json {
        let value = serde_json::json!({
            "schema": "forge-warden.lint.v1",
            "rules": policy.rules().len(),
            "waivers": policy.waivers().len(),
            "policy_sha256": sha256_hex(&bytes),
        });
        return emit(&format!("{value}\n"));
    }
    emit(&format!(
        "ok: {} rule(s), {} waiver(s), policy sha256 {}\n",
        policy.rules().len(),
        policy.waivers().len(),
        sha256_hex(&bytes)
    ))
}

fn explain(code: &str) -> Result<Exit, Failure> {
    let entry = code
        .parse::<u8>()
        .ok()
        .and_then(entry_by_code)
        .or_else(|| entry_by_name(code))
        .ok_or_else(|| {
            Failure::new(
                Exit::Usage,
                format!("{code:?} is not a documented exit code or name"),
            )
        })?;
    let value = serde_json::to_string_pretty(entry)
        .map_err(|e| Failure::new(Exit::Internal, format!("serializing the registry: {e}")))?;
    emit(&format!("{value}\n"))
}

fn schema(what: &str) -> Result<Exit, Failure> {
    match what {
        "verdict" => emit(VERDICT_SCHEMA_JSON),
        "policy" => emit(POLICY_SCHEMA_JSON),
        "lint" => emit(LINT_SCHEMA_JSON),
        "error" => emit(ERROR_SCHEMA_JSON),
        "exit-entry" => emit(EXIT_ENTRY_SCHEMA_JSON),
        "exit-codes-schema" => emit(EXIT_CODES_SCHEMA_JSON),
        _ => {
            let value = serde_json::to_string_pretty(&EXIT_REGISTRY).map_err(|e| {
                Failure::new(Exit::Internal, format!("serializing the registry: {e}"))
            })?;
            emit(&format!("{value}\n"))
        }
    }
}
