//! Simulation events emitted by actors and consumed by [`crate::observer::SimObserver`].
//!
//! Display formatting lives in the `display` submodule.

mod display;
mod journal;
pub use journal::EventJournal;
mod resource;
mod station_diag;
mod vtm_idle;

pub use resource::{ResourceEvent, ResourceEventKind};
pub use station_diag::{
    BotPlanSnapshotEvent, DispatchRejectionEvent, DispatchRejectionReason, HtmArrivalEvent, HtmAssignmentEvent,
    InventoryVisitPhase, InventoryVisitPhaseEvent, OrderStationPhase, OrderStationPhaseEvent, PlanLegInfo, PlanLegKind,
    PresentationReadinessEvent, PresentationStream,
};

/// Cap on legs carried in a [`BotPlanSnapshotEvent`] — bounds the per-bot
/// payload. A committed plan rarely exceeds a handful of legs; 32 is generous.
pub const PLAN_SNAPSHOT_MAX_LEGS: usize = 32;
pub use vtm_idle::{VtmIdleReason, VtmIdleReasonEvent, VtmIdleReasonKind};

use std::sync::mpsc;

use desgraph::{CellId, RelayId, RobotType, StationId, WorldPos, ZoneId};
use serde::{Deserialize, Serialize};

use crate::model::OrderId;
use crate::model::ids::{PresentationId, SkuId, VtmCycleId};
use crate::model::location::{ToteDesignation, ToteLocation, ToteType};
use crate::model::{Destination, RackId, RobotId, StationArrivalSource, TaskId, ToteId};
use crate::observer::time_slice::ToteVisualState;
use crate::pathfinding::Waypoint;
use crate::scenario_gen::demand_pool::DistributionCenterId;
use crate::{SimDuration, SimTime};

// ---------------------------------------------------------------------------
// EventSink trait
// ---------------------------------------------------------------------------

/// Abstraction over event delivery. `mpsc::Sender` for desvis,
/// assertion callbacks for tests.
///
/// `Send + Sync` because services like `InventoryService` are shared
/// across multiple owners via `Arc` and downstream consumers
/// (`RelayInventoryIndex` etc.) carry `Send + Sync` bounds. Both `Send`
/// and `Sync` are satisfied directly by `std::sync::mpsc::Sender<T>` —
/// the stdlib reimplemented mpsc on top of a lock-free queue years ago,
/// and `Sender<T>` is `Sync` whenever `T: Send`. No wrapper needed.
pub trait EventSink: Send + Sync {
    fn send(&self, event: SimEvent);
}

/// mpsc channel adapter.
impl EventSink for mpsc::Sender<SimEvent> {
    fn send(&self, event: SimEvent) {
        let _ = mpsc::Sender::send(self, event);
    }
}

// ---------------------------------------------------------------------------
// Top-level event enum
// ---------------------------------------------------------------------------

