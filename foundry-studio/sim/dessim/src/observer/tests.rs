//! Within `observer`, `tests` feeds synthetic `SimEvent` streams through
//! the real handler path and asserts the captured facts and counters.

use std::collections::BTreeMap;
use std::sync::mpsc;

use desgraph::{CellId, RelayId, RobotType, StationId, StationType, WorldPos, ZoneId};
use rustc_hash::FxHashMap;

use crate::events::*;
use crate::model::ids::{PresentationId, SkuId, VtmCycleId};
use crate::model::{Destination, OrderId, RobotId, SimDuration, SimTime, StationArrivalSource, ToteId};
use crate::observer::activity::ActivityKind;
use crate::observer::output::MetricsOutput;
use crate::observer::time_slice::MetricsConfig;
use crate::pathfinding::{Orientation, Waypoint, WaypointAction};

use super::{SimObserver, StreamViolation};

/// Helper to create a channel and run the observer synchronously.
fn run_with_events(events: Vec<SimEvent>) -> MetricsOutput {
    run_with_events_config(events, MetricsConfig::default())
}

fn run_with_events_config(events: Vec<SimEvent>, mut config: MetricsConfig) -> MetricsOutput {
    super::test_models::bind_legacy_nav_models(&mut config, &events);
    let (tx, rx) = mpsc::channel();
    for event in events {
        tx.send(event).unwrap();
    }
    drop(tx);
    SimObserver::new(config, rx).run().expect("the facts break no rule of the event stream")
}

/// Build a Waypoint chain: PathStart at `start_cell` at `t0`, then a Move
/// into each `(cell, duration)` in `moves` with cumulative timing, then a
/// PathEnd at the final cell.
fn waypoint_chain(t0: SimTime, start_cell: u32, moves: &[(u32, SimDuration)]) -> Vec<Waypoint> {
    let mut wps = Vec::with_capacity(moves.len() + 2);
    let start = CellId(start_cell);
    wps.push(Waypoint { cell: start, orientation: Orientation::East, action: WaypointAction::PathStart { time: t0 } });
    let mut t = t0;
    let mut last_cell = start;
    for (cell, dur) in moves {
        let next_t = t + *dur;
        let cell_id = CellId(*cell);
        wps.push(Waypoint {
            cell: cell_id,
            orientation: Orientation::East,
            action: WaypointAction::Move {
                start: t,
                end: next_t,
                // Observer tests don't exercise rasterization; default to a
                // benign nonzero velocity so the action is well-formed.
                entry_velocity_mm_s: 1000.0,
                exit_velocity_mm_s: 1000.0,
            },
        });
        t = next_t;
        last_cell = cell_id;
    }
    wps.push(Waypoint { cell: last_cell, orientation: Orientation::East, action: WaypointAction::PathEnd { time: t } });
    wps
}

fn initial_pos(robot_id: u32, cell: u32, robot_type: RobotType) -> (RobotId, CellId, RobotType, WorldPos) {
    (RobotId(robot_id), CellId(cell), robot_type, WorldPos::default())
}

#[test]
fn empty_events_produces_empty_output() {
    let output = run_with_events(vec![]);
    assert!(output.time_slices.is_empty());
    assert_eq!(output.cumulative_scalars.picks_completed, 0);
}

#[test]
fn nav_event_increments_visit_counts() {
    let output = run_with_events(vec![
        SimEvent::InitialPositions(InitialPositionsEvent {
            positions: vec![initial_pos(0, 0, RobotType::Vtm)],
            htm_headings_rad: Vec::new(),
        }),
        SimEvent::Nav(NavEventData {
            robot_id: RobotId(0),
            waypoints: waypoint_chain(
                SimTime::ZERO,
                0,
                &[(10, SimDuration::seconds(1)), (11, SimDuration::ms(500)), (12, SimDuration::ms(500))],
            ),
            robot_type: RobotType::Vtm,
            time: SimTime::ZERO,
            distance_mm: 0,
            carrying_cargo: false,
        }),
    ]);

    assert_eq!(output.cumulative_grid.nav_visits_vtm.get(&10), Some(&1));
    assert_eq!(output.cumulative_grid.nav_visits_vtm.get(&11), Some(&1));
    assert_eq!(output.cumulative_grid.nav_visits_vtm.get(&12), Some(&1));
    assert!(output.cumulative_grid.nav_visits_htm.is_empty());
}

#[test]
fn nav_event_buckets_across_time_slices() {
    let config = MetricsConfig { slice_width_ms: 300_000, ..MetricsConfig::default() };
    let output = run_with_events_config(
        vec![
            SimEvent::InitialPositions(InitialPositionsEvent {
                positions: vec![initial_pos(0, 0, RobotType::Htm)],
                htm_headings_rad: Vec::new(),
            }),
            SimEvent::Nav(NavEventData {
                robot_id: RobotId(0),
                waypoints: waypoint_chain(
                    SimTime::ZERO,
                    0,
                    &[(10, SimDuration::seconds(250)), (11, SimDuration::seconds(100)), (12, SimDuration::seconds(1))],
                ),
                robot_type: RobotType::Htm,
                time: SimTime::ZERO,
                distance_mm: 0,
                carrying_cargo: false,
            }),
        ],
        config,
    );

    assert_eq!(output.time_slices.len(), 2);
    let loc0 = output.time_slices[0].bot_locations.get(&0).expect("bot 0 in slice 0");
    assert_eq!(loc0.steps.len(), 2);
    assert_eq!(loc0.steps[0].cell_id, 10);
    assert_eq!(loc0.steps[0].delta_ms, 250_000);
    assert_eq!(loc0.steps[1].cell_id, 11);
    assert_eq!(loc0.steps[1].delta_ms, 50_000);
    let loc1 = output.time_slices[1].bot_locations.get(&0).expect("bot 0 in slice 1");
    assert_eq!(loc1.steps.len(), 2);
    assert_eq!(loc1.steps[0].cell_id, 11);
    assert_eq!(loc1.steps[0].delta_ms, 50_000);
    assert_eq!(loc1.steps[1].cell_id, 12);
    assert_eq!(loc1.steps[1].delta_ms, 1_000);
    assert_eq!(loc1.start_cell, 10);
    assert_eq!(output.cumulative_grid.nav_visits_htm.get(&10), Some(&1));
    assert_eq!(output.cumulative_grid.nav_visits_htm.get(&11), Some(&1));
    assert_eq!(output.cumulative_grid.nav_visits_htm.get(&12), Some(&1));
}

#[test]
fn cycle_complete_increments_picks() {
    let output = run_with_events(vec![SimEvent::CycleComplete(CycleCompleteEvent {
        robot_id: RobotId(5),
        robot_type: RobotType::Htm,
        kind: CycleKind::Pick { sku: SkuId(0), qty: 3 },
        station_id: StationId(0),
        sim_time_ms: 10_000,
        dwell_ms: 9_000,
        arrival_source: None,
        dc_id: None,
        presentation_id: None,
        tote_presentation: None,
    })]);

    assert_eq!(output.cumulative_scalars.picks_completed, 3);
    assert_eq!(output.cumulative_scalars.presentations, 1);
    assert_eq!(output.cumulative_scalars.picks_per_station.get(&StationId(0)), Some(&3));
    assert_eq!(output.station_busy_ms.get(&StationId(0)), Some(&9_000));
}

