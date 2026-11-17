//! The planner's failure taxonomy: `PlanError`, every way a scheduling
//! step or demand pass can fail — its own invariant violations plus
//! `#[from]` conversions for the subsystem errors (inventory,
//! pathfinding, reservations, registry) that surface through planning.

use derive_more::Display;
use desgraph::{CellId, StationId};

use super::ReservationError;
use crate::model::SimTime;
use crate::model::ToteId;
use crate::model::bot::plan::PlanTaskId;
use crate::model::htm::registry::RegistryError;
use crate::model::ids::RobotId;
use crate::model::inventory::error::InventoryError;
use crate::pathfinding::PathfinderError;
use crate::state::StateError;

#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error(transparent)]
    State(#[from] StateError),

    #[error("time overlap: existing exit {}ms, proposed enter {}ms", existing_exit.as_ms(), proposed_enter.as_ms())]
    TimeOverlap { existing_exit: SimTime, proposed_enter: SimTime },

    #[error("waypoints not time-ordered at index {index}")]
    InternalTimeOrder { index: usize },

    #[error("unknown task {0:?}")]
    UnknownTask(PlanTaskId),

    #[error("carrying mismatch: expected {expected:?}, actual {actual:?}")]
    CarryingMismatch { expected: Option<ToteId>, actual: Option<ToteId> },

    #[error("resource unavailable: cell {cell:?} already reserved")]
    ResourceUnavailable { cell: CellId },

    #[error("no path: {robot:?} {from:?}→{to:?} at {sim_start_ms}ms [{context}] — schedule overconstrained")]
    PathNotFound { robot: RobotId, from: CellId, to: CellId, sim_start_ms: u64, context: &'static str },

    #[error(
        "scheduling deadlock at pass {pass}: {queue_len} items stuck (fwd={vtm_forward} del={htm_deliver} sto={htm_store} ret={vtm_return})"
    )]
    SchedulingDeadlock {
        pass: usize,
        queue_len: usize,
        vtm_forward: u32,
        htm_deliver: u32,
        htm_store: u32,
        vtm_return: u32,
    },

    #[error("task graph invariant violation [{context}]: {detail}")]
    TaskGraphInvariant { context: &'static str, detail: String },

    /// A presentation's clearance cannot take its HTM off the station's cell
    /// at its accepted departure, and no task may take it off later.
    #[error("the clearance at {station:?} cannot take {robot:?} off the station's cell at {departure}: {cause}")]
    ClearanceCannotDepart { station: StationId, robot: RobotId, departure: SimTime, cause: ClearanceBlock },

    #[error(transparent)]
    Inventory(#[from] InventoryError),

    #[error(transparent)]
    Pathfinder(#[from] PathfinderError),

    #[error(transparent)]
    Reservation(#[from] ReservationError),

    #[error(transparent)]
    HtmRegistry(#[from] RegistryError),
}

/// What keeps a clearance from taking its HTM off the station's cell at its
/// accepted departure.
#[derive(Debug, Display)]
pub enum ClearanceBlock {
    /// The relay service declined the return's last attempt, for this
    /// reason, and nothing that attempt read changed before the barrier
    /// passed the release.
    #[display("no deposit relay can take its tote: {_0}")]
    NoDepositRelay(ReservationError),
    #[display("no parking bay can take it")]
    NoParkingBay,
    /// Its deposit relay is reserved, but a robot holds the relay's access
    /// cell with no departure scheduled when the barrier passes the release.
    #[display("a robot holds its deposit relay's access cell")]
    DepositAccessHeld,
    /// The barrier passed the release before the drive was committed
    /// (scheduler time), and no other robot holds the reserved relay's access
    /// cell open-ended by then: the return or its store was still awaiting a
    /// squatter's parking or the holder's departure being scheduled.
    #[display("the barrier passed the release before its drive was committed")]
    DepartureMissed,
    /// Robots hold cells on its way to the deposit relay with no departure
    /// scheduled, so its path search would await (scheduler time) their
    /// commits before its first window is committed, which a clearance's
    /// store refuses.
    #[display("robots hold its way to the deposit relay")]
    WayHeld,
}

// TODO: move ErrContext to a proper ScheduleError when we split PlanError (task #12).

pub trait ErrContext<T> {
    fn err_context(self, context: &'static str) -> Result<T, PlanError>;
}

impl<T, E: std::fmt::Display> ErrContext<T> for Result<T, E> {
    fn err_context(self, context: &'static str) -> Result<T, PlanError> {
        self.map_err(|e| PlanError::TaskGraphInvariant { context, detail: e.to_string() })
    }
}
