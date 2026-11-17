//! Handlers for the per-slice resource diagnostics
//! ([`crate::observer::station_diag::StationDiagRecord`]): HTM
//! assignments/arrivals, dispatch rejections, per-bot work intervals,
//! per-VTM idle reasons, and relay fork-Gets. Split from `handlers.rs`
//! the same way `resource_handler.rs` is.

use crate::SimTime;
use crate::events::{
    BotPlanSnapshotEvent, CycleCompleteEvent, DispatchRejectionEvent, HtmArrivalEvent, HtmAssignmentEvent,
    HtmRelayPickupEvent, InventoryVisitPhaseEvent, OrderStationPhaseEvent, PresentationReadinessEvent,
    VtmIdleReasonEvent,
};
use crate::model::{RobotId, SimDuration};
use crate::observer::activity::Milestone;
use crate::observer::duty::BusyInterval;
use crate::observer::station_diag::{
    HtmArrivalRecord, HtmAssignmentRecord, HtmRelayPickupRecord, InventoryVisitPhaseRecord, OrderStationPhaseRecord,
    PlanLegRecord, PresentationReadinessRecord, RobotActivityRecord, StationCycleRecord, VtmIdleReasonRecord,
};

use super::state::SimObserverState;

impl SimObserverState {
    pub(super) fn handle_inventory_visit_phase(&mut self, data: &InventoryVisitPhaseEvent) {
        self.slice_at(data.at).station_diag.inventory_visit_phases.push(InventoryVisitPhaseRecord {
            conveyor_id: data.conveyor_id.0,
            order_id: data.order_id.0,
            pick_index: data.pick_index,
            phase: data.phase,
            at: data.at,
        });
    }

    pub(super) fn handle_order_station_phase(&mut self, data: &OrderStationPhaseEvent) {
        self.slice_at(data.at).station_diag.order_phases.push(OrderStationPhaseRecord {
            order_id: data.order_id.0,
            conveyor_id: data.conveyor_id.0,
            presentation_id: data.presentation_id,
            waiting_cell: data.waiting_cell.map(|cell| cell.0),
            at: data.at,
            phase: data.phase,
        });
    }

    pub(super) fn handle_presentation_readiness(&mut self, data: &PresentationReadinessEvent) {
        self.slice_at(data.ready_at).station_diag.presentation_readiness.push(PresentationReadinessRecord {
            order_id: data.order_id.0,
            pick_index: data.pick_index,
            conveyor_id: data.conveyor_id.0,
            presentation_id: data.presentation_id,
            stream: data.stream,
            ready_at: data.ready_at,
        });
    }

    pub(super) fn handle_htm_assignment(&mut self, data: &HtmAssignmentEvent) {
        self.slice_at(data.dispatch_time).station_diag.assignments.push(HtmAssignmentRecord {
            robot_id: data.robot_id.0,
            station_id: data.station_id,
            tote_id: data.tote_id.map(|tote| tote.0),
            source: data.source,
            relay_cell: data.relay_cell,
            dispatch_time: data.dispatch_time,
            quoted_arrival: data.quoted_arrival,
        });
    }

    pub(super) fn handle_htm_arrival(&mut self, data: &HtmArrivalEvent) {
        let known_from = self.current_world_time();
        self.mark_activity(data.robot_id, Milestone::Arrival { at: data.arrival, known_from });
        self.open_station_visit(data.robot_id, data.station_id, data.arrival);
        self.slice_at(data.arrival).station_diag.arrivals.push(HtmArrivalRecord {
            robot_id: data.robot_id.0,
            station_id: data.station_id,
            arrival: data.arrival,
        });
    }

    pub(super) fn handle_dispatch_rejection(&mut self, data: &DispatchRejectionEvent) {
        self.slice_at(data.at).station_diag.add_rejection(data);
    }

    /// The slice an inspector record of the simtime from `start` on is filed
    /// in: the later of the slices `start` and B stand in. A slice B has
    /// passed is final without the record; the desvis store reads the records
    /// by their start, whatever slice carries them.
    fn filing_slice(&self, start: SimTime) -> usize {
        (start.max(self.current_world_time()).as_ms() / self.config.slice_width_ms) as usize
    }

