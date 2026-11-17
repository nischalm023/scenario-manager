//! Tests for the loaded-time HTM KPI overlays — #4 Wait Loaded
//! (`htm_duty_wait_loaded_ms`), #5 Dwell Loaded (`htm_duty_dwell_loaded_ms`),
//! #6 Pick-at-station (`htm_duty_pick_at_station_ms`). Each is an additive
//! sub-slice of a coarse duty bucket, driven by `CycleComplete` (pick dwell)
//! and loaded `Wait` waypoints on `Nav` (by leg). A station visit never
//! reaches #5: it ends at its departure or, when the run stops inside it, at
//! the current world time or its recorded dwell's end if that is later, and a
//! station cell reservation past that has no simulation-time end and buys no
//! occupancy (ruling #49). A committed route wait on the presentation cell is
//! a different fact and does count.

use std::sync::Arc;
use std::sync::mpsc;

use rustc_hash::FxHashMap;

use desgraph::{CellId, CellKind, RelayId, RobotType, StationId, StationType, WorldPos};

use crate::Orientation;
use crate::events::*;
use crate::model::{OrderId, RobotId, SimDuration, SimTime, SkuId, StationArrivalSource};
use crate::observer::activity::ActivityViolation;
use crate::observer::output::MetricsOutput;
use crate::observer::time_slice::{MetricsConfig, ScalarCounters};
use crate::pathfinding::{Waypoint, WaypointAction};

use super::{SimObserver, StreamViolation};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn observe(events: Vec<SimEvent>, mut config: MetricsConfig) -> Result<MetricsOutput, StreamViolation> {
    super::test_models::bind_legacy_nav_models(&mut config, &events);
    let (tx, rx) = mpsc::channel();
    for event in events {
        tx.send(event).unwrap();
    }
    drop(tx);
    SimObserver::new(config, rx).run()
}

fn run(events: Vec<SimEvent>, config: MetricsConfig) -> MetricsOutput {
    observe(events, config).expect("the facts break no rule of the event stream")
}

/// 10s slices; `pick` and/or `putaway` seed a single station of that type.
fn config(pick: Option<u32>, putaway: Option<u32>) -> MetricsConfig {
    let mut station_types = FxHashMap::default();
    if let Some(s) = pick {
        station_types.insert(StationId(s), StationType::Pick);
    }
    if let Some(s) = putaway {
        station_types.insert(StationId(s), StationType::Putaway);
    }
    MetricsConfig { slice_width_ms: 10_000, station_types, ..MetricsConfig::default() }
}

fn seed_htm(robot: u32) -> SimEvent {
    SimEvent::InitialPositions(InitialPositionsEvent {
        positions: vec![(RobotId(robot), CellId(0), RobotType::Htm, WorldPos::default())],
        htm_headings_rad: Vec::new(),
    })
}

/// The arrival that opens a station visit.
fn arrival(robot: u32, station: u32, time_ms: u64) -> SimEvent {
    SimEvent::HtmArrival(HtmArrivalEvent {
        robot_id: RobotId(robot),
        station_id: StationId(station),
        arrival: SimTime::ms(time_ms),
    })
}

fn trip(robot: u32, purpose: TripPurpose, time_ms: u64) -> SimEvent {
    SimEvent::TripStart(TripStartEvent {
        robot_id: RobotId(robot),
        robot_type: RobotType::Htm,
        purpose,
        presentation_id: None,
        from: None,
        to: TripEndpoint::Parking,
        sim_time_ms: time_ms,
    })
}

fn wait_wp(start_ms: u64, end_ms: u64) -> Waypoint {
    Waypoint {
        cell: CellId(0),
        orientation: Orientation::East,
        action: WaypointAction::Wait { start: SimTime::ms(start_ms), end: SimTime::ms(end_ms) },
    }
}

fn move_wp(start_ms: u64, end_ms: u64) -> Waypoint {
    Waypoint {
        cell: CellId(1),
        orientation: Orientation::East,
        action: WaypointAction::Move {
            start: SimTime::ms(start_ms),
            end: SimTime::ms(end_ms),
            entry_velocity_mm_s: 0.0,
            exit_velocity_mm_s: 0.0,
        },
    }
}

fn nav(robot: u32, carrying: bool, time_ms: u64, waypoints: Vec<Waypoint>) -> SimEvent {
    SimEvent::Nav(NavEventData {
        robot_id: RobotId(robot),
        robot_type: RobotType::Htm,
        waypoints,
        time: SimTime::ms(time_ms),
        distance_mm: 0,
        carrying_cargo: carrying,
    })
}

