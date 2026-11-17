//! dessim's reactive vocabulary over ankurah-signals (0.9.3, the
//! `deterministic` feature on: listeners fire in subscription order).
//! Waits are level-triggered ONLY — a waiter re-checks its condition
//! over state on every resume and never relies on catching a
//! notification. State-carrying signals are the destination: watched
//! containers become `Mut<T>` whose `update` notifies intrinsically,
//! and waiters re-check after `notified(signal)` — which subscribes at
//! CREATION, so create it before reading what you check. [`Version`] is
//! INTERIM SCAFFOLDING for families not yet converted: a change counter
//! bumped by that family's mutators, deleted as each container
//! conversion lands.
//!
//! Every wait in the crate funnels through [`notified`], and that is all
//! it is: the crate's one wait idiom. Deciding that the run has settled
//! is not its job — the executor reports its own drained run queue to the
//! scheduler (see docs/global-barrier.md).

use std::future::Future;
use std::sync::Arc;

pub use ankurah_signals::{Mut, Signal};
use tokio::sync::mpsc;

pub fn notified(signal: &dyn Signal) -> impl Future<Output = ()> + 'static {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let guard = signal.listen(Arc::new(move |_| {
        let _ = tx.send(());
    }));
    async move {
        let _guard = guard;
        let _ = rx.recv().await;
    }
}

#[derive(Clone)]
pub struct Version(Mut<u64>);

impl Default for Version {
    fn default() -> Self {
        Self::new()
    }
}

impl Version {
    pub fn new() -> Self {
        Self(Mut::new(0))
    }

    pub fn bump(&self) {
        self.0.set(self.0.value() + 1);
    }

    pub fn seen(&self) -> u64 {
        self.0.value()
    }

    pub async fn past(&self, seen: u64) {
        loop {
            let changed = notified(&self.0);
            if self.0.value() > seen {
                return;
            }
            changed.await;
        }
    }

    pub fn signal(&self) -> &dyn Signal {
        &self.0
    }
}
