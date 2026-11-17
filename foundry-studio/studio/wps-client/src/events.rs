//! Within `wps-client`, `events` is spec 04 section 3.6: one upstream event stream per watched scope, shared by every
//! consumer in the process. WPS sends one `changeset` event per scope a changeset writes, each carrying the commit
//! place the scopes of one changeset share, with no id and no resume, and moves a listener 256 behind forward without
//! telling it; so this module marks a `Moved` whose changeset skips past the last one seen with `gap`, and after a
//! reconnect reads the head and yields `Resynced` if the scope moved meanwhile.
//! A connection counts as ready only once the head was read after it went live, and each consumer starts from the
//! head ready then: a consumer that states the changeset it last knew hears first of any move past it, so a
//! changeset landing before the watch was ready still reaches the stale-draft watcher.

use std::pin::Pin;
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::{Stream, StreamExt};
use rustc_hash::FxHashMap;
use studio_protocol::{ScopePath, Seq};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::{Receiver, Sender, channel};
use tokio::sync::watch::{self, Receiver as HeadReceiver, Sender as HeadSender};
use tokio::task::AbortHandle;

use crate::snapshot::ScopeEvent;
use crate::wps::Wps;

/// One watched scope: the shared channel, the newest changeset the upstream has told of (`None` until its first
/// connection is ready), and the upstream task ended when the last consumer goes.
struct Watch {
    sender: Sender<ScopeEvent>,
    head: HeadReceiver<Option<Seq>>,
    upstream: AbortHandle,
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.upstream.abort();
    }
}

/// The process's watched scopes.
pub struct EventHub<S: Wps> {
    wps: Arc<S>,
    watches: Mutex<FxHashMap<ScopePath, Weak<Watch>>>,
    first: Duration,
    last: Duration,
    backlog: usize,
}

/// A consumer's stream of one scope's events.
pub struct ScopeEvents {
    _watch: Arc<Watch>,
    events: Pin<Box<dyn Stream<Item = ScopeEvent> + Send>>,
}

impl Stream for ScopeEvents {
    type Item = ScopeEvent;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<ScopeEvent>> {
        self.events.as_mut().poll_next(context)
    }
}

/// Where one consumer's stream stands: waiting for the upstream to be ready, or live from a changeset on.
enum Phase {
    Waiting { known: Option<Seq> },
    Live { floor: Seq, behind: bool },
}

impl<S: Wps> EventHub<S> {
    /// A hub reconnecting from `first` doubling to `last` (wall clock), holding `backlog` events for its slowest
    /// consumer, reading a head by `GET /scopes?under=`.
    pub fn new(wps: Arc<S>, first: Duration, last: Duration, backlog: usize) -> Self {
        Self { wps, watches: Mutex::new(FxHashMap::default()), first, last, backlog }
    }