/// A simulation event sent from actors to the metrics service via `mpsc`.
#[derive(Debug, Clone, PartialEq)]
pub enum SimEvent {
    /// Floor-traversing bot (HTM/RTP) followed a pathfinder Waypoint chain.
    Nav(NavEventData),
    /// Rail-mounted VTM moved from a rack cell to a relay's VTM-side cell.
    /// Distinct from `Nav` because VTMs don't traverse the floor grid — their
    /// motion is continuous rail movement across a single cycle, not a
    /// chain of cell-by-cell pathfinder waypoints.
    VtmNav(VtmNavEventData),
    /// HTM completed a processing cycle (picks at a station).
    CycleComplete(CycleCompleteEvent),
    /// Per-cycle VTM analytics: empty/loaded/mechanical leg split.
    VtmCycle(VtmCycleEvent),
    /// One physical DA50 pickup or drop with payload state captured at the
    /// transfer boundary.
    VtmOperation(VtmOperationEvent),
    /// Resource contention event (relay reservation, tote claim, backpressure).
    Resource(ResourceEvent),
    /// All robot positions at t=0 (sent once at sim start).
    InitialPositions(InitialPositionsEvent),
    /// An order has been fully picked.
    OrderComplete(OrderCompleteEvent),
    /// An order was dispatched to a station pipeline.
    OrderDispatch(OrderDispatchEvent),
    /// An HTM was dispatched to fetch a tote.
    HtmDispatch(HtmDispatchEvent),
    /// Snapshot of station pipeline depth.
    PipelineSnapshot(PipelineSnapshotEvent),
    /// A relay cell's visual state changed (reserved/occupied or freed).
    RelayStateChange(RelayStateChangeEvent),
    /// Initial tote positions (sent once at sim start, after scenario setup).
    ToteInitialStates(ToteInitialStatesEvent),
    /// A container came into existence during the run.
    ToteCreated(ToteCreatedEvent),
    /// Physical contents, including picked units in order containers.
    ToteContentsChanged(ToteContentsChangedEvent),
    /// The destination stripe plan the run was composed with (sent once at
    /// sim start; absent for scenarios without a bulk demand pool).
    StripePlan(StripePlanEvent),
    /// A tote changed location.
    ToteMoved(ToteMovedEvent),
    /// Timed transport between conveyor landmarks, for container playback.
    ToteTransit(ToteTransitEvent),
    /// Per-pick arrival timing error (actual - target).
    ArrivalError(ArrivalErrorEvent),
    /// HTM task watchdog timeout — pick not completed within 2*F_max.
    WatchdogTimeout(WatchdogTimeoutEvent),
    /// Robot began a purposeful journey (for distance-by-purpose attribution).
    TripStart(TripStartEvent),
    /// Tote arrived at an induction (putaway) station from external.
    /// Stub: defined but not emitted until putaway station logic is available.
    Induction(InductionEvent),
    /// Classified explanation for a specific VTM's pre-cycle idle gap.
    /// Emitted by `VtmService` at each forward/return commit.
    VtmIdleReason(VtmIdleReasonEvent),
    /// A signed change in total warehouse inventory units, bridged from
    /// `InventoryEvent::ContentsChanged`. Positive = putaway / initial
    /// seeding, negative = pick. The observer accumulates these into the
    /// `inventory_units` counter so the chart shows actual stock drain.
    InventoryDelta(InventoryDeltaEvent),
    /// A tote changed location, carrying its current contents. The observer
    /// compares the tote's aisle (relay zone) before and after: when it
    /// changes, the tote's units cross an aisle boundary — logged as per-aisle
    /// inventory flux. Bridged from `InventoryEvent::LocationChanged`. (Distinct
    /// from `ToteMoved`, which carries visual state and drives rack fill %.)
    InventoryMove(InventoryMoveEvent),
    /// One HTM fork-Get at a diving board: the pickup task lifting a tote off
    /// the relay onto its own deck.
    HtmRelayPickup(HtmRelayPickupEvent),
    /// One HTM fork-Put at a diving board: the store task setting a tote down
    /// on the relay. The per-robot counterpart of `DivingBoardSlotWait`, which
    /// is relay-attributed and carries no robot.
    HtmRelayDeposit(HtmRelayDepositEvent),
    /// One committed relay reservation was delayed because the diving-board
    /// slot was still occupied by the previous tote. Drives the empty-slot
    /// wait metric (KPI row 24) — distinct from `HtmRelayPickup`.
    DivingBoardSlotWait(DivingBoardSlotWaitEvent),
    /// An HTM was bound to a station-bound chain (station inspector).
    HtmAssignment(HtmAssignmentEvent),
    /// An HTM's committed path arrival at a station presentation cell.
    HtmArrival(HtmArrivalEvent),
    /// A station-attributable dispatch rejection (station inspector).
    DispatchRejection(DispatchRejectionEvent),
    /// An order crossed an exact logical or physical EasyLoop phase boundary.
    OrderStationPhase(OrderStationPhaseEvent),
    /// One mate for a particular order pick became ready at its fixed presentation.
    PresentationReadiness(PresentationReadinessEvent),
    InventoryVisitPhase(InventoryVisitPhaseEvent),
    /// A bot's full committed plan legs (bot inspector real leg semantics).
    BotPlanSnapshot(BotPlanSnapshotEvent),
    /// The scheduler advanced the barrier B: this is the current world time
    /// from here on. The barrier ruling forbids a fact sent after this marker
    /// to be stamped before it, so the observer takes what it has read behind
    /// it as final. Not a warehouse fact: the observer never prints, forwards,
    /// digests or counts it.
    BarrierAdvanced(SimTime),
    /// Navigation holds a handoff cell for a robot, from the start of the
    /// robot's move into it, as it commits the robot's path. Navigation emits
    /// the holds on the cells where a robot hands a tote over: the relays'
    /// access cells and the stations' presentation cells.
    CellHeld(CellHeldEvent),
    /// A robot's open hold of a handoff cell ends: its next path moves it out.
    CellReleased(CellReleasedEvent),
}

impl SimEvent {
    /// The instant this fact happened, which the barrier rule holds against B:
    /// no fact reported after an advance of B may be stamped before it
    /// (`docs/global-barrier.md`). The fields name their instants in eight
    /// ways (`time`, `at`, `sim_time_ms`, `dispatch_time`, `arrival`, `start`,
    /// `end`, `ready_at`), and this match is where that stops.
    ///
    /// The two setup announcements, the robot roster and the tote roster,
    /// describe the world before it moves, so they happen at zero. `None` for
    /// what has no instant of its own: a bot's plan snapshot, whose legs are
    /// the plan's future, belongs where B stands when it is read, and B's own
    /// advance is not a fact.
    pub fn stamp(&self) -> Option<SimTime> {
        let ms = SimTime::ms;
        Some(match self {
            Self::Nav(e) => e.time,
            // A rail leg runs on two clocks, the schedule's and the geometric
            // one, which diverge under Teleport; it begins at the earlier.
            Self::VtmNav(e) => e.kind.start().min(e.geo_start),
            Self::CycleComplete(e) => ms(e.sim_time_ms),
            Self::VtmCycle(e) => ms(e.sim_time_ms),
            Self::VtmOperation(e) => ms(e.sim_time_ms),
            Self::Resource(e) => ms(e.sim_time_ms),
            Self::InitialPositions(_) | Self::ToteInitialStates(_) => SimTime::ZERO,
            Self::OrderComplete(e) => ms(e.sim_time_ms),
            Self::OrderDispatch(e) => ms(e.sim_time_ms),
            Self::HtmDispatch(e) => ms(e.sim_time_ms),
            Self::PipelineSnapshot(e) => ms(e.sim_time_ms),
            Self::RelayStateChange(e) => e.time,
            Self::ToteCreated(e) => e.time,
            Self::ToteContentsChanged(e) => e.at,
            Self::StripePlan(e) => ms(e.sim_time_ms),
            Self::ToteMoved(e) => e.time,
            Self::ToteTransit(e) => e.end,
            Self::ArrivalError(e) => ms(e.sim_time_ms),
            Self::WatchdogTimeout(e) => ms(e.sim_time_ms),
            Self::TripStart(e) => ms(e.sim_time_ms),
            Self::Induction(e) => ms(e.sim_time_ms),
            Self::VtmIdleReason(e) => e.at,
            Self::InventoryDelta(e) => e.at,
            Self::InventoryMove(e) => e.at,
            Self::HtmRelayPickup(e) => e.start,
            Self::HtmRelayDeposit(e) => e.start,
            Self::DivingBoardSlotWait(e) => ms(e.sim_time_ms),
            Self::HtmAssignment(e) => e.dispatch_time,
            Self::HtmArrival(e) => e.arrival,
            Self::DispatchRejection(e) => e.at,
            Self::OrderStationPhase(e) => e.at,
            Self::PresentationReadiness(e) => e.ready_at,
            Self::InventoryVisitPhase(e) => e.at,
            Self::CellHeld(e) => e.from,
            Self::CellReleased(e) => e.at,
            Self::BotPlanSnapshot(_) | Self::BarrierAdvanced(_) => return None,
        })
    }
}

