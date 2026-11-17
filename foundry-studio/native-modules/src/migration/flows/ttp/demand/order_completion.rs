//! Per-order physical-pick completion tracking.
//!
//! The orders-completed KPI counts an order as done at the instant its
//! *last physical pick* lands at the presentation cell — not when its last
//! line is dispatched (which precedes the pick by the full retrieve-chain
//! latency). `PickDemand` registers each order's total pick count when
//! it starts picking; the `Presentation` task calls [`record_pick`] as each
//! pick fires. The call that drives the remaining count to zero reports the
//! order's completion time, and the planner emits `SimEvent::OrderComplete`.
//!
//! When an order is registered for close-out, its completion is QUEUED here as a
//! [`ClosedOrder`]: completion is discovered inside a `Presentation` task,
//! which cannot dispatch the tote trip itself, and `record_pick` destroys the
//! order's progress entry at the same instant — without the queue the fact
//! would be gone before `OrderCloseOut` could learn of it. The [`version`]
//! signal bumps on every close so the close-out actor wakes instead of polls.
//!
//! [`record_pick`]: OrderCompletionTracker::record_pick
//! [`version`]: OrderCompletionTracker::version

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use desgraph::StationId;
use rustc_hash::FxHashMap;

use crate::composition::resources::inputs;
use crate::model::SimTime;
use crate::model::ids::OrderId;
use crate::signals::Version;
use crate::state::{StateError, StateManager, States};

/// An order whose last physical pick has landed, waiting for its tote to be
/// taken to extraction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClosedOrder {
    pub order_id: OrderId,
    /// The station that picked it; where its order tote is loaded.
    pub station_id: StationId,
    pub units: u32,
    pub completed_at: SimTime,
}

struct OrderProgress {
    station_id: StationId,
    units: u32,
    remaining_picks: u32,
    close_out: bool,
    /// Latest pick sim-time (ms) seen for this order. The planner commits
    /// tasks during pre-planning and need not finalize an order's picks in
    /// sim-time order, so the pick that zeroes `remaining_picks` is not
    /// necessarily the chronologically-last one. Tracking the max here means
    /// completion is attributed to the true last pick regardless of commit
    /// order — by the time the count hits zero, every pick has been recorded.
    last_pick_ms: u64,
}

/// Shared remaining-pick counts per in-flight order. Cloning shares the
/// underlying map, so the demand module and the pick presentations it creates
/// observe the same state.
#[derive(Clone, Default)]
pub struct OrderCompletionTracker {
    progress: Arc<Mutex<FxHashMap<OrderId, OrderProgress>>>,
    // ordered: FIFO by completion, so close-out serves oldest first.
    closed: Arc<Mutex<VecDeque<ClosedOrder>>>,
    /// Bumped when an order closes (and lands on the queue).
    version: Version,
}

impl OrderCompletionTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an order's total physical-pick count at order start. Called
    /// once per order (`start_picking` is single-call — see `OrderReadyQueue`
    /// Gotcha 4). A zero-pick order is never registered, so it can't
    /// spuriously complete.
    ///
    /// `Err(order_id)` if the order is already registered: a single-call
    /// invariant violation that would silently reset the count mid-flight and
    /// leave the order unable to complete. Surfaced, not swallowed.
    pub fn register(&self, order_id: OrderId, station_id: StationId, total_picks: u32) -> Result<(), OrderId> {
        self.register_order(order_id, station_id, total_picks, false)
    }

    /// Record that this order needs a close-out trip after picking. The intent
    /// travels with the order; the shared tracker has no workflow-wide mode.
    pub fn register_for_close_out(
        &self,
        order_id: OrderId,
        station_id: StationId,
        total_picks: u32,
    ) -> Result<(), OrderId> {
        self.register_order(order_id, station_id, total_picks, true)
    }

    fn register_order(
        &self,
        order_id: OrderId,
        station_id: StationId,
        total_picks: u32,
        close_out: bool,
    ) -> Result<(), OrderId> {
        if total_picks == 0 {
            return Ok(());
        }
        let mut map = self.progress.lock().unwrap();
        if map.contains_key(&order_id) {
            return Err(order_id);
        }
        map.insert(
            order_id,
            OrderProgress { station_id, units: total_picks, remaining_picks: total_picks, close_out, last_pick_ms: 0 },
        );
        Ok(())
    }

    /// Check a planned transfer before mutating tote contents or bot plans.
    pub fn validate_pick(&self, order_id: OrderId, qty: u32) -> Result<(), OrderId> {
        let map = self.progress.lock().unwrap();
        match map.get(&order_id) {
            Some(progress) if qty > 0 && qty <= progress.remaining_picks => Ok(()),
            _ => Err(order_id),
        }
    }

    /// Record `qty` physical picks for `order_id`, landing at `at_ms`.
    ///
    /// Returns `Ok(Some(completion_ms))` on the pick that drives the order's
    /// remaining count to zero — `completion_ms` is the latest pick time seen
    /// for the order (see [`OrderProgress::last_pick_ms`]). Returns `Ok(None)`
    /// while picks remain.
    ///
    /// `Err(order_id)` if the order was never registered, or is decremented
    /// past zero: a planner invariant violation (every pick belongs to an
    /// order that reached `start_picking`, and total picks equal the
    /// registered count). Surfaced rather than returning a believable-looking
    /// "not complete" that would drop the completion.
    pub fn record_pick(&self, order_id: OrderId, qty: u32, at_ms: u64) -> Result<Option<u64>, OrderId> {
        let mut map = self.progress.lock().unwrap();
        let progress = map.get_mut(&order_id).ok_or(order_id)?;
        progress.remaining_picks = progress.remaining_picks.checked_sub(qty).ok_or(order_id)?;
        progress.last_pick_ms = progress.last_pick_ms.max(at_ms);
        if progress.remaining_picks == 0 {
            let completion_ms = progress.last_pick_ms;
            let closed = ClosedOrder {
                order_id,
                station_id: progress.station_id,
                units: progress.units,
                completed_at: SimTime::ms(completion_ms),
            };
            let needs_close_out = progress.close_out;
            map.remove(&order_id);
            drop(map);
            if needs_close_out {
                self.closed.lock().unwrap().push_back(closed);
                self.version.bump();
            }
            Ok(Some(completion_ms))
        } else {
            Ok(None)
        }
    }

    /// Take the next order awaiting close-out, oldest first.
    pub fn take_closed(&self) -> Option<ClosedOrder> {
        self.closed.lock().unwrap().pop_front()
    }

    /// Orders awaiting close-out. A run is not done while this is non-zero.
    pub fn closed_pending(&self) -> usize {
        self.closed.lock().unwrap().len()
    }

    /// Bumped when an order closes; what `OrderCloseOut` waits on.
    pub fn version(&self) -> &Version {
        &self.version
    }
}

