//! Within `activity`, `tests` drives the interval path: milestone streams in,
//! duty totals, journal rows and slice distributions out. Its children split
//! it by what they state: the interval each milestone pattern makes, the
//! journal's gaps between intervals, what B's advances settle while the run
//! is still going, and the per-slice distribution. The fixtures are here.

mod distribution;
mod gaps;
mod intervals;
mod settlement;
mod slice_duty;

use std::sync::mpsc;

use desgraph::{CellId, RelayId, RobotType, StationId, StationType, WorldPos};

use crate::events::*;
use crate::model::ids::SkuId;
use crate::model::{OrderId, RobotId, SimDuration, SimTime, StationArrivalSource};
use crate::observer::SimObserver;
use crate::observer::StreamViolation;
use crate::observer::output::{HtmWaitCause, MetricsOutput};
use crate::observer::state::SimObserverState;
use crate::observer::test_models::bind_legacy_nav_models;
use crate::observer::time_slice::MetricsConfig;
use crate::pathfinding::{Orientation, Waypoint, WaypointAction};
use crate::sim_types::{StopReason, StopSignal};

const ROBOT: u32 = 7;
/// The putaway station `run_stopping` configures; every other station is a
/// pick station.
const PUTAWAY_STATION: StationId = StationId(1);

/// The whole stream through the observer, as a run that completes reads it.
fn run(events: Vec<SimEvent>) -> MetricsOutput {
    let mut config = MetricsConfig::default();
    bind_legacy_nav_models(&mut config, &events);
    let (tx, rx) = mpsc::channel();
    for event in events {
        tx.send(event).unwrap();
    }
    drop(tx);
    SimObserver::new(config, rx).run().expect("the facts break no rule of the event stream")
}

/// The whole stream through the observer with the run's stop signal
/// attached: what the observer returned, and why the run was told to stop.
fn run_stopping(events: Vec<SimEvent>) -> (Result<MetricsOutput, StreamViolation>, Option<StopReason>) {
    let stop = StopSignal::new();
    let (tx, rx) = mpsc::channel();
    for event in events {
        tx.send(event).unwrap();
    }
    drop(tx);
    let mut config = MetricsConfig::default();
    config.station_types.insert(PUTAWAY_STATION, StationType::Putaway);
    let observed = SimObserver::new(config, rx).with_failure_stop(stop.clone()).run();
    (observed, stop.take_reason())
}

/// The stream read into the observer's state with nothing closing it, as the
/// state stands while the run is still going.
fn observe(events: &[SimEvent]) -> SimObserverState {
    let mut config = MetricsConfig::default();
    bind_legacy_nav_models(&mut config, events);
    let mut state = SimObserverState::new(config);
    for event in events {
        read(&mut state, event);
    }
    state
}

/// Read one event as `SimObserver::run` does: an advance of B settles the
/// work behind it, and every other event is a fact.
fn read(state: &mut SimObserverState, event: &SimEvent) {
    match event {
        SimEvent::BarrierAdvanced(floor) => {
            state.settle_activity(*floor).expect("the facts break no invariant of a robot's work")
        }
        fact => state.dispatch_event(fact),
    }
}

fn seed(robots: &[u32]) -> SimEvent {
    seed_typed(&robots.iter().map(|&robot| (robot, RobotType::Htm)).collect::<Vec<_>>())
}

/// Seed a roster where each robot is the fleet given for it.
fn seed_typed(robots: &[(u32, RobotType)]) -> SimEvent {
    SimEvent::InitialPositions(InitialPositionsEvent {
        htm_headings_rad: Vec::new(),
        positions: robots.iter().map(|&(r, t)| (RobotId(r), CellId(1), t, WorldPos::default())).collect(),
    })
}

fn depart(purpose: TripPurpose, at: u64) -> SimEvent {
    let (from, to) = match purpose {
        TripPurpose::ToRelay => (TripEndpoint::Parking, TripEndpoint::Relay(RelayId(0))),
        TripPurpose::ToStation => (TripEndpoint::Relay(RelayId(0)), TripEndpoint::Station(StationId(0))),
        TripPurpose::ToReturnRelay => (TripEndpoint::Station(StationId(0)), TripEndpoint::Relay(RelayId(0))),
        TripPurpose::ToParking => (TripEndpoint::Parking, TripEndpoint::Parking),
    };
    SimEvent::TripStart(TripStartEvent {
        robot_id: RobotId(ROBOT),
        robot_type: RobotType::Htm,
        purpose,
        presentation_id: None,
        from: Some(from),
        to,
        sim_time_ms: at,
    })
}

fn fork_get(start: u64, end: u64) -> SimEvent {
    SimEvent::HtmRelayPickup(HtmRelayPickupEvent {
        robot_id: RobotId(ROBOT),
        relay_id: RelayId(0),
        start: SimTime::ms(start),
        end: SimTime::ms(end),
        wait: SimDuration::ZERO,
        resident: false,
    })
}

