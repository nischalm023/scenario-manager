//! Turning a recorded run into the spreadsheet people read it in, from a
//! terminal: `workbook <recording.worldview> --out <file.xlsx>`.
//!
//! It exists so the workbook is no longer something only the simulator can
//! produce, and only at the moment a run ends. A `.worldview` carries the
//! whole run, so the book can be written days later, on another machine, from
//! a recording someone was handed — and a run that crashed yields a partial
//! book over the slices it got through rather than nothing at all.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use dessim::metrics::worldview::read_worldview;
use workbook::{XlsxData, write_xlsx};

#[derive(Parser)]
#[command(name = "workbook", about = "Write a run's xlsx workbook from a .worldview recording")]
struct Cli {
    /// The recording to read.
    recording: PathBuf,
    /// Where to write the workbook.
    #[arg(long, short)]
    out: PathBuf,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("workbook: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    let recording = read_worldview(&cli.recording)?;
    if recording.truncated {
        // Say it and carry on. The run's own totals are in the end block the
        // file never got, so the book covers the slices the run did reach and
        // nothing after them — partial, and more use than no book at all.
        let slices = recording.profile.time_slices.len();
        eprintln!(
            "workbook: {} has no end block — the run did not finish, so this is a partial book over its {slices} slices",
            cli.recording.display()
        );
    }
    write_xlsx(&cli.out, &XlsxData::from_profile(&recording.profile))
        .map_err(|e| format!("writing {}: {e}", cli.out.display()))
}
