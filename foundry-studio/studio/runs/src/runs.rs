//! Within `crucible`, `runs` is THE RUN ITSELF FROM SUBMIT TO STORED: it takes
//! a submission, queues it, runs the simulation on a worker, records the
//! finished run as a `.worldview`, uploads it, and writes down how it ended.
//!
//! One rule decides every choice here: a recording this server writes for a
//! spec is the recording headless desvis writes for the same spec. So the run
//! goes through `dessim::harness` — the same function desvis calls, not a copy
//! of it — and the recording comes from `dessim::metrics::worldview`'s writer
//! at the end of the run, on the finished model. A cancelled run is recorded
//! the same way, from the model as it stands when the stop signal lands, and a
//! run whose simulator refused to continue is recorded not at all: it failed,
//! and it says why.
//!
//! The workers are the concurrency cap. Each one claims a queued attempt,
//! runs it to its end, and goes back for another; a submission beyond the cap
//! simply waits in the registry until a worker is free.

use std::any::Any;
use std::fs::{create_dir_all, remove_dir_all, remove_file};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use dessim::harness::{RecordingTarget, RunOverrides, prepare};
use dessim::runner::{RunnerHooks, RunnerLiveHandles};
use dessim::sim_types::{Counters, StopReason, StopSignal};
use futures::FutureExt;
use in_flight::RunsInFlight;
use runstore::layout::{AttemptNumber, RunId, recording_key, spec_key};
use runstore::{RunStorage, StoreError};
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};
use ulid::Ulid;

use crate::config::Config;
use crate::floor::Compiling;
use crate::protocol::{AttemptState, Progress, SubmittedSpec};
use crate::recording::{Cache, extent, local_path, under_ceiling};
use crate::registry::{ClaimedAttempt, Registry, RegistryError};
use crate::spec::{SpecError, lay_out};

/// How often a running simulation's counters are copied into the registry, so
/// `status` answers without the run being asked anything.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(500);

/// How many times a stored run's ending is offered to the registry before the
/// reconciling is left to the next restart, and how long between tries.
const ENDING_TRIES: u32 = 3;
const ENDING_RETRY: Duration = Duration::from_secs(2);

/// How many times a recording is offered to storage before the run is given up
/// as failed, and how long between tries. The client retries each request on
/// its own; these are for an outage that outlasts that, long enough apart for
/// a bucket to come back and few enough that a worker is not held for ever.
const UPLOAD_TRIES: u32 = 4;
const UPLOAD_RETRY: Duration = Duration::from_secs(15);

/// The runs this server has: the queue, the workers, and what each running one
/// can be told.
pub struct Runs {
    config: Config,
    registry: Registry,
    storage: Arc<dyn RunStorage>,
    /// What a cancel reaches: one stop signal and one set of counters per run
    /// currently on a worker.
    in_flight: RunsInFlight,
    /// Woken when something is queued, so a free worker starts at once rather
    /// than at the end of its poll.
    queued: Notify,
    /// Held across claiming an attempt and registering what can stop it, and
    /// across a cancel's two attempts to land. Without it a cancel could fall
    /// between the registry calling an attempt `running` and this process
    /// knowing how to reach it, and answer "nothing to cancel" for a run that
    /// then goes on to completion.
    claiming: Mutex<()>,
    /// The recordings this machine holds for the browsers reading them, and
    /// the floors compiled from them. They belong here because a run's
    /// recording becomes a cached copy the moment the run ends, and because two
    /// sockets asking for one run must not fetch or compile it twice.
    cache: Cache,
    compiling: Compiling,
}

