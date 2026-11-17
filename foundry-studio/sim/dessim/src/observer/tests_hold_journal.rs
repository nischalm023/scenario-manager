//! Within `observer`, `tests_hold_journal` pins the #272 HOLD JOURNAL: that
//! the observer writes one `Wait` row per interval the solved schedule leaves
//! between two committed actions, each carrying the cause and the cell kind
//! the D30 state family attributes from — and that the `Travel` rows around
//! them cover the rest of the chain, so the two together leave no unattributed
//! space inside a leg.
//!
//! Sibling of `tests_duty` (duty clocks) and `tests_htm_kpi_duty` (KPI
//! overlays): those assert what a run TOTALS, this asserts what it RECORDS.

use std::sync::Arc;
use std::sync::mpsc;

use desgraph::{CellId, CellKind, RelayId, RobotType, StationId, StationType, WorldPos};
use rustc_hash::FxHashMap;

use crate::Orientation;
use crate::events::*;
use crate::metrics::reporting::{D30State, MetricScope, ReportPeriod, ReportRequest, d30_state_report};
use crate::model::ids::PresentationId;
use crate::model::{OrderId, RobotId, SimDuration, SimTime, SkuId, StationArrivalSource};
use crate::observer::activity::ActivityViolation;
use crate::observer::output::{
    Activity, HtmHold, HtmJournalEndpoint, HtmLegRecord, HtmWaitCause, HtmWaitPosition, MetricsOutput,
};
use crate::observer::time_slice::MetricsConfig;
use crate::pathfinding::{Waypoint, WaypointAction};

use super::{SimObserver, StreamViolation};

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

const ROBOT: u32 = 7;
const STATION: u32 = 3;
const PUTAWAY_STATION: u32 = 4;
const RELAY: u32 = 2;

/// Cell ids, laid out so every hold position under test has a cell whose kind
/// the map table names.
const CHARGING_CELL: u32 = 10;
const FLOOR_CELL: u32 = 11;
const HOLD_CELL: u32 = 12;
const PRESENTATION_CELL: u32 = 13;
const QUEUE_CELL: u32 = 14;
const RELAY_CELL: u32 = 15;

/// A config whose cell-kind table names each fixture cell, so hold positions
/// resolve the way a real run's would.
fn config() -> MetricsConfig {
    let mut cell_kinds = vec![None; 16];
    cell_kinds[CHARGING_CELL as usize] = Some(CellKind::Charging);
    cell_kinds[FLOOR_CELL as usize] = Some(CellKind::Open);
    cell_kinds[HOLD_CELL as usize] = Some(CellKind::Highway);
    cell_kinds[PRESENTATION_CELL as usize] = Some(CellKind::Presentation);
    cell_kinds[QUEUE_CELL as usize] = Some(CellKind::QueueCell);
    cell_kinds[RELAY_CELL as usize] = Some(CellKind::Exchange); // Relay identity comes from HTM-access metadata.

    let mut station_types = FxHashMap::default();
    station_types.insert(StationId(STATION), StationType::Pick);
    station_types.insert(StationId(PUTAWAY_STATION), StationType::Putaway);
    let mut station_cells = FxHashMap::default();
    station_cells.insert(StationId(STATION), CellId(PRESENTATION_CELL));
    station_cells.insert(StationId(PUTAWAY_STATION), CellId(QUEUE_CELL));
    let mut relay_htm_access_cells = FxHashMap::default();
    relay_htm_access_cells.insert(RELAY, RELAY_CELL);

    MetricsConfig {
        slice_width_ms: 10_000,
        station_types,
        station_cells,
        relay_htm_access_cells,
        cell_kinds: Arc::new(cell_kinds),
        ..MetricsConfig::default()
    }
}

fn observe(events: Vec<SimEvent>) -> Result<MetricsOutput, StreamViolation> {
    let mut config = config();
    super::test_models::bind_legacy_nav_models(&mut config, &events);
    let (tx, rx) = mpsc::channel();
    for event in events {
        tx.send(event).unwrap();
    }
    drop(tx);
    SimObserver::new(config, rx).run()
}

fn run(events: Vec<SimEvent>) -> MetricsOutput {
    observe(events).expect("the facts break no rule of the event stream")
}

fn seed(cell: u32) -> SimEvent {
    SimEvent::InitialPositions(InitialPositionsEvent {
        positions: vec![(RobotId(ROBOT), CellId(cell), RobotType::Htm, WorldPos::default())],
        htm_headings_rad: Vec::new(),
    })
}