fn cycle(robot: u32, kind: CycleKind, station: u32, time_ms: u64, dwell_ms: u64) -> SimEvent {
    let arrival_source = match kind {
        CycleKind::Putaway { .. } => Some(StationArrivalSource::PutawayDirect),
        CycleKind::Pick { .. } | CycleKind::OrderLoad { .. } | CycleKind::OrderExtract { .. } => None,
    };
    SimEvent::CycleComplete(CycleCompleteEvent {
        robot_id: RobotId(robot),
        robot_type: RobotType::Htm,
        kind,
        station_id: StationId(station),
        sim_time_ms: time_ms,
        dwell_ms,
        arrival_source,
        dc_id: None,
        presentation_id: None,
        tote_presentation: None,
    })
}

fn pick(qty: u32) -> CycleKind {
    CycleKind::Pick { sku: SkuId(0), qty }
}

fn slice0(out: &MetricsOutput) -> &ScalarCounters {
    &out.time_slices[0].scalars
}

// ---------------------------------------------------------------------------
// #6 Pick-at-station
// ---------------------------------------------------------------------------

#[test]
fn pick_dwell_counts_at_pick_station() {
    let out = run(vec![cycle(0, pick(1), 7, 2_000, 5_000)], config(Some(7), None));
    assert_eq!(slice0(&out).htm_duty_pick_at_station_ms, 5_000);
    assert_eq!(out.cumulative_scalars.htm_duty_pick_at_station_ms, 5_000);
}

#[test]
fn putaway_dwell_excluded_from_pick_at_station() {
    let out = run(vec![cycle(0, CycleKind::Putaway { sku: SkuId(0), qty: 1 }, 9, 2_000, 4_500)], config(None, Some(9)));
    assert_eq!(slice0(&out).htm_duty_pick_at_station_ms, 0);
}

#[test]
fn pick_dwell_clips_across_slice_boundary() {
    // Dwell [8000, 12000) over 10s slices => 2000 in slice 0, 2000 in slice 1
    // (`sim_time_ms` is the dwell START) — same clipping as overlays #4/#5, so
    // a slice's #6 can never exceed that slice's clipped Processing parent.
    let out = run(vec![cycle(0, pick(1), 7, 8_000, 4_000)], config(Some(7), None));
    assert_eq!(out.time_slices[0].scalars.htm_duty_pick_at_station_ms, 2_000);
    assert_eq!(out.time_slices[1].scalars.htm_duty_pick_at_station_ms, 2_000);
    assert_eq!(out.cumulative_scalars.htm_duty_pick_at_station_ms, 4_000);
}

// ---------------------------------------------------------------------------
// #4 Wait Loaded — loaded to-station waits; the stands at a relay are gaps
// ---------------------------------------------------------------------------

/// The bot standing at the relay with the tote aboard, between its fork-Get
/// and its departure for the station, is between two intervals: a gap,
/// journaled as a no-task row carrying the tote, not a wait-loaded overlay.
/// `htm_duty_wait_loaded_ms` keeps the loaded to-station route waits the two
/// tests below cover.
#[test]
fn a_loaded_stand_after_the_fork_get_is_a_gap_not_a_wait_loaded_overlay() {
    let out = run(
        vec![
            seed_htm(0),
            SimEvent::HtmRelayPickup(HtmRelayPickupEvent {
                robot_id: RobotId(0),
                relay_id: RelayId(0),
                start: SimTime::ms(100),
                end: SimTime::ms(150),
                wait: SimDuration::ZERO,
                resident: false,
            }),
            trip(0, TripPurpose::ToStation, 200),
        ],
        config(None, None),
    );
    assert_eq!(slice0(&out).htm_duty_wait_loaded_ms, 0, "the stand is a gap, not an overlay");
    assert_eq!(out.cumulative_scalars.htm_duty_wait_loaded_ms, 0);
    assert_eq!(
        out.robot_duty_totals.get(&RobotId(0)).unwrap().loading_unloading,
        SimDuration::ms(50),
        "only the fork-Get itself is loading time"
    );
}

/// The empty sibling: a fork-Put is an operation of its own length, and the
/// stand after it is a gap, not an overlay.
#[test]
fn empty_loadunload_hold_is_not_wait_loaded() {
    let out = run(
        vec![
            seed_htm(0),
            SimEvent::HtmRelayDeposit(HtmRelayDepositEvent {
                robot_id: RobotId(0),
                relay_id: RelayId(0),
                start: SimTime::ms(100),
                end: SimTime::ms(150),
                wait: SimDuration::ZERO,
            }),
            trip(0, TripPurpose::ToRelay, 200),
        ],
        config(None, None),
    );
    assert_eq!(slice0(&out).htm_duty_wait_loaded_ms, 0);
}

