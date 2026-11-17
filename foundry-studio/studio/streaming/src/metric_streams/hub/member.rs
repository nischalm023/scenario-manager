//! Within `hub`, a `member` is ONE SOCKET'S PLACE ON ITS ATTEMPT'S HUB: its ask's view opened on the follower, with
//! the rows the view states so far, and of the hub's broadcast only what concerns that view. The batches go out each
//! named by its ask and by how many frames the follower had taken, so a member opening its view while frames come
//! misses and repeats none; a member that falls behind the broadcast lags only itself, and a view that fails closes
//! only the sockets that asked for it.

#[cfg(test)]
mod tests;

use std::sync::Arc;

use desunits::SimTime;
use studio_metrics::{Ask, Unservable};
use studio_protocol::{MetricDescriptor, MetricPoint};
use tokio::sync::broadcast::Receiver as EventReceiver;
use tokio::sync::broadcast::error::RecvError;
use tokio::task::spawn_blocking;

use super::registry::Hubs;
use super::{Event, Following, Hub, HubEnd, HubFault};

/// One socket's ask on its attempt's hub, and what the hub tells it; dropping it leaves the hub.
pub struct Member {
    hubs: Hubs,
    hub: Arc<Hub>,
    ask: Ask,
    events: EventReceiver<Event>,
    /// The frames the follower had taken as the member's view opened, whose rows its history held.
    opened_through: Option<u64>,
}

/// What the hub tells one member.
#[derive(Debug, Clone, PartialEq)]
pub enum Told {
    /// The scenario block is in, so the member's view can open.
    Described,
    /// Rows the member's view made final after it opened.
    Rows(Arc<[MetricPoint]>),
    /// The member's view can go no further.
    Failed(Unservable),
    Ended(HubEnd),
}

impl Drop for Member {
    fn drop(&mut self) {
        self.hubs.leave(&self.hub, self.ask);
    }
}

/// The member fell further behind the hub's broadcast than its backlog, and missed what fell out of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lagged;

/// A socket's view as it opened: the metric as served, every row so far, how many frames the follower had taken by
/// then, and how far the recording reached and whether it goes on.
#[derive(Debug, Clone, PartialEq)]
pub struct Viewed {
    pub descriptor: MetricDescriptor,
    pub rows: Vec<MetricPoint>,
    pub through: u64,
    pub reached: SimTime,
    pub live: bool,
}

impl Member {
    pub(super) fn new(hubs: Hubs, hub: Arc<Hub>, ask: Ask, events: EventReceiver<Event>) -> Self {
        Self { hubs, hub, ask, events, opened_through: None }
    }

    /// The member's view opened on the hub's follower, or why it cannot be served; `None` until the scenario block is
    /// in, which `Told::Described` says. A follower that panics, as the view opens or before, fails the hub, so every
    /// socket of the attempt is told its fault and the next one starts afresh.
    pub async fn view(&mut self) -> Result<Option<Result<Viewed, Unservable>>, HubFault> {
        let (hub, ask) = (self.hub.clone(), self.ask);
        let viewed = spawn_blocking(move || hub.view(ask)).await;
        let viewed = viewed.map_err(|panic| HubFault::Panicked(panic.to_string())).and_then(|viewed| viewed);
        match &viewed {
            Ok(Some(Ok(viewed))) => self.opened_through = Some(viewed.through),
            Ok(_) => {}
            Err(fault) => self.hubs.end(&self.hub, HubEnd::Failed(fault.clone())),
        }
        viewed
    }

    /// The hub's next word to the member: of the rows only those of its own view made final after the view opened,
    /// since those before are in the history it opened with.
    pub async fn next(&mut self) -> Result<Told, Lagged> {
        loop {
            let event = match self.events.recv().await {
                Ok(event) => event,
                Err(RecvError::Lagged(_)) => return Err(Lagged),
                Err(RecvError::Closed) => unreachable!("a member holds its hub, which holds the broadcast's sender"),
            };
            match event {
                Event::Described => return Ok(Told::Described),
                Event::Rows { ask, through, rows }
                    if ask == self.ask && self.opened_through.is_some_and(|opened| through > opened) =>
                {
                    return Ok(Told::Rows(rows));
                }
                Event::Failed { ask, why } if ask == self.ask => return Ok(Told::Failed(why)),
                Event::Ended(end) => return Ok(Told::Ended(end)),
                Event::Rows { .. } | Event::Failed { .. } => {}
            }
        }
    }
}

impl Hub {
    /// On a blocking thread: `ask`'s view opened on the follower, by `Member::view`'s rule.
    fn view(&self, ask: Ask) -> Result<Option<Result<Viewed, Unservable>>, HubFault> {
        let mut following = self.lock_following()?;
        let Following { follower, taken, views } = &mut *following;
        let Some(follower) = follower.as_mut() else { return Ok(None) };
        let opened = follower.open(ask);
        if opened.is_ok() {
            views.insert(ask);
        }
        let state = self.state.lock().expect("a hub");
        Ok(Some(opened.map(|opened| Viewed {
            descriptor: opened.descriptor,
            rows: opened.rows,
            through: *taken,
            reached: state.reached,
            live: state.live,
        })))
    }
}
