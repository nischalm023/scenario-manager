//! Within `finality`, `run_totals` is THE RUN'S COUNTERS AS ITS FINAL SLICES STATE THEM, and the rule by which one
//! slice's counters combine onto others. Each slice made final is carried onto the run's totals, so what is read from
//! them (the recording's end block and the figures simrunner stores from it, the run model, and the summary's
//! pathfinder and pre-embargo pick counts; its completion counts come from the journals) counts what the run's slices
//! state and no more: work the scheduler committed past the run's last slice is no slice's, so no total's. What the
//! run adds up is the sum of its slices, a level is the newest slice's reading, the peak is the highest any slice
//! reached, and an average divides the summed parts, as the flattening divides them. A slice taking in the facts
//! filed at the run's stop (`at_the_stop`) adds their counts by the same rule and keeps its own levels.

use std::hash::Hash;
use std::ops::AddAssign;

use rustc_hash::FxHashMap;

use crate::observer::counters_v2::{PipelineCounters, RackCounters, ThroughputCounters, TripDistanceCounters};
use crate::observer::duty::DutyBreakdown;
use crate::observer::time_slice::ScalarCounters;

/// Which levels counters combined onto others stand at: the newest's, as the run's totals stand at the latest slice
/// made final, or their own, as a slice taking in facts keeps the readings it is stamped with.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Levels {
    Newest,
    Kept,
}

impl ScalarCounters {
    /// Carry `slice`, the next slice made final, onto these totals.
    pub(super) fn carry(&mut self, slice: &ScalarCounters) {
        self.combine(slice, Levels::Newest);
    }

    /// Add the counts of `facts`, filed at one instant, onto this slice's, which takes them in (`at_the_stop`).
    pub(super) fn add_counts(&mut self, facts: &ScalarCounters) {
        self.combine(facts, Levels::Kept);
    }

