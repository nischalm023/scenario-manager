//! `studio-seed`: spec 04 section 3.13's development seed, run against the world-proto-server `STUDIO_WPS_URL` names.
//!
//!   studio-seed [--manifest data/seed/studio/manifest.json] [--vocabulary <dir>] [--commit <foundry commit>]
//!
//! `--vocabulary` names the folder of our component definitions (`data/seed/application/components`), against which
//! each registered schema is compared; without it the seed checks only that each is published at 0.1.0.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use wps_client::seed;
use wps_client::{BlockingCompute, Client};

#[tokio::main]
async fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let (mut manifest, mut vocabulary, mut commit) =
        (PathBuf::from("data/seed/studio/manifest.json"), None, "unknown".to_owned());
    while let Some(flag) = args.next() {
        let value = args.next();
        match (flag.as_str(), value) {
            ("--manifest", Some(path)) => manifest = PathBuf::from(path),
            ("--vocabulary", Some(path)) => vocabulary = Some(PathBuf::from(path)),
            ("--commit", Some(sha)) => commit = sha,
            (other, _) => {
                eprintln!("studio-seed: `{other}` is not --manifest, --vocabulary or --commit with a value");
                return ExitCode::FAILURE;
            }
        }
    }
    let client = match Client::from_env(Arc::new(BlockingCompute)).await {
        Ok(client) => client,
        Err(fault) => {
            eprintln!("studio-seed: {fault}");
            return ExitCode::FAILURE;
        }
    };
    match seed::run(&client, &manifest, vocabulary.as_deref(), &commit).await {
        Ok(report) => {
            for (scope, seq) in &report.written {
                println!("seeded {scope} at changeset {seq}");
            }
            for scope in &report.skipped {
                println!("skipped {scope}: it already holds a changeset");
            }
            for warning in &report.warnings {
                println!("warning: {warning}");
            }
            ExitCode::SUCCESS
        }
        Err(fault) => {
            eprintln!("studio-seed: {fault}");
            ExitCode::FAILURE
        }
    }
}
