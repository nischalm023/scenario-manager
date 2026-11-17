//! Within `run-invocation`, `watcher` is spec 08 section 3.6's durable watcher: it hands each recorded change of an
//! attempt's state to `RunEvents` in order, ends a submission's round right after its last run's own ending in that
//! order, and only then advances its cursor, so a watcher that dies before advancing repeats itself and the events' dedup keys
//! absorb the repeat. Progress is no transition: it is read every tick and sent at most once per run per interval.

use std::sync::Arc;
use std::time::Instant;

use rustc_hash::FxHashMap;
use studio_protocol::Ulid;
use tokio::select;
use tokio::time::sleep;

use crate::Fault;
use crate::settings::{LoopSettings, Wakes};
use crate::store::{Cursor, RunEvents, RunStore};

pub(crate) async fn run<S: RunStore, E: RunEvents>(
    store: Arc<S>,
    events: Arc<E>,
    settings: Arc<LoopSettings>,
    wakes: Arc<Wakes>,
) {
    let mut cursor = None;
    let mut last_progress: FxHashMap<Ulid, Instant> = FxHashMap::default();
    loop {
        if cursor.is_none() {
            cursor = store.cursor().await.map_err(|fault| complain("read its cursor", &fault)).ok();
        }
        if let Some(at) = cursor.as_mut()
            && let Err(fault) = hand_on(&*store, &*events, &wakes, at).await
        {
            complain("hand a transition on", &fault);
        }
        if let Err(fault) = progress(&*store, &*events, &settings, &mut last_progress).await {
            complain("send progress", &fault);
        }
        select! {
            () = wakes.watcher.notified() => {}
            () = sleep(settings.watch_every) => {}
        }
    }
}

/// Every transition after `cursor`, each told, its round's end told when it was the last, and the cursor advanced.
async fn hand_on<S: RunStore, E: RunEvents>(
    store: &S,
    events: &E,
    wakes: &Wakes,
    cursor: &mut Cursor,
) -> Result<(), Fault> {
    loop {
        let transitions = store.transitions_after(*cursor).await?;
        if transitions.is_empty() {
            return Ok(());
        }
        for transition in transitions {
            events.status_changed(&transition).await?;
            if transition.to.ended() {
                wakes.dispatch();
                wakes.reconcile();
                if let Some(round) = store.round_ended(&transition).await? {
                    events.submission_ended(&round).await?;
                }
            }
            store.advance(transition.at).await?;
            *cursor = transition.at;
        }
    }
}

async fn progress<S: RunStore, E: RunEvents>(
    store: &S,
    events: &E,
    settings: &LoopSettings,
    last: &mut FxHashMap<Ulid, Instant>,
) -> Result<(), Fault> {
    let running = store.progress_of_running().await?;
    last.retain(|run, _| running.iter().any(|progress| progress.key.run == *run));
    for progress in running {
        let due = last.get(&progress.key.run).is_none_or(|sent| sent.elapsed() >= settings.progress_every);
        if due {
            events.progress(&progress).await?;
            last.insert(progress.key.run.clone(), Instant::now());
        }
    }
    Ok(())
}

fn complain(doing: &str, fault: &Fault) {
    eprintln!("run-invocation: the watcher could not {doing}: {fault}");
}
