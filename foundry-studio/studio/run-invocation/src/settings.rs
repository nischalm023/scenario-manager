//! Within `run-invocation`, `settings` is what the loops are told and how they are woken: the seat limit, the Job's
//! deadline and resources, the waits of spec 08 sections 3.4 and 3.6, each in wall clock, and the three `Notify`s a
//! plan's execution, a cancel or a stored transition rings.

use std::time::Duration;

use studio_protocol::{DELIVERY_BUDGET, Endpoint};
use tokio::sync::Notify;

use crate::job::Resources;

/// `STUDIO_RUNS_AT_ONCE`: the seats held at most across the deployment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Seats(pub u32);

/// `STUDIO_RUNNER_CPU_MILLIS`: a Job's CPU, at least one core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuMillis(pub u32);

/// `STUDIO_RUNNER_MEMORY_MIB`: a Job's memory, unset until measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryMib(pub u32);

#[derive(Debug, Clone, PartialEq)]
pub struct LoopSettings {
    pub seats: Seats,
    /// The runners' listener, written into every job spec.
    pub studio: Endpoint,
    pub deadline: Duration,
    pub resources: Resources,
    /// How long a dispatched attempt may wait for its runner before it fails (`STUDIO_START_TIMEOUT_SECS`).
    pub start_timeout: Duration,
    /// How long a running attempt's cancel may go unconfirmed (`STUDIO_CANCEL_GRACE_SECS`).
    pub cancel_grace: Duration,
    /// How long a running attempt whose Job is gone may wait for its ending before it is `partial`.
    pub lost_grace: Duration,
    /// How long a Job whose runner reported may take to exit by itself before it is stopped: the runner's delivery
    /// budget, which a report taken but never answered may still spend asking again, and a minute to exit.
    pub exit_grace: Duration,
    /// How long after a creation began the reconciler leaves its Job alone: a claim another instance left counts as
    /// creating until then, and a Job the invoker does not know is re-created only after it.
    pub create_grace: Duration,
    /// The waits between a failed create and the next try; the attempt fails after the last.
    pub create_retries: Vec<Duration>,
    pub dispatch_every: Duration,
    pub watch_every: Duration,
    pub reconcile_every: Duration,
    /// At most one `run.progress` per run this often.
    pub progress_every: Duration,
    /// A streaming server whose heartbeat is older than this is dead and not picked (`STUDIO_STREAMING_STALE_SECS`).
    pub streaming_stale_after: Duration,
}

impl LoopSettings {
    /// Spec 08's defaults: 5 seats, a 6 h deadline, one core, 15 min to start, 60 s of grace (a minute also covers a
    /// creation's tries, 1 s, 5 s and 25 s apart), five minutes for a reported runner to exit, and the loops waking
    /// every 5 s, 1 s and 30 s.
    pub fn new(studio: Endpoint) -> Self {
        let seconds = Duration::from_secs;
        Self {
            seats: Seats(5),
            studio,
            deadline: seconds(21_600),
            resources: Resources { cpu_millis: CpuMillis(1_000), memory_mib: None },
            start_timeout: seconds(900),
            cancel_grace: seconds(60),
            lost_grace: seconds(60),
            exit_grace: DELIVERY_BUDGET + seconds(60),
            create_grace: seconds(60),
            create_retries: vec![seconds(1), seconds(5), seconds(25)],
            dispatch_every: seconds(5),
            watch_every: seconds(1),
            reconcile_every: seconds(30),
            progress_every: seconds(5),
            streaming_stale_after: seconds(10),
        }
    }
}

/// What wakes each loop before its next tick.
#[derive(Debug, Default)]
pub struct Wakes {
    pub dispatcher: Notify,
    pub watcher: Notify,
    pub reconciler: Notify,
}

impl Wakes {
    pub fn dispatch(&self) {
        self.dispatcher.notify_one();
    }

    pub fn watch(&self) {
        self.watcher.notify_one();
    }

    pub fn reconcile(&self) {
        self.reconciler.notify_one();
    }
}
