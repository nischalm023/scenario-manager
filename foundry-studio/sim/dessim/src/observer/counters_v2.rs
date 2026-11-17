//! Metrics V2 sub-counter structs embedded in ScalarCounters.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// HTM travel distance broken down by trip purpose.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TripDistanceCounters {
    /// Distance traveling to pickup relay (relay approach). mm.
    pub htm_to_relay_mm: u64,
    /// Distance traveling to station (loaded). mm.
    pub htm_to_station_mm: u64,
    /// Distance traveling to return relay (loaded return). mm.
    pub htm_to_return_mm: u64,
    /// Distance traveling to parking cell. mm.
    pub htm_to_parking_mm: u64,
}

/// Rack fill tracking. "Fill" = rack slots holding a **laden** (non-empty)
/// tote. Empty totes parked in a slot do not count — a rack wall-to-wall with
/// empty totes reads 0% fill, which is the inventory-health signal the KPI
/// wants. Counted by distinct `(cell, level)` slot, not by tote, so a
/// transient double-book (a new tote recorded in a slot before the previous
/// occupant's departure event lands) can never push fill above capacity.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RackCounters {
    /// Rack slots holding a laden tote, at slice end.
    pub occupancy_total: u32,
    /// Peak of `occupancy_total` seen during this slice.
    pub occupancy_peak: u32,
    /// Total rack tote-slot capacity — the denominator for rack fill %.
    /// Constant across slices (a scenario fact, threaded in via
    /// `MetricsConfig::total_rack_slots`). `0` on rackless scenarios, in
    /// which case fill % renders 0. Emitted as the `rack_capacity` key so the
    /// fill-% chart reads occupancy and capacity from the same per-slice map
    /// rather than a side-band denominator that can drift.
    pub capacity_total: u32,
    /// Per-aisle (relay zone) laden-slot count at slice end: zone_id → filled
    /// slots. `BTreeMap` for a stable key order so the derived
    /// `rack_occupancy_zone_{id}` time-series columns are deterministic.
    pub per_zone_occupancy: BTreeMap<u32, u32>,
    /// Spread of fill % across aisles at slice end. `min`/`max` bracket the
    /// warehouse fill % — plotted together they form a band whose width is
    /// the imbalance (one aisle draining while others stay full). `stddev` is
    /// the population standard deviation of per-aisle fill %, the precise
    /// spread for the xlsx summary. All `0.0` when there are no rack aisles.
    pub fill_min_pct: f64,
    pub fill_max_pct: f64,
    pub fill_stddev_pct: f64,
}

/// Station-type-aware throughput counters.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ThroughputCounters {
    /// Picks completed at Pick-type stations.
    pub pick_station_picks: u64,
    /// Presentations at Pick-type stations.
    pub pick_station_presentations: u64,
    /// Orders completed at Pick-type stations (= pallets completed).
    pub pick_station_orders: u64,
    /// Picks completed at Putaway-type stations.
    pub putaway_station_picks: u64,
    /// Presentations at Putaway-type stations.
    pub putaway_station_presentations: u64,
    /// Putaway presentations whose tote arrived chained from another station
    /// (HTM already carrying — no rack round-trip).
    pub putaway_station_presentations_direct: u64,
    /// Putaway presentations whose tote was staged at a relay by a VTM from
    /// rack and picked up by the delivering HTM there.
    pub putaway_station_presentations_from_rack: u64,
    /// Putaway-bound HTM dispatches (the HTM starts moving,
    /// `HtmAssignmentEvent`) whose chain sources a standing direct empty.
    pub putaway_dispatches_direct: u64,
    /// Putaway-bound HTM dispatches sourced by a rack retrieve (the HTM
    /// departs toward its relay pickup).
    pub putaway_dispatches_from_rack: u64,
}

/// Pipeline depth snapshots (from PipelineSnapshotEvent).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PipelineCounters {
    /// Sum of pipeline depths across all snapshots in this slice.
    /// Average = pipeline_depth_sum / pipeline_snapshot_count.
    pub pipeline_depth_sum: u64,
    /// Number of pipeline snapshots in this slice.
    pub pipeline_snapshot_count: u64,
}
