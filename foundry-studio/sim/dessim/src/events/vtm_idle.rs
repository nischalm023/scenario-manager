//! VTM idle-reason events — emitted at each cycle commit to explain why a
//! specific VTM's pre-cycle idle gap exists.
//!
//! The dispatcher (`VtmService::select_for_forward` / `select_for_return`)
//! decides `vtm_start = max(available_at, relay_start − cycle_duration)`.
//! When `vtm_start > available_at` the VTM was free early and had to wait;
//! the reason names the constraint that pinned the work later than the VTM
//! was free, so the gap is attributable to its real upstream cause:
//!
//! - `Continuous` — `vtm_start == available_at`. The VTM rolled straight
//!   into this cycle from the previous one; no idle gap.
//! - Forward cycle, by the binding floor of the relay reservation's
//!   `start = max(relay_slot_avail, after, tote_available_at)`:
//!   `AwaitingDemand` (`after` bound — work not due yet), `AwaitingTote`
//!   (tote not back in rack yet), `AwaitingSlot` (relay slot still held).
//! - `AwaitingHtmDeposit` — return cycle. The VTM was free before the HTM
//!   finished depositing the tote into the relay buffer; `gap` is measured
//!   against that deposit time.
//!
//! Lives as a top-level [`crate::events::SimEvent`] variant (not under
//! [`crate::events::ResourceEvent`]) because it carries `robot_id` and is
//! filterable / traceable per-VTM — the same pattern as `Nav`, `VtmNav`,
//! and the other per-robot events. `ResourceEvent` is
//! reserved for aggregate counters and cumulative sums that don't need
//! per-robot association.
//!
//! Known taxonomy gaps (deliberately not classified in this iteration):
//!
//! - Per-candidate "lost the race" emissions are not produced (only the
//!   winning VTM gets an event).
//! - Out-of-zone VTMs are filtered upstream and never considered.
//! - Trailing idle after a VTM's final cycle is computable post-hoc from
//!   `MetricsOutput::robot_last_seen` vs `sim_elapsed_ms`.

use serde::{Deserialize, Serialize};

use desgraph::RelayId;

use crate::events::VtmCycleDirection;
use crate::model::ids::{PresentationId, VtmCycleId};
use crate::model::{RobotId, SimDuration, SimTime};

/// Classification of a VTM's pre-cycle idle gap.
///
/// `Continuous` means no gap. The other variants record *why* the cycle
/// could not begin at `available_at` — the binding constraint that pinned
/// the work later than the VTM was free. Forward cycles attribute to the
/// floor of the relay reservation's `start`; return cycles attribute to
/// the HTM's deposit timing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VtmIdleReason {
    /// `vtm_start == available_at`. No idle gap; back-to-back cycles.
    Continuous,
    /// Forward cycle. `relay_start` was pinned by the station demand
    /// signal (`after`) — the work is not due yet. The dominant "VTMs
    /// faster than demand" case; the lever is upstream flow pacing.
    AwaitingDemand { gap: SimDuration, relay_start: SimTime },
    /// Forward cycle. `relay_start` was pinned by tote availability — the
    /// tote is not yet physically back in the rack.
    AwaitingTote { gap: SimDuration, relay_start: SimTime },
    /// Forward cycle. `relay_start` was pinned by relay-slot occupancy — a
    /// prior reservation still held the slot.
    AwaitingSlot { gap: SimDuration, relay_start: SimTime },
    /// Return cycle. The VTM was free before the HTM finished depositing
    /// the tote into the relay buffer; `gap` is measured against that
    /// `deposit_time`. The lever is HTM store-flow / station egress.
    AwaitingHtmDeposit { gap: SimDuration, deposit_time: SimTime },
}

/// Variant-only discriminant for aggregation. Observer counters key by
/// kind without exposing inner gap/time fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VtmIdleReasonKind {
    Continuous,
    AwaitingDemand,
    AwaitingTote,
    AwaitingSlot,
    AwaitingHtmDeposit,
}

impl VtmIdleReason {
    pub fn kind(&self) -> VtmIdleReasonKind {
        match self {
            Self::Continuous => VtmIdleReasonKind::Continuous,
            Self::AwaitingDemand { .. } => VtmIdleReasonKind::AwaitingDemand,
            Self::AwaitingTote { .. } => VtmIdleReasonKind::AwaitingTote,
            Self::AwaitingSlot { .. } => VtmIdleReasonKind::AwaitingSlot,
            Self::AwaitingHtmDeposit { .. } => VtmIdleReasonKind::AwaitingHtmDeposit,
        }
    }

    /// The pre-cycle idle gap, or `None` for `Continuous` (no gap). The journal
    /// derives a Wait row's `[at − gap, at)` interval from this.
    pub fn gap(&self) -> Option<SimDuration> {
        match self {
            Self::Continuous => None,
            Self::AwaitingDemand { gap, .. }
            | Self::AwaitingTote { gap, .. }
            | Self::AwaitingSlot { gap, .. }
            | Self::AwaitingHtmDeposit { gap, .. } => Some(*gap),
        }
    }
}

/// A per-robot, classified explanation for a VTM's pre-cycle idle gap.
/// Emitted by the dispatcher at cycle commit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VtmIdleReasonEvent {
    pub robot_id: RobotId,
    /// Identity of the VTM cycle this pre-cycle gap precedes (1:1 with the
    /// cycle, see [`VtmCycleId`]).
    pub vtm_cycle_id: VtmCycleId,
    /// Simulation time at which the new cycle starts (= `vtm_start`).
    pub at: SimTime,
    pub reason: VtmIdleReason,
    /// Trip ID of the cycle this gap precedes; `Some` on every live emit
    /// (forward free, return threaded via `ToteAtRelay`). Lets a Wait journal
    /// row group with the cycle's Travel/Mechanical legs.
    pub presentation_id: Option<PresentationId>,
    /// Direction of the cycle this gap precedes, so a Wait journal row carries
    /// the same PickSupport/PutawaySupport label as the cycle's other legs.
    pub direction: VtmCycleDirection,
    /// Relay this cycle is bound to. The VTM idles on the rail awaiting this
    /// relay's reservation, so it is the honest `from`/`to` for the Wait row
    /// (the gap is "waiting for relay r").
    pub relay_id: RelayId,
    /// True when this cycle's legs are on the geometric clock (Teleport).
    /// The journal skips the Wait row for such cycles — the schedule-clock
    /// gap overlaps the geometric legs. The wait still reaches the
    /// idle-reason counters and `VtmCycleEvent::pre_cycle_wait_ms`.
    pub geo_diverged: bool,
}
