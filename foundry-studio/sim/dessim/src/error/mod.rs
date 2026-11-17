pub mod planner;

pub use planner::PlanError;

use desgraph::CellId;

use crate::model::{RobotId, SimDuration, SimTime};
use crate::pathfinding::PathSelectionError;

#[derive(Clone, Debug, thiserror::Error)]
pub enum InvalidScenario {
    #[error("invalid robot models: {0}")]
    RobotModels(String),
    #[error("cannot determine initial HTM chassis heading at cell {cell_id}: {reason}")]
    HtmHeading { cell_id: u32, reason: String },
    #[error("relay {relay_id} has no zone assignment")]
    RelayWithoutZone { relay_id: u32 },
    #[error("cell {cell_id} has no position of any kind")]
    CellWithoutPosition { cell_id: u32 },
    #[error("rack {rack_id} (zone {zone_id}, x_mm {x_mm}) has no matching relay column — unreachable by VTM")]
    RackWithoutSide { rack_id: u32, zone_id: u32, x_mm: i64 },
}

#[derive(Debug, thiserror::Error)]
pub enum ReservationError {
    #[error(transparent)]
    InvalidScenario(#[from] InvalidScenario),
    #[error("zone {zone_id} not found in capacity map")]
    InvalidZone { zone_id: u32 },
    #[error("relay {relay_id} not registered")]
    InvalidRelay { relay_id: u32 },
    #[error("no path from cell {from_cell} to cell {to_cell}")]
    PathUnreachable { from_cell: u32, to_cell: u32 },
    #[error("path selection failed during relay reservation: {source}")]
    PathSelection {
        #[source]
        source: PathSelectionError,
    },
    #[error(
        "zone {zone_id}: {count}/{limit} {direction} relay admissions remain unresolved — consumers not keeping up"
    )]
    HeadroomExhausted { zone_id: u32, direction: &'static str, count: u32, limit: u32 },
    #[error("no {operation} candidates available — {reason}")]
    NoCandidates { operation: &'static str, reason: &'static str },
    /// No candidate is free, and robots hold some candidates' access cells
    /// open-ended: `held` names each such cell with its holder, earliest
    /// relay first. Only a caller that passed no parking relief is told, so
    /// that it can move a holder itself and ask again. When other relays'
    /// slots open beyond the drive's access horizon, the decline keeps the
    /// earliest departure (simtime) whose horizon reaches the first of them.
    #[error("no {operation} candidates available — robots hold the access cells of {} relays", held.len())]
    AccessHeld { operation: &'static str, held: Vec<(CellId, RobotId)>, earliest_admitted_departure: Option<SimTime> },
    /// No candidate qualifies for a drive departing (simtime) at `depart`,
    /// and some relays' slots open (simtime) later than the drive may wait
    /// for an access cell, `horizon` after `depart`: the earliest at
    /// `opens_at`. A caller with no relief is told `AccessHeld` instead when,
    /// at the selection (scheduler time), robots hold a qualifying relay's
    /// access cell with no departure scheduled.
    #[error(
        "no {operation} candidates available for a departure at {depart} — the first relay to open beyond the departure's {horizon} access horizon opens at {opens_at}"
    )]
    BeyondAccessHorizon { operation: &'static str, depart: SimTime, horizon: SimDuration, opens_at: SimTime },
    #[error("pressure relief failed — {reason}")]
    PressureReliefFailed { reason: String },
    #[error("reservation consumed before acceptance")]
    NotAccepted,
}

impl ReservationError {
    /// The earliest departure (simtime) whose access horizon reaches the
    /// first relay a decline set aside beyond it.
    pub fn earliest_admitted_departure(&self) -> Option<SimTime> {
        match self {
            Self::BeyondAccessHorizon { horizon, opens_at, .. } => Some(*opens_at - *horizon),
            Self::AccessHeld { earliest_admitted_departure, .. } => *earliest_admitted_departure,
            _ => None,
        }
    }
}

pub trait ReservationErrContext<T> {
    fn err_context(self, operation: &'static str, no_candidate_reason: &'static str) -> Result<T, ReservationError>;
}

impl<T> ReservationErrContext<T> for Result<T, PathSelectionError> {
    fn err_context(self, operation: &'static str, no_candidate_reason: &'static str) -> Result<T, ReservationError> {
        match self {
            Ok(value) => Ok(value),
            Err(PathSelectionError::NoCandidate { .. }) => {
                Err(ReservationError::NoCandidates { operation, reason: no_candidate_reason })
            }
            Err(source) => Err(ReservationError::PathSelection { source }),
        }
    }
}
