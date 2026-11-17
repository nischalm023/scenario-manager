//! VTM journey-leg journal row construction — the "VTM Journey Legs" sheet.
//!
//! The VTM analog of the HTM journal in [`super::nav_handler`]. Each committed
//! VTM cycle emits a strictly-sequential sequence of micro-phases (a pre-cycle
//! idle gap, then `RailMove → Alignment → Fork → RailMove → Alignment → Fork`), so the journal
//! partitions the VTM's wall-clock without double-counting (spec §6):
//!
//! - a `Travel` row per `RailMove` (from `handle_vtm_nav`),
//! - an `Alignment` row per endpoint approach (from `handle_vtm_nav`),
//! - a `Mechanical` row per `Fork` (from `handle_vtm_nav`),
//! - a `Wait` row per non-`Continuous` pre-cycle gap (from
//!   `handle_vtm_idle_reason`).
//!
//! These rows are *additional* output. Duty accounting and the idle-reason
//! counters are untouched — the callers still do their existing work and then
//! push one leg row each.
//!
//! The same micro-phases are the DA50's spans in the robot state channel.
//! The pre-cycle gap is not: its reason is decided at the cycle's commit, once
//! B has passed the gap, so the channel states the gap as no task.

use worldview::frames::robot_state::RobotState;

use crate::events::{ForkOp, VtmEndpoint, VtmIdleReasonEvent, VtmNavEventData, VtmNavKind};
use crate::observer::output::{Activity, VtmJournalEndpoint, VtmLegRecord};
use crate::observer::robot_state::Claim;

use super::state::SimObserverState;

impl SimObserverState {
    /// Push the one journal row implied by a `VtmNav` micro-phase: a `Travel`
    /// row for a `RailMove`, an `Alignment` row for an endpoint approach, or a
    /// `Mechanical` row for a `Fork`. Called from
    /// `handle_vtm_nav` after its duty/grid accounting.
    pub(super) fn push_vtm_nav_leg(&mut self, data: &VtmNavEventData) {
        let presentation_id = data.presentation_id.map(|id| id.0);
        let direction = data.direction;
        // Journal rows are on the leg's geometric clock (see `VtmNavEventData`).
        let start_ms = data.geo_start.as_ms();
        let duration_ms = (data.geo_end - data.geo_start).as_ms();
        let row = match &data.kind {
            VtmNavKind::RailMove { from, to, tote_id, distance_mm, .. } => {
                let from = resolve_vtm_endpoint(from);
                let to = resolve_vtm_endpoint(to);
                VtmLegRecord {
                    start_ms,
                    duration_ms,
                    activity: Activity::Travel,
                    from,
                    to,
                    from_zone_id: self.vtm_relay_zone(from),
                    to_zone_id: self.vtm_relay_zone(to),
                    distance_mm: *distance_mm,
                    vtm_id: data.robot_id.0,
                    vtm_cycle_id: Some(data.vtm_cycle_id.0),
                    tote_id: tote_id.map(|t| t.0),
                    presentation_id,
                    direction,
                    carrying_tote: tote_id.is_some(),
                    wait_reason: None,
                    notes: String::new(),
                }
            }
            VtmNavKind::Alignment { at, tote_id, .. } => {
                let at = resolve_vtm_endpoint(at);
                let zone = self.vtm_relay_zone(at);
                VtmLegRecord {
                    start_ms,
                    duration_ms,
                    activity: Activity::Alignment,
                    from: at,
                    to: at,
                    from_zone_id: zone,
                    to_zone_id: zone,
                    distance_mm: 0,
                    vtm_id: data.robot_id.0,
                    vtm_cycle_id: Some(data.vtm_cycle_id.0),
                    tote_id: tote_id.map(|tote_id| tote_id.0),
                    presentation_id,
                    direction,
                    carrying_tote: tote_id.is_some(),
                    wait_reason: None,
                    notes: alignment_note_endpoint(at).to_string(),
                }
            }
            VtmNavKind::Fork { op, at, tote_id, .. } => {
                let at = resolve_vtm_endpoint(at);
                let zone = self.vtm_relay_zone(at);
                VtmLegRecord {
                    start_ms,
                    duration_ms,
                    activity: Activity::Mechanical,
                    from: at,
                    to: at,
                    from_zone_id: zone,
                    to_zone_id: zone,
                    distance_mm: 0,
                    vtm_id: data.robot_id.0,
                    vtm_cycle_id: Some(data.vtm_cycle_id.0),
                    tote_id: Some(tote_id.0),
                    presentation_id,
                    direction,
                    // A fork-op always has the tote in hand (Get extracts it into
                    // the fork, Put deposits it from the fork).
                    carrying_tote: true,
                    wait_reason: None,
                    notes: mechanical_note_endpoint(at, *op).to_string(),
                }
            }
        };
        self.output.vtm_leg_log.push(row);
        let state = match &data.kind {
            VtmNavKind::RailMove { tote_id: Some(_), .. } => RobotState::LadenTravel,
            VtmNavKind::RailMove { tote_id: None, .. } => RobotState::EmptyTravel,
            VtmNavKind::Alignment { .. } => RobotState::Alignment,
            VtmNavKind::Fork { op: ForkOp::Get, .. } => RobotState::Pickup,
            VtmNavKind::Fork { op: ForkOp::Put, .. } => RobotState::Drop,
        };
        self.robot_state_log.note(data.robot_id, (data.geo_start, data.geo_end), state, Claim::Committed);
    }

