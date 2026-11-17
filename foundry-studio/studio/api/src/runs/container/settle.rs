//! Within `container`, `settle` is the fallback once a runner's container has exited: a runner sends its recording and
//! reports its own results over the door before it exits, so most of the time the ending is written already and this
//! changes nothing; a runner that died first leaves its mounted folder, whose `ending.json`, `metrics.json`,
//! `series.json` and recording are written through the store as the attempt's ending, or the ending its exit tells
//! when it left none (spec 08 section 3.5). A store that fails is asked four times 15 s apart (spec 08 section 3.13);
//! a file or a recording that does not read, or one that cannot be stored, ends the attempt `failed`, by the
//! platform, with the fault's words, never `partial` as a lost runner would.

use std::fmt::Display;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::time::Duration;

use desunits::SimTime;
use run_invocation::{AttemptKey, Ended, RunStore};
use serde::de::DeserializeOwned;
use studio_protocol::{EndedBy, Ending, MetricsFile, RunState, SeriesFile};
use tokio::time::sleep;

use super::super::store::{PgRunStore, Recording, Results, ResultsFault, ResultsSource};

/// How many times a store is asked before the attempt ends `failed`.
const TRIES: u32 = 4;

/// How the runner's exit went, as the invoker saw it: its words, and its deadline if that passed.
pub struct Exit<'a> {
    pub words: &'a str,
    pub deadline: Duration,
    pub deadline_passed: bool,
}

/// What the runner left in `out`, written through `store` as the attempt's ending, each store asked again after
/// `again` while it fails.
pub async fn settle(store: &PgRunStore, key: &AttemptKey, out: &Path, exit: &Exit<'_>, again: Duration) -> Ended {
    let results = match left(out) {
        Ok(Some(results)) => results,
        Ok(None) => unended(store, key, exit).await,
        Err(unread) => failed(unread),
    };
    let words = match tries(again, || store.take_results(key, results.clone())).await {
        Ok(ended) => return ended,
        Err(refused @ (ResultsFault::NotAccepted | ResultsFault::LeftUnwhole(_))) => refused.to_string(),
        Err(ResultsFault::Store(fault)) => format!("its results could not be stored after {TRIES} tries: {fault}"),
    };
    let ending = failed(words);
    match tries(again, || async { store.end(key, &ending.ending).await.map_err(ResultsFault::Store) }).await {
        Ok(ended) => ended,
        Err(fault) => {
            eprintln!("studio-api: {key}'s runner ended and not even its failure could be stored: {fault}");
            Ended::AlreadyEnded
        }
    }
}

/// The runner's ending and files; `None` when it left no `ending.json`; the words of one that does not read.
fn left(out: &Path) -> Result<Option<Results>, String> {
    let Some(ending) = read::<Ending>(out, "ending.json")? else { return Ok(None) };
    Ok(Some(Results {
        ending,
        metrics: read::<MetricsFile>(out, "metrics.json")?,
        series: read::<SeriesFile>(out, "series.json")?,
        recording: Some(out.join("recording.worldview"))
            .filter(|path| path.exists())
            .map(|path| Recording::File { path }),
        // The runner said nothing of itself; the build it ran is its image's, which the run's image names.
        runner_commit: None,
        runner_dirty: None,
        source: ResultsSource::Folder,
    }))
}

/// The ending of a runner that left none: `cancelled` after a cancel request, past its deadline, or its exit.
async fn unended(store: &PgRunStore, key: &AttemptKey, exit: &Exit<'_>) -> Results {
    let cancelled = store.attempt(key).await.ok().flatten().is_some_and(|row| row.cancel_requested);
    let (state, ended_by, words) = if cancelled {
        (RunState::Cancelled, EndedBy::Person, "cancelled; the runner did not confirm".to_owned())
    } else if exit.deadline_passed {
        let hours = exit.deadline.as_secs_f64() / 3600.0;
        (RunState::Failed, EndedBy::Platform, format!("ran past its wall-clock deadline of {hours} h"))
    } else {
        (RunState::Failed, EndedBy::Platform, format!("{} and left no ending", exit.words))
    };
    ending_only(Ending { state, ended_by, stop_reason: words, stop_detail: None, sim_ms: SimTime::ZERO })
}

fn ending_only(ending: Ending) -> Results {
    let source = ResultsSource::Folder;
    Results { ending, metrics: None, series: None, recording: None, runner_commit: None, runner_dirty: None, source }
}

fn failed(words: String) -> Results {
    let ending = Ending {
        state: RunState::Failed,
        ended_by: EndedBy::Platform,
        stop_reason: words,
        stop_detail: None,
        sim_ms: SimTime::ZERO,
    };
    ending_only(ending)
}

/// `ask`, asked again after `again` while the store fails, `TRIES` times in all; the last fault when every try
/// failed, and at once a refusal no try mends.
async fn tries<T, F: Future<Output = Result<T, ResultsFault>>>(
    again: Duration,
    mut ask: impl FnMut() -> F,
) -> Result<T, ResultsFault> {
    let mut tried = 1;
    loop {
        match ask().await {
            Err(ResultsFault::Store(fault)) if tried < TRIES => {
                eprintln!("studio-api: a store of a runner's results failed, try {tried} of {TRIES}: {fault}");
                tried += 1;
                sleep(again).await;
            }
            answer => return answer,
        }
    }
}

/// The file `name` the runner wrote in `out`; `None` when it wrote none; the words of one that does not read.
fn read<T: DeserializeOwned>(out: &Path, name: &str) -> Result<Option<T>, String> {
    let unread = |fault: &dyn Display| format!("the runner's {name} does not read: {fault}");
    match fs::read(out.join(name)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|fault| unread(&fault)),
        Err(fault) if fault.kind() == ErrorKind::NotFound => Ok(None),
        Err(fault) => Err(unread(&fault)),
    }
}

#[cfg(test)]
mod tests {
    use std::env::temp_dir;
    use std::process::id;

    use super::*;

    /// Spec 08 section 3.13: a sidecar the runner left that does not read is the attempt's `failed` ending, naming the
    /// file, rather than no ending at all.
    #[test]
    fn a_sidecar_that_does_not_read_is_named_as_the_failure() {
        let out = temp_dir().join(format!("studio-settle-{}", id()));
        fs::create_dir_all(&out).unwrap();
        let ending =
            r#"{"state":"finished","ended_by":"simulator","stop_reason":"SimTimeLimit","stop_detail":null,"sim_ms":1}"#;
        fs::write(out.join("ending.json"), ending).unwrap();
        fs::write(out.join("metrics.json"), "not json").unwrap();
        let unread = left(&out).expect_err("metrics.json does not read");
        assert!(unread.starts_with("the runner's metrics.json does not read"), "{unread}");
        fs::remove_dir_all(&out).unwrap();
    }
}