macro_rules! into_sim_event {
    ($event:ident, $event_type:ty) => {
        impl Into<SimEvent> for $event_type {
            fn into(self) -> SimEvent {
                SimEvent::$event(self)
            }
        }
    };
}

into_sim_event!(RelayStateChange, RelayStateChangeEvent);
into_sim_event!(ToteMoved, ToteMovedEvent);
into_sim_event!(ToteTransit, ToteTransitEvent);
into_sim_event!(ToteCreated, ToteCreatedEvent);
into_sim_event!(Nav, NavEventData);
into_sim_event!(TripStart, TripStartEvent);
into_sim_event!(HtmRelayPickup, HtmRelayPickupEvent);
into_sim_event!(HtmRelayDeposit, HtmRelayDepositEvent);
into_sim_event!(DivingBoardSlotWait, DivingBoardSlotWaitEvent);
into_sim_event!(HtmAssignment, HtmAssignmentEvent);
into_sim_event!(HtmArrival, HtmArrivalEvent);
into_sim_event!(DispatchRejection, DispatchRejectionEvent);
into_sim_event!(OrderStationPhase, OrderStationPhaseEvent);
into_sim_event!(PresentationReadiness, PresentationReadinessEvent);
into_sim_event!(InventoryVisitPhase, InventoryVisitPhaseEvent);
into_sim_event!(BotPlanSnapshot, BotPlanSnapshotEvent);
into_sim_event!(VtmOperation, VtmOperationEvent);
into_sim_event!(CellHeld, CellHeldEvent);
into_sim_event!(CellReleased, CellReleasedEvent);

// ---------------------------------------------------------------------------
// Event variant structs
// ---------------------------------------------------------------------------

/// Navigation event for floor-traversing bots (HTM/RTP) carrying the raw
/// pathfinder `Waypoint` chain.
///
/// The observer interprets each waypoint's `WaypointAction` to chunk the
/// bot's journey into per-slice `BotStep`s with explicit `Move`/`Wait`/
/// `Rotate` semantics. Earlier versions of this event carried a collapsed
/// `NavWaypoint` (action squashed to `Move`/`Wait`/`Rotate` and timing to
/// a single `time_delta`); that lost information about exact start/end
/// times of each phase, which the observer needed to insert correct
/// stationary-dwell steps when actions weren't contiguous in time.
#[derive(Debug, Clone, PartialEq)]
pub struct NavEventData {
    pub robot_id: RobotId,
    pub waypoints: Vec<Waypoint>,
    pub robot_type: RobotType,
    /// Simulation time (ms) when this journey started.
    pub time: SimTime,
    /// Travel distance in millimeters: the sum of the traversed edges'
    /// `distance_mm` (uniform grids: Move count × cell size; junction and
    /// dock edges carry their own physical lengths).
    pub distance_mm: u64,
    /// Whether the robot is carrying cargo for the duration of this journey.
    /// Used by the observer's Phase 4 kinematic rasterization to pick the
    /// loaded vs unloaded `KinematicParams` for trapezoidal profile sampling.
    /// Constant-speed runs ignore this field.
    pub carrying_cargo: bool,
}

/// Direction of a fork operation at an endpoint.
///
/// `Get` extracts a tote from the endpoint's slot (a rack shelf or a relay
/// buffer). `Put` deposits a tote into the slot. Each fork-op is
/// conceptually atomic — fork extends, transfers, retracts — so we don't
/// model extension and retraction as separate events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForkOp {
    Get,
    Put,
}

/// One end of a VTM cycle / position along the rail.
///
/// VTMs are rail-mounted; their physical state is fully captured by `pos`
/// (a `WorldPos` along the zone's rail). The variant distinguishes
/// "what's there": a rack slot, a relay buffer, or an empty aisle
/// position (initial spawn / between cycles where no slot or relay is
/// docked). The `cell` field is event/visualizer compat — desvis uses
/// it for heatmap accounting; scheduling never reads it.
///
/// `pos` is the floor-plane (2D) world position of the endpoint. The
/// rack vertical height isn't modeled here — that would require a
/// `WorldPos3d`. The current renderer is 2D, so `pos` is sufficient.
#[derive(Debug, Clone, PartialEq)]
pub enum VtmEndpoint {
    /// Initial spawn position (no docking). `cell` is the seed cell from
    /// scenario configuration; carried only for desvis heatmap.
    Aisle { cell: CellId, pos: WorldPos },
    /// Docked at a relay's VTM-side buffer.
    Relay { relay_id: RelayId, cell: CellId, pos: WorldPos },
    /// Docked at a rack slot.
    Slot { rack_id: RackId, cell: CellId, pos: WorldPos },
}

impl VtmEndpoint {
    pub fn cell(&self) -> CellId {
        match self {
            Self::Aisle { cell, .. } | Self::Relay { cell, .. } | Self::Slot { cell, .. } => *cell,
        }
    }