    /// Record one interval of a robot's work for the bot inspector, once it is
    /// built.
    pub(super) fn record_robot_activity(&mut self, robot: RobotId, interval: BusyInterval) {
        let slice_index = self.filing_slice(interval.start);
        self.slice_to_write(slice_index).station_diag.activity.push(RobotActivityRecord {
            robot_id: robot.0,
            kind: interval.kind,
            start: interval.start,
            end: interval.end,
        });
    }

    /// Record a per-VTM idle reason for the inspector. Called from
    /// `handle_vtm_idle_reason`. `Continuous` (no gap) carries no diagnostic
    /// and is skipped by the caller.
    pub(super) fn record_vtm_idle_reason(&mut self, data: &VtmIdleReasonEvent, relay_cell: u32) {
        self.slice_at(data.at).station_diag.vtm_idle_reasons.push(VtmIdleReasonRecord {
            robot_id: data.robot_id.0,
            reason: data.reason.kind(),
            gap_ms: data.reason.gap().map(|gap| gap.as_ms()).unwrap_or(0),
            relay_cell,
            at: data.at,
        });
    }

    /// Record a fork-Get and its preceding dwell for the relay inspector.
    /// Called from `handle_htm_relay_pickup`.
    pub(super) fn record_htm_relay_pickup(&mut self, data: &HtmRelayPickupEvent, relay_cell: u32) {
        self.slice_at(data.start).station_diag.htm_relay_pickups.push(HtmRelayPickupRecord {
            robot_id: data.robot_id.0,
            relay_cell,
            wait_ms: data.wait.as_ms(),
            at: data.start,
        });
    }

    /// Record a completed station cycle span for the inspector's
    /// Serving-vs-Idle decision. Called from `handle_cycle_complete` with the
    /// kind-aware physical busy interval `[busy_start_ms, busy_end_ms)` (pick
    /// = dwell start-anchored, putaway = fill-anchored/dwell-end), so putaway
    /// HTM dwell counts as station occupancy. A putaway is reported at its
    /// fill, so B may have passed the start of its dwell.
    pub(super) fn record_station_cycle(&mut self, data: &CycleCompleteEvent, busy_start_ms: u64, busy_end_ms: u64) {
        let slice_index = self.filing_slice(SimTime::ms(busy_start_ms));
        self.slice_to_write(slice_index).station_diag.station_cycles.push(StationCycleRecord {
            station_id: data.station_id,
            start: SimTime::ms(busy_start_ms),
            dwell: SimDuration::ms(busy_end_ms.saturating_sub(busy_start_ms)),
        });
    }

    /// Record a bot's committed plan legs, reported once per bot after the
    /// scheduler stops. The snapshot has no instant of its own, since its legs
    /// are the plan's future, so it is filed in the slice of the current world
    /// time, where it became known. The desvis store flattens plan legs across
    /// slices by robot, so the slice is only transport.
    pub(super) fn handle_bot_plan_snapshot(&mut self, data: &BotPlanSnapshotEvent) {
        let slice_index = (self.current_world_time().as_ms() / self.config.slice_width_ms) as usize;
        let plan_legs = &mut self.slice_to_write(slice_index).station_diag.plan_legs;
        for leg in &data.legs {
            plan_legs.push(PlanLegRecord {
                robot_id: data.robot_id.0,
                kind: leg.kind,
                dest_cell: leg.dest_cell,
                enter: leg.enter,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::events::{PlanLegInfo, PlanLegKind, SimEvent};
    use crate::observer::time_slice::MetricsConfig;

    use super::*;

    /// The snapshot is reported after the scheduler stops, when B stands at
    /// 250: its legs enter later, in slices B has not reached, and it is
    /// filed where B stands rather than where its first leg enters.
    #[test]
    fn a_plan_snapshot_is_filed_in_the_slice_where_b_stands() {
        let mut state = SimObserverState::new(MetricsConfig { slice_width_ms: 100, ..Default::default() });
        state.settle_activity(SimTime::ms(250)).unwrap();
        let leg = |enter| PlanLegInfo { kind: PlanLegKind::Travel, dest_cell: 9, enter: SimTime::ms(enter) };
        state.dispatch_event(&SimEvent::BotPlanSnapshot(BotPlanSnapshotEvent {
            robot_id: RobotId(3),
            legs: vec![leg(720), leg(940)],
        }));
        assert_eq!(state.slices().len(), 3, "no slice is made for the plan's future");
        let filed: Vec<_> = state.slices()[2].station_diag.plan_legs.iter().map(|leg| leg.enter.as_ms()).collect();
        assert_eq!(filed, [720, 940]);
    }
}
