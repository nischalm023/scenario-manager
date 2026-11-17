//! A presentation's clearance, within `task`: what clears a station's cell of
//! the HTM a finished operation leaves standing there, so that the HTM
//! departs when the operation ends (simtime) and never later, with the one
//! exception below. The next delivery to the cell waits meanwhile in the
//! station's queue, where the pathfinder ends its window. Its path into the
//! cell is scheduled once the standing HTM's departure is committed
//! (scheduler time): the contest for the HTM's stay is decided by the time
//! the barrier reaches the operation's end, and earlier when a waiting
//! entrant's need-by brings the decision forward, and the task that moves the
//! HTM next (a productive claim's, or the clearance's to a deposit relay or a
//! parking bay) then commits its path from that end. Both robots move
//! (simtime) at the reservations their committed paths hold. A staged empty
//! that nothing will fill is the exception: no departure is scheduled for its
//! HTM until a deposit relay is reserved for its tote (scheduler time), and
//! the clearance spawned with that relay commits the HTM's path from the
//! departure the relay was reserved for. The HTM stands on the station's cell
//! (simtime) until that departure, which may be after the operation's end. A
//! pick spawns a clearance (unless the station's belt boards the tote), as do
//! a putaway fill, an extraction, a collection off a belt, and a staged
//! empty that nothing will fill. In the contest for the HTM's stay, a
//! productive claim that departs by the operation's end (simtime) wins over
//! the clearance, and one that would depart later loses to it; the winning
//! clearance takes the HTM home with its tote or parks it. A direct supply
//! that wins but cannot depart at the operation's end hands the HTM back, and
//! the clearance takes it home at that end. A putaway fill is committed, and
//! a staged empty that nothing will fill is handed to its clearance, only
//! once its tote's return holds a deposit relay; that clearance carries the
//! relay and gives it back when a productive claim wins. When a clearance's
//! return can reserve no deposit relay, or no bay takes its HTM, by the time
//! the barrier passes the clearance's departure, or its drive cannot start
//! then, the run stops.

mod clear_presentation;
mod return_cleared_tote;
mod return_relay;

use crate::error::planner::ClearanceBlock;
use crate::model::htm::registry::{AvailableHtm, HtmAvailabilityId};
use crate::prelude::*;
use crate::scheduler::barrier::Barrier;
use rustc_hash::FxHashMap;

pub use clear_presentation::{ClearPresentation, ClearanceOutcome};
pub use return_relay::{Attempt, ReturnRelays};

/// Keeps a return's relay search, and a clearance's store, from parking again
/// a squatter whose parking found no bay while a retry would fail the same
/// way. Each retry would take and restore the squatter's stay, waking every
/// other search or store a squatter blocks to park its own again, without
/// end; a putaway fill's retry, decided (scheduler time) when the barrier
/// reaches that fill's release, would also bring the barrier one fill nearer
/// the stay's start each time. Each such stay is kept with the departure
/// (simtime) its failed parking was decided for, and all are forgotten when a
/// path commits. Until then a parking of the same stay can find a bay only by
/// departing at another simtime, which it does only after the barrier passes
/// that departure.
#[derive(Default)]
pub struct Unparkable {
    departures: FxHashMap<HtmAvailabilityId, SimTime>,
    oracle_seen: Option<Option<u64>>,
}

impl Unparkable {
    /// Forgets every stay recorded unless the oracle's version was
    /// `oracle_seen` when they were recorded.
    pub fn as_of(&mut self, oracle_seen: Option<u64>) {
        if self.oracle_seen != Some(oracle_seen) {
            self.departures.clear();
            self.oracle_seen = Some(oracle_seen);
        }
    }

    /// The departure (simtime) of `squatter`'s parking decided with the
    /// barrier at `barrier`: the stay's start, or the barrier once past it,
    /// as the parking's claim fixes it; `None` when a parking of this stay
    /// departing then already found no bay.
    pub fn untried_departure(&self, squatter: &AvailableHtm, barrier: SimTime) -> Option<SimTime> {
        let departure = squatter.action_ready_at.max(barrier);
        (self.departures.get(&squatter.availability_id()) != Some(&departure)).then_some(departure)
    }

    pub fn insert(&mut self, stay: HtmAvailabilityId, departure: SimTime) {
        self.departures.insert(stay, departure);
    }
}

/// Resolves once the barrier passes simtime `departure`: from then on, every
/// drive a clearance could still plan would take its HTM off the station's
/// cell later. It holds the instant one millisecond after `departure` as an
/// advance target, so the barrier moves there with no other work moving it.
pub(super) async fn barrier_passes(barrier: &Barrier, departure: SimTime) -> Result<(), PlanError> {
    barrier.wait_until(departure + SimDuration::ms(1)).await
}

/// Resolves once the barrier passes simtime `at`, as `barrier_passes` does,
/// but holding no advance target: it wakes on whatever moves the barrier and
/// moves nothing itself.
async fn barrier_passes_unheld(clock: Mut<SimTime>, at: SimTime) {
    loop {
        let advanced = notified(&clock);
        if clock.value() > at {
            return;
        }
        advanced.await;
    }
}

/// Why the barrier passed the release with a deposit relay reserved and no
/// drive committed: `holder`, the robot holding the relay's access cell
/// open-ended, if it is not the clearance's own `robot`.
pub(super) fn release_missed(holder: Option<RobotId>, robot: RobotId) -> ClearanceBlock {
    if holder.is_some_and(|holder| holder != robot) {
        ClearanceBlock::DepositAccessHeld
    } else {
        ClearanceBlock::DepartureMissed
    }
}

#[cfg(test)]
mod unparkable_tests;
