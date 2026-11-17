//! Per-station dispatch diagnostics: HTM assignments, HTM arrivals, and
//! dispatch rejections.
//!
//! These events drive the desvis station inspector — "which HTMs are
//! en-route to this station, and what blocked dispatch recently?". The
//! observer folds them into per-slice
//! [`crate::observer::station_diag::StationDiagRecord`]s.

use desgraph::{CellId, StationId};
use serde::{Deserialize, Serialize};

use crate::SimTime;
use crate::model::conveyor::ConveyorId;
use crate::model::{OrderId, RobotId, StationArrivalSource, ToteId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OrderStationPhase {
    Assigned,
    PreparationStarted,
    Dispatched,
    Admitted,
    Waiting,
    Presenting,
    FinalPickCompleted,
    AreaExited,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrderStationPhaseEvent {
    pub order_id: OrderId,
    pub conveyor_id: ConveyorId,
    pub presentation_id: StationId,
    pub waiting_cell: Option<CellId>,
    pub at: SimTime,
    pub phase: OrderStationPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PresentationStream {
    Order,
    Inventory,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PresentationReadinessEvent {
    pub order_id: OrderId,
    pub pick_index: u32,
    pub conveyor_id: ConveyorId,
    pub presentation_id: StationId,
    pub stream: PresentationStream,
    pub ready_at: SimTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InventoryVisitPhase {
    Released,
    Loaded,
    Boarded,
    PickCompleted,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InventoryVisitPhaseEvent {
    pub conveyor_id: ConveyorId,
    pub order_id: OrderId,
    pub pick_index: u32,
    pub phase: InventoryVisitPhase,
    pub at: SimTime,
}

/// An HTM was bound to a station-bound chain.
///
/// Emitted at the moment the HTM's identity becomes known — at the direct
/// putaway feed commit (the HTM is taken from the idle registry there) and
/// at the `HtmPickup` commit for from-rack chains (pick and putaway), where
/// the HTM is first selected. Dispatch-commit points cannot emit this for
/// from-rack chains because no HTM has been chosen yet. A direct supply off
/// an operator's presentation cell, which may be refused, emits it once its
/// delivery commits its first window or its whole path, before its arrival,
/// and a refused one never does.
#[derive(Debug, Clone, PartialEq)]
pub struct HtmAssignmentEvent {
    pub robot_id: RobotId,
    pub station_id: StationId,
    pub tote_id: Option<ToteId>,
    pub source: StationArrivalSource,
    /// Relay HTM-access cell id the chain fetches through, or `None` for
    /// direct-putaway chains (which chain from a pick station with no relay
    /// leg). A cell id (not `RelayId`) so the desvis relay inspector — keyed
    /// by clicked cell — matches without a relay→cell table.
    pub relay_cell: Option<u32>,
    /// Planner pass number in which this assignment committed. Ties the
    /// binding back to a specific planning pass for forensics.
    /// When the HTM departs toward its first leg of the chain.
    pub dispatch_time: SimTime,
    /// Station arrival quoted at dispatch commit ([`crate::model::StationArrivalPlan`]).
    pub quoted_arrival: SimTime,
}

/// The HTM's committed path arrival at the station's presentation cell —
/// emitted where `StationArrivalDiagnostics::record` runs (`HtmDeliver`
/// commit). Closes the matching [`HtmAssignmentEvent`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HtmArrivalEvent {
    pub robot_id: RobotId,
    pub station_id: StationId,
    pub arrival: SimTime,
}

/// Why a station-attributable dispatch attempt produced no chain this pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DispatchRejectionReason {
    /// Pick WIP cap saturated forward of the embargo — station yields.
    ConwipCapSaturated,
    /// A ready order could not be started: station admission blocked at its
    /// reservation time.
    OrderAdmissionBlocked,
    /// A ready order could not be started: inventory reservation failed.
    OrderInventoryUnavailable,
    /// A top-up order started but no line of it could be dispatched this pass.
    TopupNoDispatch,
    /// Post-commit counter-race re-check exceeded the WIP cap; the relay
    /// reservation was cancelled.
    CounterRace,
    /// Read-only relay quote found no candidate tote/relay.
    RelayNoCandidates,
    /// Relay-zone deadlock headroom exhausted for the required direction.
    RelayHeadroomExhausted,
    /// Commit-time conflict on the quoted relay: the selection went stale
    /// between quote and commit (slot or access cell taken, availability
    /// moved past the quoted start).
    AccessCellConflict,
    /// A station wanted work but no admissible proposal existed this pass.
    NoProposal,
    /// The station has capacity to open another order but the ready queue held
    /// no eligible order for it — demand-starved. Distinct from
    /// `OrderAdmissionBlocked`/`OrderInventoryUnavailable`, which fire when a
    /// ready order *existed* but couldn't be started.
    NoReadyOrders,
    /// Open orders exist at the station but every line is already dispatched
    /// (in flight) — nothing left to fetch this pass; the station is waiting
    /// on inbound arrivals, not on a fresh dispatch decision.
    NoOpenLines,
}

impl DispatchRejectionReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::ConwipCapSaturated => "CONWIP cap saturated",
            Self::OrderAdmissionBlocked => "order admission blocked",
            Self::OrderInventoryUnavailable => "order inventory unavailable",
            Self::TopupNoDispatch => "top-up: no line dispatched",
            Self::CounterRace => "counter race (cap re-check)",
            Self::RelayNoCandidates => "relay: no candidates",
            Self::RelayHeadroomExhausted => "relay: headroom exhausted",
            Self::AccessCellConflict => "relay: quote went stale at commit",
            Self::NoProposal => "no admissible proposal",
            Self::NoReadyOrders => "no ready orders (demand-starved)",
            Self::NoOpenLines => "all open lines in flight",
        }
    }
}

impl std::fmt::Display for DispatchRejectionReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// A station-attributable dispatch rejection. `count > 1` when the producer
/// aggregated several same-reason rejections from one planning pass (e.g. the
/// per-order admission loop in pick top-up).
#[derive(Debug, Clone, PartialEq)]
pub struct DispatchRejectionEvent {
    pub station_id: StationId,
    pub at: SimTime,
    pub reason: DispatchRejectionReason,
    /// Rejections folded into this event (>= 1).
    pub count: u64,
    /// Compact per-variant context (e.g. zone for headroom). `None` when the
    /// reason needs none.
    pub detail: Option<String>,
}

/// Planner `Leg` kind, serializable (the model's `Leg::Parking` carries a
/// cell, so this is a flattened unit-variant mirror for the wire/profile).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanLegKind {
    Travel,
    RelayApproach,
    StationApproach,
    StationWork,
    RelayDeposit,
    Parking,
    VtmTravel,
    VtmForkOp,
    /// HTM sets a tote onto a station's fixed conveyance without moving. It
    /// has no destination waypoint, so a snapshot filtered on one never shows
    /// it; the variant exists so the mapping stays total.
    ConveyorDeposit,
    /// HTM lifts a carrier off a station's belt.
    ConveyorCollect,
}

impl PlanLegKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Travel => "travel",
            Self::RelayApproach => "relay approach",
            Self::StationApproach => "station approach",
            Self::StationWork => "station work",
            Self::RelayDeposit => "relay deposit",
            Self::Parking => "parking",
            Self::VtmTravel => "VTM travel",
            Self::VtmForkOp => "VTM fork-op",
            Self::ConveyorDeposit => "conveyor deposit",
            Self::ConveyorCollect => "conveyor collect",
        }
    }
}

/// One committed plan leg: its kind, destination cell, and the time the bot is
/// action-ready at that destination.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlanLegInfo {
    pub kind: PlanLegKind,
    pub dest_cell: u32,
    pub enter: SimTime,
}

/// A bot's full committed plan legs, emitted once per bot at planning
/// completion (the offline solver has no per-slice tick; the whole plan is
/// known at the end). desvis filters legs by cursor to show "remaining legs".
/// Capped at [`crate::events::PLAN_SNAPSHOT_MAX_LEGS`] legs.
#[derive(Debug, Clone, PartialEq)]
pub struct BotPlanSnapshotEvent {
    pub robot_id: RobotId,
    pub legs: Vec<PlanLegInfo>,
}
