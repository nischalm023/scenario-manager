//! Within `hub`, the `walk` is THE TASK THAT FEEDS AN ATTEMPT'S FRAMES TO ITS HUB'S FOLLOWER: it reads the recording
//! through the provider, its history in batches and then each frame as the server takes it in, hands each batch to
//! `studio_metrics`' follower on a blocking thread, the first frame making the follower from the scenario block, and
//! broadcasts every batch of rows the follower made final, until the copy's end or until the hub's last member left.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use bytes::Bytes;
use desunits::SimTime;
use rustc_hash::{FxHashMap, FxHashSet};
use studio_metrics::{Ask, Follower, Made};
use studio_protocol::{MetricPoint, RunOutcome};
use tokio::select;
use tokio::task::spawn_blocking;

use super::registry::Hubs;
use super::{Event, Following, Hub, HubEnd, HubFault};
use crate::cache::Known;
use crate::provider::{Provider, Reading, Stamped, ended_outcome};
use crate::streams::StreamKey;

/// Frames through the follower per blocking call.
const BATCH: usize = 32;

impl Hub {
    /// `frames` of `reading` through the follower on a blocking thread, and what that made known told.
    async fn take(self: &Arc<Self>, reading: &Reading, frames: &[Stamped]) -> Result<(), HubFault> {
        let bytes = reading.bytes(frames).await.map_err(HubFault::Read)?;
        let (hub, live) = (self.clone(), reading.live());
        spawn_blocking(move || hub.follow(bytes, live)).await.map_err(|panic| HubFault::Panicked(panic.to_string()))?
    }

    /// On a blocking thread: the views no socket holds closed, then `frames` through the follower, the first making it
    /// from the scenario block, and every batch of rows they made final broadcast under its ask, the frames taken
    /// counted first.
    fn follow(&self, frames: Vec<Bytes>, live: bool) -> Result<(), HubFault> {
        let mut following = self.lock_following()?;
        let Following { follower, taken, views } = &mut *following;
        if let Some(follower) = follower.as_mut() {
            let state = self.state.lock().expect("a hub");
            for ask in views.extract_if(|ask| !state.asks.contains_key(ask)) {
                follower.close(&ask);
            }
        }
        let described = follower.is_some();
        let mut made: Vec<Made> = Vec::new();
        for bytes in frames {
            match follower.as_mut() {
                Some(follower) => {
                    made.extend(follower.frame(&bytes).map_err(|fault| HubFault::Recording(fault.to_string()))?);
                }
                None => *follower = Some(follower_from(&bytes)?),
            }
            *taken += 1;
        }
        {
            let mut state = self.state.lock().expect("a hub");
            state.reached = follower.as_ref().map_or(SimTime::ZERO, Follower::reached);
            state.live = live;
        }
        if !described && follower.is_some() {
            let _ = self.events.send(Event::Described);
        }
        self.tell(made, *taken, views);
        Ok(())
    }

    /// The copy was read to its end without an end block, as one cut short is: the follower states its last slice
    /// whole, and the rows that made final are told.
    fn cut_short(&self) -> Result<(), HubFault> {
        let mut following = self.lock_following()?;
        let Following { follower, taken, views } = &mut *following;
        let Some(follower) = follower.as_mut() else { return Ok(()) };
        let made = follower.cut_short();
        self.state.lock().expect("a hub").reached = follower.reached();
        // The copy's end counts as one more step taken: a view opened before it is told the row, and one opened after
        // it holds the row in its history.
        *taken += 1;
        self.tell(made, *taken, views);
        Ok(())
    }

    /// The rows `made` final broadcast in one batch per ask, the frames taken counted first, and each view that can go
    /// no further closed with why.
    fn tell(&self, made: Vec<Made>, taken: u64, views: &mut FxHashSet<Ask>) {
        let mut batches: FxHashMap<Ask, Vec<MetricPoint>> = FxHashMap::default();
        let mut failed = Vec::new();
        for Made { ask, rows } in made {
            match rows {
                Ok(rows) => batches.entry(ask).or_default().extend(rows),
                Err(why) => failed.push((ask, why)),
            }
        }
        for (ask, rows) in batches {
            let _ = self.events.send(Event::Rows { ask, through: taken, rows: rows.into() });
        }
        for (ask, why) in failed {
            views.remove(&ask);
            let _ = self.events.send(Event::Failed { ask, why });
        }
    }
}

/// How a hub's walk stopped short of a fault.
enum Walked {
    /// The copy was read to its end: the reading, and how the end block said the run ended when the copy carried one.
    ToEnd(Reading, Option<RunOutcome>),
    /// Its last socket left.
    Stopped,
}

pub(super) async fn follow(hubs: Hubs, hub: Arc<Hub>, known: Result<Option<Known>, String>) {
    let StreamKey { run, attempt } = &hub.attempt;
    let end = match walk(hubs.provider(), &hub, known).await {
        Ok(Walked::ToEnd(reading, end_block)) => HubEnd::Over(ended_outcome(&reading, end_block)),
        Ok(Walked::Stopped) => return,
        Err(fault) => {
            eprintln!("studio-streaming: {run}/{}: the metrics stop: {fault}", attempt.0);
            HubEnd::Failed(fault)
        }
    };
    hubs.end(&hub, end);
}

/// The attempt's recording read whole through the follower, its history in batches and then each frame as it comes,
/// with whatever came behind it without a wait, so a burst goes through in batches too.
async fn walk(provider: &Provider, hub: &Arc<Hub>, known: Result<Option<Known>, String>) -> Result<Walked, HubFault> {
    let mut reading = provider.open(&hub.attempt, known).await.map_err(HubFault::Unreached)?;
    let history = reading.history().to_vec();
    for batch in history.chunks(BATCH) {
        if hub.stopped.load(Ordering::Acquire) {
            return Ok(Walked::Stopped);
        }
        hub.take(&reading, batch).await?;
    }
    loop {
        let next = select! {
            () = hub.stop.notified() => return Ok(Walked::Stopped),
            next = reading.next() => next.map_err(HubFault::Read)?,
        };
        match next {
            Some(frame) => {
                let mut frames = vec![frame];
                frames.extend(reading.ready(BATCH - 1));
                hub.take(&reading, &frames).await?;
            }
            None => break,
        }
    }
    let following = hub.lock_following()?;
    let follower =
        following.follower.as_ref().ok_or_else(|| HubFault::Recording("it holds no scenario block".to_owned()))?;
    let outcome = follower.outcome();
    drop(following);
    if outcome.is_none() {
        hub.cut_short()?;
    }
    Ok(Walked::ToEnd(reading, outcome))
}

/// The follower of the recording whose scenario block is `scenario`.
fn follower_from(scenario: &[u8]) -> Result<Follower, HubFault> {
    Follower::from_scenario(scenario).map_err(|fault| HubFault::Recording(fault.to_string()))
}
