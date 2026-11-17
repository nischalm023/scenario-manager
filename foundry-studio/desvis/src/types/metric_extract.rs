//! MetricKind value extraction from dynamic metric maps.

use super::metrics::{MetricKind, RollingKind};
use std::collections::HashMap;

impl MetricKind {
    /// Why this metric's value would be *undefined* for the loaded scenario,
    /// if it is.
    ///
    /// `Some(reason)` means the metric measures a structural quantity the
    /// scenario simply doesn't have — presentations per station-hour with no
    /// stations, any rack-fill metric with no rack storage slots — so there is
    /// no meaningful number to plot. Deliberately narrower than "the scenario
    /// can't produce this event": a metric that merely reads zero this slice
    /// (no trips, no presentations) is a quiet `0.0`, not unavailable, and
    /// still plots.
    ///
    /// The gate keys on three structural scenario inputs — station count,
    /// rack-slot count, and relay-cell count. The station axis is a literal
    /// divide-by-zero (÷ stations); the rack and relay axes are structural
    /// checks (their percent denominators are the per-slice `rack_capacity` /
    /// `relay_cells_total`, guarded separately, so the gate is what turns a
    /// rackless/relayless scenario's flat-zero lines into an explicit
    /// "unavailable" rather than avoiding a division). Metrics that are
    /// *conceptually* inapplicable yet still numerically 0 — e.g. putaway
    /// rates in a pick-only scenario — are not surfaced as unavailable and will
    /// plot a flat-zero line; widening that needs per-station-type counts
    /// threaded through, which this does not attempt.
    ///
    /// Single source of the structural-availability rules:
    /// [`MetricKind::extract`] early-returns this as `Err`, and
    /// [`MetricKind::unavailable_label`] wraps it for display.
    pub fn unavailable_reason(
        &self,
        num_stations: usize,
        total_rack_slots: u32,
        total_relay_cells: u32,
    ) -> Option<&'static str> {
        match self {
            MetricKind::CumulativePresPerStnHr if num_stations == 0 => Some("scenario has no stations"),
            // Every rack-storage metric measures the rack; on a scenario with
            // no rack slots they're all structurally undefined, not merely 0.
            MetricKind::RackOccupancy
            | MetricKind::RackFillPercent
            | MetricKind::RackMaxFillPercent
            | MetricKind::RackFillMinPercent
            | MetricKind::RackFillMaxPercent
                if total_rack_slots == 0 =>
            {
                Some("scenario has no rack storage slots")
            }
            // Relay-occupancy metrics measure relay cells; undefined with none.
            MetricKind::RelayOccupancyPercent | MetricKind::RelayReservationUtilPercent if total_relay_cells == 0 => {
                Some("scenario has no relays")
            }
            _ => None,
        }
    }

    /// Display-ready unavailability label — e.g. `"Unavailable: scenario has
    /// no rack storage slots"`. The single phrasing shared by every UI
    /// surface (sidebar rows, the "+ Add chart" catalog, the chart panel);
    /// `None` when the metric is available. Wraps [`MetricKind::unavailable_reason`].
    pub fn unavailable_label(
        &self,
        num_stations: usize,
        total_rack_slots: u32,
        total_relay_cells: u32,
    ) -> Option<String> {
        self.unavailable_reason(num_stations, total_rack_slots, total_relay_cells)
            .map(|reason| format!("Unavailable: {reason}"))
    }

    /// Extract the metric value from a dynamic metrics map.
    ///
    /// Returns `Err(reason)` when the metric is structurally unavailable for
    /// this scenario (see [`MetricKind::unavailable_reason`]); the reason is
    /// the same on every slice, so callers surface it once rather than per
    /// slice. An `Ok(0.0)` from the remaining arms means "no samples in this
    /// slice" — a quiet slice, not unavailability.
    ///
    /// Extraction metrics return unavailable for legacy profiles without their keys.
    /// Other metrics panic if a requested key is missing from the slice. This is a
    /// load-bearing invariant: every key referenced below must be emitted
    /// unconditionally by `dessim::metrics::profile::convert::scalars_to_map`
    /// — silent zero-fallbacks here would let typos and removed-but-not-
    /// renamed counters return believable-looking 0.0s for the life of a
    /// run. Better to halt at first sight than to publish wrong numbers.
    pub fn extract(
        &self,
        m: &HashMap<String, f64>,
        slice_width_ms: u64,
        num_stations: usize,
        total_rack_slots: u32,
        total_relay_cells: u32,
    ) -> Result<f64, &'static str> {
        if let Some(reason) = self.unavailable_reason(num_stations, total_rack_slots, total_relay_cells) {
            return Err(reason);
        }
        let get = |key: &str| -> f64 {
            *m.get(key).unwrap_or_else(|| {
                panic!(
                    "MetricKind::{:?} requested missing slice key {key:?} — emit it from \
                     dessim::metrics::profile::convert::scalars_to_map",
                    self
                )
            })
        };
        Ok(match self {
            MetricKind::PicksPerSlice => get("picks_completed"),
            MetricKind::PicksPerHourRate => {
                let slice_hours = slice_width_ms as f64 / 3_600_000.0;
                if slice_hours > 0.0 { get("picks_completed") / slice_hours } else { 0.0 }
            }
            MetricKind::CumulativePicks => get("picks_completed"),
            MetricKind::PutawaysPerSlice => get("putaways_completed"),
            MetricKind::CumulativePutaways => get("putaways_completed"),
            MetricKind::PickPresentationsPerSlice => get("pick_presentations"),
            MetricKind::CumulativePickPresentations => get("pick_presentations"),
            MetricKind::PutawayPresentationsPerSlice => get("putaway_presentations"),
            MetricKind::CumulativePutawayPresentations => get("putaway_presentations"),
            MetricKind::PutawayPresentationsDirectPerSlice => get("putaway_station_presentations_direct"),
            MetricKind::CumulativePutawayPresentationsDirect => get("putaway_station_presentations_direct"),
            MetricKind::PutawayPresentationsFromRackPerSlice => get("putaway_station_presentations_from_rack"),
            MetricKind::CumulativePutawayPresentationsFromRack => get("putaway_station_presentations_from_rack"),
            MetricKind::CumulativeOrders => get("orders_completed"),
            MetricKind::OrdersPerHourRate => {
                let slice_hours = slice_width_ms as f64 / 3_600_000.0;
                if slice_hours > 0.0 { get("orders_completed") / slice_hours } else { 0.0 }
            }
            MetricKind::OrderExtractionsPerSlice
            | MetricKind::OrderExtractionsPerHourRate
            | MetricKind::ExtractedUnitsPerSlice
            | MetricKind::ExtractedUnitsPerHourRate => {
                let key = match self {
                    MetricKind::OrderExtractionsPerSlice | MetricKind::OrderExtractionsPerHourRate => {
                        "order_extract_presentations"
                    }
                    _ => "order_extracted_units",
                };
                let value = m.get(key).copied().ok_or("profile did not record completed extraction work")?;
                if matches!(self, MetricKind::OrderExtractionsPerHourRate | MetricKind::ExtractedUnitsPerHourRate) {
                    if slice_width_ms == 0 { 0.0 } else { value * 3_600_000.0 / slice_width_ms as f64 }
                } else {
                    value
                }
            }
            MetricKind::Presentations => get("presentations"),
            MetricKind::CumulativePresentations => get("presentations"),
            MetricKind::CumulativePresPerStnHr => {
                // num_stations > 0 guaranteed by the unavailable_reason gate.
                let slice_hours = slice_width_ms as f64 / 3_600_000.0;
                if slice_hours > 0.0 { get("presentations") / num_stations as f64 / slice_hours } else { 0.0 }
            }
            // Time-weighted "average # of robots in this state within the slice."
            // Derived as duty_ms / slice_width_ms. Idle = fleet_size − busy
            // (fleet_size from instantaneous state counts is constant). Falls
            // back to instantaneous counts when slice_width_ms is zero
            // (degenerate config) to avoid divide-by-zero.
            MetricKind::VtmIdle => {
                let busy =
                    (get("vtm_duty_traveling_ms") + get("vtm_duty_loading_unloading_ms")) / slice_width_ms as f64;
                let fleet = vtm_fleet_size(m);
                (fleet - busy).max(0.0)
            }
            MetricKind::VtmTraveling => duty_per_slice(get("vtm_duty_traveling_ms"), slice_width_ms),
            MetricKind::VtmLoadingUnloading => duty_per_slice(get("vtm_duty_loading_unloading_ms"), slice_width_ms),
            MetricKind::HtmIdle => {
                let busy = (get("htm_duty_traveling_ms")
                    + get("htm_duty_loading_unloading_ms")
                    + get("htm_duty_processing_ms"))
                    / slice_width_ms as f64;
                let fleet = htm_fleet_size(m);
                (fleet - busy).max(0.0)
            }
            MetricKind::HtmTraveling => duty_per_slice(get("htm_duty_traveling_ms"), slice_width_ms),
            // Carve-out subset of HtmTraveling (rotation portion).
            MetricKind::HtmRotating => duty_per_slice(get("htm_duty_rotating_ms"), slice_width_ms),
            MetricKind::HtmProcessing => duty_per_slice(get("htm_duty_processing_ms"), slice_width_ms),
            // KPI #4/#5/#6 overlays. Sub-slices of travel and station visits —
            // deliberately NOT in the HtmIdle busy sum above (they overlap the
            // coarse buckets; double-counting would deflate idle).
            MetricKind::HtmWaitLoaded => duty_per_slice(get("htm_duty_wait_loaded_ms"), slice_width_ms),
            MetricKind::HtmDwellLoaded => duty_per_slice(get("htm_duty_dwell_loaded_ms"), slice_width_ms),
            MetricKind::HtmPickAtStation => duty_per_slice(get("htm_duty_pick_at_station_ms"), slice_width_ms),
            MetricKind::RelayReservationFailures => get("relay_reservation_failures"),
            // Per-slice instantaneous: how much sim-time elapsed per
            // wall-second while processing *this* slice's events. Noisy
            // (a single GC pause can dominate one slice); pair with
            // `SimSpeedAvg` for the converged overall ratio.
            MetricKind::SimSpeed => {
                let wall_ms = get("wall_clock_duration_ms");
                if wall_ms > 0.0 { slice_width_ms as f64 / wall_ms } else { 0.0 }
            }
            // Running average computed cumulatively in `graph.rs`; the
            // per-slice fallback below is unreachable from the chart path
            // but keeps the match exhaustive if extract is called with a
            // single slice in isolation.
            MetricKind::SimSpeedAvg => {
                let wall_ms = get("wall_clock_duration_ms");
                if wall_ms > 0.0 { slice_width_ms as f64 / wall_ms } else { 0.0 }
            }
            MetricKind::WallClockPerSlice => get("wall_clock_duration_ms") / 1000.0,
            MetricKind::VtmTravelDistance => get("vtm_travel_distance_mm") / 1000.0,
            MetricKind::HtmTravelDistance => get("htm_travel_distance_mm") / 1000.0,
            MetricKind::VtmForwardTrips => get("vtm_forward_trips"),
            MetricKind::VtmReturnTrips => get("vtm_return_trips"),
            MetricKind::HtmPickupTrips => get("htm_pickup_trips"),
            MetricKind::HtmReturnTrips => get("htm_return_trips"),
            MetricKind::VtmForwardCycleAvg => {
                let trips = get("vtm_forward_trips");
                if trips > 0.0 { get("vtm_forward_cycle_ms") / trips / 1000.0 } else { 0.0 }
            }
            MetricKind::VtmReturnCycleAvg => {
                let trips = get("vtm_return_trips");
                if trips > 0.0 { get("vtm_return_cycle_ms") / trips / 1000.0 } else { 0.0 }
            }
            MetricKind::HtmCycleAvg => {
                let trips = get("htm_pickup_trips");
                if trips > 0.0 { get("htm_cycle_ms") / trips / 1000.0 } else { 0.0 }
            }
            MetricKind::HtmOperatorWaitAvg => {
                let count = get("htm_operator_wait_count");
                if count > 0.0 { get("htm_operator_wait_ms") / count / 1000.0 } else { 0.0 }
            }
            MetricKind::HtmRoundTripDistanceAvg => {
                let trips = get("htm_return_trips");
                if trips > 0.0 { get("htm_round_trip_distance_mm") / trips / 1000.0 } else { 0.0 }
            }
            MetricKind::HtmRoundTripTimeAvg => {
                let trips = get("htm_return_trips");
                if trips > 0.0 { get("htm_round_trip_ms") / trips / 1000.0 } else { 0.0 }
            }
            MetricKind::HtmRoundTripSpeed => {
                let time_ms = get("htm_round_trip_ms");
                if time_ms > 0.0 { get("htm_round_trip_distance_mm") / time_ms } else { 0.0 }
            }
            MetricKind::HtmReturnRelayRetries => get("htm_return_relay_retries"),
            MetricKind::OperatorQueueWaits => get("operator_queue_waits"),
            MetricKind::HtmToRelayDistance => get("htm_to_relay_mm") / 1000.0,
            MetricKind::HtmToStationDistance => get("htm_to_station_mm") / 1000.0,
            MetricKind::HtmToReturnDistance => get("htm_to_return_mm") / 1000.0,
            MetricKind::HtmToParkingDistance => get("htm_to_parking_mm") / 1000.0,
            MetricKind::RackOccupancy => get("rack_occupancy"),
            // Ledger-derived per-slice snapshot of total inventory units in
            // the warehouse. Drains on picks, refills on putaways. Sourced
            // from `SimEvent::InventoryDelta` bridged from the inventory
            // service's `ContentsChanged` events.
            MetricKind::CurrentInventory => get("inventory_units"),
            MetricKind::PipelineDepthAvg => get("pipeline_depth_avg"),
            MetricKind::PickStationPicksRate => {
                let slice_hours = slice_width_ms as f64 / 3_600_000.0;
                if slice_hours > 0.0 { get("pick_station_picks") / slice_hours } else { 0.0 }
            }
            MetricKind::PickStationPresentations => get("pick_station_presentations"),
            MetricKind::PutawaysPerHourRate => {
                let slice_hours = slice_width_ms as f64 / 3_600_000.0;
                if slice_hours > 0.0 { get("putaway_station_presentations") / slice_hours } else { 0.0 }
            }
            MetricKind::PutawayItemsPerHourRate => {
                let slice_hours = slice_width_ms as f64 / 3_600_000.0;
                if slice_hours > 0.0 { get("putaway_station_picks") / slice_hours } else { 0.0 }
            }
            MetricKind::PutawayTotesStoredPerHour => {
                let hours = slice_width_ms as f64 / 3_600_000.0;
                if hours > 0.0 { get("putaway_totes_stored") / hours } else { 0.0 }
            }
            MetricKind::CumulativePallets => get("pick_station_orders"),
            MetricKind::HtmWaitAtRelay => get("htm_wait_at_relay_ms") / 1000.0,
            MetricKind::HtmWaitAtRelayAvg => {
                let count = get("htm_relay_pickup_count");
                if count > 0.0 { get("htm_wait_at_relay_ms") / count / 1000.0 } else { 0.0 }
            }
            MetricKind::DivingBoardSlotWait => get("diving_board_slot_wait_ms") / 1000.0,
            MetricKind::DivingBoardSlotWaitAvg => {
                let count = get("diving_board_presentation_count");
                if count > 0.0 { get("diving_board_slot_wait_ms") / count / 1000.0 } else { 0.0 }
            }
            MetricKind::RackFillPercent => {
                // rack_occupancy = filled (laden) slots; numerator and
                // denominator both from the slice map, so chart and run agree
                // by construction. total_rack_slots > 0 (hence rack storage
                // exists) guaranteed by the unavailable_reason gate.
                let capacity = get("rack_capacity");
                if capacity > 0.0 { 100.0 * get("rack_occupancy") / capacity } else { 0.0 }
            }
            MetricKind::RackMaxFillPercent => {
                let capacity = get("rack_capacity");
                if capacity > 0.0 { 100.0 * get("rack_occupancy_peak") / capacity } else { 0.0 }
            }
            // Already a percentage, computed across aisles in the observer.
            MetricKind::RackFillMinPercent => get("rack_fill_min_pct"),
            MetricKind::RackFillMaxPercent => get("rack_fill_max_pct"),
            // Relays holding a deposited tote (numerator) over total relays
            // (denominator), both from the same per-slice map so chart and run
            // agree. 0 relays → 0%.
            MetricKind::RelayOccupancyPercent => {
                let total = get("relay_cells_total");
                if total > 0.0 { 100.0 * get("relay_cells_reserved") / total } else { 0.0 }
            }
            // Reservation busy-ms over (relays × slice_width) — the per-slice
            // decomposition of the relay_utilization_pct summary.
            MetricKind::RelayReservationUtilPercent => {
                let denom = get("relay_cells_total") * slice_width_ms as f64;
                if denom > 0.0 { 100.0 * get("relay_reserved_busy_ms") / denom } else { 0.0 }
            }
        })
    }

    /// Whether this metric should be shown as a running cumulative sum.
    /// `SimSpeedAvg` is rendered as `cumulative_sim_time / cumulative_wall_clock`
    /// in `graph.rs` so it converges to the true overall speedup rather than
    /// swinging with per-slice wall-clock noise; the instantaneous `SimSpeed`
    /// stays raw.
    pub fn is_cumulative(&self) -> bool {
        matches!(
            self,
            MetricKind::CumulativePicks
                | MetricKind::CumulativePutaways
                | MetricKind::CumulativeOrders
                | MetricKind::CumulativePresentations
                | MetricKind::CumulativePickPresentations
                | MetricKind::CumulativePutawayPresentations
                | MetricKind::CumulativePutawayPresentationsDirect
                | MetricKind::CumulativePutawayPresentationsFromRack
                | MetricKind::CumulativePallets
                | MetricKind::SimSpeedAvg
        )
    }

    /// Rolling-window aggregations that make sense for this metric.
    /// Cumulative series are already monotonic, so smoothing has no meaning.
    pub fn allowed_rollings(&self) -> &'static [RollingKind] {
        if self.is_cumulative() {
            &[RollingKind::Off]
        } else if self.supports_trailing_sum() {
            RollingKind::all()
        } else {
            RollingKind::smoothing_only()
        }
    }

    /// Whether each source slice is an additive count, duration, or distance.
    /// Snapshot gauges, rates, percentages, and already-divided averages must
    /// not offer a trailing sum because summing them would invent a metric.
    fn supports_trailing_sum(&self) -> bool {
        matches!(
            self,
            MetricKind::PicksPerSlice
                | MetricKind::PutawaysPerSlice
                | MetricKind::PickPresentationsPerSlice
                | MetricKind::OrderExtractionsPerSlice
                | MetricKind::ExtractedUnitsPerSlice
                | MetricKind::PutawayPresentationsPerSlice
                | MetricKind::PutawayPresentationsDirectPerSlice
                | MetricKind::PutawayPresentationsFromRackPerSlice
                | MetricKind::Presentations
                | MetricKind::RelayReservationFailures
                | MetricKind::WallClockPerSlice
                | MetricKind::VtmTravelDistance
                | MetricKind::HtmTravelDistance
                | MetricKind::VtmForwardTrips
                | MetricKind::VtmReturnTrips
                | MetricKind::HtmPickupTrips
                | MetricKind::HtmReturnTrips
                | MetricKind::HtmReturnRelayRetries
                | MetricKind::OperatorQueueWaits
                | MetricKind::HtmToRelayDistance
                | MetricKind::HtmToStationDistance
                | MetricKind::HtmToReturnDistance
                | MetricKind::HtmToParkingDistance
                | MetricKind::PickStationPresentations
                | MetricKind::HtmWaitAtRelay
                | MetricKind::DivingBoardSlotWait
        )
    }
}

