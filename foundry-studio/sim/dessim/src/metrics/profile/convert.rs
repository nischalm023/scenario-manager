//! Within `profile`, `convert` is the FLATTENING: the observer's typed
//! counters become the string-keyed maps a time series is made of.
//!
//! The observer counts into named Rust fields, which is fast and says
//! nothing about itself; a chart, a workbook column and a recording's scalars
//! all need a key they can name. [`flatten_scalars`] and [`grid_to_dynamic`]
//! are where a field's name becomes that key, and they are the only place it
//! happens, so a key never drifts between producers. Each scalar key is
//! written with its class on the line that writes it, so a recording learns
//! whether a run's slices of a key add up from the same line that names it.
//! A family of keys a reader turns back into a typed figure is read back here
//! too, beside where it is written: [`relay_zone_busy_lanes`].

use std::collections::HashMap;
use std::hash::BuildHasher;

use desgraph::ZoneId;

use super::{DynamicGrid, SliceRecord};
use crate::model::relay::{RelayUtilizationTimeline, RelayZoneUtilizationLane};
use crate::observer::time_slice::{GridCounters, ScalarCounters};

// ---------------------------------------------------------------------------
// Conversion: ScalarCounters -> keyed values, each with its class
// ---------------------------------------------------------------------------

/// Whether a run's slices of a scalar add up, so that a recording carries
/// each slice's value onto the run's running total or in place of the reading
/// before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScalarClass {
    /// The slice's own share of a quantity the run totals: picks completed,
    /// busy time, distance travelled.
    Sum,
    /// A value a run's slices do not add up to: what stands at an instant
    /// (slots held, units in stock, robots per activity), or an average or a
    /// peak over the slice.
    Reading,
}

/// One scalar as a recording takes it: its key, its value, and the class the
/// recording declares it with.
#[derive(Debug)]
pub struct ClassedScalar {
    pub key: String,
    pub value: f64,
    pub class: ScalarClass,
}

/// The counters flattened for a recording: each key once, with its value and
/// its class.
pub fn classed_scalars(s: &ScalarCounters) -> Vec<ClassedScalar> {
    let mut scalars = Vec::new();
    flatten_scalars(s, &mut scalars);
    scalars
}

/// The counters flattened for the run model, whose slices keep each key's
/// value and have no use for its class.
pub fn scalars_to_map(s: &ScalarCounters) -> HashMap<String, f64> {
    let mut values = HashMap::new();
    flatten_scalars(s, &mut values);
    values
}

/// What the flattening hands each key to, so that one flattening serves a
/// recording, which keeps a key's value and class, and the run model, which
/// keeps the value alone. A key is handed over once, never without a class,
/// and through `sum` or `reading`, so its class sits on the line that spells
/// it.
trait ScalarCollector {
    fn keep(&mut self, key: String, value: f64, class: ScalarClass);

    fn sum(&mut self, key: impl Into<String>, value: f64) {
        self.keep(key.into(), value, ScalarClass::Sum);
    }

    fn reading(&mut self, key: impl Into<String>, value: f64) {
        self.keep(key.into(), value, ScalarClass::Reading);
    }
}

impl ScalarCollector for Vec<ClassedScalar> {
    fn keep(&mut self, key: String, value: f64, class: ScalarClass) {
        self.push(ClassedScalar { key, value, class });
    }
}

impl ScalarCollector for HashMap<String, f64> {
    fn keep(&mut self, key: String, value: f64, _class: ScalarClass) {
        self.insert(key, value);
    }
}