    /// Zone of a VTM journal endpoint, `Some` only when it is a `Relay` and the
    /// relay→zone table is populated. Stamps the `From/To Relay Zone` columns.
    fn vtm_relay_zone(&self, endpoint: VtmJournalEndpoint) -> Option<u32> {
        match endpoint {
            VtmJournalEndpoint::Relay(relay_id) => self.config.relay_zones.get(&relay_id).copied(),
            _ => None,
        }
    }

    /// Push a `Wait` journal row for a VTM's pre-cycle idle gap `[at − gap, at)`.
    /// Called from `handle_vtm_idle_reason` *in addition to* the existing
    /// counter increment. `Continuous` reasons carry no gap and are skipped —
    /// there is no wall-clock to journal. The gap is the VTM idling on the rail
    /// awaiting its bound relay's reservation, so the endpoint is that relay.
    ///
    /// Geometric-clock (Teleport) cycles are also skipped: their
    /// schedule-clock gap overlaps the geometric legs and would double-count.
    pub(super) fn push_vtm_wait_leg(&mut self, data: &VtmIdleReasonEvent) {
        if data.geo_diverged {
            return;
        }
        let Some(gap) = data.reason.gap() else {
            return;
        };
        let direction = data.direction;
        let relay = VtmJournalEndpoint::Relay(data.relay_id);
        let zone = self.vtm_relay_zone(relay);
        self.output.vtm_leg_log.push(VtmLegRecord {
            start_ms: (data.at - gap).as_ms(),
            duration_ms: gap.as_ms(),
            activity: Activity::Wait,
            from: relay,
            to: relay,
            from_zone_id: zone,
            to_zone_id: zone,
            distance_mm: 0,
            vtm_id: data.robot_id.0,
            vtm_cycle_id: Some(data.vtm_cycle_id.0),
            tote_id: None,
            presentation_id: data.presentation_id.map(|id| id.0),
            direction,
            carrying_tote: false,
            wait_reason: Some(data.reason.kind()),
            notes: String::new(),
        });
    }
}

fn alignment_note_endpoint(at: VtmJournalEndpoint) -> &'static str {
    match at {
        VtmJournalEndpoint::Rack(_) => "align @ rack",
        VtmJournalEndpoint::Relay(_) => "align @ relay",
        // A VTM aligns only to a fork target (rack slot or relay buffer); an
        // aisle rail position is neither — not emitted by the forward/return
        // tasks, but label it rather than panic on telemetry.
        VtmJournalEndpoint::Aisle => "align @ aisle",
    }
}

/// Resolve a physical `VtmEndpoint` into the journal's semantic endpoint
/// (spec DR-2): a slot maps to its rack, a relay to its relay, an empty rail
/// position to `Aisle`.
fn resolve_vtm_endpoint(endpoint: &VtmEndpoint) -> VtmJournalEndpoint {
    match endpoint {
        VtmEndpoint::Aisle { .. } => VtmJournalEndpoint::Aisle,
        VtmEndpoint::Relay { relay_id, .. } => VtmJournalEndpoint::Relay(*relay_id),
        VtmEndpoint::Slot { rack_id, .. } => VtmJournalEndpoint::Rack(*rack_id),
    }
}

/// The mechanical sub-kind of a `Fork`, by (already-resolved) endpoint (spec
/// DR-3): a fork at a rack slot is the rack fork; a fork at a relay is the
/// relay-side transfer.
fn mechanical_note_endpoint(at: VtmJournalEndpoint, op: ForkOp) -> &'static str {
    match (at, op) {
        (VtmJournalEndpoint::Rack(_), ForkOp::Get) => "fork-get @ rack",
        (VtmJournalEndpoint::Rack(_), ForkOp::Put) => "fork-put @ rack",
        (VtmJournalEndpoint::Relay(_), ForkOp::Get) => "buffer-get @ relay",
        (VtmJournalEndpoint::Relay(_), ForkOp::Put) => "buffer-put @ relay",
        // A fork at an aisle position has no slot/buffer — not emitted by the
        // forward/return tasks, but label it rather than panic on telemetry.
        (VtmJournalEndpoint::Aisle, _) => "fork @ aisle",
    }
}