/// One piece of station work by `robot`. A robot cannot work two overlapping
/// windows, so overlapping work at one station is each by its own robot.
fn station_work(robot: u32, station: u32, start: u64, end: u64, putaway: bool) -> SimEvent {
    SimEvent::CycleComplete(CycleCompleteEvent {
        robot_id: RobotId(robot),
        robot_type: RobotType::Htm,
        kind: if putaway {
            CycleKind::Putaway { sku: SkuId(0), qty: 1 }
        } else {
            CycleKind::Pick { sku: SkuId(0), qty: 1 }
        },
        station_id: StationId(station),
        sim_time_ms: if putaway { end } else { start },
        dwell_ms: end - start,
        arrival_source: putaway.then_some(StationArrivalSource::PutawayFromRack),
        dc_id: None,
        presentation_id: None,
        tote_presentation: None,
    })
}

#[test]
fn station_gaps_measure_dwell_end_to_next_start_in_simulated_order() {
    let ordered = [
        station_work(1, 1, 1_000, 4_000, false),
        station_work(1, 1, 5_000, 9_000, false),
        station_work(1, 1, 9_000, 11_000, false),
        station_work(1, 1, 14_000, 15_000, false),
        station_work(2, 2, 40_000, 41_000, false),
    ];
    let scrambled = [3, 1, 4, 0, 2].map(|index| ordered[index].clone());
    let sorted = run_with_events(ordered.to_vec());
    let output = run_with_events(scrambled.to_vec());
    assert_eq!(output.station_gaps_ms, sorted.station_gaps_ms);
    assert_eq!(output.station_first_seen, sorted.station_first_seen);
    assert_eq!(output.station_first_seen[&StationId(1)], SimTime::ms(1_000));
    assert_eq!(output.station_gaps_ms[&StationId(1)], vec![1_000, 0, 3_000]);
    assert!(!output.station_gaps_ms.contains_key(&StationId(2)), "one presentation supplies no gap sample");
}

#[test]
fn overlapping_station_work_does_not_create_idle_time_inside_a_busy_interval() {
    let output = run_with_events(vec![
        station_work(1, 1, 16_000, 17_000, false),
        station_work(2, 1, 3_000, 4_000, false),
        station_work(3, 1, 1_000, 10_000, false),
        station_work(4, 1, 9_000, 12_000, false),
        station_work(5, 1, 12_000, 13_000, false),
    ]);
    assert_eq!(output.station_gaps_ms[&StationId(1)], vec![0, 3_000]);
    assert_eq!(output.station_timelines[&StationId(1)].len(), 5, "raw overlapping work remains inspectable");
    assert_eq!(output.station_busy_ms[&StationId(1)], 15_000, "gap measurement does not change busy accounting");
}

#[test]
fn putaway_gaps_use_completed_dwell_end_and_the_next_dwell_start() {
    let config = MetricsConfig {
        station_types: [(StationId(1), StationType::Putaway)].into_iter().collect(),
        ..MetricsConfig::default()
    };
    let output = run_with_events_config(
        vec![station_work(1, 1, 8_000, 12_000, true), station_work(1, 1, 1_000, 4_000, true)],
        config,
    );
    assert_eq!(output.station_gaps_ms[&StationId(1)], vec![4_000]);
}

/// The DC on a CycleCompleteEvent must reach the PickRecord. Bulk pick keeps
/// its DC; pre-formed pick stays blank.
#[test]
fn cycle_complete_threads_dc_id_into_pick_log() {
    let output = run_with_events(vec![
        SimEvent::CycleComplete(CycleCompleteEvent {
            robot_id: RobotId(5),
            robot_type: RobotType::Htm,
            kind: CycleKind::Pick { sku: SkuId(7), qty: 1 },
            station_id: StationId(0),
            sim_time_ms: 1_000,
            dwell_ms: 9_000,
            arrival_source: None,
            dc_id: Some("087ABC004".into()),
            presentation_id: Some(PresentationId(99)),
            tote_presentation: Some(TotePresentation {
                order_id: OrderId(44),
                tote_id: ToteId(88),
                source_zone_id: ZoneId(3),
                source_relay_id: RelayId(12),
                presentation_id: PresentationId(99),
            }),
        }),
        SimEvent::OrderComplete(OrderCompleteEvent { order_id: OrderId(44), sim_time_ms: 10_000 }),
        SimEvent::CycleComplete(CycleCompleteEvent {
            robot_id: RobotId(5),
            robot_type: RobotType::Htm,
            kind: CycleKind::Pick { sku: SkuId(7), qty: 1 },
            station_id: StationId(0),
            sim_time_ms: 12_000,
            dwell_ms: 9_000,
            arrival_source: None,
            dc_id: None,
            presentation_id: None,
            tote_presentation: None,
        }),
    ]);

    assert_eq!(output.pick_log.len(), 2);
    assert_eq!(output.pick_log[0].dc_id.as_deref(), Some("087ABC004"));
    assert_eq!(output.pick_log[0].order_id, Some(OrderId(44)));
    assert_eq!(output.pick_log[0].tote_id, Some(88));
    assert_eq!(output.pick_log[0].source_zone_id, Some(3));
    assert_eq!(output.pick_log[0].source_relay_id, Some(RelayId(12)));
    assert_eq!(output.pick_log[0].presentation_id, Some(99));
    assert_eq!(output.pick_log[1].dc_id, None);
    assert_eq!(output.completed_order_log.len(), 1);
    assert_eq!(output.completed_order_log[0].order_id, OrderId(44));
    assert_eq!(output.completed_order_log[0].station_id, Some(StationId(0)));
    assert_eq!(output.completed_order_log[0].dc_id.as_deref(), Some("087ABC004"));
}

/// The presentation_id on a CycleCompleteEvent must reach the Pick/Putaway
/// logs; a putaway with no chain (`None`) stays blank.
#[test]
fn cycle_complete_threads_presentation_id_into_logs() {
    use desgraph::StationType;

    use crate::model::StationArrivalSource;
    use crate::model::ids::PresentationId;

    // The observer hard-asserts each cycle's kind against its station's
    // configured type, and putaway cycles must carry an arrival_source —
    // so the pick and the putaways run at separate, correctly-typed
    // stations, and the putaways arrive PutawayDirect.
    let mut station_types = FxHashMap::default();
    station_types.insert(StationId(0), StationType::Pick);
    station_types.insert(StationId(1), StationType::Putaway);
    let config = MetricsConfig { station_types, ..MetricsConfig::default() };

    let output = run_with_events_config(
        vec![
            SimEvent::CycleComplete(CycleCompleteEvent {
                robot_id: RobotId(5),
                robot_type: RobotType::Htm,
                kind: CycleKind::Pick { sku: SkuId(7), qty: 1 },
                station_id: StationId(0),
                sim_time_ms: 1_000,
                dwell_ms: 9_000,
                arrival_source: None,
                dc_id: None,
                presentation_id: Some(PresentationId(42)),
                tote_presentation: None,
            }),
            SimEvent::CycleComplete(CycleCompleteEvent {
                robot_id: RobotId(5),
                robot_type: RobotType::Htm,
                kind: CycleKind::Putaway { sku: SkuId(7), qty: 1 },
                station_id: StationId(1),
                sim_time_ms: 2_000,
                dwell_ms: 0,
                arrival_source: Some(StationArrivalSource::PutawayDirect),
                dc_id: None,
                presentation_id: Some(PresentationId(99)),
                tote_presentation: None,
            }),
            SimEvent::CycleComplete(CycleCompleteEvent {
                robot_id: RobotId(5),
                robot_type: RobotType::Htm,
                kind: CycleKind::Putaway { sku: SkuId(7), qty: 1 },
                station_id: StationId(1),
                sim_time_ms: 3_000,
                dwell_ms: 0,
                arrival_source: Some(StationArrivalSource::PutawayDirect),
                dc_id: None,
                presentation_id: None,
                tote_presentation: None,
            }),
        ],
        config,
    );

    assert_eq!(output.pick_log.len(), 1);
    assert_eq!(output.pick_log[0].presentation_id, Some(42));
    assert_eq!(output.putaway_log.len(), 2);
    assert_eq!(output.putaway_log[0].presentation_id, Some(99));
    assert_eq!(output.putaway_log[1].presentation_id, None);
}