    pub fn pos(&self) -> WorldPos {
        match self {
            Self::Aisle { pos, .. } | Self::Relay { pos, .. } | Self::Slot { pos, .. } => *pos,
        }
    }
}

/// One phase of a VTM cycle. VTMs are rail-mounted lifts that move totes
/// between rack slots and relay positions; a typical VtmForward cycle is
/// `Alignment@slot` → `Fork(Get)@slot` → `RailMove slot→relay` →
/// `Alignment@relay` → `Fork(Put)@relay`. Because
/// The sequence starts with empty rail travel. Because the motion isn't
/// cell-by-cell pathfinder traversal, VTM events are
/// distinct from [`NavEventData`]. Each kind carries the window it occupies,
/// so a rail cycle needs nothing else to say what the lift was doing when.
#[derive(Debug, Clone, PartialEq)]
pub enum VtmNavKind {
    /// Alignment immediately before a fork insertion at an endpoint. The lift
    /// stays at `at.pos()` for the whole window. `tote_id` is populated when
    /// the DA50 is aligning to drop a tote and absent before a pickup.
    Alignment { at: VtmEndpoint, tote_id: Option<ToteId>, start: SimTime, end: SimTime },
    /// Atomic fork-op at an endpoint — fork extends, transfers `tote_id`
    /// (`Put` or `Get`), and retracts. The lift stays at `at.pos()` for
    /// the whole window.
    Fork { op: ForkOp, at: VtmEndpoint, tote_id: ToteId, start: SimTime, end: SimTime },
    /// Lift in continuous rail movement between a slot and a relay (in
    /// either direction). The renderer lerps from `from.pos()` to
    /// `to.pos()` over `[start, end]`. `tote_id` is `Some` when the VTM is
    /// carrying a tote, `None` for empty repositioning.
    RailMove {
        from: VtmEndpoint,
        to: VtmEndpoint,
        tote_id: Option<ToteId>,
        start: SimTime,
        end: SimTime,
        distance_mm: u64,
    },
}

impl VtmNavKind {
    /// Position/render start (instant clock; the scheduler's timing).
    pub fn start(&self) -> SimTime {
        match self {
            Self::Alignment { start, .. } | Self::Fork { start, .. } | Self::RailMove { start, .. } => *start,
        }
    }

    /// Position/render end (instant clock; the scheduler's timing).
    pub fn end(&self) -> SimTime {
        match self {
            Self::Alignment { end, .. } | Self::Fork { end, .. } | Self::RailMove { end, .. } => *end,
        }
    }
}

/// One phase of a VTM cycle (see [`VtmNavKind`]). A typical VtmForward cycle
/// emits six of these in succession: empty `RailMove` to the slot,
/// `Alignment@slot`, `Fork(Get)@slot`, loaded `RailMove slot→relay`,
/// `Alignment@relay`, `Fork(Put)@relay`.
#[derive(Debug, Clone, PartialEq)]
pub struct VtmNavEventData {
    pub robot_id: RobotId,
    /// The one VTM cycle this leg belongs to (unlike `presentation_id`,
    /// which spans the whole presentation chain).
    pub vtm_cycle_id: VtmCycleId,
    pub kind: VtmNavKind,
    /// Geometric window for this leg, read by the leg journal and VTM duty.
    /// Equals the kind's instant `start`/`end` outside Teleport.
    pub geo_start: SimTime,
    /// See `geo_start`.
    pub geo_end: SimTime,
    /// Trip ID grouping this leg with the rest of its presentation cycle and
    /// the correlated HTM legs. `Some` on every live VtmForward/VtmReturn emit;
    /// `Option` so test fixtures and any future id-less reposition can omit it.
    pub presentation_id: Option<PresentationId>,
    /// Whether this leg belongs to a Forward (PickSupport) or Return
    /// (PutawaySupport) cycle. Carried per-leg because [`VtmNavKind`] alone
    /// can't distinguish an empty-approach leg's direction.
    pub direction: VtmCycleDirection,
}

/// What kind of operation completed at the station.
///
/// Each variant carries the SKU + qty that physically moved, so per-event
/// row consumers (xlsx export) can render a single line without needing
/// to recover that data from elsewhere.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CycleKind {
    Pick {
        sku: SkuId,
        qty: u32,
    },
    Putaway {
        sku: SkuId,
        qty: u32,
    },
    /// An order's units are loaded into its order tote at the pick station it
    /// was worked at. `qty` is the order's unit count, for reporting only: the
    /// units were already removed from inventory by their own `Pick` cycles,
    /// and an order tote holds no modelled inventory.
    OrderLoad {
        qty: u32,
    },
    /// A full order tote is emptied at an extraction station and its contents
    /// leave the building.
    OrderExtract {
        qty: u32,
    },
}

/// The tote presentation a pick cycle completes: which tote is presented,
/// the order it serves, and the storage location it was fetched from (aisle
/// zone and diving-board relay). The pick flow already threads these
/// identities through its task chain — the order rides `PresentationOp::Pick`,
/// the source location arrives from the upstream fetch tasks — and the
/// `Presentation` task copies them onto the [`CycleCompleteEvent`] it emits,
/// so the observer writes pick facts with source attribution instead of
/// correlating separate event streams. One value = one physical presentation,
/// however many pick lines it satisfies (per-line SKU/quantity stay on the
/// pick facts).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TotePresentation {
    pub order_id: OrderId,
    pub tote_id: ToteId,
    pub source_zone_id: ZoneId,
    pub source_relay_id: RelayId,
    pub presentation_id: PresentationId,
}

