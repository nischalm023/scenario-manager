//! Unit tests for the relay-utilization per-slice bucketing — the pure
//! interval → slice-bucket math the heat-lane timeline is built from.

use crate::model::relay::{
    RelayUtilizationTimeline, RelayZoneUtilizationLane, bucket_busy_interval, bucket_merged_intervals,
};

#[test]
fn bucket_splits_interval_at_slice_boundaries() {
    let mut busy = vec![0u64; 4];
    // [500, 2500) over 1000ms slices: 500 in slice 0, 1000 in slice 1, 500 in slice 2.
    bucket_busy_interval(&mut busy, 500, 2500, 1000);
    assert_eq!(busy, vec![500, 1000, 500, 0]);
}

#[test]
fn bucket_clamps_to_tracked_horizon_and_ignores_empty() {
    let mut busy = vec![0u64; 2];
    // Runs past the last bucket: the overflow is dropped, not wrapped.
    bucket_busy_interval(&mut busy, 1500, 9000, 1000);
    assert_eq!(busy, vec![0, 500]);
    // Zero-length and inverted intervals are no-ops.
    bucket_busy_interval(&mut busy, 700, 700, 1000);
    bucket_busy_interval(&mut busy, 900, 200, 1000);
    assert_eq!(busy, vec![0, 500]);
}

#[test]
fn merged_intervals_count_overlap_once() {
    let mut busy = vec![0u64; 4];
    // [0,1500) and [1000,2500) overlap on [1000,1500). Merged to [0,2500), the
    // overlap is counted once: slice 1 caps at its full 1000ms, not 1500.
    bucket_merged_intervals(&mut busy, vec![(0, 1500), (1000, 2500)], 1000);
    assert_eq!(busy, vec![1000, 1000, 500, 0]);
}

#[test]
fn merged_intervals_join_touching_and_absorb_contained() {
    let mut busy = vec![0u64; 3];
    // Touching ([0,1000) then [1000,2000)) join into one run; a contained
    // interval ([1200,1500) inside [1000,2000)) adds nothing beyond it.
    // Out-of-order input is sorted first.
    bucket_merged_intervals(&mut busy, vec![(1000, 2000), (1200, 1500), (0, 1000)], 1000);
    assert_eq!(busy, vec![1000, 1000, 0]);
}

#[test]
fn merged_intervals_keep_disjoint_runs_separate() {
    let mut busy = vec![0u64; 3];
    // A gap between [0,1000) and [2000,3000) leaves slice 1 empty.
    bucket_merged_intervals(&mut busy, vec![(2000, 3000), (0, 1000)], 1000);
    assert_eq!(busy, vec![1000, 0, 1000]);
}

#[test]
fn timeline_percentages_are_busy_over_relays_times_horizon() {
    // Two equal-length zones; overall and per-zone % must be exactly
    // busy / (relays × horizon), so the heat-lane labels and the summary the
    // chart integrates to can't drift from the underlying busy-ms buckets.
    let timeline = RelayUtilizationTimeline {
        slice_width_ms: 1000,
        zones: vec![
            // 600ms busy over 2 relays × 3000ms horizon = 10%.
            RelayZoneUtilizationLane { zone_id: 1, relays: 2, busy_ms: vec![100, 200, 300] },
            // 900ms busy over 1 relay × 3000ms horizon = 30%.
            RelayZoneUtilizationLane { zone_id: 2, relays: 1, busy_ms: vec![600, 0, 300] },
        ],
    };
    let per_zone = timeline.per_zone_pct();
    assert_eq!(per_zone, vec![(1, 10.0), (2, 30.0)]);
    // Overall: 1500ms busy over (2+1) relays × 3000ms horizon = 16.67%.
    assert!((timeline.overall_pct() - (1500.0 / 9000.0 * 100.0)).abs() < 1e-9);
}