#[test]
fn station_busy_ms_accumulates_across_cycles() {
    // Two presentations at station 1 (9s + 12s dwell) and one at station 2
    // (9s). Verifies per-station accumulation and that distinct stations
    // are tracked independently.
    let output = run_with_events(vec![
        SimEvent::CycleComplete(CycleCompleteEvent {
            robot_id: RobotId(5),
            robot_type: RobotType::Htm,
            kind: CycleKind::Pick { sku: SkuId(0), qty: 1 },
            station_id: StationId(1),
            sim_time_ms: 1_000,
            dwell_ms: 9_000,
            arrival_source: None,
            dc_id: None,
            presentation_id: None,
            tote_presentation: None,
        }),
        SimEvent::CycleComplete(CycleCompleteEvent {
            robot_id: RobotId(5),
            robot_type: RobotType::Htm,
            kind: CycleKind::Pick { sku: SkuId(0), qty: 1 },
            station_id: StationId(1),
            sim_time_ms: 20_000,
            dwell_ms: 12_000,
            arrival_source: None,
            dc_id: None,
            presentation_id: None,
            tote_presentation: None,
        }),
        SimEvent::CycleComplete(CycleCompleteEvent {
            robot_id: RobotId(6),
            robot_type: RobotType::Htm,
            kind: CycleKind::Pick { sku: SkuId(0), qty: 1 },
            station_id: StationId(2),
            sim_time_ms: 5_000,
            dwell_ms: 9_000,
            arrival_source: None,
            dc_id: None,
            presentation_id: None,
            tote_presentation: None,
        }),
    ]);

    assert_eq!(output.station_busy_ms.get(&StationId(1)), Some(&21_000));
    assert_eq!(output.station_busy_ms.get(&StationId(2)), Some(&9_000));
}

#[test]
fn putaway_presentations_bucket_by_arrival_source() {
    use desgraph::StationType;
    use rustc_hash::FxHashMap;

    use crate::model::StationArrivalSource;

    let mut station_types = FxHashMap::default();
    station_types.insert(StationId(7), StationType::Putaway);
    let config = MetricsConfig { station_types, ..MetricsConfig::default() };

    let output = run_with_events_config(
        vec![
            // 2 direct + 1 from-rack at the same putaway station.
            SimEvent::CycleComplete(CycleCompleteEvent {
                robot_id: RobotId(1),
                robot_type: RobotType::Htm,
                kind: CycleKind::Putaway { sku: SkuId(0), qty: 4 },
                station_id: StationId(7),
                sim_time_ms: 1_000,
                dwell_ms: 9_000,
                arrival_source: Some(StationArrivalSource::PutawayDirect),
                dc_id: None,
                presentation_id: None,
                tote_presentation: None,
            }),
            SimEvent::CycleComplete(CycleCompleteEvent {
                robot_id: RobotId(2),
                robot_type: RobotType::Htm,
                kind: CycleKind::Putaway { sku: SkuId(0), qty: 2 },
                station_id: StationId(7),
                sim_time_ms: 12_000,
                dwell_ms: 9_000,
                arrival_source: Some(StationArrivalSource::PutawayFromRack),
                dc_id: None,
                presentation_id: None,
                tote_presentation: None,
            }),
            SimEvent::CycleComplete(CycleCompleteEvent {
                robot_id: RobotId(3),
                robot_type: RobotType::Htm,
                kind: CycleKind::Putaway { sku: SkuId(0), qty: 1 },
                station_id: StationId(7),
                sim_time_ms: 23_000,
                dwell_ms: 9_000,
                arrival_source: Some(StationArrivalSource::PutawayDirect),
                dc_id: None,
                presentation_id: None,
                tote_presentation: None,
            }),
        ],
        config,
    );

    let t = &output.cumulative_scalars.throughput;
    assert_eq!(t.putaway_station_presentations, 3);
    assert_eq!(t.putaway_station_presentations_direct, 2);
    assert_eq!(t.putaway_station_presentations_from_rack, 1);
    // Direct + FromRack must reconcile with the unbucketed total.
    assert_eq!(
        t.putaway_station_presentations,
        t.putaway_station_presentations_direct + t.putaway_station_presentations_from_rack
    );
}

#[test]
fn putaway_dispatches_bucket_by_arrival_source() {
    let assignment = |robot: u32, source: StationArrivalSource, at_ms: u64| {
        SimEvent::HtmAssignment(HtmAssignmentEvent {
            robot_id: RobotId(robot),
            station_id: StationId(7),
            tote_id: None,
            source,
            relay_cell: None,
            dispatch_time: SimTime::ms(at_ms),
            quoted_arrival: SimTime::ms(at_ms + 60_000),
        })
    };
    let output = run_with_events(vec![
        // 2 direct + 1 from-rack putaway dispatches; the pick-chain
        // assignment is not putaway staging and must not count.
        assignment(1, StationArrivalSource::PutawayDirect, 1_000),
        assignment(2, StationArrivalSource::PutawayFromRack, 12_000),
        assignment(3, StationArrivalSource::PutawayDirect, 23_000),
        assignment(4, StationArrivalSource::PickFromRack, 34_000),
    ]);
    let t = &output.cumulative_scalars.throughput;
    assert_eq!(t.putaway_dispatches_direct, 2);
    assert_eq!(t.putaway_dispatches_from_rack, 1);
    // Each dispatch lands in the slice its HTM starts moving in.
    let from_rack_slices: Vec<u64> = output
        .time_slices
        .iter()
        .filter(|slice| slice.scalars.throughput.putaway_dispatches_from_rack > 0)
        .map(|slice| slice.start_ms)
        .collect();
    assert_eq!(from_rack_slices, vec![10_000], "the 12 s dispatch lands in the slice starting at 10 s");
}

#[test]
fn resource_events_counted() {
    let output = run_with_events(vec![
        SimEvent::Resource(ResourceEvent { kind: ResourceEventKind::RelayReservationSuccess, sim_time_ms: 1000 }),
        SimEvent::Resource(ResourceEvent { kind: ResourceEventKind::ToteClaimFailure, sim_time_ms: 2000 }),
        SimEvent::Resource(ResourceEvent { kind: ResourceEventKind::BackpressureTrigger, sim_time_ms: 3000 }),
    ]);

    assert_eq!(output.cumulative_scalars.relay_reservation_successes, 1);
    assert_eq!(output.cumulative_scalars.tote_claim_failures, 1);
    assert_eq!(output.cumulative_scalars.backpressure_events, 1);
}