#[test]
fn a_same_relay_deposit_then_pickup_books_only_the_two_operations() {
    // A deposit then a pickup at the same relay, with no leg between: the
    // stands after each operation are gaps, so only the operations book duty.
    let out = run(
        vec![
            seed_htm(0),
            SimEvent::HtmRelayDeposit(HtmRelayDepositEvent {
                robot_id: RobotId(0),
                relay_id: RelayId(0),
                start: SimTime::ms(1_000),
                end: SimTime::ms(1_500),
                wait: SimDuration::ZERO,
            }),
            SimEvent::HtmRelayPickup(HtmRelayPickupEvent {
                robot_id: RobotId(0),
                relay_id: RelayId(0),
                start: SimTime::ms(5_000),
                end: SimTime::ms(5_500),
                wait: SimDuration::ZERO,
                resident: false,
            }),
            trip(0, TripPurpose::ToStation, 6_000),
        ],
        config(None, None),
    );
    assert_eq!(slice0(&out).htm_duty_wait_loaded_ms, 0, "neither stand is an overlay; both are gaps");
    assert_eq!(
        out.robot_duty_totals.get(&RobotId(0)).unwrap().loading_unloading,
        SimDuration::ms(1_000),
        "only the two operations themselves, 500ms each"
    );
}

#[test]
fn loaded_to_station_waits_are_wait_loaded() {
    let out = run(
        vec![
            seed_htm(0),
            trip(0, TripPurpose::ToStation, 0),
            nav(0, true, 0, vec![move_wp(0, 1_000), wait_wp(1_000, 3_000)]),
        ],
        config(None, None),
    );
    assert_eq!(slice0(&out).htm_duty_wait_loaded_ms, 2_000); // Move excluded, Wait counted
    assert_eq!(out.cumulative_scalars.htm_duty_wait_loaded_ms, 2_000);
}

#[test]
fn empty_or_wrong_leg_waits_are_not_wait_loaded() {
    // Empty leg: skipped. To-relay leg (empty inbound): skipped.
    let out = run(
        vec![
            seed_htm(0),
            trip(0, TripPurpose::ToStation, 0),
            nav(0, false, 0, vec![wait_wp(0, 2_000)]), // not carrying
            trip(0, TripPurpose::ToRelay, 5_000),
            nav(0, true, 5_000, vec![wait_wp(5_000, 6_000)]), // loaded but ToRelay
        ],
        config(None, None),
    );
    assert_eq!(slice0(&out).htm_duty_wait_loaded_ms, 0);
}

// ---------------------------------------------------------------------------
// #5 Dwell Loaded — loaded to-return waits only
// ---------------------------------------------------------------------------

#[test]
fn station_processing_ends_with_the_dwell_and_is_not_dwell_loaded() {
    // Ruling #49: the return departs at action-ready time, so the visit and
    // the recorded dwell both end at 6000. Nothing at the station belongs to
    // KPI #5; that overlay is reserved for loaded to-return route waits.
    let out = run(
        vec![
            seed_htm(0),
            arrival(0, 7, 2_000),
            cycle(0, pick(1), 7, 2_000, 4_000),
            trip(0, TripPurpose::ToReturnRelay, 6_000),
        ],
        config(Some(7), None),
    );
    assert_eq!(slice0(&out).htm_duty_dwell_loaded_ms, 0);
    assert_eq!(out.cumulative_scalars.htm_duty_dwell_loaded_ms, 0);
    assert_eq!(slice0(&out).htm_duty_pick_at_station_ms, 4_000);
    assert_eq!(out.robot_duty_totals.get(&RobotId(0)).unwrap().processing, SimDuration::ms(4_000));
}

#[test]
fn loaded_to_return_waits_are_dwell_loaded() {
    let out = run(
        vec![seed_htm(0), trip(0, TripPurpose::ToReturnRelay, 0), nav(0, true, 0, vec![wait_wp(0, 1_500)])],
        config(None, None),
    );
    assert_eq!(slice0(&out).htm_duty_dwell_loaded_ms, 1_500);
}

/// The return leg's opening wait is blocked-returning time like any other:
/// the path search parks the loaded D30 on the presentation cell until the
/// corridor out of the station clears, and which cell it stands on does not
/// change what it is doing.
#[test]
fn an_outbound_wait_on_the_presentation_cell_is_dwell_loaded() {
    let mut cfg = config(None, None);
    cfg.cell_kinds = Arc::new(vec![Some(CellKind::Presentation)]);
    let out =
        run(vec![seed_htm(0), trip(0, TripPurpose::ToReturnRelay, 0), nav(0, true, 0, vec![wait_wp(0, 1_500)])], cfg);
    assert_eq!(slice0(&out).htm_duty_dwell_loaded_ms, 1_500);
    assert_eq!(out.cumulative_scalars.htm_duty_dwell_loaded_ms, 1_500);
}

