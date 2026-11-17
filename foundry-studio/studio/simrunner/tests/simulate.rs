//! `simulate` on the canned fixture's own spec, a repository world with synthetic orders, cut to minutes of simtime:
//! the four files are left, the ending is the simulator's, the two files are the recording's, and a stop signal ends
//! the run `cancelled` by the person with the recording of what ran.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use dessim::harness::{PreparedRun, RunOverrides, prepare};
use dessim::sim_types::Counters;
use serde::de::DeserializeOwned;
use simrunner::recording::derive;
use simrunner::simulate::{RECORDING_FILE, simulate};
use simrunner::stop::RunnerStop;
use studio_protocol::{AttemptNumber, EndedBy, Ending, MetricsFile, RunState, SeriesFile, Ulid};
use tempfile::tempdir;

const RUN: &str = "01KZAVPQNES65HV4EZADPCS7E8";
const FIVE_MINUTES: Duration = Duration::from_secs(300);

fn prepared(minutes: Duration) -> PreparedRun {
    let spec = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/canned/spec.json");
    let overrides = RunOverrides { sim_duration: Some(minutes), num_threads: Some(1), ..RunOverrides::default() };
    prepare(&spec, &overrides).expect("the fixture's spec prepares")
}

fn read<T: DeserializeOwned>(folder: &Path, name: &str) -> T {
    serde_json::from_slice(&fs::read(folder.join(name)).unwrap_or_else(|_| panic!("{name} is left"))).expect(name)
}

#[test]
fn a_run_leaves_its_recording_and_the_files_derived_from_it() {
    let out = tempdir().expect("a folder");
    let run: Ulid = RUN.parse().expect("a ULID");
    let left = simulate(prepared(FIVE_MINUTES), out.path(), &run, AttemptNumber(1), RunnerStop::new(), Counters::new(), None)
        .expect("the run simulates");
    let ending = &left.ending;
    assert_eq!((ending.state, ending.ended_by), (RunState::Finished, EndedBy::Simulator), "{ending:?}");
    assert_eq!(ending.stop_detail, None, "no stub sentence");
    // Where the last slice ends: the writer closes the slice the limit fell in, so the ending's instant is the limit
    // or one slice past it, as the canned fixture's was.
    let limit = FIVE_MINUTES.as_millis() as u64;
    assert!((limit..limit + 60_000).contains(&ending.sim_ms.as_ms()), "it ran to its simtime limit: {ending:?}");
    let written: Ending = read(out.path(), "ending.json");
    assert_eq!(&written, ending, "the ending answered is the one written");
    assert_eq!(left.recording.as_deref(), Some(out.path().join(RECORDING_FILE).as_path()), "the recording answered");
    let bytes = fs::read(out.path().join(RECORDING_FILE)).expect("the recording is left");
    let (metrics, series, stopped) = derive(&bytes).expect("the recording derives");
    assert_eq!(stopped.at, ending.sim_ms);
    assert_eq!(read::<MetricsFile>(out.path(), "metrics.json"), metrics, "metrics.json is the recording's");
    assert_eq!(read::<SeriesFile>(out.path(), "series.json"), series, "series.json is the recording's");
    assert_eq!((left.metrics, left.series), (Some(metrics.clone()), Some(series)), "the files answered are the left");
    assert!(!metrics.metrics.is_empty(), "the run declared metrics");
}

#[test]
fn a_stop_signal_ends_the_run_cancelled_with_the_recording_so_far() {
    let out = tempdir().expect("a folder");
    let run: Ulid = RUN.parse().expect("a ULID");
    let stop = RunnerStop::new();
    let stopper = Arc::clone(&stop);
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(300));
        stopper.cancel();
    });
    let ending =
        simulate(prepared(Duration::from_secs(3 * 3600)), out.path(), &run, AttemptNumber(2), stop, Counters::new(), None)
            .expect("the run simulates")
            .ending;
    assert_eq!((ending.state, ending.ended_by), (RunState::Cancelled, EndedBy::Person), "{ending:?}");
    assert!(ending.sim_ms.as_ms() < 3 * 3_600_000, "it stopped before its limit: {ending:?}");
    assert!(out.path().join(RECORDING_FILE).is_file(), "the recording of what ran is left");
    let _: MetricsFile = read(out.path(), "metrics.json");
}
