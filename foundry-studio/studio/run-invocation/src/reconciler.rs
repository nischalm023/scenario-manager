//! Within `run-invocation`, `reconciler` is spec 08 section 3.6's loop that squares every attempt holding a seat with
//! what the invoker says of its Job: it frees the seat once no pod of an ended attempt's Job runs, stopping the Job
//! unless its runner reported, which then exits by itself and leaves the Job and its log to the platform's keeping,
//! re-creates under a fresh token a recorded Job the invoker does not know while its attempt may still start (the
//! store re-reading that under the attempt's row lock, the create going the dispatcher's guarded way), leaves a
//! running cancellation to the runner's control channel, and writes the endings only the platform can see: a runner
//! that never started, a cancel never confirmed, a runner lost. A Job some creator may still be creating is left alone.
//! What the invoker says of each Job is kept on the attempt's row in words, so a read of the attempt sees the
//! platform's view without asking the platform.

use std::sync::Arc;
use std::time::{Duration, Instant};

use desunits::SimTime;
use rustc_hash::FxHashMap;
use studio_protocol::{EndedBy, Ending, RunState, WallClock};
use tokio::select;
use tokio::time::sleep;

use crate::Fault;
use crate::dispatcher::spawn_creation;
use crate::job::{Creator, Invoker, JobName, JobStatus};
use crate::settings::{LoopSettings, Wakes};
use crate::store::{HeldAttempt, Reserved, RunStore};

/// What squaring one held attempt came to.
enum Squared {
    /// Its Job is known stopped and its seat freed.
    Freed,
    /// The invoker does not know its Job, which may still start: it is to be created again.
    Absent,
    Left,
}

pub(crate) async fn run<S: RunStore, I: Invoker>(
    store: Arc<S>,
    invoker: Arc<I>,
    settings: Arc<LoopSettings>,
    wakes: Arc<Wakes>,
    creator: Creator,
) {
    // When each running attempt's Job was first seen gone with no ending: the lost-runner grace counts from there.
    let mut gone_since: FxHashMap<JobName, Instant> = FxHashMap::default();
    loop {
        match store.held().await {
            Ok(held) => {
                gone_since.retain(|job, _| held.iter().any(|attempt| attempt.job == *job));
                for attempt in held.into_iter().filter(|attempt| !being_created(attempt, &creator, &settings)) {
                    let squared = match square(&*store, &*invoker, &settings, &attempt, &mut gone_since).await {
                        Ok(Squared::Absent) => recreate(&store, &invoker, &settings, &wakes, &creator, &attempt).await,
                        Ok(Squared::Freed) => {
                            wakes.dispatch();
                            Ok(())
                        }
                        Ok(Squared::Left) => Ok(()),
                        Err(fault) => Err(fault),
                    };
                    if let Err(fault) = squared {
                        eprintln!("run-invocation: the reconciler could not square {}: {fault}", attempt.key);
                    }
                }
            }
            Err(fault) => eprintln!("run-invocation: the reconciler could not read the seats held: {fault}"),
        }
        select! {
            () = wakes.reconciler.notified() => {}
            () = sleep(settings.reconcile_every) => {}
        }
    }
}

/// Whether some creator may still be creating the attempt's Job, so no status of it can free its seat (spec 08
/// section 3.4, step 5): these loops, until every create of it has returned, or another instance, whose claim is a
/// stopped process's once the grace has passed since its creation began.
fn being_created(attempt: &HeldAttempt, me: &Creator, settings: &LoopSettings) -> bool {
    match &attempt.creator {
        None => false,
        Some(creator) if creator == me => true,
        Some(_) => since(attempt.claimed_at) < settings.create_grace,
    }
}

/// Create again the recorded Job of `attempt`, which the invoker does not know: the store re-reads, under the
/// attempt's row lock, that it is still queued, not cancelled and past the creation grace, and claims it for these
/// loops with a fresh token; the create then goes the dispatcher's guarded way.
async fn recreate<S: RunStore, I: Invoker>(
    store: &Arc<S>,
    invoker: &Arc<I>,
    settings: &Arc<LoopSettings>,
    wakes: &Arc<Wakes>,
    creator: &Creator,
    attempt: &HeldAttempt,
) -> Result<(), Fault> {
    let Some(token) = store.reclaim(&attempt.key, creator, settings.create_grace).await? else { return Ok(()) };
    let claimed = Reserved { key: attempt.key.clone(), job: attempt.job.clone(), image: attempt.image.clone(), token };
    spawn_creation(store, invoker, settings, wakes, creator, claimed);
    Ok(())
}