/// `duty_ms / slice_width_ms` — the time-weighted "average # of robots in
/// this state at any moment within the slice." Returns 0 for a
/// zero-width slice (degenerate config).
fn duty_per_slice(duty_ms: f64, slice_width_ms: u64) -> f64 {
    if slice_width_ms == 0 { 0.0 } else { duty_ms / slice_width_ms as f64 }
}

/// VTM fleet size, summed from the instantaneous `vtm_state_*` distribution
/// in the slice's metrics map. Constant across slices in a well-formed run;
/// recomputed per call to keep the helper stateless.
fn vtm_fleet_size(m: &HashMap<String, f64>) -> f64 {
    m.iter().filter(|(k, _)| k.starts_with("vtm_state_")).map(|(_, v)| *v).sum()
}

/// HTM fleet size, summed from the instantaneous `htm_state_*` distribution.
fn htm_fleet_size(m: &HashMap<String, f64>) -> f64 {
    m.iter().filter(|(k, _)| k.starts_with("htm_state_")).map(|(_, v)| *v).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extraction_charts_keep_operations_units_and_legacy_absence_distinct() {
        let mut values =
            HashMap::from([("order_extract_presentations".into(), 2.0), ("order_extracted_units".into(), 7.0)]);
        let extract = |metric: MetricKind, values: &HashMap<String, f64>| metric.extract(values, 60_000, 2, 0, 0);
        assert_eq!(extract(MetricKind::OrderExtractionsPerSlice, &values), Ok(2.0));
        assert_eq!(extract(MetricKind::ExtractedUnitsPerSlice, &values), Ok(7.0));
        assert_eq!(extract(MetricKind::OrderExtractionsPerHourRate, &values), Ok(120.0));
        assert_eq!(extract(MetricKind::ExtractedUnitsPerHourRate, &values), Ok(420.0));
        values.insert("order_extract_presentations".into(), 0.0);
        assert_eq!(extract(MetricKind::OrderExtractionsPerSlice, &values), Ok(0.0));
        assert!(extract(MetricKind::OrderExtractionsPerSlice, &HashMap::new()).is_err());
        assert!(extract(MetricKind::ExtractedUnitsPerSlice, &HashMap::new()).is_err());
        assert!(MetricKind::OrderExtractionsPerSlice.allowed_rollings().contains(&RollingKind::TrailingSum));
        assert!(!MetricKind::OrderExtractionsPerHourRate.allowed_rollings().contains(&RollingKind::TrailingSum));
    }
}