fn trip(purpose: TripPurpose, to: TripEndpoint, time_ms: u64) -> SimEvent {
    trip_for(purpose, to, time_ms, None)
}

fn trip_for(purpose: TripPurpose, to: TripEndpoint, time_ms: u64, presentation_id: Option<PresentationId>) -> SimEvent {
    SimEvent::TripStart(TripStartEvent {
        robot_id: RobotId(ROBOT),
        robot_type: RobotType::Htm,
        purpose,
        presentation_id,
        from: None,
        to,
        sim_time_ms: time_ms,
    })
}

/// The arrival that opens a station visit — the milestone `HtmDeliver`
/// reports when the bot reaches the presentation cell.
fn arrival(time_ms: u64) -> SimEvent {
    arrival_at(STATION, time_ms)
}

/// An arrival naming the station it happened at. A fixture that departs for
/// one station and lands at another is not a run the simulator can produce,
/// and the presentation cell is resolved per station, so the two have to agree.
fn arrival_at(station: u32, time_ms: u64) -> SimEvent {
    SimEvent::HtmArrival(HtmArrivalEvent {
        robot_id: RobotId(ROBOT),
        station_id: StationId(station),
        arrival: SimTime::ms(time_ms),
    })
}

fn move_wp(cell: u32, start_ms: u64, end_ms: u64) -> Waypoint {
    Waypoint {
        cell: CellId(cell),
        orientation: Orientation::East,
        action: WaypointAction::Move {
            start: SimTime::ms(start_ms),
            end: SimTime::ms(end_ms),
            entry_velocity_mm_s: 0.0,
            exit_velocity_mm_s: 0.0,
        },
    }
}

fn wait_wp(cell: u32, start_ms: u64, end_ms: u64) -> Waypoint {
    Waypoint {
        cell: CellId(cell),
        orientation: Orientation::East,
        action: WaypointAction::Wait { start: SimTime::ms(start_ms), end: SimTime::ms(end_ms) },
    }
}

fn nav(time_ms: u64, distance_mm: u64, waypoints: Vec<Waypoint>) -> SimEvent {
    SimEvent::Nav(NavEventData {
        robot_id: RobotId(ROBOT),
        robot_type: RobotType::Htm,
        waypoints,
        time: SimTime::ms(time_ms),
        distance_mm,
        carrying_cargo: false,
    })
}

fn holds(output: &MetricsOutput) -> Vec<(HtmHold, u64, u64)> {
    output
        .htm_leg_log
        .iter()
        .filter(|row| row.activity == Activity::Wait)
        .map(|row| (row.hold.expect("live wait rows carry hold facts"), row.start.as_ms(), row.duration.as_ms()))
        .collect()
}

/// Holds other than the seeded bot's idle bookends. Every fixture seeds a
/// robot that starts undispatched, so gaps bracket the case
/// under test; that row is pinned on its own in
/// `the_no_task_split_reads_the_tracked_cell_not_the_endpoint`.
fn task_holds(output: &MetricsOutput) -> Vec<(HtmHold, u64, u64)> {
    holds(output).into_iter().filter(|(hold, ..)| hold.cause != HtmWaitCause::NoTask).collect()
}