/// One held attempt against its Job's status.
async fn square<S: RunStore, I: Invoker>(
    store: &S,
    invoker: &I,
    settings: &LoopSettings,
    attempt: &HeldAttempt,
    gone_since: &mut FxHashMap<JobName, Instant>,
) -> Result<Squared, Fault> {
    let status = invoker.status(&attempt.job).await?;
    let key = &attempt.key;
    store.platform_status(key, &status.to_string()).await?;
    let platform = |stop_reason: String| ending(RunState::Failed, EndedBy::Platform, stop_reason);
    match attempt.state {
        state if state.ended() => {
            if status.stopped() {
                store.job_stopped(key).await?;
                return Ok(Squared::Freed);
            }
            // A cancel, a deadline or the platform's own ending stops the Job; one whose runner reported is stopped
            // only when it has not exited within the grace.
            if attempt.reported_at.is_none_or(|at| since(at) >= settings.exit_grace) {
                invoker.stop(&attempt.job).await?;
            }
        }
        RunState::Queued => {
            let waited = since(attempt.dispatched_at);
            match status {
                // A Job created moments ago may not show yet; one the invoker still does not know after the grace is
                // created again.
                JobStatus::Absent if waited < settings.start_timeout => {
                    if since(attempt.claimed_at) >= settings.create_grace {
                        return Ok(Squared::Absent);
                    }
                }
                JobStatus::Absent => {
                    store.end(key, &platform(never_started(settings, None))).await?;
                }
                JobStatus::Pending { reason } if waited >= settings.start_timeout => {
                    store.end(key, &platform(never_started(settings, reason.as_deref()))).await?;
                    invoker.stop(&attempt.job).await?;
                }
                JobStatus::Running if waited >= settings.start_timeout => {
                    store.end(key, &platform(never_started(settings, None))).await?;
                    invoker.stop(&attempt.job).await?;
                }
                JobStatus::Ended { reason } => {
                    let reason = reason.map_or_else(String::new, |reason| format!(": {reason}"));
                    store.end(key, &platform(format!("the runner ended before it read its inputs{reason}"))).await?;
                }
                JobStatus::Pending { .. } | JobStatus::Running | JobStatus::Unknown { .. } => {}
            }
        }
        _ => {
            if let Some(asked) = attempt.cancel_requested_at {
                if since(asked) >= settings.cancel_grace {
                    let words = "cancelled; the runner did not confirm".to_owned();
                    store.end(key, &ending(RunState::Cancelled, EndedBy::Person, words)).await?;
                    invoker.stop(&attempt.job).await?;
                    return Ok(Squared::Left);
                }
                // The row is relayed over runner control. Deleting the Job here would send a platform
                // termination before the runner can confirm the person's request and retain its recording.
            }
            if status.stopped() {
                let first = *gone_since.entry(attempt.job.clone()).or_insert_with(Instant::now);
                if first.elapsed() >= settings.lost_grace {
                    let why = match status {
                        JobStatus::Ended { reason: Some(reason) } => format!("the runner was lost: {reason}"),
                        _ => "the runner was lost and no ending arrived".to_owned(),
                    };
                    store.end(key, &ending(RunState::Partial, EndedBy::Platform, why)).await?;
                }
            } else {
                gone_since.remove(&attempt.job);
            }
        }
    }
    Ok(Squared::Left)
}

fn ending(state: RunState, ended_by: EndedBy, stop_reason: String) -> Ending {
    Ending { state, ended_by, stop_reason, stop_detail: None, sim_ms: SimTime::ZERO }
}

fn never_started(settings: &LoopSettings, reason: Option<&str>) -> String {
    let minutes = settings.start_timeout.as_secs().div_ceil(60);
    let reason = reason.map_or_else(String::new, |reason| format!(": {reason}"));
    format!("the runner never started within {minutes} min{reason}")
}

/// The wall-clock time since `at`, none if `at` is ahead of this machine's clock.
fn since(at: WallClock) -> Duration {
    (WallClock::now().instant() - at.instant()).to_std().unwrap_or_default()
}
