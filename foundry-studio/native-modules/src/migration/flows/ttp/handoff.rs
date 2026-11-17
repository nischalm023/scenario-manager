//! Intra-chain handoff types for the TTP task pipeline.
//!
//! Each type is the input one task's commit produces for its successor.
//! Chains are code (task-model v2 ruling 8): the committing predecessor
//! constructs the successor task WITH its handoff and spawns it — no
//! conduit sits between them.

use desgraph::{RelayId, ZoneId};

use crate::events::{ToteSource, TripEndpoint};
use crate::model::ids::{PresentationId, RackSlot, ToteId};
use crate::model::relay::RelayReservation;
use crate::model::reservation::cell_reservation::CellReservation;
use crate::model::{SimDuration, SimTime, StationArrivalPlan};
use crate::pathfinding::Orientation;

/// An HTM holding a tote and an open-ended reservation on its current
/// cell: HtmPickup → HtmDeliver (HTM at the relay), the direct-putaway
/// demand generator → HtmDeliver (HTM already carrying the empty), and
/// HtmDeliver → Presentation (HTM at the station cell).
///
/// The HTM id is `cell_reservation.robot()`; the current cell is
/// `cell_reservation.cell()`. `zone_id` is the zone of the relay,
/// propagated from VtmForward's `RelayReservation`.
pub struct HtmCarryingTote {
    pub tote_id: ToteId,
    pub zone_id: ZoneId,
    pub cell_reservation: CellReservation,
    /// When the bot is action-ready at `cell_reservation.cell()`. Under the
    /// Phase 1 contract the reservation's `window().start` is leading-edge
    /// enter time (one traverse before action-ready for path-derived holds),
    /// so downstream tasks must read this field instead.
    pub action_ready_at: SimTime,
    /// Facing at the end of the preceding path, retained across stationary work.
    pub orientation: Orientation,
    pub station_arrival_plan: Option<StationArrivalPlan>,
    /// Trip ID for the journal — minted at chain birth, constant across all
    /// legs of this presentation cycle.
    pub presentation_id: PresentationId,
    /// Relay where this tote entered the HTM leg. Preserved across delivery so
    /// the completed pick can be attributed without consulting mutable state.
    pub source_relay_id: Option<RelayId>,
    /// Declared semantic origin of the HtmDeliver leg this handoff feeds:
    /// `Some(Relay)` for a standard pickup→deliver chain; `None` for a direct
    /// putaway chain, whose producer knows only a cell, not a semantic endpoint.
    /// HtmDeliver passes this through as the leg's declared `from`; the observer
    /// prefers continuity and uses it only as a fallback (so `None` is honest —
    /// "don't know", not a fabricated `Parking`).
    pub deliver_from: Option<TripEndpoint>,
}

/// VtmForward → HtmPickup: VTM committed, tote en route to relay.
/// Carries the RAII relay reservation for HtmPickup to finalize.
pub struct RelayHandoff {
    pub reservation: RelayReservation<ToteId>,
    pub deposit_time: SimTime,
    /// HTM→relay travel estimate (per-relay; 0 for resident-HTM relays).
    /// HtmPickup subtracts this from `deposit_time` in `actionable_at` so
    /// dispatch fires early enough for the chosen HTM to JIT-depart and
    /// arrive around the deposit, instead of waiting until the tote has
    /// already landed and eating the full travel time on pickup.
    pub estimated_htm_to_relay: SimDuration,
    pub station_arrival_plan: Option<StationArrivalPlan>,
    /// Trip ID for the journal — minted at chain birth, threaded into
    /// HtmPickup's `TripStartEvent` and on into `HtmCarryingTote`.
    pub presentation_id: PresentationId,
}

/// HtmStore → VtmReturn: HTM deposited tote at relay, VTM picks it up.
pub struct ToteAtRelay {
    pub tote_id: ToteId,
    pub relay_id: RelayId,
    pub rack_slot: RackSlot,
    pub reservation: RelayReservation<RackSlot>,
    pub deposit_time: SimTime,
    /// Trip ID of the presentation cycle whose store leg deposited this tote.
    /// HtmStore holds it (`Option`, since direct-putaway store legs mint no id);
    /// VtmReturn stamps it onto the return cycle's journal events so a forward
    /// PickSupport cycle and its PutawaySupport return share one chain_id.
    pub presentation_id: Option<PresentationId>,
    /// Which flow this returning tote belongs to, derived from the origin
    /// station's `StationType` captured at the return chain's birth (putaway
    /// station ⇒ `Putaway`, else `Pick`). Threaded onto the return cycle's
    /// `VtmCycleEvent` for the pick-vs-putaway idle-gap split.
    pub tote_source: ToteSource,
}
