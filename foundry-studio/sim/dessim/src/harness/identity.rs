//! Within `harness`, `identity` is WHO RAN IT: the code the binary was built
//! from and the machine and instant it ran on.
//!
//! Run provenance — code identity and the host/time of each run.
//!
//! Stamped onto every recorded profile and xlsx export so a result can be
//! traced back to the code that produced it. The temporary Walmart app uses
//! its build stamp; development runs inspect the working checkout.
//!
//! Reading the checkout the process stands in is right for someone running
//! desvis from their own tree and wrong everywhere else: a binary copied to
//! another machine, a run driven from a directory that is no checkout at all,
//! or two builds being compared cell for cell that have to state one identity
//! between them. So a caller who knows better states a field in the
//! environment and what it states is what the run records:
//!
//! - `DESVIS_RUN_BRANCH` — the branch, in place of `git rev-parse --abbrev-ref
//!   HEAD`. Any one-line text.
//! - `DESVIS_RUN_COMMIT` — the commit, in place of `git rev-parse HEAD`. Any
//!   one-line text; its first seven characters become `commit_short`, which
//!   names the recording file and has no variable of its own.
//! - `DESVIS_RUN_DIRTY` — exactly `true` or `false`, in place of asking git
//!   whether tracked files differ from `HEAD`.
//! - `DESVIS_RUN_HOSTNAME` — the machine, in place of `hostname`. Any one-line
//!   text.
//! - `DESVIS_RUN_TIMESTAMP` — when the run started, RFC 3339, in place of the
//!   clock.
//!
//! A stated field is held to what the capture itself would produce — trimmed,
//! one line, never empty — because it travels into the recording, the file's
//! name and the workbook's cover sheet, where nothing downstream could tell a
//! stated field from a captured one. A value that fails that refuses the run
//! and names the variable, rather than recording something the caller did not
//! mean.

use std::env::var;
use std::process::Command;

use chrono::{DateTime, Utc};

/// Build and host identity for a single run.
pub struct RunIdentity {
    /// Git branch name (symbolic `HEAD`).
    pub branch: String,
    /// Full 40-char git commit hash.
    pub commit: String,
    /// Short (7-char) git commit hash.
    pub commit_short: String,
    /// Tracked working-tree changes vs `HEAD` — the recorded commit does not
    /// fully describe the run when this is set.
    pub dirty: bool,
    /// Hostname of the machine that ran the sim.
    pub hostname: String,
    /// RFC3339 UTC timestamp of capture.
    pub timestamp: String,
}

impl RunIdentity {
    /// Capture it, or say why this process cannot.
    ///
    /// A server deployed outside a checkout has no git to ask, and a provenance
    /// of empty strings travels into the recording, the file's name and the
    /// workbook's cover as if it were real. So a command that fails or says
    /// nothing is a refusal naming the variables that state the answer, and the
    /// caller decides what to do about it.
    pub fn try_capture() -> Result<Self, String> {
        let mut missing = Vec::new();
        let identity = Self::capture_or_empty();
        for (field, variable, value) in [
            ("the branch", "DESVIS_RUN_BRANCH", &identity.branch),
            ("the commit", "DESVIS_RUN_COMMIT", &identity.commit),
            ("the machine", "DESVIS_RUN_HOSTNAME", &identity.hostname),
        ] {
            if value.is_empty() {
                missing.push(format!("{field} ({variable})"));
            }
        }
        match missing.is_empty() {
            true => Ok(identity),
            false => Err(format!(
                "this process cannot say what code it is running: {} could not be read from the checkout \
                 it stands in. Set the variables named, which is what a build outside a checkout does",
                missing.join(", ")
            )),
        }
    }