/// What a submission or a cancel can fail to do.
#[derive(Debug, thiserror::Error)]
pub enum RunsError {
    #[error("{0}")]
    Spec(#[from] SpecError),
    #[error("{0}")]
    Registry(#[from] RegistryError),
    #[error("`{key}` could not be {doing}: {source}")]
    Storage {
        /// What was being done to it, so a failure names the operation as well
        /// as the object — an operator reading this has to know whether the
        /// object is there.
        doing: &'static str,
        /// The object key, which is what identifies a stranded recording.
        key: String,
        #[source]
        source: StoreError,
    },
    #[error("there is no run `{0}`")]
    NoSuchRun(String),
    #[error("run `{run}` is `{state}`, so there is nothing to cancel")]
    NothingToCancel { run: String, state: String },
    #[error("{0}")]
    Prepare(String),
}

impl Runs {
    /// Assemble the runs of one server.
    pub fn new(config: Config, registry: Registry, storage: Arc<dyn RunStorage>) -> Arc<Self> {
        let cache = Cache::new(config.cache_bytes);
        Arc::new(Self {
            cache,
            compiling: Compiling::default(),
            config,
            registry,
            storage,
            in_flight: RunsInFlight::default(),
            queued: Notify::new(),
            claiming: Mutex::new(()),
        })
    }

    /// The registry, for the handlers that only read it.
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// What the server was told, for the handlers that serve what was recorded.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Where the recordings are kept, for the same handlers.
    pub fn storage(&self) -> &Arc<dyn RunStorage> {
        &self.storage
    }

    /// The recordings this machine holds.
    pub fn cache(&self) -> &Cache {
        &self.cache
    }

    /// The floors compiled from them.
    pub fn compiling(&self) -> &Compiling {
        &self.compiling
    }

    /// Accept a submission: run it all the way through the harness's own
    /// preparation, store the spec whole, and write the run down.
    ///
    /// The whole preparation, not a shape check — the spec is parsed as a
    /// `WorldSpec`, its world document is loaded, its scenario resolved and its
    /// configuration settled, which is the same function the worker will call.
    /// So a spec that cannot run is refused on the socket the client is still
    /// holding, rather than becoming a queued run that fails minutes later, and
    /// the name the recording will carry is known here and written down beside
    /// the run. The folder the preparation needs is removed on every path.
    ///
    /// What comes back is the run and a token: the attempt is NOT queued yet,
    /// so no worker can start before the client has been told the run exists.
    /// [`activate`](Self::activate) is what queues it.
    pub async fn submit(&self, submitted: &SubmittedSpec) -> Result<Accepted, RunsError> {
        let run = Ulid::new().to_string();
        let id = RunId::new(run.clone()).map_err(|failure| RunsError::Prepare(failure.to_string()))?;

        let temp_dir = self.config.temp_dir.clone();
        let checked = submitted.clone();
        let run_for_check = run.clone();
        // Off the async runtime: loading a world document is a second of CPU
        // and a websocket task is not the place to spend it.
        let name = tokio::task::spawn_blocking(move || prepared_name(&checked, &temp_dir, &run_for_check))
            .await
            .map_err(|join| RunsError::Prepare(join.to_string()))??;

        let bundle = serde_json::to_vec(submitted).expect("a submission re-encodes");
        let key = spec_key(&id);
        self.storage.put(&key, bundle.into()).await.map_err(|source| RunsError::Storage {
            doing: "stored",
            key: key.clone(),
            source,
        })?;
        // A registry that refuses leaves the object with no row pointing at it,
        // and nothing collects such an object; it goes back out here.
        if let Err(failure) = self.registry.submit(&run, &name, &key).await {
            if let Err(source) = self.storage.delete(&key).await {
                error!(run = %run, key = %key, %source, "the spec of a run that was not recorded is still stored");
            }
            return Err(failure.into());
        }
        info!(run = %run, name = %name, "accepted");
        Ok(Accepted { run })
    }

    /// Queue an accepted run, once its client has been told it exists.
    pub async fn activate(&self, accepted: Accepted) {
        let Accepted { run } = accepted;
        match self.registry.queue_first_attempt(&run).await {
            Ok(()) => {
                self.queued.notify_one();
                info!(run = %run, "queued");
            }
            Err(failure) => error!(run = %run, %failure, "an accepted run could not be queued"),
        }
    }

    /// Stop a run, and say what was actually stopped.
    ///
    /// A run nothing has started yet ends here and then; a run on a worker is
    /// told to stop and ends when its simulator next looks. A run that is over,
    /// or one whose attempt this process is not running, is NOT reported as
    /// cancelled — a reply of `cancelled` for a run that goes on to finish is
    /// the one answer a caller cannot recover from. The signal is registered
    /// against the run before its attempt is claimed (`work`), so there is no
    /// window in which a run is `running` and unreachable from here.
    pub async fn cancel(&self, run: &str) -> Result<(), RunsError> {
        let Some(summary) = self.registry.summary(run).await? else {
            return Err(RunsError::NoSuchRun(run.to_string()));
        };
        let _claiming = self.claiming.lock().await;
        if self.in_flight.stop(run) {
            info!(run = %run, "asked to stop");
            return Ok(());
        }
        if self.registry.cancel_queued(run).await? {
            info!(run = %run, "cancelled before it started");
            return Ok(());
        }
        let state = summary.attempts.last().map(|attempt| attempt.state.as_str()).unwrap_or("queued");
        warn!(run = %run, state, "nothing to cancel");
        Err(RunsError::NothingToCancel { run: run.to_string(), state: state.to_string() })
    }

    /// Start the workers. Each runs until the process ends.
    pub fn start_workers(self: &Arc<Self>) {
        for worker in 0..self.config.runs_at_once {
            let runs = Arc::clone(self);
            tokio::spawn(async move { runs.work(worker).await });
        }
        info!(workers = self.config.runs_at_once, "workers started");
    }

    /// One worker: claim, run, and go round again. A claim that finds nothing
    /// waits to be woken, with a poll behind it so a worker freed by another
    /// process's submission still picks it up.
    async fn work(self: Arc<Self>, worker: usize) {
        loop {
            let taken = {
                // Claimed and registered under one lock: the moment the registry
                // calls this attempt `running`, a cancel can reach it.
                let _claiming = self.claiming.lock().await;
                match self.registry.claim_queued().await {
                    Ok(Some(claimed)) => {
                        let watching = Watching { counters: Counters::new(), stop: StopSignal::new() };
                        self.in_flight.insert(&claimed.run, &watching.stop, &watching.counters);
                        Some((claimed, watching))
                    }
                    Ok(None) => None,
                    Err(failure) => {
                        error!(worker, %failure, "the registry would not hand out work");
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        continue;
                    }
                }
            };
            match taken {
                Some((claimed, watching)) => {
                    let run = claimed.run.clone();
                    let n = claimed.n;
                    // A panic anywhere in the run's own bookkeeping — not the
                    // simulation, whose panic the join already catches — would
                    // otherwise end this task and take a worker with it for the
                    // life of the process, leaving the attempt `running` until a
                    // restart calls it `partial`.
                    if AssertUnwindSafe(self.run_one(claimed, watching)).catch_unwind().await.is_err() {
                        error!(run = %run, worker, "the worker panicked around the run");
                        self.in_flight.remove(&run);
                        self.failed(&run, n, "the server panicked while running this attempt").await;
                    }
                }
                None => {
                    tokio::select! {
                        _ = self.queued.notified() => {}
                        _ = tokio::time::sleep(Duration::from_secs(2)) => {}
                    }
                }
            }
        }
    }

    /// Run one claimed attempt through to its recording and its outcome.
    ///
    /// `watching` is already registered against the run, so a cancel that
    /// arrives while the spec is being fetched sets the stop signal and the
    /// simulator stops at the first place it looks.
    async fn run_one(&self, claimed: ClaimedAttempt, watching: Watching) {
        let ClaimedAttempt { run, n, name, spec_key } = claimed;
        let Watching { counters, stop } = watching;
        // Values read straight out of the registry, not minted here: a row this
        // build did not write is this attempt's failure, not this worker's.
        let (attempt, id) = match (AttemptNumber::new(n), RunId::new(run.clone())) {
            (Ok(attempt), Ok(id)) => (attempt, id),
            (attempt, id) => {
                let why = attempt.err().map(|e| e.to_string()).or_else(|| id.err().map(|e| e.to_string()));
                self.in_flight.remove(&run);
                let why = why.unwrap_or_default();
                return self.failed(&run, n, &format!("the registry holds a row this server cannot read: {why}")).await;
            }
        };
        let path = self.recording_path(&id, attempt);

        let folder = match self.prepare_folder(&run, &spec_key).await {
            Ok(folder) => folder,
            Err(failure) => {
                self.in_flight.remove(&run);
                return self.failed(&run, n, &failure.to_string()).await;
            }
        };

        let publisher = self.publish_progress(run.clone(), n, counters.clone());
        let stamped = (run.clone(), n);
        let outcome =
            self.simulate(folder.spec_path.clone(), name, counters.clone(), stop, path.clone(), stamped).await;
        publisher.abort();
        // One last reading before the attempt closes. The publisher writes every
        // half second and is stopped here, so without this a run that took less
        // than that reports nothing and every other run keeps the reading from
        // its last tick rather than its last instant.
        let reached = Progress { sim_ms: counters.shown_sim_time().as_ms(), picks: counters.shown_picks() };
        if let Err(failure) = self.registry.record_progress(&run, n, reached).await {
            warn!(run = %run, attempt = n, %failure, "the run's last progress could not be written down");
        }
        self.in_flight.remove(&run);
        let _ = remove_dir_all(&folder.root);

        match outcome {
            Ok(recorded) => {
                let key = recording_key(&id, attempt);
                match self.store_recording(&run, n, &path, &key).await {
                    Ok(()) => {
                        self.record_ending(&run, n, &key, &recorded, &path).await;
                        under_ceiling(&self.config.temp_dir, self.config.cache_bytes, &path);
                    }
                    Err(failure) => {
                        self.failed(&run, n, &failure.to_string()).await;
                        // Nothing was stored, so nothing will read this.
                        let _ = remove_file(&path);
                    }
                }
            }
            Err(reason) => {
                self.failed(&run, n, &reason).await;
                let _ = remove_file(&path);
            }
        }
    }

    /// Write down how a stored run ended. The local copy STAYS either way.
    ///
    /// It stays because it is now the cache an attachment reads: the instant
    /// the registry publishes the object key, a browser may ask for this run,
    /// and a file removed here would be removed out from under the reader that
    /// had just been handed its path. What bounds the folder is the cache
    /// ceiling in `recording`, not a delete on the way out.
    async fn record_ending(&self, run: &str, n: u32, key: &str, recorded: &Recorded, path: &Path) {
        let state = recorded.state;
        // A failure keeps its reason in both columns: a reader showing what went
        // wrong reads `error`, one showing why the simulator stopped reads
        // `stop_reason`, and here they are the same sentence.
        let error = matches!(state, AttemptState::Failed).then_some(recorded.stop_reason.as_str());
        // What the recording holds goes down with the ending, read here because
        // this is the one moment the file is certainly on this machine. A file
        // that will not read leaves those three columns empty rather than the
        // ending unwritten: the run still ended, and `status` reads the extent
        // off the stored object for whatever the registry cannot say.
        let held = match extent(path) {
            Ok(held) => Some(held),
            Err(failure) => {
                warn!(run = %run, attempt = n, %failure, "the recording's extent could not be read as it was stored");
                None
            }
        };
        for attempt_at_it in 1..=ENDING_TRIES {
            let ended =
                self.registry.attempt_ended(run, n, state, Some(&recorded.stop_reason), Some(key), error, held).await;
            match ended {
                Ok(()) => {
                    info!(run = %run, attempt = n, state = state.as_str(), key = %key, "recorded");
                    return;
                }
                Err(RegistryError::NotRunning { .. }) => {
                    warn!(run = %run, attempt = n, key = %key, "the attempt was closed by someone else; its ending stands");
                    return;
                }
                Err(failure) if attempt_at_it < ENDING_TRIES => {
                    warn!(run = %run, attempt = n, key = %key, %failure, "the ending did not take; trying again");
                    tokio::time::sleep(ENDING_RETRY).await;
                }
                Err(failure) => error!(
                    run = %run, attempt = n, key = %key, path = %path.display(), %failure,
                    "the recording is stored and its ending is not written down; the local copy is kept \
                     and a restart will reconcile the two"
                ),
            }
        }
    }

    /// Fetch the stored spec and lay it back out for the harness.
    async fn prepare_folder(&self, run: &str, key: &str) -> Result<Folder, RunsError> {
        let bundle = self.storage.get(key).await.map_err(|source| RunsError::Storage {
            doing: "read back",
            key: key.to_string(),
            source,
        })?;
        let submitted: SubmittedSpec = serde_json::from_slice(&bundle).map_err(|source| SpecError::NotJson(source))?;
        let root = tempdir_under(&self.config.temp_dir, run, "spec")?;
        let laid = lay_out(&submitted, &root)?;
        Ok(Folder { root, spec_path: laid.spec_path })
    }

    /// Run the simulation, its recording written as it goes, off the async
    /// runtime.
    ///
    /// The simulator panics on anything it cannot proceed through — a
    /// composition that will not build, a scheduler invariant — and that is the
    /// contract everywhere else too. Here the panic is caught at the join and
    /// becomes the attempt's failure, so one bad spec does not take the server
    /// down with it.
    async fn simulate(
        &self,
        spec_path: PathBuf,
        name: String,
        counters: Counters,
        stop: Arc<StopSignal>,
        path: PathBuf,
        stamped: (String, u32),
    ) -> Result<Recorded, String> {
        let joined = tokio::task::spawn_blocking(move || {
            // The run's name is the only thing the server says about a run that
            // the spec does not; everything else a run needs said is in the
            // spec, which is why the protocol takes a spec and nothing else.
            let overrides = RunOverrides { run_name: Some(name), ..RunOverrides::default() };
            let prepared = prepare(&spec_path, &overrides).map_err(|failure| failure.to_string())?;
            let live = RunnerLiveHandles { counters: Some(counters), stop_signal: Some(stop) };
            let hooks = RunnerHooks { live: Some(live), subscribers: Vec::new(), recording: None };
            let target = RecordingTarget { path, run: stamped.0, attempt: stamped.1, tap: None };
            let (profile, _summary) = prepared.execute(hooks, Some(target)).map_err(|failure| failure.to_string())?;
            let stop_reason = profile.sim_result.stop_reason.clone();
            Ok(Recorded { state: state_for(&stop_reason), stop_reason: stop_reason.to_string() })
        })
        .await;

        match joined {
            Ok(result) => result,
            Err(join) if join.is_panic() => Err(panic_message(join.into_panic())),
            Err(join) => Err(join.to_string()),
        }
    }

    /// Upload a recording whole, streamed off the disk it was written to rather
    /// than read into memory beside every other run in flight. The local copy
    /// stays; `record_ending` says why.
    ///
    /// A failed upload is tried again, `UPLOAD_TRIES` times in all, because
    /// the recording is the whole product of a run that may have taken an hour
    /// and the failure is usually the bucket's, which passes. Only the last
    /// failure is returned; every one is logged.
    async fn store_recording(&self, run: &str, n: u32, path: &Path, key: &str) -> Result<(), RunsError> {
        for attempt_at_it in 1..=UPLOAD_TRIES {
            match self.storage.put_file(key, path).await {
                Ok(_) => return Ok(()),
                Err(source) if attempt_at_it < UPLOAD_TRIES => {
                    warn!(run = %run, attempt = n, key = %key, %source, "the upload did not take; trying again");
                    tokio::time::sleep(UPLOAD_RETRY).await;
                }
                Err(source) => {
                    return Err(RunsError::Storage { doing: "stored", key: key.to_string(), source });
                }
            }
        }
        unreachable!("every try returns")
    }

    /// Copy a running simulation's progress figures, which its final slices
    /// state, into the registry until the run ends and the task is dropped.
    fn publish_progress(&self, run: String, n: u32, counters: Counters) -> JoinHandle<()> {
        let registry = self.registry.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(PROGRESS_INTERVAL).await;
                let progress = Progress { sim_ms: counters.shown_sim_time().as_ms(), picks: counters.shown_picks() };
                if let Err(failure) = registry.record_progress(&run, n, progress).await {
                    warn!(run = %run, %failure, "progress could not be written down");
                }
            }
        })
    }

    /// Where an attempt's recording is written while it is being written: the
    /// storage key, under the temporary folder, so a file and the object it
    /// becomes have one name.
    fn recording_path(&self, run: &RunId, attempt: AttemptNumber) -> PathBuf {
        local_path(&self.config.temp_dir, run, attempt)
    }

    /// Write down that an attempt failed, and why.
    async fn failed(&self, run: &str, n: u32, reason: &str) {
        error!(run = %run, reason, "the run failed");
        let ended =
            self.registry.attempt_ended(run, n, AttemptState::Failed, Some(reason), None, Some(reason), None).await;
        if let Err(failure) = ended {
            error!(run = %run, %failure, "the failure could not be recorded");
        }
    }
}