/// HTM cycle completion event.
///
/// Emitted at the START of an operator dwell (`presentation_time`). The dwell
/// duration is carried alongside so consumers can derive `[start, start+dwell)`
/// for per-station busy-time accounting.
#[derive(Debug, Clone, PartialEq)]
pub struct CycleCompleteEvent {
    pub robot_id: RobotId,
    pub robot_type: RobotType,
    pub kind: CycleKind,
    pub station_id: StationId,
    /// Simulation time (ms) when the operator dwell begins.
    pub sim_time_ms: u64,
    /// Operator dwell duration (ms) at the station for this presentation.
    /// Used by the observer to accumulate per-station busy time.
    pub dwell_ms: u64,
    /// How the tote reached this station. `Some` for putaway cycles
    /// (PutawayDirect = chained from a pick station; PutawayFromRack =
    /// VTM-staged via a relay handoff). `None` for pick cycles — the
    /// pick chain has no equivalent branch.
    pub arrival_source: Option<StationArrivalSource>,
    /// Forward distribution center. `Some` for bulk-mode picks; `None` for
    /// putaways and pre-formed-order picks. Surfaces as the xlsx Picks DC column.
    pub dc_id: Option<DistributionCenterId>,
    /// Trip ID of the completed pick/putaway. `None` for a putaway whose tote
    /// carried no chain. Lets the Picks/Putaways sheets join to the journey legs.
    pub presentation_id: Option<PresentationId>,
    /// The tote presentation this cycle completes. `Some` for live pick
    /// presentations; `None` for putaway cycles and legacy/synthetic events.
    pub tote_presentation: Option<TotePresentation>,
}

/// Direction of a VTM cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VtmCycleDirection {
    /// Rack → relay (delivers tote to HTM).
    Forward,
    /// Relay → rack (stores tote back).
    Return,
}

/// Physical operation performed by a DA50. The endpoint is part of the kind
/// so impossible combinations (for example, dropping to a relay while labeled
/// as a storage pickup) cannot enter the reporting facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VtmOperationKind {
    PickupStorage,
    DropRelay,
    PickupRelay,
    DropStorage,
}

/// Tote contents observed at the instant of a physical operation.
///
/// Live simulations emit `Empty` or `NonEmpty`. `Unknown` is retained as an
/// explicit reporting state for imported or future facts whose payload was
/// not observed; it must never be folded into either known category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TotePayloadState {
    Empty,
    NonEmpty,
    Unknown,
}

impl TotePayloadState {
    pub fn from_empty(is_empty: bool) -> Self {
        if is_empty { Self::Empty } else { Self::NonEmpty }
    }
}

/// Immutable physical-operation fact emitted at the pickup/drop boundary.
/// Payload is captured here rather than reconstructed from later inventory.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VtmOperationEvent {
    /// Scheduled transfer boundary. Unlike geometric journey-leg diagnostics,
    /// throughput follows the active motion mode (including Teleport).
    pub sim_time_ms: u64,
    pub vtm_id: RobotId,
    pub vtm_cycle_id: VtmCycleId,
    pub zone_id: ZoneId,
    pub kind: VtmOperationKind,
    pub rack_id: RackId,
    pub rack_level: u16,
    pub relay_id: RelayId,
    pub tote_id: ToteId,
    pub payload: TotePayloadState,
    pub presentation_id: Option<PresentationId>,
}

/// Which warehouse flow the tote a VTM cycle moves belongs to. Lets the
/// pre-cycle idle gap be attributed to the pick vs. putaway pipeline.
///
/// - `Pick`: a pick-emptied / pick-bound tote. Forward: a `PickFromRack`
///   station-arrival plan (rack → relay → station retrieve). Return: the
///   store leg's origin station is NOT a putaway station.
/// - `Putaway`: a putaway-filled / putaway-bound tote. Forward: a
///   `PutawayFromRack` / `PutawayDirect` plan. Return: the store leg's
///   origin station is a putaway station.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToteSource {
    Pick,
    Putaway,
}

impl std::fmt::Display for ToteSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Pick => "Pick",
            Self::Putaway => "Putaway",
        })
    }
}

/// Per-cycle VTM analytics: empty positioning leg vs loaded working leg vs
/// mechanical (fork + buffer) time. Emitted at commit from `VtmForward` and
/// `VtmReturn`. The leg split is read straight off the scheduler's
/// `CycleBreakdown` — no extra computation.
///
/// Leg fields and `total_cycle_ms` are on the geometric cycle, so under
/// Teleport consecutive cycles' windows can overlap on one VTM — union by
/// `vtm_cycle_id`, don't sum. `sim_time_ms` and `pre_cycle_wait_ms` are
/// schedule-clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VtmCycleEvent {
    pub vtm_id: RobotId,
    /// Identity of this VTM cycle (1:1 with the cycle, see [`VtmCycleId`]).
    pub vtm_cycle_id: VtmCycleId,
    pub zone_id: ZoneId,
    pub direction: VtmCycleDirection,
    /// Empty positioning leg: previous endpoint → task start
    /// (current→rack for Forward, current→relay for Return).
    pub empty_leg_ms: u64,
    /// Loaded working leg (rack→relay for Forward, relay→rack for Return).
    pub loaded_leg_ms: u64,
    /// Fork time: rack-side transfer (`CycleBreakdown.rack_transfer_secs`).
    pub fork_ms: u64,
    /// Legacy relay-service time: relay alignment + relay transfer.
    pub buffer_ms: u64,
    /// Rack-side alignment before fork insertion.
    pub rack_alignment_ms: u64,
    /// Relay-side alignment before fork insertion.
    pub relay_alignment_ms: u64,
    /// Relay pickup/drop transfer time, excluding alignment.
    pub relay_transfer_ms: u64,
    /// Total cycle duration (empty + loaded + rack alignment + rack transfer
    /// + relay alignment + relay transfer). `buffer_ms` already contains the
    /// final two relay-service components and remains for compatibility.
    pub total_cycle_ms: u64,
    /// Sim time when the cycle started (`vtm_start`).
    pub sim_time_ms: u64,
    /// Relay this cycle services (delivers to / retrieves from).
    pub relay_id: RelayId,
    /// Rack slot's rack this cycle services.
    pub rack_id: RackId,
    /// Tote moved by this cycle.
    pub tote_id: ToteId,
    /// Trip ID — `Some` on every live emit (forward free, return threaded via
    /// `ToteAtRelay`); joins the cycle to its legs and the HTM side.
    pub presentation_id: Option<PresentationId>,
    /// Pre-cycle idle gap the VTM waited before this cycle (`vtm_start −
    /// available_at`); zero for back-to-back (`Continuous`) cycles.
    pub pre_cycle_wait_ms: u64,
    /// Classification of that pre-cycle gap (mirrors the `VtmIdleReason`
    /// emitted just before this cycle).
    pub wait_reason: VtmIdleReasonKind,
    /// Which flow the moved tote belongs to. Forward: read off the
    /// `station_arrival_plan.source`. Return: derived from the origin
    /// `StationType` captured at the store chain's birth. Lets the idle-gap
    /// diagnostic split pick-flow from putaway-flow VTM waits.
    pub tote_source: ToteSource,
}

