//! Within `simrunner`, `simulate` is the run itself: the prepared run executed through dessim's harness on a thread
//! of its own, its recording written as the run goes as desvis headless writes one (the harness's, the same writer
//! crucible uses), the two files derived from that recording, and the ending the simulator's stop reason spells, answered as
//! what the run left and kept in the output folder too. A cancel reaches it as the harness's stop signal, so the
//! recording holds what ran up to the slice the signal landed in. While it runs, the stream, when one is open, reads
//! the recording's frames as the harness's tap tells it the file grew (`stream`'s `RecordingTap`).

use std::any::Any;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::Builder;

use serde::Serialize;

use dessim::harness::{PreparedRun, RecordingTarget};
use dessim::runner::{RunnerHooks, RunnerLiveHandles};
use dessim::sim_types::{Counters, StopReason};
use desunits::SimTime;
use studio_protocol::{AttemptNumber, EndedBy, Ending, MetricsFile, RunState, SeriesFile, Ulid};

use crate::attempt::Stop;
use crate::recording::derive;
use crate::stop::RunnerStop;
use crate::stream::RecordingTap;

/// The recording's name in the output folder, which the API stores under the attempt.
pub const RECORDING_FILE: &str = "recording.worldview";

/// What an attempt left, as its runner delivers it: its ending, and when the simulation ran, its recording on disk and
/// the two files derived from it. Its parts are known from the run itself, never read back from the folder.
#[derive(Debug, Clone, PartialEq)]
pub struct Left {
    pub ending: Ending,
    pub recording: Option<PathBuf>,
    pub metrics: Option<MetricsFile>,
    pub series: Option<SeriesFile>,
}

impl Left {
    /// An ending and nothing else, as an attempt that stopped before its simulation ran leaves it.
    pub fn ending_only(ending: Ending) -> Self {
        Self { ending, recording: None, metrics: None, series: None }
    }
}

/// Run `prepared` to its end, `stop` being how a cancel reaches it, `counters` what the heartbeat reads as it goes and
/// `tap` the open stream's tap on the recording, and leave the four files in `out`: the recording, `metrics.json` and
/// `series.json` derived from it, and `ending.json`. What is answered is what was written.
pub fn simulate(
    prepared: PreparedRun,
    out: &Path,
    run: &Ulid,
    attempt: AttemptNumber,
    stop: Arc<RunnerStop>,
    counters: Counters,
    tap: Option<RecordingTap>,
) -> Result<Left, Stop> {
    let recording = out.join(RECORDING_FILE);
    let left = match executed(prepared, &recording, run, attempt, stop.clone(), counters, tap) {
        Ok(reason) => {
            let bytes = fs::read(&recording)
                .map_err(|fault| Stop::Platform(format!("the recording could not be read back: {fault}")))?;
            let (metrics, series, stopped) =
                derive(&bytes).map_err(|fault| Stop::Platform(format!("the recording does not derive: {fault}")))?;
            write(&out.join("metrics.json"), &to_json(&metrics))?;
            write(&out.join("series.json"), &to_json(&series))?;
            let (state, ended_by) = if stop.stopped_by_platform(&reason) {
                (RunState::Failed, EndedBy::Platform)
            } else {
                outcome(&reason)
            };
            let ending =
                Ending { state, ended_by, stop_reason: reason.to_string(), stop_detail: None, sim_ms: stopped.at };
            Left { ending, recording: Some(recording), metrics: Some(metrics), series: Some(series) }
        }
        // A run the simulator refused leaves no recording, not a partial one: the API would store it.
        Err(words) => {
            let _ = fs::remove_file(&recording);
            Left::ending_only(Ending {
                state: RunState::Failed,
                ended_by: EndedBy::Simulator,
                stop_reason: words,
                stop_detail: None,
                sim_ms: SimTime::ZERO,
            })
        }
    };
    write(&out.join("ending.json"), &to_json(&left.ending))?;
    Ok(left)
}

/// The simulation on a thread of its own, its recording written at `path`; the reason it stopped, or the simulator's
/// words when it refused to go on. dessim panics on anything it cannot proceed through, so the panic is read off the
/// join and becomes the attempt's failure, never the process's.
fn executed(
    prepared: PreparedRun,
    path: &Path,
    run: &Ulid,
    attempt: AttemptNumber,
    stop: Arc<RunnerStop>,
    counters: Counters,
    tap: Option<RecordingTap>,
) -> Result<StopReason, String> {
    let (path, run) = (path.to_path_buf(), run.as_str().to_owned());
    let thread = Builder::new()
        .name("simulate".to_owned())
        .spawn(move || {
            let live = RunnerLiveHandles { counters: Some(counters), stop_signal: Some(stop.signal.clone()) };
            let hooks = RunnerHooks { live: Some(live), subscribers: Vec::new(), recording: None };
            let tap = tap.map(|tap| Box::new(tap) as Box<dyn Write + Send>);
            let target = RecordingTarget { path, run, attempt: attempt.0, tap };
            let (profile, _summary) = prepared.execute(hooks, Some(target)).map_err(|failure| failure.to_string())?;
            Ok(profile.sim_result.stop_reason)
        })
        .map_err(|fault| format!("the simulation thread could not start: {fault}"))?;
    match thread.join() {
        Ok(outcome) => outcome,
        Err(panic) => Err(panic_words(panic)),
    }
}

/// Which outcome a stop reason is, as crucible reads it: an orderly stop is `finished` (every order done, the
/// simulated duration reached, or the spec's own wall-clock limit); a stop signal is `cancelled` by the person who
/// sent it; the simulator's error or its watchdog (a run that deadlocked) is `failed`.
fn outcome(reason: &StopReason) -> (RunState, EndedBy) {
    match reason {
        StopReason::Complete | StopReason::SimTimeLimit | StopReason::WallTimeout => {
            (RunState::Finished, EndedBy::Simulator)
        }
        StopReason::UserStop => (RunState::Cancelled, EndedBy::Person),
        StopReason::Error(_) | StopReason::Watchdog(_) => (RunState::Failed, EndedBy::Simulator),
    }
}

fn panic_words(payload: Box<dyn Any + Send>) -> String {
    if let Some(words) = payload.downcast_ref::<String>() {
        return words.clone();
    }
    if let Some(words) = payload.downcast_ref::<&str>() {
        return (*words).to_owned();
    }
    "the simulation panicked and said nothing".to_owned()
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), Stop> {
    fs::write(path, bytes).map_err(|fault| Stop::Platform(format!("{} could not be written: {fault}", path.display())))
}

fn to_json<T: Serialize>(value: &T) -> Vec<u8> {
    serde_json::to_vec(value).expect("a runner's file is JSON")
}

#[cfg(test)]
mod tests {
    use super::outcome;
    use dessim::sim_types::StopReason;
    use studio_protocol::{EndedBy, RunState};

    #[test]
    fn only_an_orderly_stop_is_a_finish() {
        assert_eq!(outcome(&StopReason::SimTimeLimit), (RunState::Finished, EndedBy::Simulator));
        assert_eq!(outcome(&StopReason::Complete), (RunState::Finished, EndedBy::Simulator));
        assert_eq!(outcome(&StopReason::WallTimeout), (RunState::Finished, EndedBy::Simulator));
        assert_eq!(outcome(&StopReason::UserStop), (RunState::Cancelled, EndedBy::Person));
        assert_eq!(outcome(&StopReason::Error("x".into())), (RunState::Failed, EndedBy::Simulator));
        assert_eq!(outcome(&StopReason::Watchdog("x".into())), (RunState::Failed, EndedBy::Simulator));
    }
}