/// The roster the run starts with is counted in the first slice's distribution, every robot in a gap, and the run's
/// totals stand at that reading, its only slice's.
#[test]
fn initial_positions_seed_the_activity_distribution() {
    let output = run_with_events(vec![
        SimEvent::InitialPositions(InitialPositionsEvent {
            positions: vec![
                initial_pos(0, 10, RobotType::Vtm),
                initial_pos(1, 11, RobotType::Vtm),
                initial_pos(2, 20, RobotType::Htm),
            ],
            htm_headings_rad: Vec::new(),
        }),
        SimEvent::OrderComplete(OrderCompleteEvent { order_id: OrderId(0), sim_time_ms: 5_000 }),
    ]);

    let first = &output.time_slices[0].scalars;
    assert_eq!(first.vtm_activity_counts.get(&ActivityKind::Gap), Some(&2));
    assert_eq!(first.htm_activity_counts.get(&ActivityKind::Gap), Some(&1));
    assert_eq!(output.cumulative_scalars.vtm_activity_counts, first.vtm_activity_counts);
    assert_eq!(output.cumulative_scalars.htm_activity_counts, first.htm_activity_counts);
}

/// The per-slice distribution is a reading of the intervals: a robot inside
/// one at the slice's start counts under its kind, a robot inside none counts
/// as a gap.
#[test]
fn the_distribution_counts_robots_inside_an_interval_against_those_in_a_gap() {
    let rail = |robot: u32, start_ms: u64, end_ms: u64| {
        SimEvent::VtmNav(VtmNavEventData {
            robot_id: RobotId(robot),
            vtm_cycle_id: VtmCycleId(0),
            kind: VtmNavKind::RailMove {
                from: VtmEndpoint::Aisle { cell: CellId(10), pos: WorldPos::default() },
                to: VtmEndpoint::Aisle { cell: CellId(12), pos: WorldPos::default() },
                tote_id: None,
                start: SimTime::ms(start_ms),
                end: SimTime::ms(end_ms),
                distance_mm: 0,
            },
            geo_start: SimTime::ms(start_ms),
            geo_end: SimTime::ms(end_ms),
            presentation_id: None,
            direction: VtmCycleDirection::Forward,
        })
    };
    // Robot 0 is on the rail across the second slice's start; robot 1 never
    // moves at all.
    let width = MetricsConfig::default().slice_width_ms;
    let output = run_with_events(vec![
        SimEvent::InitialPositions(InitialPositionsEvent {
            positions: vec![initial_pos(0, 10, RobotType::Vtm), initial_pos(1, 11, RobotType::Vtm)],
            htm_headings_rad: Vec::new(),
        }),
        rail(0, width - 1_000, width + 1_000),
    ]);

    let slice_1 = &output.time_slices[1].scalars;
    assert_eq!(slice_1.vtm_activity_counts.get(&ActivityKind::Travel), Some(&1), "robot 0 is mid-leg");
    assert_eq!(slice_1.vtm_activity_counts.get(&ActivityKind::Gap), Some(&1), "robot 1 is in a gap");
}

/// Two orders completed, the second at the run's stop, which is the end of its one slice: the slice takes that order
/// in, so the run's total counts both, as its summary does.
#[test]
fn order_complete_counted() {
    let output = run_with_events(vec![
        SimEvent::OrderComplete(OrderCompleteEvent { order_id: OrderId(0), sim_time_ms: 5000 }),
        SimEvent::OrderComplete(OrderCompleteEvent { order_id: OrderId(1), sim_time_ms: 10_000 }),
    ]);
    assert_eq!(output.time_slices.len(), 1, "the run's one slice ends at its stop");
    assert_eq!(output.time_slices[0].scalars.orders_completed, 2, "and states both orders");
    assert_eq!(output.cumulative_scalars.orders_completed, 2);
    assert_eq!(output.completions_through(10_000).completed_orders, 2, "as the summary counts them");
}

#[test]
fn robot_presence_tracked() {
    let output = run_with_events(vec![
        SimEvent::InitialPositions(InitialPositionsEvent {
            positions: vec![initial_pos(0, 0, RobotType::Vtm)],
            htm_headings_rad: Vec::new(),
        }),
        SimEvent::Nav(NavEventData {
            robot_id: RobotId(0),
            waypoints: waypoint_chain(SimTime::ZERO, 0, &[(10, SimDuration::ms(2500)), (11, SimDuration::ms(1500))]),
            robot_type: RobotType::Vtm,
            time: SimTime::ZERO,
            distance_mm: 0,
            carrying_cargo: false,
        }),
    ]);

    assert_eq!(output.cumulative_grid.robot_presence_vtm_ms.get(&10), Some(&2500));
    assert_eq!(output.cumulative_grid.robot_presence_vtm_ms.get(&11), Some(&1500));
}

#[test]
fn rack_fill_tracks_moves() {
    use crate::events::InventoryDeltaEvent;
    use crate::model::ToteId;
    use crate::model::ids::SkuId;
    use crate::observer::time_slice::ToteVisualState;

    // Fill = slots holding a laden tote, so the three racked totes must hold
    // stock to count. Seed one unit each at t=0.
    let seed = |tote: u32, rack: u32| {
        SimEvent::InventoryDelta(InventoryDeltaEvent {
            delta: 1,
            sku: SkuId(0),
            tote_id: ToteId(tote),
            at: SimTime::ms(0),
        })
    };
    let initial = ToteInitialStatesEvent {
        order_totes: Vec::new(),
        totes: vec![
            (0, ToteVisualState::InRack { cell_id: 100, level: 0 }),
            (1, ToteVisualState::InRack { cell_id: 101, level: 0 }),
            (2, ToteVisualState::InRack { cell_id: 102, level: 0 }),
        ],
    };
    // Tote 0 leaves the rack (→ OnBot) at t=1s, returns at t=3s.
    let move_out = ToteMovedEvent {
        tote_id: ToteId(0),
        new_state: ToteVisualState::OnBot { bot_id: 9 },
        time: SimTime::ms(1_000),
    };
    let move_back = ToteMovedEvent {
        tote_id: ToteId(0),
        new_state: ToteVisualState::InRack { cell_id: 100, level: 0 },
        time: SimTime::ms(3_000),
    };

    let config = MetricsConfig { slice_width_ms: 1_000, ..MetricsConfig::default() };
    let output = run_with_events_config(
        vec![
            SimEvent::ToteInitialStates(initial),
            seed(0, 0),
            seed(1, 1),
            seed(2, 2),
            SimEvent::ToteMoved(move_out),
            SimEvent::ToteMoved(move_back),
            // B passes the move back, so the run reaches into the slice it opens.
            SimEvent::BarrierAdvanced(SimTime::ms(3_001)),
        ],
        config,
    );

    // Slice 0: 3 filled slots.
    assert_eq!(output.time_slices[0].scalars.rack.occupancy_total, 3);
    // Slice 1 (tote 0 out): 2 filled slots — its slot is freed even though the
    // laden tote still exists (just not in rack).
    assert_eq!(output.time_slices[1].scalars.rack.occupancy_total, 2);
    // Slice 2: no events — extend_slices_through seeds it with the running count (2).
    assert_eq!(output.time_slices[2].scalars.rack.occupancy_total, 2);
    // Slice 3 (tote 0 back): 3 filled slots again.
    assert_eq!(output.time_slices[3].scalars.rack.occupancy_total, 3);
    assert_eq!(output.cumulative_scalars.rack.occupancy_total, 3);
    assert_eq!(output.cumulative_scalars.rack.occupancy_peak, 3);
}