/// A submission the server has taken on: the run exists and its spec is
/// stored, and nothing will start it until [`Runs::activate`] is called.
pub struct Accepted {
    /// The run's id, which is what the client is told.
    pub run: String,
}

/// Prepare a submission the way the worker will, and report the name its
/// recording will carry.
///
/// The folder is removed on every path, so a refused submission leaves nothing
/// behind. Blocking: this loads a world document.
fn prepared_name(submitted: &SubmittedSpec, temp_dir: &Path, run: &str) -> Result<String, RunsError> {
    let root = tempdir_under(temp_dir, run, "accepting")?;
    let prepared = lay_out(submitted, &root)
        .map_err(RunsError::from)
        .and_then(|laid| {
            let overrides = RunOverrides { run_name: submitted.name.clone(), ..RunOverrides::default() };
            prepare(&laid.spec_path, &overrides).map_err(|failure| RunsError::Prepare(failure.to_string()))
        })
        .map(|prepared| prepared.name);
    let _ = remove_dir_all(&root);
    prepared
}

/// What a run publishes while it is going and what can stop it, registered
/// against the run before its attempt is claimed.
struct Watching {
    counters: Counters,
    stop: Arc<StopSignal>,
}

/// A submission written out, and the folder to remove when the run is over.
struct Folder {
    root: PathBuf,
    spec_path: PathBuf,
}