    /// Add what `other` adds up onto these counters, and stand at its levels when `levels` says so. Every field is
    /// named, so a counter added later does not compile until it is named here, where its rule is chosen.
    fn combine(&mut self, other: &ScalarCounters, levels: Levels) {
        let ScalarCounters {
            picks_completed,
            pre_embargo_picks,
            putaways_completed,
            putaway_totes_stored,
            picks_per_station,
            presentations,
            pick_presentations,
            putaway_presentations,
            order_load_presentations,
            order_extract_presentations,
            order_extracted_units,
            orders_completed,
            vtm_activity_counts,
            htm_activity_counts,
            backpressure_events,
            tote_claim_successes,
            tote_claim_failures,
            relay_reservation_successes,
            relay_reservation_failures,
            pathfinder_queries,
            pathfinder_failures,
            vtm_forward_matches,
            vtm_return_matches,
            vtm_cross_zone_assignments,
            vtm_scoring_duration_ns,
            starvation_age_bonus_applied,
            deadlock_headroom_triggered,
            wall_clock_duration_ms,
            vtm_travel_distance_mm,
            htm_travel_distance_mm,
            vtm_forward_trips,
            vtm_return_trips,
            htm_pickup_trips,
            htm_return_trips,
            vtm_forward_cycle_ms,
            vtm_return_cycle_ms,
            htm_cycle_ms,
            htm_operator_wait_ms,
            htm_operator_wait_count,
            htm_round_trip_distance_mm,
            htm_round_trip_ms,
            htm_return_relay_retries,
            operator_queue_waits,
            trip_distance,
            rack,
            inventory_units,
            relay_cells_reserved,
            relay_cells_total,
            relay_reserved_busy_ms,
            relay_reserved_busy_per_zone,
            throughput,
            pipeline,
            htm_wait_at_relay_ms,
            htm_relay_pickup_count,
            diving_board_slot_wait_ms,
            diving_board_presentation_count,
            duty_per_robot,
            vtm_idle_reason_counts,
            vtm_duty_traveling_ms,
            vtm_duty_loading_unloading_ms,
            htm_duty_traveling_ms,
            htm_duty_rotating_ms,
            htm_duty_loading_unloading_ms,
            htm_duty_processing_ms,
            htm_duty_wait_loaded_ms,
            htm_duty_dwell_loaded_ms,
            htm_duty_pick_at_station_ms,
        } = other;
        self.picks_completed += picks_completed;
        self.pre_embargo_picks += pre_embargo_picks;
        self.putaways_completed += putaways_completed;
        self.putaway_totes_stored += putaway_totes_stored;
        self.presentations += presentations;
        self.pick_presentations += pick_presentations;
        self.putaway_presentations += putaway_presentations;
        self.order_load_presentations += order_load_presentations;
        self.order_extract_presentations += order_extract_presentations;
        self.order_extracted_units += order_extracted_units;
        self.orders_completed += orders_completed;
        self.backpressure_events += backpressure_events;
        self.tote_claim_successes += tote_claim_successes;
        self.tote_claim_failures += tote_claim_failures;
        self.relay_reservation_successes += relay_reservation_successes;
        self.relay_reservation_failures += relay_reservation_failures;
        self.pathfinder_queries += pathfinder_queries;
        self.pathfinder_failures += pathfinder_failures;
        self.vtm_forward_matches += vtm_forward_matches;
        self.vtm_return_matches += vtm_return_matches;
        self.vtm_cross_zone_assignments += vtm_cross_zone_assignments;
        self.vtm_scoring_duration_ns += vtm_scoring_duration_ns;
        self.starvation_age_bonus_applied += starvation_age_bonus_applied;
        self.deadlock_headroom_triggered += deadlock_headroom_triggered;
        self.vtm_travel_distance_mm += vtm_travel_distance_mm;
        self.htm_travel_distance_mm += htm_travel_distance_mm;
        self.vtm_forward_trips += vtm_forward_trips;
        self.vtm_return_trips += vtm_return_trips;
        self.htm_pickup_trips += htm_pickup_trips;
        self.htm_return_trips += htm_return_trips;
        self.vtm_forward_cycle_ms += vtm_forward_cycle_ms;
        self.vtm_return_cycle_ms += vtm_return_cycle_ms;
        self.htm_cycle_ms += htm_cycle_ms;
        self.htm_operator_wait_ms += htm_operator_wait_ms;
        self.htm_operator_wait_count += htm_operator_wait_count;
        self.htm_round_trip_ms += htm_round_trip_ms;
        self.htm_return_relay_retries += htm_return_relay_retries;
        self.operator_queue_waits += operator_queue_waits;
        self.relay_reserved_busy_ms += relay_reserved_busy_ms;
        self.htm_wait_at_relay_ms += htm_wait_at_relay_ms;
        self.htm_relay_pickup_count += htm_relay_pickup_count;
        self.diving_board_slot_wait_ms += diving_board_slot_wait_ms;
        self.diving_board_presentation_count += diving_board_presentation_count;
        self.vtm_duty_traveling_ms += vtm_duty_traveling_ms;
        self.vtm_duty_loading_unloading_ms += vtm_duty_loading_unloading_ms;
        self.htm_duty_traveling_ms += htm_duty_traveling_ms;
        self.htm_duty_rotating_ms += htm_duty_rotating_ms;
        self.htm_duty_loading_unloading_ms += htm_duty_loading_unloading_ms;
        self.htm_duty_processing_ms += htm_duty_processing_ms;
        self.htm_duty_wait_loaded_ms += htm_duty_wait_loaded_ms;
        self.htm_duty_dwell_loaded_ms += htm_duty_dwell_loaded_ms;
        self.htm_duty_pick_at_station_ms += htm_duty_pick_at_station_ms;
        self.wall_clock_duration_ms += wall_clock_duration_ms;
        self.htm_round_trip_distance_mm += htm_round_trip_distance_mm;
        add_by_key(&mut self.picks_per_station, picks_per_station);
        add_by_key(&mut self.relay_reserved_busy_per_zone, relay_reserved_busy_per_zone);
        add_by_key(&mut self.vtm_idle_reason_counts, vtm_idle_reason_counts);
        for (robot, duty) in duty_per_robot {
            let DutyBreakdown { traveling, loading_unloading, processing } = duty;
            let total = self.duty_per_robot.entry(*robot).or_default();
            total.traveling += *traveling;
            total.loading_unloading += *loading_unloading;
            total.processing += *processing;
        }
        self.trip_distance.carry(trip_distance);
        self.throughput.carry(throughput);
        self.pipeline.carry(pipeline);
        if levels == Levels::Newest {
            self.rack.carry(rack);
            self.vtm_activity_counts.clone_from(vtm_activity_counts);
            self.htm_activity_counts.clone_from(htm_activity_counts);
            self.inventory_units = *inventory_units;
            self.relay_cells_reserved = *relay_cells_reserved;
            self.relay_cells_total = *relay_cells_total;
        }
    }
}

impl TripDistanceCounters {
    fn carry(&mut self, slice: &TripDistanceCounters) {
        let TripDistanceCounters { htm_to_relay_mm, htm_to_station_mm, htm_to_return_mm, htm_to_parking_mm } = slice;
        self.htm_to_relay_mm += htm_to_relay_mm;
        self.htm_to_station_mm += htm_to_station_mm;
        self.htm_to_return_mm += htm_to_return_mm;
        self.htm_to_parking_mm += htm_to_parking_mm;
    }
}

impl ThroughputCounters {
    fn carry(&mut self, slice: &ThroughputCounters) {
        let ThroughputCounters {
            pick_station_picks,
            pick_station_presentations,
            pick_station_orders,
            putaway_station_picks,
            putaway_station_presentations,
            putaway_station_presentations_direct,
            putaway_station_presentations_from_rack,
            putaway_dispatches_direct,
            putaway_dispatches_from_rack,
        } = slice;
        self.pick_station_picks += pick_station_picks;
        self.pick_station_presentations += pick_station_presentations;
        self.pick_station_orders += pick_station_orders;
        self.putaway_station_picks += putaway_station_picks;
        self.putaway_station_presentations += putaway_station_presentations;
        self.putaway_station_presentations_direct += putaway_station_presentations_direct;
        self.putaway_station_presentations_from_rack += putaway_station_presentations_from_rack;
        self.putaway_dispatches_direct += putaway_dispatches_direct;
        self.putaway_dispatches_from_rack += putaway_dispatches_from_rack;
    }
}

