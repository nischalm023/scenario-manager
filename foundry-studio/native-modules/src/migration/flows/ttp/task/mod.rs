//! The TTP flow's concrete tasks — the legs of tote delivery. A parent
//! task authors its chain (PickItem, ReturnTote); each leg decides one
//! thing, schedules the resources for it, and hands its successor a
//! handoff: the open-ended reservations it created, which the successor
//! bounds when it commits its own departure.
//!
//! Cell holds and relay slot holds have different lifetimes even though a
//! relay has a cell: a bot can release the cell (drive away) while the
//! relay slot stays held (tote still on it), and a bot can hold the cell
//! open-ended past its relay op until its next leg commits. VTMs commit no
//! cell holds — VtmForward and VtmReturn touch only relay slot holds.

pub mod clearance;
pub mod htm_deliver;
pub mod htm_pickup;
pub mod htm_store;
pub mod pick;
pub mod presentation;
pub mod return_tote;
pub mod vtm_forward;
pub mod vtm_return;

pub use htm_deliver::*;
pub use htm_pickup::*;
pub use htm_store::*;
pub use pick::*;
pub use presentation::*;
pub use return_tote::*;
pub use vtm_forward::*;
pub use vtm_return::*;

// ---------------------------------------------------------------------------
// Nav event helpers (shared by all tasks that emit Nav events)
// ---------------------------------------------------------------------------

pub(crate) use crate::flows::transport::{commit_plan_requests, move_distance};
use crate::model::{SimDuration, SimTime};

/// Seven leg boundaries from `start` over six phase durations (secs), in
/// the caller's cycle-phase order.
fn vtm_leg_boundaries(start: SimTime, phase_secs: [f64; 6]) -> [SimTime; 7] {
    let mut out = [start; 7];
    for (i, secs) in phase_secs.iter().enumerate() {
        out[i + 1] = out[i] + SimDuration::from_secs(*secs);
    }
    out
}