/// A run that reached its end and was written out.
struct Recorded {
    state: AttemptState,
    stop_reason: String,
}

/// Which outcome a stop reason is.
///
/// An orderly stop is `finished`: every order completed, or the simulated
/// duration reached, or the wall-clock limit the spec asked for. Being told to
/// stop is `cancelled`. The simulator reporting an error of its own is
/// `failed`, and so is the watchdog — it fires when a task never completed, so
/// a run it stops is a deadlocked run and calling that a clean finish would
/// have `list`, `status` and `crucible-cli submit --wait` all report a success
/// and exit 0. Either way the recording holds what happened up to the stop and
/// is stored.
fn state_for(reason: &StopReason) -> AttemptState {
    match reason {
        StopReason::UserStop => AttemptState::Cancelled,
        StopReason::Error(_) | StopReason::Watchdog(_) => AttemptState::Failed,
        StopReason::Complete | StopReason::SimTimeLimit | StopReason::WallTimeout => AttemptState::Finished,
    }
}

/// What a panicking run said, so the attempt's `error` is the simulator's own
/// words rather than "the thread panicked".
fn panic_message(payload: Box<dyn Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    "the simulation panicked and said nothing".to_string()
}

/// A folder of this server's own, named after the run it belongs to.
fn tempdir_under(temp_dir: &Path, run: &str, what: &str) -> Result<PathBuf, SpecError> {
    let path = temp_dir.join("specs").join(format!("{run}-{what}"));
    let _ = remove_dir_all(&path);
    create_dir_all(&path).map_err(|source| SpecError::Write { path: path.clone(), source })?;
    Ok(path)
}