#[test]
fn dwell_loaded_wait_clips_across_slice_boundary() {
    // Wait [9000, 12000) over 10s slices => 1000 in slice 0, 2000 in slice 1.
    let out = run(
        vec![seed_htm(0), trip(0, TripPurpose::ToReturnRelay, 0), nav(0, true, 0, vec![wait_wp(9_000, 12_000)])],
        config(None, None),
    );
    assert_eq!(out.time_slices[0].scalars.htm_duty_dwell_loaded_ms, 1_000);
    assert_eq!(out.time_slices[1].scalars.htm_duty_dwell_loaded_ms, 2_000);
}

#[test]
fn a_station_visit_closed_by_a_departure_without_a_dwell_trips_the_invariant() {
    // No CycleComplete => no dwell => a visit closed by a DEPARTURE lost the
    // fact of its operator's work, in every build. A visit with no departure
    // is exempt — see `a_station_visit_with_no_departure_and_no_dwell_is_tolerated`.
    let observed = observe(
        vec![seed_htm(0), arrival(0, 7, 2_000), trip(0, TripPurpose::ToReturnRelay, 5_000)],
        config(None, None),
    );
    let departure = SimTime::ms(5_000);
    assert_eq!(
        observed.unwrap_err(),
        StreamViolation::Activity(ActivityViolation::VisitWithoutDwell { robot: RobotId(0), departure })
    );
}

#[test]
fn a_station_visit_with_no_departure_and_no_dwell_is_tolerated() {
    // End-of-run seam: a stop (sim-time cap / wall-timeout / UI) cancels the
    // chain between the HtmDeliver commit, which already reported the arrival,
    // and the Presentation commit, which would have recorded the dwell. The
    // visit runs on to the current world time like any visit the run stopped
    // inside, not to the run's last fact. With no dwell recorded it is all
    // awaiting station service, which books no #5 time, and it is no
    // invariant failure.
    let run_end = SimEvent::OrderComplete(OrderCompleteEvent { order_id: OrderId(1), sim_time_ms: 10_000 });
    let out = run(
        vec![seed_htm(0), arrival(0, 7, 2_000), SimEvent::BarrierAdvanced(SimTime::ms(6_000)), run_end],
        config(None, None),
    );
    assert_eq!(out.robot_duty_totals[&RobotId(0)].processing.as_ms(), 4_000, "the visit runs [2s, 6s)");
    assert_eq!(slice0(&out).htm_duty_dwell_loaded_ms, 0);
    assert_eq!(out.cumulative_scalars.htm_duty_dwell_loaded_ms, 0);
}

// ---------------------------------------------------------------------------
// Family invariant: per slice, the #6 dwell overlay is contained in
// Processing. KPI #5 belongs to loaded waits on the return leg, not here.
// ---------------------------------------------------------------------------

#[test]
fn pick_dwell_overlay_is_contained_in_processing_per_slice() {
    // One pick cycle whose station visit and dwell are both
    // [2000, 14000), spanning two 10s slices. The prior lump-sum #6 booking
    // dumped all 12s into slice 0; clipping must preserve containment.
    let out = run(
        vec![
            seed_htm(0),
            arrival(0, 7, 2_000),
            cycle(0, pick(1), 7, 2_000, 12_000),
            trip(0, TripPurpose::ToReturnRelay, 14_000),
        ],
        config(Some(7), None),
    );
    for (index, slice) in out.time_slices.iter().enumerate() {
        let s = &slice.scalars;
        assert!(
            s.htm_duty_pick_at_station_ms <= s.htm_duty_processing_ms,
            "slice {index}: #6 ({}) exceeds Processing ({})",
            s.htm_duty_pick_at_station_ms,
            s.htm_duty_processing_ms
        );
        assert_eq!(s.htm_duty_dwell_loaded_ms, 0, "station dwell must not enter KPI #5");
    }
    // Exact split, proving #6 clips at slice boundaries like Processing.
    assert_eq!(out.time_slices[0].scalars.htm_duty_pick_at_station_ms, 8_000);
    assert_eq!(out.time_slices[1].scalars.htm_duty_pick_at_station_ms, 4_000);
}

// ---------------------------------------------------------------------------
// convert ↔ extract key agreement
// ---------------------------------------------------------------------------

#[test]
fn scalars_to_map_emits_overlay_keys() {
    let m = crate::metrics::profile::scalars_to_map(&Default::default());
    assert!(m.contains_key("htm_duty_wait_loaded_ms"));
    assert!(m.contains_key("htm_duty_dwell_loaded_ms"));
    assert!(m.contains_key("htm_duty_pick_at_station_ms"));
}