impl PipelineCounters {
    fn carry(&mut self, slice: &PipelineCounters) {
        let PipelineCounters { pipeline_depth_sum, pipeline_snapshot_count } = slice;
        self.pipeline_depth_sum += pipeline_depth_sum;
        self.pipeline_snapshot_count += pipeline_snapshot_count;
    }
}

impl RackCounters {
    /// The rack's fill stands at the newest slice's reading; its peak is the highest any slice reached.
    fn carry(&mut self, slice: &RackCounters) {
        let RackCounters {
            occupancy_total,
            occupancy_peak,
            capacity_total,
            per_zone_occupancy,
            fill_min_pct,
            fill_max_pct,
            fill_stddev_pct,
        } = slice;
        self.occupancy_total = *occupancy_total;
        self.occupancy_peak = self.occupancy_peak.max(*occupancy_peak);
        self.capacity_total = *capacity_total;
        self.per_zone_occupancy.clone_from(per_zone_occupancy);
        self.fill_min_pct = *fill_min_pct;
        self.fill_max_pct = *fill_max_pct;
        self.fill_stddev_pct = *fill_stddev_pct;
    }
}

/// Add each of `slice`'s values onto the total under its key.
fn add_by_key<K: Copy + Eq + Hash, V: Copy + Default + AddAssign>(
    totals: &mut FxHashMap<K, V>,
    slice: &FxHashMap<K, V>,
) {
    for (key, value) in slice {
        *totals.entry(*key).or_default() += *value;
    }
}

#[cfg(test)]
mod tests {
    use desgraph::{StationId, ZoneId};

    use super::*;
    use crate::model::SimDuration;
    use crate::observer::activity::ActivityKind;

    /// Two slices carried in turn: what the run adds up sums, under its key where it has one; a level stands at the
    /// later slice's reading; the rack's peak is the higher of the two slices' peaks.
    #[test]
    fn the_totals_add_what_adds_up_and_stand_at_the_newest_level() {
        let mut first = ScalarCounters { picks_completed: 3, inventory_units: 40, ..ScalarCounters::default() };
        first.rack = RackCounters { occupancy_total: 9, occupancy_peak: 12, ..RackCounters::default() };
        first.picks_per_station.insert(StationId(1), 3);
        first.relay_reserved_busy_per_zone.insert(ZoneId(4), SimDuration::ms(500));
        first.htm_activity_counts.insert(ActivityKind::Travel, 2);
        let mut later = ScalarCounters { picks_completed: 4, inventory_units: 38, ..ScalarCounters::default() };
        later.rack = RackCounters { occupancy_total: 7, occupancy_peak: 8, ..RackCounters::default() };
        later.picks_per_station.extend([(StationId(1), 1), (StationId(2), 5)]);
        later.relay_reserved_busy_per_zone.insert(ZoneId(4), SimDuration::ms(250));
        later.htm_activity_counts.insert(ActivityKind::Gap, 3);

        let mut totals = ScalarCounters::default();
        totals.carry(&first);
        totals.carry(&later);
        assert_eq!(totals.picks_completed, 7, "picks add");
        let by_station = [StationId(1), StationId(2)].map(|station| totals.picks_per_station.get(&station).copied());
        assert_eq!(by_station, [Some(4), Some(5)], "and add under their station");
        assert_eq!(totals.relay_reserved_busy_per_zone.get(&ZoneId(4)), Some(&SimDuration::ms(750)), "busy time adds");
        assert_eq!(totals.inventory_units, 38, "the stock stands where the later slice left it");
        let rack = &totals.rack;
        assert_eq!(
            (rack.occupancy_total, rack.occupancy_peak),
            (7, 12),
            "the fill at the later reading, the peak higher"
        );
        assert_eq!(totals.htm_activity_counts, later.htm_activity_counts, "robots per activity at the later reading");
    }

    /// A slice taking in the facts filed at the run's stop adds their counts and keeps the readings it was stamped
    /// with, which the facts' own empty readings would otherwise replace.
    #[test]
    fn a_slice_taking_in_facts_adds_their_counts_and_keeps_its_readings() {
        let mut slice = ScalarCounters { orders_completed: 1, inventory_units: 40, ..ScalarCounters::default() };
        slice.rack = RackCounters { occupancy_total: 9, occupancy_peak: 12, ..RackCounters::default() };
        slice.htm_activity_counts.insert(ActivityKind::Travel, 2);
        let facts = ScalarCounters { orders_completed: 1, picks_completed: 3, ..ScalarCounters::default() };

        slice.add_counts(&facts);
        assert_eq!((slice.orders_completed, slice.picks_completed), (2, 3), "the counts add");
        assert_eq!(slice.inventory_units, 40, "the stock stands as stamped");
        assert_eq!((slice.rack.occupancy_total, slice.rack.occupancy_peak), (9, 12), "and so does the rack");
        assert_eq!(slice.htm_activity_counts.get(&ActivityKind::Travel), Some(&2), "and robots per activity");
    }
}