/// Initial positions of all robots at simulation start.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InitialPositionsEvent {
    /// (robot_id, cell, robot_type, world_pos) for each robot.
    pub positions: Vec<(RobotId, CellId, RobotType, WorldPos)>,
    /// HTM chassis headings, clockwise from East in map coordinates.
    #[serde(default)]
    pub htm_headings_rad: Vec<(RobotId, f64)>,
}

/// Order completion event.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderCompleteEvent {
    pub order_id: OrderId,
    pub sim_time_ms: u64,
}

/// Order dispatch decision event (forensic audit).
#[derive(Debug, Clone, PartialEq)]
pub struct OrderDispatchEvent {
    pub order_id: OrderId,
    pub station_id: StationId,
    pub score: u32,
    pub candidates_count: u32,
    pub pipeline_depth_before: u32,
    pub pipeline_target: u32,
    pub trigger: DispatchTrigger,
    pub sim_time_ms: u64,
}

/// HTM dispatched to fetch a tote (forensic audit).
#[derive(Debug, Clone, PartialEq)]
pub struct HtmDispatchEvent {
    pub robot_id: RobotId,
    pub tote_id: ToteId,
    pub station_id: StationId,
    pub relay_id: RelayId,
    pub sim_time_ms: u64,
}

/// Station pipeline depth snapshot (forensic audit).
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineSnapshotEvent {
    pub station_id: StationId,
    pub scheduled_picks_count: u32,
    pub next_free_time_ms: u64,
    pub max_inflight_picks: u32,
    pub sim_time_ms: u64,
}

/// Relay cell visual state change (for playback visualization).
#[derive(Debug, Clone, PartialEq)]
pub struct RelayStateChangeEvent {
    /// Cell ID (htm_access.0) of the relay.
    pub cell_id: u32,
    /// `true` = cell became reserved/occupied; `false` = cell freed.
    pub reserved: bool,
    /// Simulation time of this state change.
    pub time: SimTime,
}

/// A container's immutable identity and first visible state at its birth time.
#[derive(Debug, Clone, PartialEq)]
pub struct ToteCreatedEvent {
    pub tote_id: ToteId,
    pub tote_type: ToteType,
    pub designation: ToteDesignation,
    pub initial_state: ToteVisualState,
    pub time: SimTime,
}

/// Physical payload after a contents mutation, separate from available stock.
#[derive(Debug, Clone, PartialEq)]
pub struct ToteContentsChangedEvent {
    pub tote_id: ToteId,
    pub units: u64,
    pub at: SimTime,
}

/// All tote positions at simulation start.
#[derive(Debug, Clone, PartialEq)]
pub struct ToteInitialStatesEvent {
    /// (tote_id, visual_state) for every tote.
    pub totes: Vec<(u32, ToteVisualState)>,
    /// The ids of every tote designated ORDER, sent once because the
    /// designation is permanent: "a permanent per-instance designation"
    /// (Vinay, 2026-08-07), so a tote cannot change pool mid-run and one
    /// statement at t=0 holds for the whole run.
    ///
    /// Ids rather than a per-tote field, because the order pool is the small
    /// side -- a row of empties against 170,000 placements -- and because a
    /// consumer that does not care pays nothing to ignore a list it never
    /// reads.
    pub order_totes: Vec<u32>,
}

/// The static destination pick-station stripe plan this run was composed
/// with (spec: specs/putaway_pick_locality/plan.md §4.1): destinations in
/// first-appearance order laid out as contiguous intervals over the
/// physically-ordered pick stations. The *planned* side of the Destinations
/// reporting; completed facts reconcile against it downstream. Serde
/// derives because the snapshot is persisted on `SummaryStats` so profiles
/// replay the plan the run actually used.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StripePlanEvent {
    /// Sim time this plan takes effect. The stripe state is a step
    /// function sampled at change events: one emission per (re)computation
    /// — exactly one, at t=0, under the static v1 map; dynamic restriping
    /// later emits again and the consumers already treat the log as a
    /// series (Daniel, 2026-07-30).
    pub sim_time_ms: u64,
    /// Pick stations in physical order along the bank.
    pub stations: Vec<StationId>,
    /// Total demand units — the length of the cumulative-unit line.
    pub total_units: u64,
    /// One entry per destination, in first-appearance order.
    pub entries: Vec<StripePlanEntry>,
}