/// A fork-Get by a bot that was already standing on the relay's access cell:
/// no leg, no `TripStart`, and a stand of `wait` before the operation. The
/// pickup task is accepted at the fork-Get's start, so the stand comes
/// before any task held the bot.
fn fork_get_resident(start: u64, end: u64, wait: u64) -> SimEvent {
    SimEvent::HtmRelayPickup(HtmRelayPickupEvent {
        robot_id: RobotId(ROBOT),
        relay_id: RelayId(0),
        start: SimTime::ms(start),
        end: SimTime::ms(end),
        wait: SimDuration::ms(wait),
        resident: true,
    })
}

fn fork_put(start: u64, end: u64) -> SimEvent {
    SimEvent::HtmRelayDeposit(HtmRelayDepositEvent {
        robot_id: RobotId(ROBOT),
        relay_id: RelayId(0),
        start: SimTime::ms(start),
        end: SimTime::ms(end),
        wait: SimDuration::ZERO,
    })
}

/// A nav chain for `ROBOT` ending at `end`, which is all these tests read off
/// it: one `Move` waypoint covering `[start, end)`.
fn nav_to(cell: u32, start: u64, end: u64) -> SimEvent {
    SimEvent::Nav(NavEventData {
        robot_id: RobotId(ROBOT),
        waypoints: vec![Waypoint {
            cell: CellId(cell),
            orientation: Orientation::East,
            action: WaypointAction::Move {
                start: SimTime::ms(start),
                end: SimTime::ms(end),
                entry_velocity_mm_s: 0.0,
                exit_velocity_mm_s: 0.0,
            },
        }],
        robot_type: RobotType::Htm,
        time: SimTime::ms(start),
        distance_mm: 0,
        carrying_cargo: false,
    })
}

fn arrive(at: u64) -> SimEvent {
    arrive_of(ROBOT, at)
}

fn arrive_of(robot: u32, at: u64) -> SimEvent {
    SimEvent::HtmArrival(HtmArrivalEvent {
        robot_id: RobotId(robot),
        station_id: StationId(0),
        arrival: SimTime::ms(at),
    })
}

fn dwell(at: u64, ms: u64) -> SimEvent {
    dwell_of(ROBOT, at, ms)
}

fn dwell_of(robot: u32, at: u64, ms: u64) -> SimEvent {
    SimEvent::CycleComplete(CycleCompleteEvent {
        robot_id: RobotId(robot),
        robot_type: RobotType::Htm,
        kind: CycleKind::Pick { sku: SkuId(1), qty: 1 },
        station_id: StationId(0),
        sim_time_ms: at,
        dwell_ms: ms,
        arrival_source: None,
        dc_id: None,
        presentation_id: None,
        tote_presentation: None,
    })
}

/// A putaway station cycle for `ROBOT` filled at `fill_at` after a dwell of
/// `ms`, so its work runs `[fill_at - ms, fill_at)`.
fn putaway_filled(fill_at: u64, ms: u64) -> SimEvent {
    SimEvent::CycleComplete(CycleCompleteEvent {
        robot_id: RobotId(ROBOT),
        robot_type: RobotType::Htm,
        kind: CycleKind::Putaway { sku: SkuId(1), qty: 1 },
        station_id: PUTAWAY_STATION,
        sim_time_ms: fill_at,
        dwell_ms: ms,
        arrival_source: Some(StationArrivalSource::PutawayFromRack),
        dc_id: None,
        presentation_id: None,
        tote_presentation: None,
    })
}

fn order_complete(at: u64) -> SimEvent {
    SimEvent::OrderComplete(OrderCompleteEvent { order_id: OrderId(1), sim_time_ms: at })
}

/// Every journal `Wait` row of `cause`, as `(start_ms, duration_ms, carrying)`.
fn holds(output: &MetricsOutput, cause: HtmWaitCause) -> Vec<(u64, u64, bool)> {
    let mut rows: Vec<(u64, u64, bool)> = output
        .htm_leg_log
        .iter()
        .filter(|r| r.hold.map(|h| h.cause) == Some(cause))
        .map(|r| (r.start.as_ms(), r.duration.as_ms(), r.carrying_cargo))
        .collect();
    rows.sort();
    rows
}

/// Every no-task Wait row, as `(start_ms, duration_ms, carrying)`.
fn gaps(output: &MetricsOutput) -> Vec<(u64, u64, bool)> {
    holds(output, HtmWaitCause::NoTask)
}

/// `robot`'s duty as `(travel, fork operations, station visits)` in ms.
fn duty(output: &MetricsOutput, robot: u32) -> (u64, u64, u64) {
    let d = output.robot_duty_totals.get(&RobotId(robot)).copied().unwrap_or_default();
    (d.traveling.as_ms(), d.loading_unloading.as_ms(), d.processing.as_ms())
}