/// Hand `m` every key the counters state, each once, with its value and its
/// class.
fn flatten_scalars(s: &ScalarCounters, m: &mut impl ScalarCollector) {
    m.sum("picks_completed", s.picks_completed as f64);
    m.sum("putaways_completed", s.putaways_completed as f64);
    m.sum("putaway_totes_stored", s.putaway_totes_stored as f64);
    m.sum("presentations", s.presentations as f64);
    m.sum("pick_presentations", s.pick_presentations as f64);
    m.sum("putaway_presentations", s.putaway_presentations as f64);
    m.sum("order_load_presentations", s.order_load_presentations as f64);
    m.sum("order_extract_presentations", s.order_extract_presentations as f64);
    m.sum("order_extracted_units", s.order_extracted_units as f64);
    m.sum("orders_completed", s.orders_completed as f64);
    m.sum("backpressure_events", s.backpressure_events as f64);
    m.sum("tote_claim_successes", s.tote_claim_successes as f64);
    m.sum("tote_claim_failures", s.tote_claim_failures as f64);
    m.sum("relay_reservation_successes", s.relay_reservation_successes as f64);
    m.sum("relay_reservation_failures", s.relay_reservation_failures as f64);
    m.sum("pathfinder_queries", s.pathfinder_queries as f64);
    m.sum("pathfinder_failures", s.pathfinder_failures as f64);
    m.sum("vtm_forward_matches", s.vtm_forward_matches as f64);
    m.sum("vtm_return_matches", s.vtm_return_matches as f64);
    m.sum("vtm_cross_zone_assignments", s.vtm_cross_zone_assignments as f64);
    m.sum("vtm_scoring_duration_ns", s.vtm_scoring_duration_ns as f64);
    m.sum("starvation_age_bonus_applied", s.starvation_age_bonus_applied as f64);
    m.sum("deadlock_headroom_triggered", s.deadlock_headroom_triggered as f64);
    m.sum("wall_clock_duration_ms", s.wall_clock_duration_ms);

    // Travel distance.
    m.sum("vtm_travel_distance_mm", s.vtm_travel_distance_mm as f64);
    m.sum("htm_travel_distance_mm", s.htm_travel_distance_mm as f64);

    // Trip completion counts.
    m.sum("vtm_forward_trips", s.vtm_forward_trips as f64);
    m.sum("vtm_return_trips", s.vtm_return_trips as f64);
    m.sum("htm_pickup_trips", s.htm_pickup_trips as f64);
    m.sum("htm_return_trips", s.htm_return_trips as f64);

    // Cycle durations (cumulative).
    m.sum("vtm_forward_cycle_ms", s.vtm_forward_cycle_ms as f64);
    m.sum("vtm_return_cycle_ms", s.vtm_return_cycle_ms as f64);
    m.sum("htm_cycle_ms", s.htm_cycle_ms as f64);
    m.sum("htm_operator_wait_ms", s.htm_operator_wait_ms as f64);
    m.sum("htm_operator_wait_count", s.htm_operator_wait_count as f64);

    // Round trip distance and time.
    m.sum("htm_round_trip_distance_mm", s.htm_round_trip_distance_mm);
    m.sum("htm_round_trip_ms", s.htm_round_trip_ms as f64);

    // Congestion.
    m.sum("htm_return_relay_retries", s.htm_return_relay_retries as f64);
    m.sum("operator_queue_waits", s.operator_queue_waits as f64);

    // Computed averages (convenience keys).
    if s.vtm_forward_trips > 0 {
        m.reading("vtm_forward_cycle_avg_ms", s.vtm_forward_cycle_ms as f64 / s.vtm_forward_trips as f64);
    }
    if s.vtm_return_trips > 0 {
        m.reading("vtm_return_cycle_avg_ms", s.vtm_return_cycle_ms as f64 / s.vtm_return_trips as f64);
    }
    if s.htm_pickup_trips > 0 {
        m.reading("htm_cycle_avg_ms", s.htm_cycle_ms as f64 / s.htm_pickup_trips as f64);
    }
    if s.htm_operator_wait_count > 0 {
        m.reading("htm_operator_wait_avg_ms", s.htm_operator_wait_ms as f64 / s.htm_operator_wait_count as f64);
    }
    if s.htm_return_trips > 0 {
        m.reading("htm_round_trip_avg_distance_mm", s.htm_round_trip_distance_mm / s.htm_return_trips as f64);
        m.reading("htm_round_trip_avg_ms", s.htm_round_trip_ms as f64 / s.htm_return_trips as f64);
    }

    // Trip distance by purpose.
    m.sum("htm_to_relay_mm", s.trip_distance.htm_to_relay_mm as f64);
    m.sum("htm_to_station_mm", s.trip_distance.htm_to_station_mm as f64);
    m.sum("htm_to_return_mm", s.trip_distance.htm_to_return_mm as f64);
    m.sum("htm_to_parking_mm", s.trip_distance.htm_to_parking_mm as f64);

    // Rack fill = slots holding a laden (non-empty) tote; empty totes don't
    // count. Numerator (`rack_occupancy`) and denominator (`rack_capacity`)
    // are read from the same per-slice map. `rack_occupancy_peak` is the
    // per-slice max (the "max fill" numerator).
    m.reading("rack_occupancy", s.rack.occupancy_total as f64);
    m.reading("rack_occupancy_peak", s.rack.occupancy_peak as f64);
    m.reading("rack_capacity", s.rack.capacity_total as f64);
    // Per-aisle fill spread. min/max bracket the warehouse fill % (a band on
    // the chart); stddev is the precise imbalance for the xlsx (no chart
    // MetricKind — Time Series column only).
    m.reading("rack_fill_min_pct", s.rack.fill_min_pct);
    m.reading("rack_fill_max_pct", s.rack.fill_max_pct);
    m.reading("rack_fill_stddev_pct", s.rack.fill_stddev_pct);
    // Per-aisle fill → Time Series columns (no dedicated MetricKind).
    for (&zone_id, &occ) in &s.rack.per_zone_occupancy {
        m.reading(format!("rack_occupancy_zone_{zone_id}"), occ as f64);
    }

    // Warehouse inventory units (ledger-derived snapshot at slice end).
    m.reading("inventory_units", s.inventory_units as f64);

    // Relay occupancy: relays holding a deposited tote awaiting HTM pickup
    // (numerator) over total relays (denominator), both from the same slice.
    m.reading("relay_cells_reserved", s.relay_cells_reserved as f64);
    m.reading("relay_cells_total", s.relay_cells_total as f64);
    // Relay reservation busy-ms this slice (planner ledger). Divide by
    // relay_cells_total * slice_width_ms for the reservation utilization %.
    m.sum("relay_reserved_busy_ms", s.relay_reserved_busy_ms as f64);
    for (&zone, &busy) in &s.relay_reserved_busy_per_zone {
        m.sum(relay_zone_busy_key(zone), busy.as_ms() as f64);
    }

    // Station-type throughput.
    m.sum("pick_station_picks", s.throughput.pick_station_picks as f64);
    m.sum("pick_station_presentations", s.throughput.pick_station_presentations as f64);
    m.sum("pick_station_orders", s.throughput.pick_station_orders as f64);
    m.sum("putaway_station_picks", s.throughput.putaway_station_picks as f64);
    m.sum("putaway_station_presentations", s.throughput.putaway_station_presentations as f64);
    m.sum("putaway_station_presentations_direct", s.throughput.putaway_station_presentations_direct as f64);
    m.sum("putaway_station_presentations_from_rack", s.throughput.putaway_station_presentations_from_rack as f64);
    m.sum("putaway_dispatches_direct", s.throughput.putaway_dispatches_direct as f64);
    m.sum("putaway_dispatches_from_rack", s.throughput.putaway_dispatches_from_rack as f64);

    // Pipeline depth. `pipeline_depth_avg` is inserted unconditionally (0.0
    // when no snapshots landed in this slice) so downstream consumers can
    // panic on missing keys without a special case for "no pipeline activity
    // this slice."
    m.sum("pipeline_depth_sum", s.pipeline.pipeline_depth_sum as f64);
    m.sum("pipeline_snapshot_count", s.pipeline.pipeline_snapshot_count as f64);
    let pipeline_depth_avg = if s.pipeline.pipeline_snapshot_count > 0 {
        s.pipeline.pipeline_depth_sum as f64 / s.pipeline.pipeline_snapshot_count as f64
    } else {
        0.0
    };
    m.reading("pipeline_depth_avg", pipeline_depth_avg);

    // Per-station picks.
    for (&station_id, &count) in &s.picks_per_station {
        m.sum(format!("picks_station_{}", station_id.0), count as f64);
    }

    // Robots per activity kind at the slice's start.
    for (&kind, &count) in &s.vtm_activity_counts {
        m.reading(format!("vtm_state_{}", kind.column_suffix()), count as f64);
    }
    for (&kind, &count) in &s.htm_activity_counts {
        m.reading(format!("htm_state_{}", kind.column_suffix()), count as f64);
    }

    // Fleet duty aggregates (sum across robots of this type, in ms).
    // Dividing by slice_width_ms in the consumer yields "average number of
    // robots in this state at any moment within the slice."
    m.sum("vtm_duty_traveling_ms", s.vtm_duty_traveling_ms as f64);
    m.sum("vtm_duty_loading_unloading_ms", s.vtm_duty_loading_unloading_ms as f64);
    m.sum("htm_duty_traveling_ms", s.htm_duty_traveling_ms as f64);
    m.sum("htm_duty_rotating_ms", s.htm_duty_rotating_ms as f64);
    m.sum("htm_duty_loading_unloading_ms", s.htm_duty_loading_unloading_ms as f64);
    m.sum("htm_duty_processing_ms", s.htm_duty_processing_ms as f64);

    // Loaded-time KPI overlays (#4/#5/#6). Overlap the coarse buckets above;
    // see ScalarCounters docs. Must be unconditional — metric_extract panics
    // on a missing key.
    m.sum("htm_duty_wait_loaded_ms", s.htm_duty_wait_loaded_ms as f64);
    m.sum("htm_duty_dwell_loaded_ms", s.htm_duty_dwell_loaded_ms as f64);
    m.sum("htm_duty_pick_at_station_ms", s.htm_duty_pick_at_station_ms as f64);

    // HtmPickup dwell-at-relay (HTM waiting for the tote to be deposited).
    m.sum("htm_wait_at_relay_ms", s.htm_wait_at_relay_ms as f64);
    m.sum("htm_relay_pickup_count", s.htm_relay_pickup_count as f64);

    // Empty diving-board slot wait (KPI row 24).
    m.sum("diving_board_slot_wait_ms", s.diving_board_slot_wait_ms as f64);
    m.sum("diving_board_presentation_count", s.diving_board_presentation_count as f64);
}