    /// Follow `scope` from the head it stands at once the watch is ready, sharing the upstream stream with every
    /// other consumer of it; with `known`, a head past it is told first as `Resynced`.
    pub fn watch(&self, scope: &ScopePath, known: Option<Seq>) -> ScopeEvents {
        let mut watches = self.watches.lock().expect("the hub's lock");
        let watch = match watches.get(scope).and_then(Weak::upgrade) {
            Some(watch) => watch,
            None => {
                let (sender, _) = channel(self.backlog);
                let (told, head) = watch::channel(None);
                let task = tokio::spawn(upstream(
                    self.wps.clone(),
                    scope.clone(),
                    sender.clone(),
                    told,
                    (self.first, self.last),
                ));
                let watch = Arc::new(Watch { sender, head, upstream: task.abort_handle() });
                watches.insert(scope.clone(), Arc::downgrade(&watch));
                watch
            }
        };
        // Subscribed before the baseline is read, so no move after the baseline is missed.
        let receiver = watch.sender.subscribe();
        let state = (receiver, watch.head.clone(), Phase::Waiting { known }, scope.clone());
        let events = futures_util::stream::unfold(state, |(mut receiver, mut head, mut phase, scope)| async move {
            loop {
                match phase {
                    Phase::Waiting { known } => {
                        let ready = *head.borrow_and_update();
                        let Some(baseline) = ready else {
                            head.changed().await.ok()?;
                            continue;
                        };
                        phase = Phase::Live { floor: baseline, behind: false };
                        if known.is_some_and(|known| baseline > known) {
                            let event = ScopeEvent::Resynced { scope: scope.clone(), head: baseline };
                            return Some((event, (receiver, head, phase, scope)));
                        }
                    }
                    Phase::Live { floor, behind } => match next_event(&mut receiver).await? {
                        Ok(ScopeEvent::Moved { seq, .. }) | Ok(ScopeEvent::Resynced { head: seq, .. })
                            if seq <= floor => {}
                        Ok(ScopeEvent::Moved { scope: moved, seq, author, tally, gap, commit }) => {
                            phase = Phase::Live { floor: seq, behind: false };
                            let event =
                                ScopeEvent::Moved { scope: moved, seq, author, tally, gap: gap || behind, commit };
                            return Some((event, (receiver, head, phase, scope)));
                        }
                        Ok(ScopeEvent::Resynced { scope: moved, head: seq }) => {
                            phase = Phase::Live { floor: seq, behind: false };
                            let event = ScopeEvent::Resynced { scope: moved, head: seq };
                            return Some((event, (receiver, head, phase, scope)));
                        }
                        Err(()) => phase = Phase::Live { floor, behind: true },
                    },
                }
            }
        });
        ScopeEvents { _watch: watch, events: Box::pin(events) }
    }
}

/// The next event, `Err` when this consumer fell behind and lost some, `None` when the upstream ended.
async fn next_event(receiver: &mut Receiver<ScopeEvent>) -> Option<Result<ScopeEvent, ()>> {
    match receiver.recv().await {
        Ok(event) => Some(Ok(event)),
        Err(RecvError::Lagged(_)) => Some(Err(())),
        Err(RecvError::Closed) => None,
    }
}

async fn head<S: Wps>(wps: &S, scope: &ScopePath) -> Option<Seq> {
    let rows = wps.scopes_under(scope).await.ok()?;
    rows.into_iter().find(|row| row.scope == scope.as_str()).map(|row| Seq::new(row.head))
}

/// The one connection to WPS for `scope`, reconnecting with backoff from the first wait doubling to the last (wall
/// clock). Each connection is ready once the head is read after it went live: every changeset after that read
/// reaches the stream, every one before it the read, and `told` carries the newest changeset told of before it is
/// broadcast.
async fn upstream<S: Wps>(
    wps: Arc<S>,
    scope: ScopePath,
    sender: Sender<ScopeEvent>,
    told: HeadSender<Option<Seq>>,
    (first, last): (Duration, Duration),
) {
    let mut wait = first;
    loop {
        if let Ok(mut stream) = wps.subscribe(&scope).await
            && let Some(now) = head(&*wps, &scope).await
        {
            let seen = *told.borrow();
            if seen.is_none_or(|seen| now > seen) {
                told.send_replace(Some(now));
                if seen.is_some() {
                    let _ = sender.send(ScopeEvent::Resynced { scope: scope.clone(), head: now });
                }
            }
            wait = first;
            while let Some(Ok(event)) = stream.next().await {
                let Some(seen) = *told.borrow() else { break };
                if event.seq <= seen {
                    continue;
                }
                told.send_replace(Some(event.seq));
                let gap = event.seq.get() > seen.get() + 1;
                let moved = ScopeEvent::Moved {
                    scope: scope.clone(),
                    seq: event.seq,
                    author: event.author,
                    tally: event.tally,
                    gap,
                    commit: event.commit,
                };
                let _ = sender.send(moved);
            }
        }
        tokio::time::sleep(wait).await;
        wait = (wait * 2).min(last);
    }
}
