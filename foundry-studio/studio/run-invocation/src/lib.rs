//! `run-invocation` is spec 08 sections 3.4 and 3.6's three loops, which exist so that cluster time is spent only on
//! approved runs, never on more than about five at once (ruling D61), and so that every change of a run's state reaches
//! its conversation exactly once even across a restart. The dispatcher reserves seats and creates Jobs, the watcher
//! hands each recorded state change on in order, and the reconciler squares the attempts holding seats with what the
//! invoker says of their Jobs. They work over three traits, so this crate opens no table and writes no words: an
//! `Invoker` (spec 08 section 3.5: the API's local one this week, Nithin's Jobs one in November), and the `RunStore`
//! and `RunEvents` the API implements over its tables and journal (the architecture review, finding 7). Every timing
//! here is wall clock.

mod dispatcher;
mod job;
mod reconciler;
mod settings;
mod store;
mod token;
mod watcher;

use std::error::Error;
use std::sync::Arc;

use tokio::task::JoinHandle;

pub use job::{Creator, Invoker, JobName, JobRequest, JobStatus, Resources, RunnerSetting};
pub use settings::{CpuMillis, LoopSettings, MemoryMib, Seats, Wakes};
pub use store::{
    AttemptCount, AttemptKey, AttemptRow, Cursor, Ended, HeldAttempt, Reserved, Round, RunEvents, RunStore,
    RunningProgress, Transition, TransitionId, TxId,
};
pub use token::Token;

/// Whatever a store, an invoker or an events writer failed with: the loops log it and try again on their next turn.
pub type Fault = Box<dyn Error + Send + Sync>;

/// The three loops over one store, one events writer and one invoker, creating Jobs as one `Creator`. One API replica
/// runs them (spec 08 section 3.4).
pub struct Loops<S, E, I> {
    store: Arc<S>,
    events: Arc<E>,
    invoker: Arc<I>,
    settings: Arc<LoopSettings>,
    wakes: Arc<Wakes>,
    creator: Creator,
}

impl<S: RunStore, E: RunEvents, I: Invoker> Loops<S, E, I> {
    pub fn new(store: Arc<S>, events: Arc<E>, invoker: Arc<I>, settings: LoopSettings, wakes: Arc<Wakes>) -> Self {
        Self { store, events, invoker, settings: Arc::new(settings), wakes, creator: Creator::mint() }
    }

    /// Start the three loops; they run until their handles are aborted.
    pub fn spawn(self) -> Running {
        let handles = vec![
            tokio::spawn(dispatcher::run(
                self.store.clone(),
                self.invoker.clone(),
                self.settings.clone(),
                self.wakes.clone(),
                self.creator.clone(),
            )),
            tokio::spawn(watcher::run(self.store.clone(), self.events, self.settings.clone(), self.wakes.clone())),
            tokio::spawn(reconciler::run(self.store, self.invoker, self.settings, self.wakes, self.creator)),
        ];
        Running(handles)
    }
}

/// The running loops; dropping this stops them, as a test's end or a second start of the same state does.
pub struct Running(Vec<JoinHandle<()>>);

impl Drop for Running {
    fn drop(&mut self) {
        for handle in &self.0 {
            handle.abort();
        }
    }
}
