use anyhow::Result;
use clap::{Parser, Subcommand};
use foundry_world_tools::{adapter, expand, flatten};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "foundry-world", version, about = "Foundry application document tools")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Expand a Foundry scenario through the native application reader.
    Expand {
        scenario: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Merge layers into the authored document accepted by WPS import.
    Flatten {
        scenario: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Describe, project or ingest a WASM adapter against a Foundry scenario.
    #[command(trailing_var_arg = true)]
    Adapter {
        #[arg(allow_hyphen_values = true, num_args = 1..)]
        arguments: Vec<String>,
    },
}

fn main() -> Result<ExitCode> {
    match Cli::parse().command {
        Command::Expand { scenario, out } => expand::run(&scenario, out.as_deref())?,
        Command::Flatten { scenario, out } => flatten::run(&scenario, out.as_deref())?,
        Command::Adapter { arguments } => return Ok(worldadapt_cli::run(&arguments, adapter::site)),
    }
    Ok(ExitCode::SUCCESS)
}