/// One destination's stripe in a [`StripePlanEvent`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StripePlanEntry {
    pub destination: Destination,
    /// The destination's total demand units.
    pub units: u64,
    /// `(station, overlap)` per eligible station in physical order, where
    /// `overlap` is the interval overlap in exact unit·N space; a station's
    /// share of the destination is `overlap / (units · station_count)`.
    /// Allocation semantics are proportional to these shares (D13) — there
    /// is deliberately no "primary station" concept.
    pub eligible: Vec<(StationId, u64)>,
}

/// A tote changed location.
#[derive(Debug, Clone, PartialEq)]
pub struct ToteMovedEvent {
    pub tote_id: ToteId,
    pub new_state: ToteVisualState,
    pub time: SimTime,
}

/// A robot's hold of a handoff cell (`SimEvent::CellHeld`): no other robot may
/// enter the cell from `from` until the hold ends.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellHeldEvent {
    pub robot_id: RobotId,
    pub cell: CellId,
    /// When the robot starts its move into the cell, or starts its path there.
    pub from: SimTime,
    /// When the robot ends its move out, if the same path moves it out; while
    /// `None`, the hold lasts until a `SimEvent::CellReleased` ends it.
    pub until: Option<SimTime>,
}

/// The end of a robot's open hold of a handoff cell (`SimEvent::CellReleased`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellReleasedEvent {
    pub robot_id: RobotId,
    pub cell: CellId,
    /// When the robot ends its move out of the cell.
    pub at: SimTime,
}

/// A tote's completed travel between conveyor landmarks. The viewer projects
/// this interval onto the equipment drawing; it does not change custody or
/// prescribe conveyor motor speed, acceleration, or lateral-transfer controls.
///
/// The journey happens at its arrival (`end`): the reporter learns where and
/// when the tote lands only once the presentation claim is settled, so it
/// reports the journey whole, after B has passed the boarding. A viewer
/// following the run keeps the tote at `from_cell` until the slice it lands
/// in; a reader holding the record scrubs through the journey it spans.
#[derive(Debug, Clone, PartialEq)]
pub struct ToteTransitEvent {
    pub tote_id: ToteId,
    pub from_cell: CellId,
    pub to_cell: CellId,
    pub start: SimTime,
    pub end: SimTime,
}

/// A single inventory mutation. Bridged from `InventoryEvent::ContentsChanged`
/// so the metrics observer sees every `tote.add_item` / `tote.remove_item`
/// mutation (including initial seeding at T=0). Drives the warehouse
/// unit-count ledger (`delta`) behind the CurrentInventory chart.
#[derive(Debug, Clone, PartialEq)]
pub struct InventoryDeltaEvent {
    /// Signed change in unit count for this SKU. Positive = added, negative
    /// = removed.
    pub delta: i32,
    /// The SKU whose quantity changed.
    pub sku: SkuId,
    /// The tote whose contents changed.
    pub tote_id: ToteId,
    /// Simulation time of the underlying inventory mutation.
    pub at: SimTime,
}

/// A tote relocation with the contents it is carrying at the move instant.
/// Bridged from `InventoryEvent::LocationChanged`; an empty tote carries no
/// flux, so its `contents` is left empty (and the bridge skips the contents
/// read-lock). The observer tracks each tote's last settled aisle (relay zone)
/// and logs inventory flux only when a tote settles in a different aisle than
/// it was last in — transit hops and same-aisle round trips produce nothing
/// (see `handle_inventory_move`).
///
/// Distinct from [`ToteMovedEvent`]: that one carries a `ToteVisualState` for
/// render/fill tracking; this one carries physical contents for the per-aisle
/// inventory-flux log.
#[derive(Debug, Clone, PartialEq)]
pub struct InventoryMoveEvent {
    /// The tote that moved.
    pub tote_id: ToteId,
    /// Location before the move.
    pub from: ToteLocation,
    /// Location after the move.
    pub to: ToteLocation,
    /// SKU units the tote is carrying at `at` — the value of the crossing.
    pub contents: Vec<(SkuId, u32)>,
    /// Simulation time of the move.
    pub at: SimTime,
}

/// Per-pick arrival error: actual HTM arrival minus planned target_arrival.
/// Positive = arrived late, negative = arrived early.
#[derive(Debug, Clone, PartialEq)]
pub struct ArrivalErrorEvent {
    pub order_id: OrderId,
    pub station_id: StationId,
    /// target_arrival from ScheduledPick (ms).
    pub target_arrival_ms: u64,
    /// Actual HTM arrival time (ms).
    pub actual_arrival_ms: u64,
    /// Signed error in milliseconds (actual - target). Positive = late.
    pub error_ms: i64,
    pub sim_time_ms: u64,
}

/// HTM task watchdog sentinel — a dispatched pick was not completed within the
/// timeout (2*(F_max + pipeline_depth*P)). The simulation is stopped gracefully
/// via the stop flag. No pipeline state is mutated (the HTM is autonomous and
/// cannot be cancelled). This event captures the diagnostic context for analysis.
#[derive(Debug, Clone, PartialEq)]
pub struct WatchdogTimeoutEvent {
    pub task_id: TaskId,
    pub order_id: OrderId,
    pub station_id: StationId,
    pub timeout_secs: f64,
    pub sim_time_ms: u64,
}

