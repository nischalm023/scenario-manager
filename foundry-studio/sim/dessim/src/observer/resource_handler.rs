//! Resource contention event handler.

use super::state::SimObserverState;
use super::time_slice::ScalarCounters;
use crate::SimTime;
use crate::events::{ResourceEvent, ResourceEventKind};

impl SimObserverState {
    pub(super) fn handle_resource(&mut self, data: &ResourceEvent) {
        count_resource(&mut self.slice_at(SimTime::ms(data.sim_time_ms)).scalars, &data.kind);
    }
}

/// Count one resource event into a slice's counters.
fn count_resource(scalars: &mut ScalarCounters, kind: &ResourceEventKind) {
    match *kind {
        ResourceEventKind::RelayReservationSuccess => {
            scalars.relay_reservation_successes += 1;
        }
        ResourceEventKind::RelayReservationFailure => {
            scalars.relay_reservation_failures += 1;
        }
        ResourceEventKind::ToteClaimSuccess => {
            scalars.tote_claim_successes += 1;
        }
        ResourceEventKind::ToteClaimFailure => {
            scalars.tote_claim_failures += 1;
        }
        ResourceEventKind::BackpressureTrigger => {
            scalars.backpressure_events += 1;
        }
        ResourceEventKind::PathfinderQuery => {
            scalars.pathfinder_queries += 1;
        }
        ResourceEventKind::PathfinderFailure => {
            scalars.pathfinder_failures += 1;
        }
        ResourceEventKind::VtmScoreForwardSelected => {
            scalars.vtm_forward_matches += 1;
        }
        ResourceEventKind::VtmScoreReturnSelected => {
            scalars.vtm_return_matches += 1;
        }
        ResourceEventKind::VtmCrossZoneAssignment => {
            scalars.vtm_cross_zone_assignments += 1;
        }
        ResourceEventKind::VtmScoringDuration { duration_ns } => {
            scalars.vtm_scoring_duration_ns += duration_ns;
        }
        ResourceEventKind::ZonePressureSnapshot { .. } => {}
        ResourceEventKind::StarvationAgeBonusApplied => {
            scalars.starvation_age_bonus_applied += 1;
        }
        ResourceEventKind::DeadlockHeadroomTriggered => {
            scalars.deadlock_headroom_triggered += 1;
        }
        ResourceEventKind::VtmForwardCompleted { cycle_ms } => {
            scalars.vtm_forward_trips += 1;
            scalars.vtm_forward_cycle_ms += cycle_ms;
        }
        ResourceEventKind::VtmReturnCompleted { cycle_ms } => {
            scalars.vtm_return_trips += 1;
            scalars.vtm_return_cycle_ms += cycle_ms;
        }
        ResourceEventKind::HtmPickupCompleted { cycle_ms } => {
            scalars.htm_pickup_trips += 1;
            scalars.htm_cycle_ms += cycle_ms;
        }
        ResourceEventKind::HtmReturnCompleted { cycle_distance_mm, cycle_ms } => {
            scalars.htm_return_trips += 1;
            scalars.htm_round_trip_distance_mm += cycle_distance_mm;
            scalars.htm_round_trip_ms += cycle_ms;
        }
        ResourceEventKind::HtmOperatorWaitCompleted { wait_ms } => {
            scalars.htm_operator_wait_ms += wait_ms;
            scalars.htm_operator_wait_count += 1;
        }
        ResourceEventKind::HtmReturnRelayRetry => {
            scalars.htm_return_relay_retries += 1;
        }
        ResourceEventKind::OperatorQueueWait => {
            scalars.operator_queue_waits += 1;
        }
    }
}
