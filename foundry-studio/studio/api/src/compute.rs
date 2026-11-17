//! Within `studio-api`, `compute` is spec 01 section 3.15's compute pool. It exists so that CPU-bound computations
//! never starve the threads that serve routes, streams and turns in the one replica: each runs on a thread of the
//! pool's own, waiting for one in a single first-come queue, and a computation that finds the queue full is refused at
//! once, `compute-busy` with a wall-clock wait, rather than queued without bound. The flagged export's check (spec 04
//! section 3.3) is its first user. Identical computations run once because each owner's cache waits on the first
//! request for a key (the client's export cache does), so the pool runs what it is handed.

use std::io;
use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::str::FromStr;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::Builder;
use std::time::Duration;

use wps_client::{Compute, ComputeBusy};

/// How long a refused computation's caller waits before asking again, in wall clock: `Retry-After` and
/// `retry_after_s` on `compute-busy`.
pub const RETRY_AFTER: Duration = Duration::from_secs(2);

/// `STUDIO_COMPUTE_WORKERS`: the pool's own threads, apart from those serving routes, streams and turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Workers(NonZeroUsize);

/// `STUDIO_COMPUTE_QUEUE`: how many computations may wait for a thread before the next is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueDepth(usize);

/// The pool's size: spec 01 section 3.1's defaults are 2 threads and 64 waiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComputeBudget {
    pub workers: Workers,
    pub queue: QueueDepth,
}

impl Default for ComputeBudget {
    fn default() -> Self {
        Self { workers: Workers(NonZeroUsize::new(2).expect("two")), queue: QueueDepth(64) }
    }
}

impl Workers {
    pub fn get(self) -> usize {
        self.0.get()
    }
}

impl QueueDepth {
    pub fn get(self) -> usize {
        self.0
    }
}

impl FromStr for Workers {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        text.parse().map(Self).map_err(|_| "a whole number of threads, 1 or more".to_owned())
    }
}

impl FromStr for QueueDepth {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        text.parse().map(Self).map_err(|_| "a whole number of computations, 0 or more".to_owned())
    }
}

type Job = Box<dyn FnOnce() + Send>;

/// The pool: its queue, which its threads empty, each leaving once the pool is dropped.
pub struct ComputePool {
    queue: SyncSender<Job>,
}

impl ComputePool {
    pub fn new(budget: ComputeBudget) -> io::Result<Self> {
        let (queue, jobs) = sync_channel::<Job>(budget.queue.get());
        let jobs = Arc::new(Mutex::new(jobs));
        for n in 0..budget.workers.get() {
            let jobs = jobs.clone();
            Builder::new().name(format!("studio-compute-{n}")).spawn(move || work(&jobs))?;
        }
        Ok(Self { queue })
    }
}

/// One thread's life: the next computation, run, until the pool is gone. One idle thread at a time waits on the queue
/// and the rest on its lock, so with a queue of 0 a computation is taken only by a thread free to run it.
fn work(jobs: &Mutex<Receiver<Job>>) {
    loop {
        let Ok(Ok(job)) = jobs.lock().map(|jobs| jobs.recv()) else { return };
        // A computation that panics drops its answer unsent, which fails its caller, and the thread lives on.
        let _ = catch_unwind(AssertUnwindSafe(job));
    }
}

impl Compute for ComputePool {
    fn submit(&self, job: Job) -> Result<(), ComputeBusy> {
        match self.queue.try_send(job) {
            Ok(()) => Ok(()),
            // `Disconnected` would need every thread gone, and a thread leaves only once the pool is dropped.
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => Err(ComputeBusy { retry_after: RETRY_AFTER }),
        }
    }
}