    #[cfg(not(feature = "walmart-demo"))]
    fn capture_or_empty() -> Self {
        let commit = stated("DESVIS_RUN_COMMIT").unwrap_or_else(|| git_output(&["rev-parse", "HEAD"]));
        // `commit_short` names the recording's file, so it follows the commit:
        // an empty commit gives an empty short form, and `try_capture` refuses.
        let commit_short = commit.get(..7).unwrap_or(commit.as_str()).to_string();
        Self {
            branch: stated("DESVIS_RUN_BRANCH").unwrap_or_else(|| git_output(&["rev-parse", "--abbrev-ref", "HEAD"])),
            commit,
            commit_short,
            // Mirror `git describe --dirty`: only tracked edits count, so the
            // untracked xlsx output and review files don't falsely flag a run.
            dirty: match stated("DESVIS_RUN_DIRTY") {
                Some(value) if value == "true" || value == "false" => value == "true",
                Some(value) => refuse("DESVIS_RUN_DIRTY", &value, "exactly `true` or `false`"),
                None => !git_output(&["status", "--porcelain", "--untracked-files=no"]).is_empty(),
            },
            hostname: stated("DESVIS_RUN_HOSTNAME").unwrap_or_else(|| command_output("hostname", &[])),
            timestamp: match stated("DESVIS_RUN_TIMESTAMP") {
                Some(value) if DateTime::parse_from_rfc3339(&value).is_ok() => value,
                Some(value) => refuse("DESVIS_RUN_TIMESTAMP", &value, "an RFC 3339 instant"),
                None => Utc::now().to_rfc3339(),
            },
        }
    }

    #[cfg(feature = "walmart-demo")]
    fn capture_or_empty() -> Self {
        let commit = option_env!("WALMART_DEMO_COMMIT").unwrap_or("unknown").to_string();
        let commit_short = commit.get(..7).unwrap_or(commit.as_str()).to_string();
        Self {
            branch: option_env!("WALMART_DEMO_BRANCH").unwrap_or("walmart-demo").to_string(),
            commit,
            commit_short,
            dirty: option_env!("WALMART_DEMO_DIRTY") == Some("true"),
            hostname: command_output("/bin/hostname", &[]),
            timestamp: Utc::now().to_rfc3339(),
        }
    }
}

/// What the caller stated for one field, if anything, trimmed the way the
/// capture path trims what it reads from a command.
///
/// An unset variable is no statement. A variable set to nothing but blanks is
/// a statement of nothing, which no field can be, so it refuses the run rather
/// than quietly falling back to the checkout and recording a provenance the
/// caller did not choose.
#[cfg(not(feature = "walmart-demo"))]
fn stated(variable: &str) -> Option<String> {
    let value = var(variable).ok()?;
    let value = value.trim();
    if value.is_empty() {
        refuse(variable, &value.to_string(), "a value");
    }
    if value.lines().count() > 1 {
        refuse(variable, &value.to_string(), "one line");
    }
    Some(value.to_owned())
}

/// Refuse a stated field, naming the variable and what it owes.
///
/// `capture` has no error channel and its result travels into the recording,
/// the file's name and the workbook — so a value that is not what it claims to
/// be stops the run here rather than being recorded as if it were.
#[cfg(not(feature = "walmart-demo"))]
fn refuse(variable: &str, value: &String, owes: &str) -> ! {
    panic!("{variable} is {value:?}; it must be {owes}");
}

#[cfg(not(feature = "walmart-demo"))]
fn git_output(args: &[&str]) -> String {
    command_output("git", args)
}

/// What a command said, or nothing at all.
///
/// A command that could not be run, or that ran and failed, said nothing —
/// `git rev-parse` outside a checkout exits 128 and writes an empty stdout, and
/// taking that as the branch records a run whose provenance is two empty
/// strings. `try_capture` turns nothing into a refusal that names what to set.
fn command_output(program: &str, args: &[&str]) -> String {
    let Ok(output) = Command::new(program).args(args).output() else { return String::new() };
    match output.status.success() {
        true => String::from_utf8_lossy(&output.stdout).trim().to_string(),
        false => String::new(),
    }
}