fn travels(output: &MetricsOutput) -> Vec<&HtmLegRecord> {
    output.htm_leg_log.iter().filter(|row| row.activity == Activity::Travel).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// A `Wait` waypoint the pathfinder baked into the route becomes its own row,
/// and the travel rows on either side stop and restart around it — so the
/// chain's whole span is covered by rows instead of leaving the hold in the
/// space between them. Distance splits by move time and sums back exactly.
#[test]
fn an_in_path_wait_splits_the_leg_and_journals_a_route_hold() {
    let output = run(vec![
        seed(FLOOR_CELL),
        trip(TripPurpose::ToRelay, TripEndpoint::Relay(RelayId(RELAY)), 0),
        nav(
            0,
            900,
            vec![
                Waypoint {
                    cell: CellId(FLOOR_CELL),
                    orientation: Orientation::East,
                    action: WaypointAction::PathStart { time: SimTime::ms(0) },
                },
                move_wp(HOLD_CELL, 0, 1_000),
                wait_wp(HOLD_CELL, 1_000, 4_000),
                move_wp(RELAY_CELL, 4_000, 6_000),
            ],
        ),
    ]);

    let travels = travels(&output);
    assert_eq!(
        travels.iter().map(|row| (row.start.as_ms(), row.duration.as_ms())).collect::<Vec<_>>(),
        [(0, 1_000), (4_000, 2_000)],
        "one row per run of moves, on the real clock"
    );
    assert_eq!(travels.iter().map(|row| row.distance_mm).sum::<u64>(), 900, "the split conserves the event's distance");
    assert_eq!(travels.iter().map(|row| row.distance_mm).collect::<Vec<_>>(), [300, 600]);
    assert_eq!(
        holds(&output),
        [(HtmHold { cause: HtmWaitCause::RouteHold, position: HtmWaitPosition::Route }, 1_000, 3_000)]
    );
    // Travel + hold cover [0, 6_000) exactly: no space left between rows.
    let covered: u64 = output.htm_leg_log.iter().map(|row| row.duration.as_ms()).sum();
    assert_eq!(covered, 6_000);
}

/// A station visit journals the hold before the operator dwell. The return
/// begins exactly when the dwell ends; there is no post-dwell occupancy.
#[test]
fn a_station_visit_journals_the_pre_dwell_hold() {
    let output = run(vec![
        seed(FLOOR_CELL),
        arrival(1_000),
        trip(TripPurpose::ToStation, TripEndpoint::Station(StationId(STATION)), 0),
        nav(0, 100, vec![move_wp(PRESENTATION_CELL, 0, 1_000)]),
        // The visit opens on arrival; the dwell runs [3_000, 8_000), and the
        // return departs at the dwell end.
        SimEvent::CycleComplete(CycleCompleteEvent {
            robot_id: RobotId(ROBOT),
            robot_type: RobotType::Htm,
            kind: CycleKind::Pick { sku: SkuId(1), qty: 1 },
            station_id: StationId(STATION),
            sim_time_ms: 3_000,
            dwell_ms: 5_000,
            arrival_source: None,
            dc_id: None,
            tote_presentation: None,
            presentation_id: None,
        }),
        trip(TripPurpose::ToReturnRelay, TripEndpoint::Relay(RelayId(RELAY)), 8_000),
        // Live HtmStore reports its TripStart, then its Nav, which moves
        // bot_positions on to the relay; the visit's holds are placed from
        // the station's own cell.
        nav(8_000, 100, vec![move_wp(RELAY_CELL, 8_000, 9_000)]),
    ]);

    assert_eq!(
        task_holds(&output),
        [(
            HtmHold { cause: HtmWaitCause::AwaitingStationService, position: HtmWaitPosition::Presentation },
            1_000,
            2_000
        )]
    );
}

/// A post-dwell stand that a committed departure closes is real occupancy:
/// the store leg's admitted start can now land after the presentation
/// finished, and the D30 stands on the presentation cell until it. The return
/// leg's departure is the fact that bounds it, so the row carries
/// `AwaitingDeparture` and the D30 report accepts it.
#[test]
fn a_departure_bounded_post_dwell_stand_is_journaled_as_awaiting_departure() {
    let output = run(vec![
        seed(FLOOR_CELL),
        arrival(1_000),
        trip(TripPurpose::ToStation, TripEndpoint::Station(StationId(STATION)), 0),
        nav(0, 100, vec![move_wp(PRESENTATION_CELL, 0, 1_000)]),
        SimEvent::CycleComplete(CycleCompleteEvent {
            robot_id: RobotId(ROBOT),
            robot_type: RobotType::Htm,
            kind: CycleKind::Pick { sku: SkuId(1), qty: 1 },
            station_id: StationId(STATION),
            sim_time_ms: 3_000,
            dwell_ms: 5_000,
            arrival_source: None,
            dc_id: None,
            tote_presentation: None,
            presentation_id: None,
        }),
        trip(TripPurpose::ToReturnRelay, TripEndpoint::Relay(RelayId(RELAY)), 9_000),
    ]);

    assert!(task_holds(&output).contains(&(
        HtmHold { cause: HtmWaitCause::AwaitingDeparture, position: HtmWaitPosition::Presentation },
        8_000,
        1_000,
    )));
}

/// A station visit the run stopped inside before its departure was scheduled
/// stands after its dwell until the current world time, when that is later,
/// since a departure scheduled later could not start before it. The bot
/// loitering on the presentation cell from its dwell's end until then is
/// journaled as awaiting departure, not as a gap, and nothing journals a tail
/// past it for the D30 report to reject (ruling #49).
#[test]
fn a_station_visit_the_run_stopped_inside_stands_until_the_current_world_time() {
    let output = run(vec![
        seed(FLOOR_CELL),
        arrival(1_000),
        trip(TripPurpose::ToStation, TripEndpoint::Station(StationId(STATION)), 0),
        nav(0, 100, vec![move_wp(PRESENTATION_CELL, 0, 1_000)]),
        SimEvent::CycleComplete(CycleCompleteEvent {
            robot_id: RobotId(ROBOT),
            robot_type: RobotType::Htm,
            kind: CycleKind::Pick { sku: SkuId(1), qty: 1 },
            station_id: StationId(STATION),
            sim_time_ms: 3_000,
            dwell_ms: 5_000,
            arrival_source: None,
            dc_id: None,
            tote_presentation: None,
            presentation_id: None,
        }),
        // B passes the dwell's end with no departure scheduled, and the run's
        // last fact comes after it.
        SimEvent::BarrierAdvanced(SimTime::ms(12_000)),
        SimEvent::OrderComplete(OrderCompleteEvent { order_id: OrderId(1), sim_time_ms: 20_000 }),
    ]);

    assert_eq!(
        task_holds(&output),
        [
            (
                HtmHold { cause: HtmWaitCause::AwaitingStationService, position: HtmWaitPosition::Presentation },
                1_000,
                2_000,
            ),
            (HtmHold { cause: HtmWaitCause::AwaitingDeparture, position: HtmWaitPosition::Presentation }, 8_000, 4_000),
        ],
        "the hold before the dwell, and the stand after it until the current world time"
    );
}

/// The same stop before the presentation was committed: no dwell is recorded,
/// so nobody served the bot, and the whole visit, to the current world time,
/// awaits station service. None of it is Station processing, which is the
/// recorded dwell and nothing else (#272).
#[test]
fn a_station_visit_the_run_stopped_inside_with_no_dwell_awaits_station_service() {
    let output = run(vec![
        seed(FLOOR_CELL),
        arrival(1_000),
        trip(TripPurpose::ToStation, TripEndpoint::Station(StationId(STATION)), 0),
        nav(0, 100, vec![move_wp(PRESENTATION_CELL, 0, 1_000)]),
        // B passes the arrival with no presentation committed, and the run's
        // last fact comes after it.
        SimEvent::BarrierAdvanced(SimTime::ms(12_000)),
        SimEvent::OrderComplete(OrderCompleteEvent { order_id: OrderId(1), sim_time_ms: 20_000 }),
    ]);

    assert_eq!(
        task_holds(&output),
        [(
            HtmHold { cause: HtmWaitCause::AwaitingStationService, position: HtmWaitPosition::Presentation },
            1_000,
            11_000,
        )],
        "the whole visit, from the arrival to the current world time"
    );
    let request = ReportRequest::for_run(MetricScope::FullRun, ReportPeriod::Overall, output.sim_elapsed_ms, 0);
    let report = d30_state_report(&output.htm_leg_log, 1, request).expect("the journal breaks no D30 report rule");
    let processing = report.data.state_rows.iter().find(|row| row.state == D30State::StationProcessing);
    assert_eq!(processing.and_then(|row| row.duration_ms), Some(0), "no dwell, so no Station processing");
}

/// Offline planning may report cycle N+1's future-stamped completion before
/// cycle N's visit closes. Pending dwells are joined by containment, so the
/// later fact cannot take the earlier one's place and fabricate a post-dwell
/// tail.
#[test]
fn a_future_cycle_completion_does_not_replace_the_open_presentation() {
    let first = PresentationId(1);
    let second = PresentationId(2);
    let cycle = |sku, at, presentation_id| {
        SimEvent::CycleComplete(CycleCompleteEvent {
            robot_id: RobotId(ROBOT),
            robot_type: RobotType::Htm,
            kind: CycleKind::Pick { sku: SkuId(sku), qty: 1 },
            station_id: StationId(STATION),
            sim_time_ms: at,
            dwell_ms: 5_000,
            arrival_source: None,
            dc_id: None,
            tote_presentation: None,
            presentation_id: Some(presentation_id),
        })
    };
    let output = run(vec![
        seed(FLOOR_CELL),
        arrival(1_000),
        trip_for(TripPurpose::ToStation, TripEndpoint::Station(StationId(STATION)), 0, Some(first)),
        nav(0, 100, vec![move_wp(PRESENTATION_CELL, 0, 1_000)]),
        cycle(1, 3_000, first),
        // Future-stamped second cycle arrives before the first close.
        cycle(2, 13_000, second),
        trip_for(TripPurpose::ToReturnRelay, TripEndpoint::Relay(RelayId(RELAY)), 8_000, Some(first)),
        trip_for(TripPurpose::ToStation, TripEndpoint::Station(StationId(STATION)), 10_000, Some(second)),
        arrival(10_000),
        trip_for(TripPurpose::ToReturnRelay, TripEndpoint::Relay(RelayId(RELAY)), 18_000, Some(second)),
    ]);

    assert_eq!(
        task_holds(&output),
        [
            (
                HtmHold { cause: HtmWaitCause::AwaitingStationService, position: HtmWaitPosition::Presentation },
                1_000,
                2_000,
            ),
            (
                HtmHold { cause: HtmWaitCause::AwaitingStationService, position: HtmWaitPosition::Presentation },
                10_000,
                3_000,
            ),
        ],
        "both presentations close against their own dwell; no post-dwell tripwire is fabricated"
    );
}

/// A later presentation's future-stamped TripStart can also be read before the
/// current visit closes. The close must use the context frozen when the visit
/// opened, not the mutable current trip.
#[test]
fn a_future_trip_does_not_replace_the_open_presentation() {
    let first = PresentationId(1);
    let second = PresentationId(2);
    let cycle = |sku, at, presentation_id| {
        SimEvent::CycleComplete(CycleCompleteEvent {
            robot_id: RobotId(ROBOT),
            robot_type: RobotType::Htm,
            kind: CycleKind::Pick { sku: SkuId(sku), qty: 1 },
            station_id: StationId(STATION),
            sim_time_ms: at,
            dwell_ms: 5_000,
            arrival_source: None,
            dc_id: None,
            tote_presentation: None,
            presentation_id: Some(presentation_id),
        })
    };
    let output = run(vec![
        seed(FLOOR_CELL),
        arrival(1_000),
        trip_for(TripPurpose::ToStation, TripEndpoint::Station(StationId(STATION)), 0, Some(first)),
        nav(0, 100, vec![move_wp(PRESENTATION_CELL, 0, 1_000)]),
        cycle(1, 3_000, first),
        cycle(2, 13_000, second),
        // Offline planning exposes the next cycle's trip before the return leg
        // that closes the first visit, so the stream is out of instant order.
        trip_for(TripPurpose::ToStation, TripEndpoint::Station(StationId(STATION)), 10_000, Some(second)),
        trip_for(TripPurpose::ToReturnRelay, TripEndpoint::Relay(RelayId(RELAY)), 8_000, Some(first)),
        arrival(10_000),
    ]);

    assert_eq!(
        task_holds(&output),
        [
            (
                HtmHold { cause: HtmWaitCause::AwaitingStationService, position: HtmWaitPosition::Presentation },
                1_000,
                2_000,
            ),
            (
                HtmHold { cause: HtmWaitCause::AwaitingStationService, position: HtmWaitPosition::Presentation },
                10_000,
                3_000,
            ),
        ],
        "each close uses the presentation context captured when its visit opened"
    );
}

/// A live route hold on a station queue cell is In Queue, not Congestion.
/// A station visit does not open until the D30 reaches the presentation cell,
/// so the queue fact comes from the inbound nav chain, not from the visit.
#[test]
fn an_in_path_queue_hold_reads_as_in_queue() {
    let output = run(vec![
        seed(FLOOR_CELL),
        trip(TripPurpose::ToStation, TripEndpoint::Station(StationId(STATION)), 0),
        nav(
            0,
            200,
            vec![
                move_wp(QUEUE_CELL, 0, 1_000),
                wait_wp(QUEUE_CELL, 1_000, 3_000),
                move_wp(PRESENTATION_CELL, 3_000, 4_000),
            ],
        ),
        arrival(4_000),
        SimEvent::CycleComplete(CycleCompleteEvent {
            robot_id: RobotId(ROBOT),
            robot_type: RobotType::Htm,
            kind: CycleKind::Pick { sku: SkuId(1), qty: 1 },
            station_id: StationId(STATION),
            sim_time_ms: 4_000,
            dwell_ms: 5_000,
            arrival_source: None,
            dc_id: None,
            tote_presentation: None,
            presentation_id: None,
        }),
        trip(TripPurpose::ToReturnRelay, TripEndpoint::Relay(RelayId(RELAY)), 9_000),
        nav(9_000, 100, vec![move_wp(RELAY_CELL, 9_000, 10_000)]),
    ]);

    assert_eq!(
        task_holds(&output),
        [(HtmHold { cause: HtmWaitCause::RouteHold, position: HtmWaitPosition::StationQueue }, 1_000, 2_000)],
        "the live queue wait is positional even though its task cause is RouteHold"
    );
}

/// The return-slot wait is journaled at the relay's access cell, from the
/// per-robot `HtmRelayDeposit` fact. The schedule does not hold the D30 at
/// the station for a slot — its departure is the presentation action-ready
/// time — so there is no origin-side row to look for (#272's open question).
#[test]
fn a_return_slot_wait_is_journaled_at_the_relay_access_cell() {
    let output = run(vec![
        seed(PRESENTATION_CELL),
        trip(TripPurpose::ToReturnRelay, TripEndpoint::Relay(RelayId(RELAY)), 0),
        nav(0, 500, vec![move_wp(RELAY_CELL, 0, 2_000)]),
        SimEvent::HtmRelayDeposit(HtmRelayDepositEvent {
            robot_id: RobotId(ROBOT),
            relay_id: RelayId(RELAY),
            start: SimTime::ms(5_000),
            end: SimTime::ms(7_000),
            wait: SimDuration::ms(3_000),
        }),
    ]);

    assert_eq!(
        task_holds(&output),
        [(HtmHold { cause: HtmWaitCause::AwaitingSlot, position: HtmWaitPosition::Relay }, 2_000, 3_000)],
        "the dwell runs from arrival to deposit, on the access cell"
    );
    // It sits strictly after the leg's travel row, so it double-counts nothing.
    let travel = travels(&output)[0];
    assert_eq!((travel.start.as_ms(), travel.duration.as_ms()), (0, 2_000));
}

/// A positive gap between the return path's start and its first action is a
/// route hold on the presentation cell: the path search waits out the corridor
/// the D30 leaves through, and both ends of the gap are the D30's own
/// committed actions. It is ordinary occupancy, not ruling #49's tripwire —
/// that one is the factless tail, station time past both the current world
/// time and the recorded dwell's end, which nothing journals.
#[test]
fn an_initial_return_path_gap_is_journaled_at_presentation() {
    let output = run(vec![
        seed(PRESENTATION_CELL),
        SimEvent::TripStart(TripStartEvent {
            robot_id: RobotId(ROBOT),
            robot_type: RobotType::Htm,
            purpose: TripPurpose::ToReturnRelay,
            presentation_id: Some(PresentationId(1)),
            from: Some(TripEndpoint::Station(StationId(STATION))),
            to: TripEndpoint::Relay(RelayId(RELAY)),
            sim_time_ms: 8_000,
        }),
        nav(
            8_000,
            100,
            vec![
                Waypoint {
                    cell: CellId(PRESENTATION_CELL),
                    orientation: Orientation::East,
                    action: WaypointAction::PathStart { time: SimTime::ms(8_000) },
                },
                move_wp(RELAY_CELL, 9_000, 10_000),
            ],
        ),
    ]);

    let hold = output.htm_leg_log.iter().find(|row| row.activity == Activity::Wait).expect("initial return-path hold");
    assert_eq!((hold.start.as_ms(), hold.duration.as_ms()), (8_000, 1_000));
    assert_eq!(hold.from, HtmJournalEndpoint::PickStation(StationId(STATION)));
    assert_eq!(hold.to, HtmJournalEndpoint::Relay(RelayId(RELAY)));
    assert_eq!(hold.hold, Some(HtmHold { cause: HtmWaitCause::RouteHold, position: HtmWaitPosition::Presentation }));
    assert!(
        !output
            .htm_leg_log
            .iter()
            .any(|row| { row.hold.is_some_and(|hold| hold.cause == HtmWaitCause::PostStationService) })
    );
}

/// A path hold that occupies a presentation-kind cell partway along a leg is
/// the same route fact as the leg's opening one — nothing about the chain
/// position changes the cause.
#[test]
fn a_mid_route_presentation_hold_is_not_post_station_service() {
    let output = run(vec![
        seed(PRESENTATION_CELL),
        SimEvent::TripStart(TripStartEvent {
            robot_id: RobotId(ROBOT),
            robot_type: RobotType::Htm,
            purpose: TripPurpose::ToReturnRelay,
            presentation_id: Some(PresentationId(1)),
            from: Some(TripEndpoint::Station(StationId(STATION))),
            to: TripEndpoint::Relay(RelayId(RELAY)),
            sim_time_ms: 0,
        }),
        nav(
            0,
            100,
            vec![
                Waypoint {
                    cell: CellId(PRESENTATION_CELL),
                    orientation: Orientation::East,
                    action: WaypointAction::PathStart { time: SimTime::ms(0) },
                },
                move_wp(FLOOR_CELL, 0, 1_000),
                wait_wp(PRESENTATION_CELL, 1_000, 2_000),
                move_wp(RELAY_CELL, 2_000, 3_000),
            ],
        ),
    ]);

    assert!(task_holds(&output).contains(&(
        HtmHold { cause: HtmWaitCause::RouteHold, position: HtmWaitPosition::Presentation },
        1_000,
        1_000,
    )));
    assert!(
        !output
            .htm_leg_log
            .iter()
            .any(|row| { row.hold.is_some_and(|hold| hold.cause == HtmWaitCause::PostStationService) })
    );
}

/// The parked-vs-elsewhere split is positional: it reads the cell the D30 was
/// tracked at, never the journal endpoint (which doubles as the fallback for
/// an unknown trip origin, and would call every unplaced idle "parked").
#[test]
fn the_no_task_split_reads_the_tracked_cell_not_the_endpoint() {
    for (idle_cell, expected) in [
        (CHARGING_CELL, HtmWaitPosition::Charging),
        (FLOOR_CELL, HtmWaitPosition::Route),
        (RELAY_CELL, HtmWaitPosition::Relay),
    ] {
        let output = run(vec![
            seed(idle_cell),
            trip(TripPurpose::ToRelay, TripEndpoint::Relay(RelayId(RELAY)), 4_000),
            // Live producer order: the trip and its whole future nav are
            // reported together, as the task commits the leg.
            nav(4_000, 100, vec![move_wp(RELAY_CELL, 4_000, 5_000)]),
        ]);

        assert_eq!(
            holds(&output),
            [(HtmHold { cause: HtmWaitCause::NoTask, position: expected }, 0, 4_000)],
            "idle at cell {idle_cell}"
        );
    }
}

/// With no cell-kind table (a mapless or unit-test run) a hold is journaled
/// with its cause and an honestly unplaced position — never defaulted to
/// floor, and never to parked.
#[test]
fn a_hold_with_no_map_table_is_unplaced_not_defaulted() {
    let (tx, rx) = mpsc::channel();
    for event in [seed(CHARGING_CELL), trip(TripPurpose::ToRelay, TripEndpoint::Relay(RelayId(RELAY)), 1_000)] {
        tx.send(event).unwrap();
    }
    drop(tx);
    let output =
        SimObserver::new(MetricsConfig::default(), rx).run().expect("the facts break no rule of the event stream");

    assert_eq!(
        holds(&output),
        [(HtmHold { cause: HtmWaitCause::NoTask, position: HtmWaitPosition::Unknown }, 0, 1_000)]
    );
}

/// A putaway's `CycleComplete` fires at `fill_at` — the dwell END, not its
/// start — so the hold journal must use the resolved window or the pre-dwell
/// row swallows the dwell and Station processing reads zero. A 2 h headless
/// run on `cencora_phase2` caught exactly that.
#[test]
fn a_putaway_dwell_is_bracketed_on_its_physical_window() {
    let output = run(vec![
        seed(FLOOR_CELL),
        arrival_at(PUTAWAY_STATION, 1_000),
        trip(TripPurpose::ToStation, TripEndpoint::Station(StationId(PUTAWAY_STATION)), 0),
        nav(0, 100, vec![move_wp(QUEUE_CELL, 0, 1_000)]),
        // Presented empty over [3_000, 8_000), reported at its end.
        SimEvent::CycleComplete(CycleCompleteEvent {
            robot_id: RobotId(ROBOT),
            robot_type: RobotType::Htm,
            kind: CycleKind::Putaway { sku: SkuId(1), qty: 1 },
            station_id: StationId(PUTAWAY_STATION),
            sim_time_ms: 8_000,
            dwell_ms: 5_000,
            arrival_source: Some(StationArrivalSource::PutawayDirect),
            dc_id: None,
            tote_presentation: None,
            presentation_id: None,
        }),
        trip(TripPurpose::ToReturnRelay, TripEndpoint::Relay(RelayId(RELAY)), 8_000),
    ]);

    assert_eq!(
        task_holds(&output),
        [(
            HtmHold { cause: HtmWaitCause::AwaitingStationService, position: HtmWaitPosition::StationQueue },
            1_000,
            2_000
        )],
        "the pre-dwell hold stops at the resolved window, leaving [3_000, 8_000) uncovered"
    );
}

/// A dwell that runs past the departure closing its visit escaped that visit,
/// and saying so is ruling #49's tripwire. It has to be reported even when the
/// visit did find a well-formed dwell of its own: otherwise whether the check
/// fires depends on which dwell the queue yields first, and an escaping window
/// sitting behind a good one goes unreported here and is then absorbed by
/// whichever later visit does contain it — reaching the journal as if it had
/// belonged there all along.
#[test]
fn an_escaping_dwell_behind_a_well_formed_one_is_still_reported() {
    let cycle = |at, dwell_ms| {
        SimEvent::CycleComplete(CycleCompleteEvent {
            robot_id: RobotId(ROBOT),
            robot_type: RobotType::Htm,
            kind: CycleKind::Pick { sku: SkuId(1), qty: 1 },
            station_id: StationId(STATION),
            sim_time_ms: at,
            dwell_ms,
            arrival_source: None,
            dc_id: None,
            tote_presentation: None,
            presentation_id: None,
        })
    };
    let observed = observe(vec![
        seed(FLOOR_CELL),
        arrival(1_000),
        trip_for(TripPurpose::ToStation, TripEndpoint::Station(StationId(STATION)), 0, None),
        nav(0, 100, vec![move_wp(PRESENTATION_CELL, 0, 1_000)]),
        // Queued first, and it runs past the departure below: [6s, 11s)
        // against a visit of [1s, 8s).
        cycle(6_000, 5_000),
        // A well-formed dwell for the same visit, queued behind it.
        cycle(3_000, 2_000),
        trip_for(TripPurpose::ToReturnRelay, TripEndpoint::Relay(RelayId(RELAY)), 8_000, None),
    ]);

    let (dwell, visit) = ((SimTime::ms(6_000), SimTime::ms(11_000)), (SimTime::ms(1_000), SimTime::ms(8_000)));
    assert_eq!(
        observed.unwrap_err(),
        StreamViolation::Activity(ActivityViolation::DwellEscapesVisit { robot: RobotId(ROBOT), dwell, visit })
    );
}

/// Each station visit is held on ITS OWN station's presentation cell, which
/// the run's config states. A delivery reports its arrival before its own
/// trip and nav, so where the robot was last tracked when a visit opens is
/// where the previous visit left it, and would hand the second visit the
/// first visit's cell.
///
/// The two stations are deliberately given cells of different KINDS, because
/// the journal surfaces a hold's cell only as its position — so a kind is the
/// only way a stale cell becomes visible to a test.
#[test]
fn each_visit_is_held_on_its_own_stations_cell() {
    let output = run(vec![
        seed(FLOOR_CELL),
        // First visit, at the pick station, on a Presentation cell.
        arrival(1_000),
        trip(TripPurpose::ToStation, TripEndpoint::Station(StationId(STATION)), 0),
        nav(0, 100, vec![move_wp(PRESENTATION_CELL, 0, 1_000)]),
        SimEvent::CycleComplete(CycleCompleteEvent {
            robot_id: RobotId(ROBOT),
            robot_type: RobotType::Htm,
            kind: CycleKind::Pick { sku: SkuId(1), qty: 1 },
            station_id: StationId(STATION),
            sim_time_ms: 3_000,
            dwell_ms: 2_000,
            arrival_source: None,
            dc_id: None,
            tote_presentation: None,
            presentation_id: None,
        }),
        trip(TripPurpose::ToReturnRelay, TripEndpoint::Relay(RelayId(RELAY)), 5_000),
        // Second visit, at the putaway station, on a cell of a different kind.
        arrival_at(PUTAWAY_STATION, 10_000),
        trip(TripPurpose::ToStation, TripEndpoint::Station(StationId(PUTAWAY_STATION)), 9_000),
        nav(9_000, 100, vec![move_wp(QUEUE_CELL, 9_000, 10_000)]),
        SimEvent::CycleComplete(CycleCompleteEvent {
            robot_id: RobotId(ROBOT),
            robot_type: RobotType::Htm,
            kind: CycleKind::Putaway { sku: SkuId(1), qty: 1 },
            station_id: StationId(PUTAWAY_STATION),
            sim_time_ms: 14_000,
            dwell_ms: 2_000,
            arrival_source: Some(StationArrivalSource::PutawayDirect),
            dc_id: None,
            tote_presentation: None,
            presentation_id: None,
        }),
        trip(TripPurpose::ToReturnRelay, TripEndpoint::Relay(RelayId(RELAY)), 15_000),
    ]);

    let positions: Vec<HtmWaitPosition> = task_holds(&output).into_iter().map(|(hold, ..)| hold.position).collect();
    assert_eq!(
        positions,
        // The pick visit's pre-dwell hold, then the putaway visit's pre-dwell
        // hold and its post-dwell tail — all three on their own visit's cell.
        [HtmWaitPosition::Presentation, HtmWaitPosition::StationQueue, HtmWaitPosition::StationQueue],
        "the second visit is held on the putaway station's cell, not on the first visit's"
    );
}
