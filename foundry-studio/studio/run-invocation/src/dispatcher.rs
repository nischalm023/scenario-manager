//! Within `run-invocation`, `dispatcher` is spec 08 section 3.4's loop that turns approved attempts into Jobs, at most
//! the seat limit at a time: it reserves a seat in the store's own transaction (so the count holds if the API dies
//! before the create), the row carrying its creator's claim until every create has returned, reads the attempt again
//! just before creating its Job (a cancel or an ending before it means no create; one after it is the reconciler's to
//! act on once the claim is let go), and fails an attempt whose Job could not be created after the retries.

use std::sync::Arc;
use std::time::Duration;

use desunits::SimTime;
use rustc_hash::FxHashMap;
use studio_protocol::{EndedBy, Ending, FORMAT, JobSpec, RunState, StreamingServer};
use tokio::select;
use tokio::time::sleep;

use crate::job::{Creator, Invoker, JobName, JobRequest};
use crate::settings::{LoopSettings, Wakes};
use crate::store::{AttemptKey, Reserved, RunStore};
use crate::token::Token;

/// How long a claim that could not be let go waits before it is let go again (wall clock).
const RELEASE_AGAIN: Duration = Duration::from_secs(1);

pub(crate) async fn run<S: RunStore, I: Invoker>(
    store: Arc<S>,
    invoker: Arc<I>,
    settings: Arc<LoopSettings>,
    wakes: Arc<Wakes>,
    creator: Creator,
) {
    loop {
        loop {
            match store.reserve(settings.seats, &creator).await {
                Ok(Some(reserved)) => spawn_creation(&store, &invoker, &settings, &wakes, &creator, reserved),
                Ok(None) => break,
                Err(fault) => {
                    eprintln!("run-invocation: the dispatcher could not reserve a seat: {fault}");
                    break;
                }
            }
        }
        select! {
            () = wakes.dispatcher.notified() => {}
            () = sleep(settings.dispatch_every) => {}
        }
    }
}

/// Create the Job of an attempt `creator` claimed, the dispatcher's reservation or the reconciler's re-creation, in a
/// task of its own; once every create of it has returned the claim is let go and the reconciler woken.
pub(crate) fn spawn_creation<S: RunStore, I: Invoker>(
    store: &Arc<S>,
    invoker: &Arc<I>,
    settings: &Arc<LoopSettings>,
    wakes: &Arc<Wakes>,
    creator: &Creator,
    reserved: Reserved,
) {
    let (store, invoker, settings) = (store.clone(), invoker.clone(), settings.clone());
    let (wakes, creator) = (wakes.clone(), creator.clone());
    tokio::spawn(async move {
        let key = reserved.key.clone();
        create(&*store, &*invoker, &settings, reserved).await;
        release(&*store, &key, &creator).await;
        wakes.reconcile();
    });
}

/// Create one claimed attempt's Job (section 3.4, steps 3 and 4).
async fn create<S: RunStore, I: Invoker>(store: &S, invoker: &I, settings: &LoopSettings, reserved: Reserved) {
    let Reserved { key, job, image, token } = reserved;
    match still_wanted(store, &key).await {
        Some(true) => {}
        // Cancelled or ended before its Job existed: none is created, so none can run, and the seat frees.
        Some(false) => {
            if let Err(fault) = store.job_stopped(&key).await {
                eprintln!("run-invocation: {key}'s seat, whose Job was never created, did not free: {fault}");
            }
            return;
        }
        None => return,
    }
    // The stream is observation: an attempt with no live streaming server runs unstreamed rather than waiting.
    let streaming = match store.stream_to(&key, settings.streaming_stale_after).await {
        Ok(Some(server)) => Some(server),
        Ok(None) => {
            eprintln!("run-invocation: {key} runs unstreamed: no streaming server has a fresh heartbeat");
            None
        }
        Err(fault) => {
            eprintln!("run-invocation: no streaming server could be picked, so {key} runs unstreamed: {fault}");
            None
        }
    };
    let request = request(settings, &key, job, image, token, streaming);
    let mut answer = invoker.start(request.clone()).await;
    for wait in &settings.create_retries {
        if answer.is_ok() {
            break;
        }
        sleep(*wait).await;
        answer = invoker.start(request.clone()).await;
    }
    if let Err(fault) = answer {
        let ending = Ending {
            state: RunState::Failed,
            ended_by: EndedBy::Platform,
            stop_reason: format!("its Job could not be created: {fault}"),
            stop_detail: None,
            sim_ms: SimTime::ZERO,
        };
        if let Err(fault) = store.end_uncreated(&key, &ending).await {
            eprintln!("run-invocation: {key} could not be ended after its Job failed to be created: {fault}");
        }
    }
}

/// Let go of `creator`'s claim once every create of the attempt's Job has returned, asking again until the store
/// answers: until it does, the reconciler leaves the Job to its creator.
async fn release<S: RunStore>(store: &S, key: &AttemptKey, creator: &Creator) {
    while let Err(fault) = store.release(key, creator).await {
        eprintln!("run-invocation: {key}'s creation claim was not let go, and is asked again: {fault}");
        sleep(RELEASE_AGAIN).await;
    }
}

/// Whether the attempt still wants its Job just before its create: queued and not cancelled; `None` when it cannot be
/// read.
async fn still_wanted<S: RunStore>(store: &S, key: &AttemptKey) -> Option<bool> {
    match store.attempt(key).await {
        Ok(Some(row)) => Some(row.state == RunState::Queued && !row.cancel_requested),
        Ok(None) => Some(false),
        Err(fault) => {
            eprintln!("run-invocation: {key} could not be read before its create: {fault}");
            None
        }
    }
}

/// The job request for one attempt: its name, its plan's image, its spec naming the runners' listener and the
/// streaming server picked, its token apart, the deadline, the resources and the labels the reconciler reads.
pub(crate) fn request(
    settings: &LoopSettings,
    key: &AttemptKey,
    job: JobName,
    image: String,
    token: Token,
    streaming: Option<StreamingServer>,
) -> JobRequest {
    let spec = JobSpec { v: FORMAT, run: key.run.clone(), attempt: key.n, studio: settings.studio.clone(), streaming };
    let labels = FxHashMap::from_iter([
        ("agent-studio/attempt".to_owned(), key.n.0.to_string()),
        ("agent-studio/run".to_owned(), key.run.to_string()),
    ]);
    JobRequest { name: job, image, spec, token, deadline: settings.deadline, resources: settings.resources, labels }
}