/// The key a relay zone's busy time is flattened to, and read back by.
fn relay_zone_busy_key(zone: ZoneId) -> String {
    format!("relay_reserved_busy_zone_{}_ms", zone.0)
}

/// Each relay zone's busy lane over `slices`, as the slices state it: what
/// desvis's Gantt draws a run's relay lanes from, live on the slices received
/// so far and on playback on a recording's. `relays_per_zone` names each relay
/// zone with its relay count, as the run's map states them; the lanes come out
/// in zone order.
///
/// `None` when a slice states no busy time for one of the zones. Every slice
/// the observer makes final states every relay zone's, so such a slice is
/// from a recording written before slices stated it, or from another run than
/// the map's, and no lane drawn over it would be true.
pub fn relay_zone_busy_lanes(
    slices: &[SliceRecord],
    slice_width_ms: u64,
    relays_per_zone: impl IntoIterator<Item = (ZoneId, u32)>,
) -> Option<RelayUtilizationTimeline> {
    let mut zones = relays_per_zone
        .into_iter()
        .map(|(zone, relays)| {
            let key = relay_zone_busy_key(zone);
            let busy_ms =
                slices.iter().map(|slice| slice.metrics.get(&key).map(|&ms| ms as u64)).collect::<Option<_>>()?;
            Some(RelayZoneUtilizationLane { zone_id: zone.0, relays, busy_ms })
        })
        .collect::<Option<Vec<_>>>()?;
    zones.sort_unstable_by_key(|lane| lane.zone_id);
    Some(RelayUtilizationTimeline { slice_width_ms, zones })
}