/// What a cancel reaches: the stop signal and counters of every run currently
/// on a worker.
///
/// A plain mutex over a map: it is touched twice per run and once per cancel.
mod in_flight {
    use std::sync::{Arc, Mutex};

    use dessim::sim_types::{Counters, StopReason, StopSignal};
    use rustc_hash::FxHashMap;

    #[derive(Default)]
    pub struct RunsInFlight {
        running: Mutex<FxHashMap<String, (Arc<StopSignal>, Counters)>>,
    }

    impl RunsInFlight {
        pub fn insert(&self, run: &str, stop: &Arc<StopSignal>, counters: &Counters) {
            self.running
                .lock()
                .expect("no run panics holding this")
                .insert(run.to_string(), (Arc::clone(stop), counters.clone()));
        }

        pub fn remove(&self, run: &str) {
            self.running.lock().expect("no run panics holding this").remove(run);
        }

        /// Tell a run to stop; reports whether there was one to tell.
        pub fn stop(&self, run: &str) -> bool {
            match self.running.lock().expect("no run panics holding this").get(run) {
                Some((signal, _)) => {
                    signal.stop(StopReason::UserStop);
                    true
                }
                None => false,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{panic_message, state_for};
    use crate::protocol::AttemptState;
    use dessim::sim_types::StopReason;

    #[test]
    fn only_an_orderly_stop_is_a_finish() {
        assert_eq!(state_for(&StopReason::UserStop), AttemptState::Cancelled);
        assert_eq!(state_for(&StopReason::SimTimeLimit), AttemptState::Finished);
        assert_eq!(state_for(&StopReason::Complete), AttemptState::Finished);
        assert_eq!(state_for(&StopReason::WallTimeout), AttemptState::Finished);
        assert_eq!(state_for(&StopReason::Error("no route".into())), AttemptState::Failed);
        // A watchdog stop is a deadlocked run, not a clean one.
        assert_eq!(state_for(&StopReason::Watchdog("stuck".into())), AttemptState::Failed);
    }

    #[test]
    fn a_panicking_run_reports_what_it_said() {
        assert_eq!(panic_message(Box::new("composition failed".to_string())), "composition failed");
        assert_eq!(panic_message(Box::new("map must have grid_bounds")), "map must have grid_bounds");
        assert!(panic_message(Box::new(7u8)).contains("said nothing"));
    }
}