#[test]
fn double_booked_slot_counts_once() {
    // Two totes recorded InRack at the SAME (cell, level) — the artifact that
    // used to push fill above capacity. Slot-based counting must report 1
    // filled slot, not 2.
    use crate::events::InventoryDeltaEvent;
    use crate::model::ToteId;
    use crate::model::ids::SkuId;
    use crate::observer::time_slice::ToteVisualState;

    let seed = |tote: u32| {
        SimEvent::InventoryDelta(InventoryDeltaEvent {
            delta: 1,
            sku: SkuId(0),
            tote_id: ToteId(tote),
            at: SimTime::ms(0),
        })
    };
    let config = MetricsConfig { slice_width_ms: 1_000, total_rack_slots: 1, ..MetricsConfig::default() };
    let output = run_with_events_config(
        vec![
            SimEvent::ToteInitialStates(ToteInitialStatesEvent {
                order_totes: Vec::new(),
                totes: vec![(0, ToteVisualState::InRack { cell_id: 100, level: 0 })],
            }),
            seed(0),
            seed(1),
            // Tote 1 arrives at the slot tote 0 already occupies (stale entry).
            SimEvent::ToteMoved(ToteMovedEvent {
                tote_id: ToteId(1),
                new_state: ToteVisualState::InRack { cell_id: 100, level: 0 },
                time: SimTime::ms(1_000),
            }),
            SimEvent::BarrierAdvanced(SimTime::ms(1_001)),
        ],
        config,
    );
    // Both totes are laden and "in" slot (100,0), but it's one slot.
    assert_eq!(output.time_slices[1].scalars.rack.occupancy_total, 1);
}

#[test]
fn per_aisle_fill_and_capacity() {
    use std::sync::Arc;

    use crate::events::InventoryDeltaEvent;
    use crate::model::ToteId;
    use crate::model::ids::SkuId;
    use crate::observer::time_slice::ToteVisualState;

    // Two aisles: zone 0 = cells {100, 101} (cap 2), zone 1 = cell {200}
    // (cap 1). vtc=1 so total capacity = 3 slots.
    let rack_cell_zone: FxHashMap<u32, u32> = [(100, 0), (101, 0), (200, 1)].into_iter().collect();
    let zone_capacity: BTreeMap<u32, u32> = [(0, 2), (1, 1)].into_iter().collect();
    let config = MetricsConfig {
        slice_width_ms: 1_000,
        total_rack_slots: 3,
        rack_cell_zone: Arc::new(rack_cell_zone),
        zone_capacity: Arc::new(zone_capacity),
        ..MetricsConfig::default()
    };

    let initial = ToteInitialStatesEvent {
        order_totes: Vec::new(),
        totes: vec![
            (0, ToteVisualState::InRack { cell_id: 100, level: 0 }), // aisle 0
            (1, ToteVisualState::InRack { cell_id: 200, level: 0 }), // aisle 1
        ],
    };
    let seed = |tote: u32| {
        SimEvent::InventoryDelta(InventoryDeltaEvent {
            delta: 1,
            sku: SkuId(0),
            tote_id: ToteId(tote),
            at: SimTime::ms(0),
        })
    };
    // Tote 0 leaves its rack (aisle 0) at t=1s.
    let move_out = ToteMovedEvent {
        tote_id: ToteId(0),
        new_state: ToteVisualState::OnBot { bot_id: 9 },
        time: SimTime::ms(1_000),
    };

    let output = run_with_events_config(
        vec![
            SimEvent::ToteInitialStates(initial),
            seed(0),
            seed(1),
            SimEvent::ToteMoved(move_out),
            SimEvent::BarrierAdvanced(SimTime::ms(1_001)),
        ],
        config,
    );

    // Capacity is a constant, stamped on every slice (the fill-% denominator).
    assert_eq!(output.time_slices[0].scalars.rack.capacity_total, 3);
    assert_eq!(output.time_slices[1].scalars.rack.capacity_total, 3);

    // Slice 0: aisle 0 has one filled slot, aisle 1 one.
    let s0 = &output.time_slices[0].scalars.rack.per_zone_occupancy;
    assert_eq!(s0.get(&0), Some(&1));
    assert_eq!(s0.get(&1), Some(&1));
    // Slice 1: tote 0 left aisle 0 → aisle 0 empties, aisle 1 unchanged.
    let s1 = &output.time_slices[1].scalars.rack.per_zone_occupancy;
    assert_eq!(s1.get(&0), Some(&0));
    assert_eq!(s1.get(&1), Some(&1));

    // Spread across aisles. Slice 0: aisle 0 = 1/2 = 50%, aisle 1 = 1/1 = 100%.
    assert_eq!(output.time_slices[0].scalars.rack.fill_min_pct, 50.0);
    assert_eq!(output.time_slices[0].scalars.rack.fill_max_pct, 100.0);
    // Slice 1: aisle 0 drained to 0%, aisle 1 still 100% → band opens to 0..100.
    assert_eq!(output.time_slices[1].scalars.rack.fill_min_pct, 0.0);
    assert_eq!(output.time_slices[1].scalars.rack.fill_max_pct, 100.0);
}

#[test]
fn laden_fill_tracks_contents_and_moves() {
    use std::sync::Arc;

    use crate::events::InventoryDeltaEvent;
    use crate::model::ToteId;
    use crate::model::ids::SkuId;
    use crate::observer::time_slice::ToteVisualState;

    // One aisle: zone 0 = cell 100.
    let rack_cell_zone: FxHashMap<u32, u32> = [(100, 0)].into_iter().collect();
    let config = MetricsConfig {
        slice_width_ms: 1_000,
        total_rack_slots: 1,
        rack_cell_zone: Arc::new(rack_cell_zone),
        ..MetricsConfig::default()
    };

    let events = vec![
        // T=0: tote 0 sits in rack (empty until seeded).
        SimEvent::ToteInitialStates(ToteInitialStatesEvent {
            order_totes: Vec::new(),
            totes: vec![(0, ToteVisualState::InRack { cell_id: 100, level: 0 })],
        }),
        // T=0: seeding stows 5 units → tote becomes laden in rack.
        SimEvent::InventoryDelta(InventoryDeltaEvent {
            delta: 5,
            sku: SkuId(3),
            tote_id: ToteId(0),
            at: SimTime::ms(0),
        }),
        // T=1s: the last 5 units are picked → tote still racked, now empty.
        SimEvent::InventoryDelta(InventoryDeltaEvent {
            delta: -5,
            sku: SkuId(3),
            tote_id: ToteId(0),
            at: SimTime::ms(1_000),
        }),
        // T=2s: the empty tote leaves the rack.
        SimEvent::ToteMoved(ToteMovedEvent {
            tote_id: ToteId(0),
            new_state: ToteVisualState::OnBot { bot_id: 9 },
            time: SimTime::ms(2_000),
        }),
        SimEvent::BarrierAdvanced(SimTime::ms(2_001)),
    ];
    let output = run_with_events_config(events, config);

    // Slice 0: slot filled (racked + laden).
    assert_eq!(output.time_slices[0].scalars.rack.occupancy_total, 1);
    assert_eq!(output.time_slices[0].scalars.rack.per_zone_occupancy.get(&0), Some(&1));
    // Slice 1: still racked but now empty → the slot no longer counts as
    // filled, even though a tote physically occupies it (the whole point of
    // the redefinition — empty totes don't count as fill).
    assert_eq!(output.time_slices[1].scalars.rack.occupancy_total, 0);
    assert_eq!(output.time_slices[1].scalars.rack.per_zone_occupancy.get(&0), Some(&0));
    // Slice 2: tote gone entirely.
    assert_eq!(output.time_slices[2].scalars.rack.occupancy_total, 0);
}