// ---------------------------------------------------------------------------
// Conversion: GridCounters -> DynamicGrid
// ---------------------------------------------------------------------------

/// Convert typed GridCounters to a dynamic grid map.
pub fn grid_to_dynamic(g: &GridCounters) -> DynamicGrid {
    let mut d = HashMap::new();

    if !g.nav_visits_vtm.is_empty() {
        d.insert("nav_visits_vtm".into(), u32_to_u64_map(&g.nav_visits_vtm));
    }
    if !g.nav_visits_htm.is_empty() {
        d.insert("nav_visits_htm".into(), u32_to_u64_map(&g.nav_visits_htm));
    }
    // `robot_presence_*` are FxHashMap-backed in `GridCounters`; the
    // serialization-facing `DynamicGrid` is a std HashMap. Convert once at
    // slice-flush time — the per-slice cost is negligible compared to
    // serialization, and decouples the hot-path field type from the
    // wire-format type.
    if !g.robot_presence_vtm_ms.is_empty() {
        d.insert("robot_presence_vtm_ms".into(), g.robot_presence_vtm_ms.iter().map(|(&k, &v)| (k, v)).collect());
    }
    if !g.robot_presence_htm_ms.is_empty() {
        d.insert("robot_presence_htm_ms".into(), g.robot_presence_htm_ms.iter().map(|(&k, &v)| (k, v)).collect());
    }

    d
}