/// Robot began a purposeful journey: the departure of one leg of a task.
/// Observer uses this to attribute subsequent Nav distance to the trip purpose,
/// to build the per-leg journal row (the "HTM Trips" sheet), and to open the
/// leg's interval.
#[derive(Debug, Clone, PartialEq)]
pub struct TripStartEvent {
    pub robot_id: RobotId,
    pub robot_type: RobotType,
    pub purpose: TripPurpose,
    /// Trip ID grouping all legs of one presentation cycle. `Some` for cycle
    /// legs (pickup/deliver/store); `None` for parking relocations, which are
    /// not part of any cycle.
    pub presentation_id: Option<PresentationId>,
    /// Declared semantic origin of this leg, or `None` when the producer does
    /// not know it (e.g. a direct-putaway chain, where the bot's origin is only
    /// a cell). The observer prefers leg-to-leg continuity (`last_trip_to`) and
    /// uses this only as the fallback — so `None` means "rely on continuity",
    /// never a fabricated origin.
    pub from: Option<TripEndpoint>,
    /// Semantic destination of this leg.
    pub to: TripEndpoint,
    pub sim_time_ms: u64,
}

/// The purpose of a robot's journey, for distance-by-purpose metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TripPurpose {
    /// HTM traveling to pickup relay.
    ToRelay,
    /// HTM traveling (loaded) to station.
    ToStation,
    /// HTM traveling to return relay after processing.
    ToReturnRelay,
    /// HTM traveling to a parking cell.
    ToParking,
}

/// Semantic endpoint of a journey leg. The station *kind* (Pick vs Putaway) is
/// resolved downstream at the observer via `config.station_types` — it is
/// deliberately NOT carried here, so the event stays kind-agnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TripEndpoint {
    /// A parking cell. Also used as the idle-origin endpoint for a pickup leg
    /// whose bot departed from a parking/idle floor cell (the common case).
    Parking,
    Relay(RelayId),
    Station(StationId),
}

/// Tote arrived at an induction (putaway) station.
/// Stub: not emitted until putaway station logic is available on feat/putaway.
#[derive(Debug, Clone, PartialEq)]
pub struct InductionEvent {
    pub station_id: StationId,
    pub tote_id: ToteId,
    pub case_count: u32,
    pub sim_time_ms: u64,
}

/// The pickup task's fork-Get at a diving board: the HTM lifts a tote off the
/// relay's buffer onto its own deck. One event per committed `HtmPickup`,
/// always — an operation happened whether or not the bot waited for it.
///
/// The operation IS the fact; the dwell before it is one of its fields. That
/// dwell is the HTM standing on the relay's access cell until the tote is
/// there to lift, and it is zero when the bot arrived on time.
#[derive(Debug, Clone, PartialEq)]
pub struct HtmRelayPickupEvent {
    pub robot_id: RobotId,
    pub relay_id: RelayId,
    /// Fork-Get start — the instant the fork begins lifting the tote.
    pub start: SimTime,
    /// Fork-Get end — the instant the bot is action-ready with the tote
    /// aboard, which is when the tote passes into the bot's custody.
    /// `end - start` is the bot model's pickup time.
    pub end: SimTime,
    /// Arrival-to-fork-Get dwell on the relay access cell; zero when the bot
    /// arrived just in time.
    pub wait: SimDuration,
    /// The HTM was already standing on this relay's access cell when the
    /// chain chose it, so it never drove here and the whole wait is the VTM
    /// cycle's lead time elapsing under it. False means it drove to meet a
    /// quoted deposit instant and arrived before the tote — the two carry
    /// opposite meanings and are reported apart.
    pub resident: bool,
}

/// The store task's fork-Put at a diving board: the HTM sets the tote it
/// carried back down on the relay's buffer. One event per committed
/// `HtmStore`, always.
///
/// `wait` is the ONLY per-robot fact for a return slot wait, and it is a
/// residual, not a plan input: `HtmStore` departs the station at its
/// presentation action-ready time regardless of the slot, so the schedule
/// never holds the D30 at its origin — the wait always lands at the relay
/// (issue #272's open question, resolved 2026-07-30). That relay-cell
/// placement is the model's current simplification; physically gating cell
/// entry awaits an arrive-after path constraint (#153). Its
/// `DivingBoardSlotWaitEvent` sibling measures a strictly larger, relay-
/// attributed quantity that still contains the HTM's whole travel time.
#[derive(Debug, Clone, PartialEq)]
pub struct HtmRelayDepositEvent {
    pub robot_id: RobotId,
    pub relay_id: RelayId,
    /// Fork-Put start — the instant the fork begins setting the tote down.
    pub start: SimTime,
    /// Fork-Put end — the instant the bot is action-ready and empty, which is
    /// when the tote passes into the relay's custody. `end - start` is the
    /// bot model's deposit time.
    pub end: SimTime,
    /// Arrival-to-fork-Put dwell on the relay access cell; zero when the slot
    /// was valid on arrival.
    pub wait: SimDuration,
}

/// A committed relay reservation whose start was delayed by `wait_ms`
/// specifically because the diving-board slot was still occupied by the
/// previous tote. `wait_ms` is the slot's contribution to the reservation
/// start beyond every non-slot constraint (HTM arrival, tote frontier,
/// VTM availability, station admission) — zero when the slot was not the
/// binding constraint. One event per committed reservation. Distinct from
/// `HtmRelayPickupEvent` (the HTM's own fork-Get and its dwell).
#[derive(Debug, Clone, PartialEq)]
pub struct DivingBoardSlotWaitEvent {
    pub relay_id: RelayId,
    pub wait_ms: u64,
    /// Sim time of the reservation start (when the slot wait ends).
    pub sim_time_ms: u64,
}

/// What triggered a dispatch decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchTrigger {
    /// Initial pipeline fill at simulation start.
    Bootstrap,
    /// T1: HTM departed station after completing picks.
    HtmDeparture,
    /// T4: An HTM became idle and available for work.
    HtmIdle,
    /// T5: Periodic safety-net timer fired.
    Periodic,
}