#[test]
fn aisle_state_facts_split_storage_payload_and_physical_relay_occupancy() {
    use std::sync::Arc;

    use crate::events::InventoryDeltaEvent;
    use crate::model::ToteId;
    use crate::model::ids::SkuId;
    use crate::observer::time_slice::ToteVisualState;

    let rack_cell_zone = [(100, 0)].into_iter().collect();
    let zone_capacity = [(0, 2)].into_iter().collect();
    let relay_cell_zone = [(500, 0)].into_iter().collect();
    let relay_zone_capacity = [(0, 1)].into_iter().collect();
    let config = MetricsConfig {
        slice_width_ms: 1_000,
        total_rack_slots: 2,
        rack_cell_zone: Arc::new(rack_cell_zone),
        zone_capacity: Arc::new(zone_capacity),
        relay_cell_zone: Arc::new(relay_cell_zone),
        relay_zone_capacity: Arc::new(relay_zone_capacity),
        ..MetricsConfig::default()
    };
    let output = run_with_events_config(
        vec![
            SimEvent::ToteInitialStates(ToteInitialStatesEvent {
                order_totes: Vec::new(),
                totes: vec![
                    (0, ToteVisualState::InRack { cell_id: 100, level: 0 }),
                    (1, ToteVisualState::AtRelay { cell_id: 500 }),
                ],
            }),
            SimEvent::InventoryDelta(InventoryDeltaEvent {
                delta: 5,
                sku: SkuId(3),
                tote_id: ToteId(0),
                at: SimTime::ZERO,
            }),
            SimEvent::ToteMoved(ToteMovedEvent {
                tote_id: ToteId(0),
                new_state: ToteVisualState::OnBot { bot_id: 9 },
                time: SimTime::ms(1_000),
            }),
            SimEvent::ToteMoved(ToteMovedEvent {
                tote_id: ToteId(1),
                new_state: ToteVisualState::OnBot { bot_id: 9 },
                time: SimTime::ms(2_000),
            }),
            SimEvent::BarrierAdvanced(SimTime::ms(2_001)),
        ],
        config,
    );

    let at = |time_ms| {
        output
            .aisle_state_log
            .iter()
            .filter(|record| record.zone_id == 0 && record.sim_time_ms <= time_ms)
            .next_back()
            .copied()
            .unwrap()
    };
    let initial = at(0);
    assert_eq!(initial.storage_capacity, 2);
    assert_eq!(initial.storage_occupied, 1);
    assert_eq!(initial.storage_empty, 0);
    assert_eq!(initial.storage_non_empty, 1);
    assert_eq!(initial.storage_unknown, 0);
    assert_eq!(initial.diving_board_capacity, 1);
    assert_eq!(initial.diving_board_occupied, 1);

    let after_storage_pickup = at(1_000);
    assert_eq!(after_storage_pickup.storage_occupied, 0);
    assert_eq!(after_storage_pickup.diving_board_occupied, 1);

    let after_relay_pickup = at(2_000);
    assert_eq!(after_relay_pickup.storage_occupied, 0);
    assert_eq!(after_relay_pickup.diving_board_occupied, 0);
}

#[test]
fn aisle_flux_logged_on_tote_crossings() {
    use std::sync::Arc;

    use desgraph::RelayId;

    use crate::events::InventoryMoveEvent;
    use crate::model::ids::SkuId;
    use crate::model::location::ToteLocation;
    use crate::model::{RackId, RobotId, ToteId};

    // Racks 5 → aisle 1; racks 6, 7 → aisle 2; relay 3 → aisle 2.
    let rack_id_zone: FxHashMap<u32, u32> = [(5, 1), (6, 2), (7, 2)].into_iter().collect();
    let relay_zones: FxHashMap<RelayId, u32> = [(RelayId(3), 2)].into_iter().collect();
    let config = MetricsConfig {
        slice_width_ms: 1_000,
        rack_id_zone: Arc::new(rack_id_zone),
        relay_zones,
        ..MetricsConfig::default()
    };

    let mv = |from, to, contents: Vec<(SkuId, u32)>, at| {
        SimEvent::InventoryMove(InventoryMoveEvent { tote_id: ToteId(12), from, to, contents, at: SimTime::ms(at) })
    };

    // One physically-consistent journey (each `from` is the prior `to`): a
    // genuine aisle-1 → aisle-2 crossing, then a same-aisle round trip through
    // transit, a same-aisle direct shuffle, and finally an empty cross-aisle
    // move — only the first is a logged crossing.
    let in_rack = |rack, z| ToteLocation::InRack { rack_id: RackId(rack), z_index: z };
    let robot = ToteLocation::WithRobot { robot_id: RobotId(9) };
    let relay3 = ToteLocation::AtRelay { relay_id: RelayId(3) };
    let output = run_with_events_config(
        vec![
            // Leaves rack 5 / aisle 1 for transit (40× SKU 7): a transit hop,
            // not yet a crossing — the aisle it left is remembered.
            mv(in_rack(5, 0), robot, vec![(SkuId(7), 40)], 1_000),
            // Settles at relay 3 / aisle 2 with 37 units (3 picked in transit):
            // the crossing, valued at what it carries on arrival → −37 aisle 1,
            // +37 aisle 2. The 3 picked units left via a station, not an aisle.
            mv(robot, relay3.clone(), vec![(SkuId(7), 37)], 2_000),
            // Rides a robot back out of aisle 2...
            mv(relay3, robot, vec![(SkuId(7), 37)], 3_000),
            // ...and settles at rack 7, still aisle 2: a same-aisle round trip
            // through transit → no crossing.
            mv(robot, in_rack(7, 0), vec![(SkuId(7), 37)], 4_000),
            // Same-aisle direct shuffle (rack 7 → reserved at rack 7): no row.
            mv(in_rack(7, 0), ToteLocation::Reserved { rack_id: RackId(7), z_index: 1 }, vec![(SkuId(7), 37)], 5_000),
            // Empties, then crosses back to aisle 1 carrying nothing → no row.
            mv(ToteLocation::Reserved { rack_id: RackId(7), z_index: 1 }, robot, vec![], 6_000),
            mv(robot, in_rack(5, 0), vec![], 7_000),
        ],
        config,
    );

    assert_eq!(output.aisle_flux_log.len(), 2);
    let leave = &output.aisle_flux_log[0];
    assert_eq!(
        (leave.zone_id, leave.tote_id, leave.sku, leave.delta, leave.sim_time_ms),
        (1, 12, SkuId(7), -37, 2_000)
    );
    let enter = &output.aisle_flux_log[1];
    assert_eq!((enter.zone_id, enter.tote_id, enter.sku, enter.delta, enter.sim_time_ms), (2, 12, SkuId(7), 37, 2_000));
}