fn u32_to_u64_map<S: BuildHasher>(m: &HashMap<u32, u32, S>) -> HashMap<u32, u64> {
    m.iter().map(|(&k, &v)| (k, v as u64)).collect()
}

// ---------------------------------------------------------------------------
// Convenience: extract known metrics from dynamic map
// ---------------------------------------------------------------------------

impl SliceRecord {
    /// Get a metric value by key. Panics if the key is missing: every key
    /// callers read must be emitted unconditionally by `scalars_to_map`
    /// above — a silent 0.0 fallback masked too many "stale key" drift
    /// bugs to keep around.
    pub fn get(&self, key: &str) -> f64 {
        *self
            .metrics
            .get(key)
            .unwrap_or_else(|| panic!("SliceRecord missing key {key:?} — emit it from scalars_to_map"))
    }
}

/// Extract VTM/HTM visit counts from a DynamicGrid for heatmap rendering.
///
/// Returns (vtm_layer, htm_layer) as references to the sparse maps.
pub fn grid_visit_layers(grid: &DynamicGrid) -> (Option<&HashMap<u32, u64>>, Option<&HashMap<u32, u64>>) {
    (grid.get("nav_visits_vtm"), grid.get("nav_visits_htm"))
}

#[cfg(test)]
mod tests {
    use desgraph::StationId;
    use rustc_hash::FxHashMap;

    use super::*;
    use crate::model::SimDuration;
    use crate::observer::activity::ActivityKind;

    /// A slice as the run model holds it, stating `per_zone`'s relay busy time.
    fn slice(start_ms: u64, per_zone: &[(u32, u64)]) -> SliceRecord {
        let per_zone = per_zone.iter().map(|&(zone, ms)| (ZoneId(zone), SimDuration::ms(ms))).collect();
        let scalars = ScalarCounters { relay_reserved_busy_per_zone: per_zone, ..Default::default() };
        SliceRecord {
            start_ms,
            metrics: scalars_to_map(&scalars),
            grid: DynamicGrid::default(),
            bot_locations: FxHashMap::default(),
            relay_state: Default::default(),
            tote_state: Default::default(),
            station_diag: Default::default(),
        }
    }

    /// Each zone's lane reads back the figure every slice states for it, with
    /// the relay count the map gives the zone, in zone order whatever order
    /// the map names the zones in.
    #[test]
    fn each_zones_lane_reads_back_its_figure_in_every_slice() {
        let slices = [slice(0, &[(4, 300), (2, 0)]), slice(10_000, &[(4, 0), (2, 7_500)])];
        let timeline = relay_zone_busy_lanes(&slices, 10_000, [(ZoneId(4), 3), (ZoneId(2), 1)]).expect("both stated");
        assert_eq!(timeline.slice_width_ms, 10_000);
        let lanes: Vec<(u32, u32, Vec<u64>)> =
            timeline.zones.into_iter().map(|lane| (lane.zone_id, lane.relays, lane.busy_ms)).collect();
        assert_eq!(lanes, [(2, 1, vec![0, 7_500]), (4, 3, vec![300, 0])]);
    }

