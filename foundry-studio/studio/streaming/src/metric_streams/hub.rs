//! Within `metric_streams`, `hub` is ONE ATTEMPT'S RECORDING FOLLOWED ONCE FOR EVERY METRIC SOCKET OF THE ATTEMPT: the
//! first socket's join on the `registry` starts the hub's `walk`, which hands each frame to `studio_metrics`' follower,
//! which computes every metric at once, and each socket holds its ask on the hub as a `member`. This module holds what
//! the three share: the follower, the hub's state and broadcast, and how a hub ends.

pub(super) mod member;
pub(super) mod registry;
mod walk;

use std::fmt;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard};

use desunits::SimTime;
use rustc_hash::{FxHashMap, FxHashSet};
use studio_metrics::{Ask, Follower, Unservable};
use studio_protocol::{MetricPoint, RunOutcome};
use tokio::sync::Notify;
use tokio::sync::broadcast::{Sender as EventSender, channel as event_channel};

use crate::provider::{Fault, Unreached};
use crate::streams::StreamKey;

/// Events a socket may fall behind its hub by before it is told it lagged.
const BACKLOG: usize = 1024;

struct Hub {
    attempt: StreamKey,
    /// Taken by the walk for each batch of frames and by a socket opening its view, never both at once.
    following: Mutex<Following>,
    state: Mutex<State>,
    events: EventSender<Event>,
    stopped: AtomicBool,
    stop: Notify,
}

/// The attempt's follower, made from the scenario block, the frames it has taken, and the views opened on it.
#[derive(Default)]
struct Following {
    follower: Option<Follower>,
    taken: u64,
    views: FxHashSet<Ask>,
}

struct State {
    /// How far the follower has read, and whether an open attempt's frames still follow.
    reached: SimTime,
    live: bool,
    /// How the run ended, once the walk has read the copy to its end; a hub that fails is forgotten instead.
    over: Option<RunOutcome>,
    members: usize,
    /// How many sockets hold each ask; a view no socket holds is closed with the next batch of frames.
    asks: FxHashMap<Ask, usize>,
}

/// What a hub broadcasts to its sockets.
#[derive(Debug, Clone)]
enum Event {
    /// The scenario block is in, so a socket can open its view.
    Described,
    /// Rows one ask's view made final once the follower had taken `through` frames.
    Rows {
        ask: Ask,
        through: u64,
        rows: Arc<[MetricPoint]>,
    },
    /// One ask's view can go no further.
    Failed {
        ask: Ask,
        why: Unservable,
    },
    Ended(HubEnd),
}

/// How a hub's walk ended: the copy was read to its end, saying how the run ended, or the hub failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HubEnd {
    Over(RunOutcome),
    Failed(HubFault),
}

/// Why a hub failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HubFault {
    Unreached(Unreached),
    /// The copy stopped short of its end.
    Read(Fault),
    /// The recording does not read as the follower takes it: its scenario block, or a later frame.
    Recording(String),
    /// The follower panicked, a defect, and is left poisoned for every view of the hub.
    Panicked(String),
}

impl fmt::Display for HubFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreached(unreached) => unreached.fmt(f),
            Self::Read(fault) => fault.fmt(f),
            Self::Recording(why) => write!(f, "the recording does not read: {why}"),
            Self::Panicked(why) => write!(f, "the follower panicked: {why}"),
        }
    }
}

impl Hub {
    fn new(attempt: StreamKey) -> Self {
        let state = State { reached: SimTime::ZERO, live: false, over: None, members: 0, asks: FxHashMap::default() };
        Self {
            attempt,
            following: Mutex::default(),
            state: Mutex::new(state),
            events: event_channel(BACKLOG).0,
            stopped: AtomicBool::new(false),
            stop: Notify::new(),
        }
    }

    /// The follower, or the hub's fault once a panic as it was held has poisoned it.
    fn lock_following(&self) -> Result<MutexGuard<'_, Following>, HubFault> {
        let poisoned = |_| HubFault::Panicked("earlier, as a view opened or a batch of frames went through".to_owned());
        self.following.lock().map_err(poisoned)
    }
}