#[test]
fn inventory_delta_tracks_running_units() {
    use crate::events::InventoryDeltaEvent;
    use crate::model::ToteId;
    use crate::model::ids::SkuId;

    // This test exercises only the inventory_units ledger, not the per-aisle
    // flux log (which is driven by InventoryMove, not InventoryDelta).
    let delta = |d: i32, at_ms: u64| {
        SimEvent::InventoryDelta(InventoryDeltaEvent {
            delta: d,
            sku: SkuId(0),
            tote_id: ToteId(0),
            at: SimTime::ms(at_ms),
        })
    };
    // T=0: init seeding adds 1000 units total (one event per tote).
    // T=1s and T=3s: picks of 5 and 7 units (negative deltas).
    // T=3.5s: a putaway of 2 units (positive delta) — proves the metric
    // refills, unlike a picks-only proxy.
    let events = vec![delta(600, 0), delta(400, 0), delta(-5, 1_000), delta(-7, 3_000), delta(2, 3_500)];
    let config = MetricsConfig { slice_width_ms: 1_000, ..MetricsConfig::default() };
    let output = run_with_events_config(events, config);

    assert_eq!(output.time_slices[0].scalars.inventory_units, 1_000);
    assert_eq!(output.time_slices[1].scalars.inventory_units, 995);
    // Slice 2 has no events: extend_slices_through seeds it with the running total.
    assert_eq!(output.time_slices[2].scalars.inventory_units, 995);
    assert_eq!(output.time_slices[3].scalars.inventory_units, 990);
    assert_eq!(output.cumulative_scalars.inventory_units, 990);
}

/// The StripePlan bookend must land on the output verbatim — it is the
/// planned side of the Destinations reporting, and a dropped or mangled
/// snapshot would silently blank the planned matrix.
#[test]
fn stripe_plan_event_lands_on_output() {
    use crate::events::{StripePlanEntry, StripePlanEvent};
    use desgraph::StationId;
    let plan = StripePlanEvent {
        sim_time_ms: 0,
        stations: vec![StationId(7)],
        total_units: 3,
        entries: vec![StripePlanEntry {
            destination: Destination::DistributionCenter("A".into()),
            units: 3,
            eligible: vec![(StationId(7), 3)],
        }],
    };
    let output = run_with_events(vec![SimEvent::StripePlan(plan.clone())]);
    assert_eq!(output.stripe_plan_log, vec![plan]);
}

// Watchdog tests are in observer/watchdog.rs

#[test]
fn late_container_creation_stays_in_its_birth_slice_and_never_appears_early() {
    use crate::model::{ToteDesignation, ToteType};
    use crate::observer::time_slice::{ToteStateRecord, ToteVisualState};

    let output = run_with_events_config(
        vec![
            SimEvent::ToteInitialStates(ToteInitialStatesEvent {
                totes: vec![(1, ToteVisualState::InRack { cell_id: 4, level: 0 })],
                order_totes: vec![1],
            }),
            // Planned events can arrive out of simulation-time order, even within
            // a slice. Finalization must reconstruct their chronological state.
            SimEvent::ToteMoved(ToteMovedEvent {
                tote_id: ToteId(2),
                new_state: ToteVisualState::Extracted,
                time: SimTime::ms(2800),
            }),
            SimEvent::ToteCreated(ToteCreatedEvent {
                tote_id: ToteId(2),
                tote_type: ToteType::Carton,
                designation: ToteDesignation::Order,
                initial_state: ToteVisualState::AtCell { cell_id: 9 },
                time: SimTime::ms(2500),
            }),
            SimEvent::ToteMoved(ToteMovedEvent {
                tote_id: ToteId(1),
                new_state: ToteVisualState::OnBot { bot_id: 7 },
                time: SimTime::ms(4500),
            }),
        ],
        MetricsConfig { slice_width_ms: 1000, ..Default::default() },
    );
    assert_eq!(output.time_slices.len(), 5);
    for slice in &output.time_slices[..2] {
        assert!(slice.tote_state.creations.is_empty());
        assert!(slice.tote_state.start_states.iter().all(|(id, _)| *id != 2));
        assert!(slice.tote_state.transitions.iter().all(|(_, id, _)| *id != 2));
    }
    let birth = &output.time_slices[2].tote_state;
    assert!(birth.start_states.iter().all(|(id, _)| *id != 2));
    assert_eq!(birth.creations.len(), 1);
    assert_eq!((birth.creations[0].offset_ms, birth.creations[0].tote_id), (500, 2));
    assert_eq!(birth.creations[0].tote_type, ToteType::Carton);
    assert_eq!(birth.creations[0].designation, ToteDesignation::Order);
    assert_eq!(
        birth.transitions,
        vec![(500, 2, ToteVisualState::AtCell { cell_id: 9 }), (800, 2, ToteVisualState::Extracted)]
    );
    assert_eq!(
        output.time_slices[0].tote_state.order_totes,
        vec![1],
        "late birth must not rewrite the startup designation list"
    );
    for slice in &output.time_slices[3..] {
        assert!(slice.tote_state.creations.is_empty());
        let states = ToteStateRecord::positions_at(
            output.time_slices.iter().map(|slice| (slice.start_ms, &slice.tote_state)),
            slice.start_ms,
        );
        assert!(states.contains(&(2, ToteVisualState::Extracted)));
    }
    assert_eq!(output.event_digest.by_kind.get("ToteCreated"), Some(&1));
}

#[test]
fn completed_work_uses_dwell_end_and_the_inventory_visit_identity() {
    let make = |kind: CycleKind, start: u64, visit: Option<u32>| {
        SimEvent::CycleComplete(CycleCompleteEvent {
            robot_id: RobotId(5),
            robot_type: RobotType::Htm,
            kind,
            station_id: if matches!(kind, CycleKind::OrderExtract { .. }) { StationId(4) } else { StationId(3) },
            sim_time_ms: start,
            dwell_ms: 9_000,
            arrival_source: None,
            dc_id: None,
            presentation_id: Some(PresentationId(77)),
            tote_presentation: visit.map(|id| TotePresentation {
                order_id: OrderId(1),
                tote_id: ToteId(id),
                source_zone_id: ZoneId(2),
                source_relay_id: RelayId(4),
                presentation_id: PresentationId(id as u64),
            }),
        })
    };
    let output = run_with_events_config(
        vec![
            make(CycleKind::Pick { sku: SkuId(0), qty: 2 }, 5_000, Some(20)),
            make(CycleKind::Pick { sku: SkuId(0), qty: 3 }, 15_000, Some(21)),
            make(CycleKind::OrderExtract { qty: 5 }, 25_000, None),
        ],
        MetricsConfig {
            slice_width_ms: 10_000,
            station_types: [(StationId(3), StationType::Pick), (StationId(4), StationType::Extraction)]
                .into_iter()
                .collect(),
            ..MetricsConfig::default()
        },
    );
    assert_eq!(output.time_slices[0].scalars.picks_completed, 0);
    assert_eq!(output.time_slices[1].scalars.picks_completed, 2);
    assert_eq!(output.time_slices[2].scalars.picks_completed, 3);
    assert_eq!(output.time_slices[2].scalars.order_extract_presentations, 0);
    assert_eq!(output.time_slices[3].scalars.order_extract_presentations, 1);
    assert_eq!(output.time_slices[3].scalars.order_extracted_units, 5);
    assert_eq!(
        output.pick_log.iter().map(|pick| (pick.sim_time_ms, pick.presentation_id)).collect::<Vec<_>>(),
        [(14_000, Some(20)), (24_000, Some(21))]
    );
    assert_eq!(output.extraction_log[0].sim_time_ms, 34_000);
    assert_eq!(output.extraction_log[0].presentation_id, Some(77));
    assert_eq!(output.sim_elapsed_ms, 34_000);
}