    /// An idle zone's figure is stated, as zero. A zone the map names that
    /// any slice states no figure for, the first or a later one, is a run and
    /// a map that disagree, never an idle zone, and gives no lanes.
    #[test]
    fn a_zone_any_slice_states_no_figure_for_gives_no_lanes() {
        let zones = [(ZoneId(4), 3), (ZoneId(5), 2)];
        let idle = relay_zone_busy_lanes(&[slice(0, &[(4, 300), (5, 0)])], 10_000, zones).expect("both stated");
        assert_eq!(idle.zones[1].busy_ms, [0], "an idle zone's lane");
        assert!(relay_zone_busy_lanes(&[slice(0, &[(4, 300)])], 10_000, zones).is_none(), "the first slice");
        let later = [slice(0, &[(4, 300), (5, 0)]), slice(10_000, &[(4, 300)])];
        assert!(relay_zone_busy_lanes(&later, 10_000, zones).is_none(), "a later slice");
    }

    /// One case: what it states on counters that otherwise state nothing.
    type Stating = fn(&mut ScalarCounters);

    /// The classes of the keys the flattening writes only once `stating` has
    /// stated its case: one family's keys, whatever they are named, or the
    /// averages a count makes possible.
    fn classes_added(stating: Stating) -> Vec<ScalarClass> {
        let mut counters = ScalarCounters::default();
        stating(&mut counters);
        let stating_nothing = scalars_to_map(&ScalarCounters::default());
        classed_scalars(&counters)
            .into_iter()
            .filter(|scalar| !stating_nothing.contains_key(&scalar.key))
            .map(|scalar| scalar.class)
            .collect()
    }

    /// Each key is written once, with its class, so a recording declares it
    /// once, as what it is. What stands at an instant, per aisle and per
    /// activity too, and every average or peak is a reading; what a slice
    /// contributes, per station and per relay zone too, is a sum. A family
    /// writes a key only for each member its counters state, and an average
    /// only once its count is stated, so each case is stated here and found by
    /// the keys it adds, whatever they are named.
    #[test]
    fn every_key_is_written_with_its_class() {
        use ScalarClass::{Reading, Sum};
        let cases: [(&str, Stating, &[ScalarClass]); 10] = [
            ("VTMs per activity", |s| _ = s.vtm_activity_counts.insert(ActivityKind::Gap, 4), &[Reading]),
            ("HTMs per activity", |s| _ = s.htm_activity_counts.insert(ActivityKind::Travel, 6), &[Reading]),
            ("slots held per aisle", |s| _ = s.rack.per_zone_occupancy.insert(3, 7), &[Reading]),
            ("picks per station", |s| _ = s.picks_per_station.insert(StationId(2), 5), &[Sum]),
            (
                "busy time per relay zone",
                |s| _ = s.relay_reserved_busy_per_zone.insert(ZoneId(4), SimDuration::ms(9)),
                &[Sum],
            ),
            ("the average forward cycle", |s| s.vtm_forward_trips = 1, &[Reading]),
            ("the average return cycle", |s| s.vtm_return_trips = 1, &[Reading]),
            ("the average HTM cycle", |s| s.htm_pickup_trips = 1, &[Reading]),
            ("the average operator wait", |s| s.htm_operator_wait_count = 1, &[Reading]),
            ("the average round trip's time and distance", |s| s.htm_return_trips = 1, &[Reading, Reading]),
        ];
        let mut every_case = ScalarCounters::default();
        for (case, stating, classes) in cases {
            assert_eq!(classes_added(stating), classes, "{case}");
            stating(&mut every_case);
        }
        let flattened = classed_scalars(&every_case);
        let classed: FxHashMap<&str, ScalarClass> =
            flattened.iter().map(|scalar| (scalar.key.as_str(), scalar.class)).collect();
        assert_eq!(classed.len(), flattened.len(), "each key is written once");
        for key in [
            "rack_occupancy",
            "rack_occupancy_peak",
            "rack_capacity",
            "rack_fill_min_pct",
            "rack_fill_max_pct",
            "rack_fill_stddev_pct",
            "inventory_units",
            "relay_cells_reserved",
            "relay_cells_total",
            "pipeline_depth_avg",
        ] {
            assert_eq!(classed.get(key), Some(&Reading), "{key}");
        }
        for key in ["picks_completed", "relay_reserved_busy_ms", "pipeline_depth_sum", "wall_clock_duration_ms"] {
            assert_eq!(classed.get(key), Some(&Sum), "{key}");
        }
    }
}
