//! Within `hub`, the `registry` is EVERY HUB THIS SERVER RUNS AND WHO HOLDS EACH: a socket's join finds its attempt's
//! hub or starts one with its walk, counting the socket and its ask on the hub; its leaving lets a view no socket holds
//! close, and the last one out stops the hub. The walk's end is kept for later joins and told to every member, and a
//! failed hub is forgotten at once, so the next socket starts afresh.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use rustc_hash::FxHashMap;
use studio_metrics::Ask;
use studio_protocol::RunOutcome;

use super::member::Member;
use super::walk::follow;
use super::{Event, Hub, HubEnd};
use crate::cache::Known;
use crate::provider::Provider;
use crate::streams::StreamKey;

/// Every hub this server runs, shared by the metric sockets.
#[derive(Clone)]
pub struct Hubs {
    inner: Arc<Inner>,
}

struct Inner {
    provider: Provider,
    running: Mutex<FxHashMap<StreamKey, Arc<Hub>>>,
}

/// One socket's place in its attempt's hub, with how the run ended if the hub had read the copy to its end as the
/// socket joined. A failed hub is never joined: it is forgotten as it fails, under the lock a join takes.
pub struct Joined {
    pub over: Option<RunOutcome>,
    pub member: Member,
}

impl Hubs {
    pub fn new(provider: Provider) -> Self {
        Self { inner: Arc::new(Inner { provider, running: Mutex::default() }) }
    }

    /// A socket's join on the hub of `attempt` for `ask`, the hub started when none runs; `known` is where the tables
    /// say the store holds the attempt, which only a starting hub reads.
    pub fn join(&self, attempt: &StreamKey, ask: Ask, known: Result<Option<Known>, String>) -> Joined {
        let mut running = self.inner.running.lock().expect("the hubs");
        let hub = match running.get(attempt) {
            Some(hub) => hub.clone(),
            None => {
                let hub = Arc::new(Hub::new(attempt.clone()));
                running.insert(attempt.clone(), hub.clone());
                tokio::spawn(follow(self.clone(), hub.clone(), known));
                hub
            }
        };
        let mut state = hub.state.lock().expect("a hub");
        state.members += 1;
        *state.asks.entry(ask).or_insert(0) += 1;
        let events = hub.events.subscribe();
        Joined { over: state.over, member: Member::new(self.clone(), hub.clone(), ask, events) }
    }

    /// The provider every hub's walk reads its attempt through.
    pub(super) fn provider(&self) -> &Provider {
        &self.inner.provider
    }

    /// How many hubs run.
    pub fn running(&self) -> usize {
        self.inner.running.lock().expect("the hubs").len()
    }

    /// A socket left: the last one asking its ask lets the view close, and the last one out stops the hub.
    pub(super) fn leave(&self, hub: &Arc<Hub>, ask: Ask) {
        let mut running = self.inner.running.lock().expect("the hubs");
        let mut state = hub.state.lock().expect("a hub");
        if let Some(holding) = state.asks.get_mut(&ask) {
            *holding -= 1;
            if *holding == 0 {
                state.asks.remove(&ask);
            }
        }
        state.members -= 1;
        if state.members == 0 {
            forget(&mut running, hub);
            hub.stopped.store(true, Ordering::Release);
            hub.stop.notify_one();
        }
    }

    /// The hub's end told to its members: a run read to its end kept for later joins, a failed hub forgotten.
    pub(super) fn end(&self, hub: &Arc<Hub>, end: HubEnd) {
        let mut running = self.inner.running.lock().expect("the hubs");
        let mut state = hub.state.lock().expect("a hub");
        match &end {
            HubEnd::Over(outcome) => state.over = Some(*outcome),
            HubEnd::Failed(_) => forget(&mut running, hub),
        }
        let _ = hub.events.send(Event::Ended(end));
    }
}

/// `hub` out of the running map, unless a newer hub of its attempt has taken its place.
fn forget(running: &mut FxHashMap<StreamKey, Arc<Hub>>, hub: &Arc<Hub>) {
    if running.get(&hub.attempt).is_some_and(|held| Arc::ptr_eq(held, hub)) {
        running.remove(&hub.attempt);
    }
}