#[test]
fn occupancy_replays_actual_changes_before_integrating_out_of_order_siblings() {
    use crate::metrics::reporting::{MetricScope, ReportPeriod, ReportRequest, aisle_occupancy_report};
    use crate::observer::time_slice::ToteVisualState;
    use std::sync::Arc;

    let config = MetricsConfig {
        slice_width_ms: 50,
        total_rack_slots: 2,
        rack_cell_zone: Arc::new([(10, 0), (11, 0)].into_iter().collect()),
        zone_capacity: Arc::new([(0, 2)].into_iter().collect()),
        relay_cell_zone: Arc::new([(50, 0)].into_iter().collect()),
        relay_zone_capacity: Arc::new([(0, 1)].into_iter().collect()),
        ..MetricsConfig::default()
    };
    let moved = |id, at, state| {
        SimEvent::ToteMoved(ToteMovedEvent { tote_id: ToteId(id), time: SimTime::ms(at), new_state: state })
    };
    let events = vec![
        SimEvent::ToteInitialStates(ToteInitialStatesEvent {
            totes: vec![
                (0, ToteVisualState::InRack { cell_id: 10, level: 0 }),
                (1, ToteVisualState::InRack { cell_id: 11, level: 0 }),
            ],
            order_totes: vec![0, 1],
        }),
        SimEvent::ToteContentsChanged(ToteContentsChangedEvent { tote_id: ToteId(0), units: 3, at: SimTime::ZERO }),
        SimEvent::ToteContentsChanged(ToteContentsChangedEvent { tote_id: ToteId(1), units: 4, at: SimTime::ZERO }),
        moved(0, 100, ToteVisualState::OnBot { bot_id: 9 }),
        moved(0, 200, ToteVisualState::InRack { cell_id: 10, level: 0 }),
        moved(1, 150, ToteVisualState::AtRelay { cell_id: 50 }),
        moved(1, 250, ToteVisualState::InRack { cell_id: 11, level: 0 }),
        // A filled order tote is emptied without changing inventory stock.
        SimEvent::ToteContentsChanged(ToteContentsChangedEvent { tote_id: ToteId(0), units: 0, at: SimTime::ms(275) }),
    ];
    let unordered = run_with_events_config(events.clone(), config.clone());
    let mut ordered = events;
    ordered.swap(4, 5);
    let ordered = run_with_events_config(ordered, config);
    assert_eq!(unordered.aisle_state_log, ordered.aisle_state_log);
    assert_eq!(
        unordered.time_slices.iter().map(|slice| slice.scalars.rack.occupancy_total).collect::<Vec<_>>(),
        [2, 2, 1, 0, 1, 1]
    );
    assert_eq!(unordered.cumulative_scalars.inventory_units, 0, "order contents are not available stock");
    let request = ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, 300, 0);
    let report = aisle_occupancy_report(&unordered.aisle_state_log, request).unwrap();
    let row = &report.data.rows[0];
    assert_eq!(row.storage_occupied.unwrap().numerator, 400);
    assert_eq!(row.storage_non_empty.unwrap().numerator, 375);
    assert_eq!(row.storage_empty.unwrap().numerator, 25);
    assert_eq!(row.diving_board_occupied.unwrap().numerator, 100);
}

/// The barrier's advance marker is not a warehouse fact: the observer reads it
/// and neither forwards, digests nor counts it.
#[test]
fn the_barrier_marker_is_read_but_not_forwarded_digested_or_counted() {
    let (tx, rx) = mpsc::channel();
    let (subscriber, forwarded) = mpsc::channel();
    for event in [
        SimEvent::BarrierAdvanced(SimTime::ms(1_000)),
        SimEvent::OrderComplete(OrderCompleteEvent { order_id: OrderId(1), sim_time_ms: 2_000 }),
        SimEvent::BarrierAdvanced(SimTime::ms(3_000)),
    ] {
        tx.send(event).unwrap();
    }
    drop(tx);
    let output = SimObserver::new(MetricsConfig::default(), rx)
        .subscribe_events(subscriber)
        .run()
        .expect("the facts break no rule of the event stream");
    assert_eq!(output.total_events, 1);
    assert_eq!(output.event_digest.by_kind.into_iter().collect::<Vec<_>>(), [("OrderComplete".to_string(), 1)]);
    assert!(matches!(forwarded.try_iter().collect::<Vec<_>>()[..], [SimEvent::OrderComplete(_)]));
}

/// A D30's pickup says it stood waiting at the relay for 2 s before its fork-Get at 3 s, while its nav says it was
/// still travelling there until 3 s: two committed facts over one stretch of one robot's time. The run stops on them
/// at the first barrier it reads after them, rather than let the one read first stand, and reads no fact past that
/// barrier: the nav stamped behind it would stop the run for that instead.
#[test]
fn a_relay_wait_over_the_travel_its_nav_commits_stops_the_run() {
    let events = vec![
        SimEvent::InitialPositions(InitialPositionsEvent {
            positions: vec![initial_pos(1, 0, RobotType::Htm)],
            htm_headings_rad: Vec::new(),
        }),
        SimEvent::Nav(NavEventData {
            robot_id: RobotId(1),
            waypoints: waypoint_chain(SimTime::ZERO, 0, &[(1, SimDuration::seconds(1)), (2, SimDuration::seconds(2))]),
            robot_type: RobotType::Htm,
            time: SimTime::ZERO,
            distance_mm: 0,
            carrying_cargo: false,
        }),
        SimEvent::HtmRelayPickup(HtmRelayPickupEvent {
            robot_id: RobotId(1),
            relay_id: RelayId(0),
            start: SimTime::seconds(3),
            end: SimTime::seconds(4),
            wait: SimDuration::seconds(2),
            resident: false,
        }),
        SimEvent::BarrierAdvanced(SimTime::seconds(1)),
        SimEvent::Nav(NavEventData {
            robot_id: RobotId(1),
            waypoints: waypoint_chain(SimTime::ZERO, 2, &[(1, SimDuration::seconds(1))]),
            robot_type: RobotType::Htm,
            time: SimTime::ZERO,
            distance_mm: 0,
            carrying_cargo: false,
        }),
    ];
    let mut config = MetricsConfig::default();
    super::test_models::bind_legacy_nav_models(&mut config, &events);
    let (tx, rx) = mpsc::channel();
    events.into_iter().for_each(|event| tx.send(event).unwrap());
    drop(tx);
    let refused = SimObserver::new(config, rx).run().expect_err("one robot committed to travel and a wait at once");
    let StreamViolation::CommittedSpansOverlap(overlap) = refused else { panic!("{refused}") };
    assert_eq!(overlap.robot, RobotId(1));
    assert_eq!((overlap.first.0, overlap.first.1), (SimTime::ZERO, SimTime::seconds(3)), "the travel, read first");
    assert_eq!((overlap.second.0, overlap.second.1), (SimTime::seconds(1), SimTime::seconds(3)), "the wait");
}
