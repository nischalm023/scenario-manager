//! The trailing global barrier: the scheduler's settled present. `now` is
//! the frontier B — planning behind it is final, and no new claim may
//! start before it — and `wait_until` suspends work whose admission instant
//! must be in the settled past (the pick embargo is the first consumer),
//! registering that instant as an advance target. The scheduler advances
//! B at executor quiescence, never past a live candidacy's start time,
//! and refuses acceptances that start behind it — both read generically
//! off [`Proposal::start_time`]. Disabled, B pins to zero and a
//! future-instant wait refuses: an embargo requires a present.
//! Design: docs/global-barrier.md.
//!
//! [`Proposal::start_time`]: super::task::proposal::Proposal::start_time

use std::cell::{Cell, RefCell};

// ordered: the earliest registered target is the advance point.
use std::collections::BTreeMap;

use crate::model::SimTime;
use crate::model::bot::plan::PlanError;
use crate::signals::{Mut, notified};

pub struct Barrier {
    now: Mut<SimTime>,
    enabled: bool,
    targets: RefCell<BTreeMap<SimTime, u32>>,
    advances: Cell<u64>,
}

impl Barrier {
    pub fn new(enabled: bool) -> Self {
        Self { now: Mut::new(SimTime::ZERO), enabled, targets: RefCell::new(BTreeMap::new()), advances: Cell::new(0) }
    }

    /// The settled frontier B: no new admission may start before it.
    pub fn floor(&self) -> SimTime {
        self.now.value()
    }

    /// The current world time: `floor()` read as the floor for what is
    /// emitted, not for what is admitted. It is the simtime B has reached, as
    /// of this moment in scheduler time, and the earliest simtime anything
    /// emitted from now on may carry.
    pub fn current_world_time(&self) -> SimTime {
        self.now.value()
    }

    /// A shared read of the clock for domain selection floors: candidacy
    /// selections max their `after` with the settled frontier so no new
    /// admission is even proposed behind B. Writes stay scheduler-only.
    pub fn clock(&self) -> Mut<SimTime> {
        self.now.clone()
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Suspend until B >= `t`, holding `t` as an advance target meanwhile.
    /// Level-triggered: the caller's condition is re-checked on every
    /// resume, never inferred from a caught notification.
    pub async fn wait_until(&self, t: SimTime) -> Result<(), PlanError> {
        if self.now.value() >= t {
            return Ok(());
        }
        if !self.enabled {
            return Err(PlanError::TaskGraphInvariant {
                context: "Barrier",
                detail: format!(
                    "wait_until({t}) with the barrier disabled would never wake; an admission instant requires a present"
                ),
            });
        }
        let _target = self.register(t);
        loop {
            let advanced = notified(&self.now);
            if self.now.value() >= t {
                return Ok(());
            }
            advanced.await;
        }
    }

    /// The earliest registered target strictly beyond B — where the
    /// scheduler advances to at executor quiescence.
    pub fn next_target(&self) -> Option<SimTime> {
        let now = self.now.value();
        self.targets.borrow().keys().copied().find(|&t| t > now)
    }

    /// Scheduler-only: jump B at executor quiescence, waking `wait_until`
    /// waiters whose instant is now in the settled past.
    pub fn advance_to(&self, t: SimTime) {
        debug_assert!(t >= self.now.value(), "the settled frontier never regresses");
        self.advances.set(self.advances.get() + 1);
        self.now.set(t);
    }

    /// End-of-run report line; `None` when the barrier never moved.
    pub fn report(&self) -> Option<String> {
        (self.advances.get() > 0)
            .then(|| format!("barrier: advances={} final={}", self.advances.get(), self.now.value()))
    }

    fn register(&self, t: SimTime) -> TargetGuard<'_> {
        *self.targets.borrow_mut().entry(t).or_insert(0) += 1;
        TargetGuard { barrier: self, t }
    }
}

// Deregisters on drop — wake and teardown alike — so the scheduler
// never advances toward an abandoned target.
struct TargetGuard<'a> {
    barrier: &'a Barrier,
    t: SimTime,
}

impl Drop for TargetGuard<'_> {
    fn drop(&mut self) {
        let mut targets = self.barrier.targets.borrow_mut();
        if let Some(count) = targets.get_mut(&self.t) {
            *count -= 1;
            if *count == 0 {
                targets.remove(&self.t);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::cell::Cell;
    use std::rc::Rc;

    use tokio::runtime::{Builder, Runtime};
    use tokio::task::LocalSet;

    fn harness() -> (Runtime, LocalSet) {
        (Builder::new_current_thread().build().expect("test runtime"), LocalSet::new())
    }

    // Test harness drives executor rotations deliberately.
    #[allow(clippy::disallowed_methods)]
    fn settle(rt: &Runtime, local: &LocalSet) {
        local.block_on(rt, async {
            tokio::task::yield_now().await;
        });
    }

    #[test]
    fn settled_instants_pass_immediately() {
        let (rt, local) = harness();
        local.block_on(&rt, async {
            let barrier = Barrier::new(true);
            barrier.wait_until(SimTime::ZERO).await.expect("zero is already settled");
            barrier.advance_to(SimTime::ms(50));
            barrier.wait_until(SimTime::ms(50)).await.expect("a reached instant is settled");
        });
    }

    #[test]
    fn disabled_barrier_refuses_future_instants_and_passes_settled_ones() {
        let (rt, local) = harness();
        local.block_on(&rt, async {
            let barrier = Barrier::new(false);
            barrier.wait_until(SimTime::ZERO).await.expect("zero is settled even disabled");
            let err = barrier.wait_until(SimTime::ms(1)).await.expect_err("a future instant would never wake");
            assert!(matches!(err, PlanError::TaskGraphInvariant { context: "Barrier", .. }));
        });
    }

    #[test]
    fn advance_wakes_the_suspended_waiter_and_clears_its_target() {
        let (rt, local) = harness();
        let barrier = Rc::new(Barrier::new(true));
        let done = Rc::new(Cell::new(false));
        {
            let barrier = barrier.clone();
            let done = done.clone();
            local.spawn_local(async move {
                barrier.wait_until(SimTime::ms(100)).await.expect("the barrier is enabled");
                done.set(true);
            });
        }
        settle(&rt, &local);
        assert_eq!(barrier.next_target(), Some(SimTime::ms(100)), "the suspended wait registered its target");
        assert!(!done.get());

        barrier.advance_to(SimTime::ms(100));
        settle(&rt, &local);
        assert!(done.get(), "the waiter woke at its instant");
        assert_eq!(barrier.next_target(), None, "the woken wait deregistered");
    }

    #[test]
    fn next_target_is_the_earliest_beyond_b() {
        let (rt, local) = harness();
        let barrier = Rc::new(Barrier::new(true));
        for instant in [200u64, 100] {
            let barrier = barrier.clone();
            local.spawn_local(async move {
                barrier.wait_until(SimTime::ms(instant)).await.expect("the barrier is enabled");
            });
        }
        settle(&rt, &local);
        assert_eq!(barrier.next_target(), Some(SimTime::ms(100)));

        barrier.advance_to(SimTime::ms(100));
        settle(&rt, &local);
        assert_eq!(barrier.next_target(), Some(SimTime::ms(200)), "the settled target is gone, the later one remains");
    }
}