impl StateManager for OrderCompletionTracker {
    fn create(states: &States) -> Result<Self, StateError> {
        inputs(states)?;
        Ok(Self::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oid(n: u32) -> OrderId {
        OrderId(n)
    }

    #[test]
    fn completion_reports_latest_pick_time_not_commit_order() {
        let tracker = OrderCompletionTracker::new();
        tracker.register(oid(1), StationId(1), 3).unwrap();
        assert_eq!(tracker.record_pick(oid(1), 1, 100).unwrap(), None);
        assert_eq!(tracker.record_pick(oid(1), 1, 300).unwrap(), None);
        // The pick that zeroes the count was committed with an earlier
        // sim-time (200) than a prior pick (300); completion is the max (300).
        assert_eq!(tracker.record_pick(oid(1), 1, 200).unwrap(), Some(300));
    }

    #[test]
    fn unregistered_order_is_an_error() {
        let tracker = OrderCompletionTracker::new();
        assert_eq!(tracker.record_pick(oid(99), 1, 0), Err(oid(99)));
    }

    #[test]
    fn double_register_is_an_error() {
        let tracker = OrderCompletionTracker::new();
        tracker.register(oid(1), StationId(1), 2).unwrap();
        assert_eq!(tracker.register(oid(1), StationId(1), 2), Err(oid(1)));
    }

    /// The completion fact must outlive the progress entry that produced it.
    #[test]
    fn completion_lands_on_the_close_out_queue() {
        let tracker = OrderCompletionTracker::new();
        tracker.register_for_close_out(oid(7), StationId(3), 2).unwrap();
        assert_eq!(tracker.closed_pending(), 0);

        assert_eq!(tracker.record_pick(oid(7), 1, 100), Ok(None));
        assert_eq!(tracker.closed_pending(), 0, "not closed while picks remain");

        assert_eq!(tracker.record_pick(oid(7), 1, 250), Ok(Some(250)));
        assert_eq!(tracker.closed_pending(), 1);

        let closed = tracker.take_closed().expect("queued");
        assert_eq!(closed.order_id, oid(7));
        assert_eq!(closed.station_id, StationId(3));
        assert_eq!(closed.units, 2, "the registered pick count, not the last qty");
        assert_eq!(closed.completed_at, SimTime::ms(250));
        assert_eq!(tracker.closed_pending(), 0);
    }

    /// Pick-only mode: the order completes but nothing queues for close-out.
    #[test]
    fn unretained_completion_is_dropped_not_queued() {
        let tracker = OrderCompletionTracker::new();
        tracker.register(oid(7), StationId(3), 1).unwrap();
        assert_eq!(tracker.record_pick(oid(7), 1, 100), Ok(Some(100)));
        assert_eq!(tracker.closed_pending(), 0);
        assert!(tracker.take_closed().is_none());
    }
    #[test]
    fn one_tracker_can_hold_orders_with_different_follow_up_work() {
        let tracker = OrderCompletionTracker::new();
        tracker.register(oid(1), StationId(1), 1).unwrap();
        tracker.register_for_close_out(oid(2), StationId(1), 1).unwrap();
        assert_eq!(tracker.record_pick(oid(1), 1, 100), Ok(Some(100)));
        assert_eq!(tracker.record_pick(oid(2), 1, 200), Ok(Some(200)));
        assert_eq!(tracker.closed_pending(), 1);
        // A consumer created later still sees the registered follow-up work.
        let late_consumer = tracker.clone();
        assert_eq!(late_consumer.take_closed().unwrap().order_id, oid(2));
        assert!(tracker.take_closed().is_none());
    }
}
