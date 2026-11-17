//! Tests for the per-robot duty-cycle accumulator (`record_interval`).
//!
//! Drives `BusyInterval` recording through `VtmNav` (VTM) and the trip,
//! fork and cycle facts a D30's legs report (HTM) end-to-end, plus a direct
//! unit test of the zero-duration guard in
//! `SimObserverState::record_interval`.

use std::sync::mpsc;

use desgraph::{CellId, RelayId, RobotType, StationId, WorldPos, ZoneId};

use crate::events::*;
use crate::model::ids::{PresentationId, VtmCycleId};
use crate::model::{RackId, RobotId, SimDuration, SimTime, ToteId};
use crate::observer::activity::ActivityKind;
use crate::observer::duty::BusyInterval;
use crate::observer::output::{
    Activity, HtmJournalEndpoint, HtmWaitCause, HtmWaitReason, MetricsOutput, VtmJournalEndpoint,
};
use crate::observer::state::SimObserverState;
use crate::observer::time_slice::MetricsConfig;
use crate::pathfinding::{Orientation, Waypoint, WaypointAction};

use super::SimObserver;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn run_with_events_config(events: Vec<SimEvent>, mut config: MetricsConfig) -> MetricsOutput {
    super::test_models::bind_legacy_nav_models(&mut config, &events);
    let (tx, rx) = mpsc::channel();
    for event in events {
        tx.send(event).unwrap();
    }
    drop(tx);
    SimObserver::new(config, rx).run().expect("the facts break no rule of the event stream")
}

fn initial_pos(robot_id: u32, cell: u32, robot_type: RobotType) -> (RobotId, CellId, RobotType, WorldPos) {
    (RobotId(robot_id), CellId(cell), robot_type, WorldPos::default())
}

fn aisle(cell: u32) -> VtmEndpoint {
    VtmEndpoint::Aisle { cell: CellId(cell), pos: WorldPos::default() }
}

fn slot(cell: u32) -> VtmEndpoint {
    VtmEndpoint::Slot { rack_id: RackId(0), cell: CellId(cell), pos: WorldPos::default() }
}

fn relay(cell: u32) -> VtmEndpoint {
    VtmEndpoint::Relay { relay_id: RelayId(0), cell: CellId(cell), pos: WorldPos::default() }
}

fn rail_move(robot_id: u32, start_ms: u64, end_ms: u64, from: VtmEndpoint, to: VtmEndpoint) -> SimEvent {
    rail_move_chain(robot_id, start_ms, end_ms, from, to, None, Some(PresentationId(1)))
}

#[allow(clippy::too_many_arguments)]
fn rail_move_chain(
    robot_id: u32,
    start_ms: u64,
    end_ms: u64,
    from: VtmEndpoint,
    to: VtmEndpoint,
    tote_id: Option<ToteId>,
    presentation_id: Option<PresentationId>,
) -> SimEvent {
    SimEvent::VtmNav(VtmNavEventData {
        robot_id: RobotId(robot_id),
        vtm_cycle_id: VtmCycleId(0),
        kind: VtmNavKind::RailMove {
            from,
            to,
            tote_id,
            start: SimTime::ms(start_ms),
            end: SimTime::ms(end_ms),
            distance_mm: 0,
        },
        geo_start: SimTime::ms(start_ms),
        geo_end: SimTime::ms(end_ms),
        presentation_id,
        direction: VtmCycleDirection::Forward,
    })
}

fn fork(robot_id: u32, start_ms: u64, end_ms: u64, at: VtmEndpoint, op: ForkOp) -> SimEvent {
    SimEvent::VtmNav(VtmNavEventData {
        robot_id: RobotId(robot_id),
        vtm_cycle_id: VtmCycleId(0),
        kind: VtmNavKind::Fork { op, at, tote_id: ToteId(7), start: SimTime::ms(start_ms), end: SimTime::ms(end_ms) },
        geo_start: SimTime::ms(start_ms),
        geo_end: SimTime::ms(end_ms),
        presentation_id: Some(PresentationId(1)),
        direction: VtmCycleDirection::Forward,
    })
}

fn alignment(robot_id: u32, start_ms: u64, end_ms: u64, at: VtmEndpoint, tote_id: Option<ToteId>) -> SimEvent {
    SimEvent::VtmNav(VtmNavEventData {
        robot_id: RobotId(robot_id),
        vtm_cycle_id: VtmCycleId(0),
        kind: VtmNavKind::Alignment { at, tote_id, start: SimTime::ms(start_ms), end: SimTime::ms(end_ms) },
        geo_start: SimTime::ms(start_ms),
        geo_end: SimTime::ms(end_ms),
        presentation_id: Some(PresentationId(1)),
        direction: VtmCycleDirection::Forward,
    })
}

fn htm_trip_start(robot_id: u32, purpose: TripPurpose, sim_time_ms: u64) -> SimEvent {
    // Derive representative endpoints from the purpose so journal rows carry the
    // semantic from/to under test. Trip ID is a fixed non-None value.
    let (from, to) = match purpose {
        TripPurpose::ToRelay => (TripEndpoint::Parking, TripEndpoint::Relay(RelayId(0))),
        TripPurpose::ToStation => (TripEndpoint::Relay(RelayId(0)), TripEndpoint::Station(StationId(0))),
        TripPurpose::ToReturnRelay => (TripEndpoint::Station(StationId(0)), TripEndpoint::Relay(RelayId(0))),
        TripPurpose::ToParking => (TripEndpoint::Parking, TripEndpoint::Parking),
    };
    SimEvent::TripStart(TripStartEvent {
        robot_id: RobotId(robot_id),
        robot_type: RobotType::Htm,
        purpose,
        presentation_id: Some(PresentationId(1)),
        from: Some(from),
        to,
        sim_time_ms,
    })
}

/// HTM nav event whose waypoints are a `PathStart` followed by one waypoint
/// per orientation in `orientations` (the chain's facing sequence). Turns are
/// derived from consecutive orientation deltas via `Orientation::turns_to`.
fn htm_nav(robot_id: u32, cell: u32, time_ms: u64, orientations: &[Orientation]) -> SimEvent {
    let c = CellId(cell);
    let mut waypoints = vec![Waypoint {
        cell: c,
        orientation: orientations[0],
        action: WaypointAction::PathStart { time: SimTime::ms(time_ms) },
    }];
    for (i, &o) in orientations.iter().enumerate() {
        let start = SimTime::ms(time_ms + i as u64 * 1_000);
        let end = SimTime::ms(time_ms + (i as u64 + 1) * 1_000);
        waypoints.push(Waypoint {
            cell: c,
            orientation: o,
            action: WaypointAction::Move { start, end, entry_velocity_mm_s: 0.0, exit_velocity_mm_s: 0.0 },
        });
    }
    SimEvent::Nav(NavEventData {
        robot_id: RobotId(robot_id),
        waypoints,
        robot_type: RobotType::Htm,
        time: SimTime::ms(time_ms),
        distance_mm: 0,
        carrying_cargo: false,
    })
}

/// A fork-Get at a relay: the milestone that ends an inbound leg and is
/// itself an interval.
fn htm_fork_get(robot_id: u32, start_ms: u64, end_ms: u64) -> SimEvent {
    SimEvent::HtmRelayPickup(HtmRelayPickupEvent {
        robot_id: RobotId(robot_id),
        relay_id: RelayId(0),
        start: SimTime::ms(start_ms),
        end: SimTime::ms(end_ms),
        wait: SimDuration::ZERO,
        resident: false,
    })
}

/// A fork-Put at a relay: the milestone that ends a return leg.
fn htm_fork_put(robot_id: u32, start_ms: u64, end_ms: u64) -> SimEvent {
    SimEvent::HtmRelayDeposit(HtmRelayDepositEvent {
        robot_id: RobotId(robot_id),
        relay_id: RelayId(0),
        start: SimTime::ms(start_ms),
        end: SimTime::ms(end_ms),
        wait: SimDuration::ZERO,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// One HTM `Rotate` waypoint of known duration is tallied into the
/// `htm_duty_rotating_ms` carve-out, and never exceeds the inclusive
/// `htm_duty_traveling_ms` for the slice.
#[test]
fn htm_rotation_carves_out_of_traveling() {
    let cell = CellId(50);
    let nav = SimEvent::Nav(NavEventData {
        robot_id: RobotId(7),
        waypoints: vec![
            Waypoint {
                cell,
                orientation: Orientation::North,
                action: WaypointAction::PathStart { time: SimTime::ms(2_000) },
            },
            // 3s in-place rotation at the start cell.
            Waypoint {
                cell,
                orientation: Orientation::East,
                action: WaypointAction::Rotate { start: SimTime::ms(2_000), end: SimTime::ms(5_000) },
            },
        ],
        robot_type: RobotType::Htm,
        time: SimTime::ms(2_000),
        distance_mm: 0,
        carrying_cargo: false,
    });
    let events = vec![
        SimEvent::InitialPositions(InitialPositionsEvent {
            positions: vec![initial_pos(7, 50, RobotType::Htm)],
            htm_headings_rad: Vec::new(),
        }),
        // The leg spans the rotation, so the carve-out invariant is testable:
        // it departs at 1s and the fork-Get that ends it is at 6s.
        htm_trip_start(7, TripPurpose::ToRelay, 1_000),
        nav,
        htm_fork_get(7, 6_000, 7_000),
    ];

    let output = run_with_events_config(events, MetricsConfig::default());
    let slice_0 = &output.time_slices[0].scalars;

    assert_eq!(slice_0.htm_duty_rotating_ms, 3_000);
    assert!(slice_0.htm_duty_rotating_ms <= slice_0.htm_duty_traveling_ms);
    assert_eq!(output.cumulative_scalars.htm_duty_rotating_ms, 3_000);
}

/// An N→E→S waypoint chain under a `ToStation` trip yields one
/// `htm_leg_log` row with 2 geometric turns at the nav event's start time.
#[test]
fn htm_leg_log_counts_turns_per_nav_event() {
    let events = vec![
        SimEvent::InitialPositions(InitialPositionsEvent {
            positions: vec![initial_pos(3, 50, RobotType::Htm)],
            htm_headings_rad: Vec::new(),
        }),
        htm_trip_start(3, TripPurpose::ToStation, 1_000),
        htm_nav(3, 50, 1_000, &[Orientation::North, Orientation::East, Orientation::South]),
    ];
    let output = run_with_events_config(events, MetricsConfig::default());

    let to_station: Vec<_> = output
        .htm_leg_log
        .iter()
        .filter(|r| r.activity == Activity::Travel && r.to == HtmJournalEndpoint::PickStation(StationId(0)))
        .collect();
    assert_eq!(to_station.len(), 1);
    assert_eq!(to_station[0].turn_count, 2);
    assert_eq!(to_station[0].robot_id, RobotId(3));
    assert_eq!(to_station[0].start.as_ms(), 1_000);
}

/// Two nav events within the same leg are NOT coalesced — each is its own
/// `htm_leg_log` row (finer granularity than the old per-leg `turn_log`).
/// The per-purpose turn total still sums correctly (1 + 2 = 3), preserving
/// aggregate fidelity.
#[test]
fn htm_leg_log_is_per_nav_event_not_per_leg() {
    let events = vec![
        SimEvent::InitialPositions(InitialPositionsEvent {
            positions: vec![initial_pos(4, 50, RobotType::Htm)],
            htm_headings_rad: Vec::new(),
        }),
        htm_trip_start(4, TripPurpose::ToRelay, 1_000),
        // First replan: N→E = 1 turn.
        htm_nav(4, 50, 1_000, &[Orientation::North, Orientation::East]),
        // Second replan, same leg: E→W = 2 turns (180°).
        htm_nav(4, 50, 5_000, &[Orientation::East, Orientation::West]),
    ];
    let output = run_with_events_config(events, MetricsConfig::default());

    let to_relay: Vec<_> = output
        .htm_leg_log
        .iter()
        .filter(|r| r.activity == Activity::Travel && r.to == HtmJournalEndpoint::Relay(RelayId(0)))
        .collect();
    assert_eq!(to_relay.len(), 2, "per-nav-event: two nav events → two rows, no coalescing");
    assert_eq!(to_relay.iter().map(|r| r.turn_count).sum::<u32>(), 3, "aggregate turns preserved");
    assert_eq!(to_relay[0].turn_count, 1);
    assert_eq!(to_relay[0].start.as_ms(), 1_000);
    assert_eq!(to_relay[1].turn_count, 2);
    assert_eq!(to_relay[1].start.as_ms(), 5_000);
}

/// A nav event with no preceding `TripStart` has no known purpose, so it
/// produces no `htm_leg_log` row — we never fabricate a record.
#[test]
fn htm_leg_log_skips_nav_without_known_purpose() {
    let events = vec![
        SimEvent::InitialPositions(InitialPositionsEvent {
            positions: vec![initial_pos(8, 50, RobotType::Htm)],
            htm_headings_rad: Vec::new(),
        }),
        // No TripStart for robot 8 → current_trip has no entry.
        htm_nav(8, 50, 1_000, &[Orientation::North, Orientation::West]),
    ];
    let output = run_with_events_config(events, MetricsConfig::default());

    // A no-task Wait row for the gap before the nav may exist, but no Travel
    // row is fabricated from a purposeless nav.
    assert!(
        !output.htm_leg_log.iter().any(|r| r.activity == Activity::Travel),
        "nav before any TripStart must not record a Travel row"
    );
}

/// A leg's journal `from` is the robot's PRIOR leg destination, not the
/// event's declared origin: a pickup right after a relay drop reads
/// Relay→Relay (workflow §2), overriding the pickup event's Parking seed.
#[test]
fn htm_leg_log_from_follows_prior_leg_destination() {
    let r = 9;
    let events = vec![
        SimEvent::InitialPositions(InitialPositionsEvent {
            positions: vec![initial_pos(r, 50, RobotType::Htm)],
            htm_headings_rad: Vec::new(),
        }),
        // Store leg ends at Relay(0) — records the robot's resting location.
        htm_trip_start(r, TripPurpose::ToReturnRelay, 0),
        htm_nav(r, 50, 0, &[Orientation::North]),
        // Pickup leg: the event declares from = Parking, but the bot is at Relay(0).
        htm_trip_start(r, TripPurpose::ToRelay, 10_000),
        htm_nav(r, 50, 10_000, &[Orientation::North]),
    ];
    let output = run_with_events_config(events, MetricsConfig::default());

    let pickup = output.htm_leg_log.iter().find(|row| row.start.as_ms() == 10_000).expect("pickup row");
    assert_eq!(
        pickup.from,
        HtmJournalEndpoint::Relay(RelayId(0)),
        "from = prior leg destination, not the Parking seed"
    );
}

/// A journey that rotates before its first move journals ONE Travel row
/// covering both: #261 §2.2 makes in-path chassis rotation part of travel, so
/// a run of moves and rotations is one row whose window is its real
/// `[start, end)`. Before #272 the row spanned summed move time stamped at
/// the first move — a window that stopped describing the clock as soon as
/// anything interrupted the moves, which is what left in-path holds in the
/// space between rows.
#[test]
fn htm_leg_log_travel_row_covers_its_rotation() {
    let r = 15;
    let c = CellId(50);
    let waypoints = vec![
        Waypoint {
            cell: c,
            orientation: Orientation::North,
            action: WaypointAction::PathStart { time: SimTime::ms(1_000) },
        },
        Waypoint {
            cell: c,
            orientation: Orientation::East,
            action: WaypointAction::Rotate { start: SimTime::ms(1_000), end: SimTime::ms(3_000) },
        },
        Waypoint {
            cell: c,
            orientation: Orientation::East,
            action: WaypointAction::Move {
                start: SimTime::ms(3_000),
                end: SimTime::ms(5_000),
                entry_velocity_mm_s: 0.0,
                exit_velocity_mm_s: 0.0,
            },
        },
    ];
    let events = vec![
        SimEvent::InitialPositions(InitialPositionsEvent {
            positions: vec![initial_pos(r, 50, RobotType::Htm)],
            htm_headings_rad: Vec::new(),
        }),
        htm_trip_start(r, TripPurpose::ToRelay, 1_000),
        SimEvent::Nav(NavEventData {
            robot_id: RobotId(r),
            waypoints,
            robot_type: RobotType::Htm,
            time: SimTime::ms(1_000),
            distance_mm: 500,
            carrying_cargo: false,
        }),
    ];
    let output = run_with_events_config(events, MetricsConfig::default());

    let travel = output.htm_leg_log.iter().find(|row| row.activity == Activity::Travel).expect("travel row");
    assert_eq!(
        (travel.start.as_ms(), travel.duration.as_ms()),
        (1_000, 4_000),
        "one row over the rotation and the move it precedes"
    );
    assert_eq!(travel.turn_count, 1, "the chain's geometric turn rides on its run");
    assert_eq!(travel.distance_mm, 500);
    assert!(
        output.htm_leg_log.iter().all(|row| row.hold.map(|hold| hold.cause) != Some(HtmWaitCause::RouteHold)),
        "an uninterrupted chain journals no route hold"
    );
}

/// A gap between tasks emits one no-task Wait row whose location is the
/// prior leg's destination (not a hardcoded Parking): after a store leg ending
/// at a relay, the bot idles *at the relay*, so the row reads `Relay`. It still
/// carries **no Trip ID even though a trip context is live** — idle belongs to
/// no cycle. (No-task is the only HTM wait reason with a live signal — spec §4.)
#[test]
fn htm_wait_row_records_no_task_idle_at_prior_leg_destination() {
    let r = 11;
    let events = vec![
        SimEvent::InitialPositions(InitialPositionsEvent {
            positions: vec![initial_pos(r, 50, RobotType::Htm)],
            htm_headings_rad: Vec::new(),
        }),
        // Store leg ends at Relay(0), its nav at 1_000, with the fork-Put [1_000, 1_500).
        htm_trip_start(r, TripPurpose::ToReturnRelay, 0),
        htm_nav(r, 50, 0, &[Orientation::North]),
        htm_fork_put(r, 1_000, 1_500),
        // Next dispatch closes the gap [1_500, 5_000); its continuity origin is
        // the prior leg's Relay(0) destination = where the bot idled.
        htm_trip_start(r, TripPurpose::ToRelay, 5_000),
        htm_fork_get(r, 9_000, 9_500),
    ];
    let output = run_with_events_config(events, MetricsConfig::default());

    let waits: Vec<_> = output.htm_leg_log.iter().filter(|x| x.activity == Activity::Wait).collect();
    assert_eq!(waits.len(), 1, "one no-task row from the gap");
    let w = waits[0];
    assert_eq!(w.wait_reason, Some(HtmWaitReason::NoTask));
    assert_eq!((w.start.as_ms(), w.duration.as_ms()), (1_500, 3_500));
    assert_eq!(w.presentation_id, None, "idle belongs to no cycle, even with a live trip context");
    assert_eq!(w.from, HtmJournalEndpoint::Relay(RelayId(0)), "idle location = prior leg's destination");
    assert_eq!(w.to, HtmJournalEndpoint::Relay(RelayId(0)));
    assert_eq!(w.distance_mm, 0);
}

/// An HTM whose last work ended before sim-end gets a final no-task Wait row
/// spanning `[last_interval_end, latest_sim_time]` at its resting location —
/// so the no-task row count and total idle scale with run length instead of
/// going flat. The trailing gap is an ordinary gap; nothing special closes it.
#[test]
fn htm_trailing_gap_emits_a_final_no_task_row() {
    let r = 12;
    let events = vec![
        SimEvent::InitialPositions(InitialPositionsEvent {
            positions: vec![initial_pos(r, 50, RobotType::Htm)],
            htm_headings_rad: Vec::new(),
        }),
        // Store leg to Relay(0), its fork-Put ending at 2_000; nothing after.
        htm_trip_start(r, TripPurpose::ToReturnRelay, 0),
        htm_nav(r, 50, 0, &[Orientation::North]),
        htm_fork_put(r, 1_500, 2_000),
        // A later unrelated event advances latest_sim_time to 9_000 without
        // giving this robot any more work.
        htm_trip_start(99, TripPurpose::ToRelay, 9_000),
    ];
    let output = run_with_events_config(events, MetricsConfig::default());

    // The trailing gap's row runs from the end of the robot's last interval.
    let terminal = output
        .htm_leg_log
        .iter()
        .find(|x| x.activity == Activity::Wait && x.robot_id == RobotId(r) && x.start.as_ms() == 2_000)
        .expect("the trailing gap is journaled from the last interval's end");
    assert_eq!(terminal.wait_reason, Some(HtmWaitReason::NoTask));
    assert_eq!(terminal.duration.as_ms(), 7_000, "spans [last interval end, latest_sim_time]");
    assert_eq!(terminal.from, HtmJournalEndpoint::Relay(RelayId(0)), "resting at the last leg's destination");
    assert!(!terminal.carrying_cargo, "terminally idle bot holds no tote");
    assert_eq!(terminal.presentation_id, None);
}

/// An `HtmRelayPickupEvent` produces one `AwaitingDeposit` Wait row — the relay
/// dwell awaiting the VTM deposit. It spans `[start − wait, start)` at the
/// relay, carries the active pickup leg's Trip ID (from `current_trip`), and the
/// bot holds no tote yet. (The dominant HTM wait, previously in no journal row.)
#[test]
fn htm_relay_pickup_produces_awaiting_deposit_row() {
    let r = 13;
    let events = vec![
        SimEvent::InitialPositions(InitialPositionsEvent {
            positions: vec![initial_pos(r, 50, RobotType::Htm)],
            htm_headings_rad: Vec::new(),
        }),
        // Pickup leg is live (Trip ID 1 via the helper) when the dwell is logged.
        htm_trip_start(r, TripPurpose::ToRelay, 1_000),
        SimEvent::HtmRelayPickup(HtmRelayPickupEvent {
            robot_id: RobotId(r),
            relay_id: RelayId(5),
            start: SimTime::ms(200_000),
            end: SimTime::ms(202_000),
            wait: SimDuration::ms(150_000),
            resident: false,
        }),
    ];
    let output = run_with_events_config(events, MetricsConfig::default());

    let deposits: Vec<_> =
        output.htm_leg_log.iter().filter(|x| x.wait_reason == Some(HtmWaitReason::AwaitingDeposit)).collect();
    assert_eq!(deposits.len(), 1, "one AwaitingDeposit row from the relay dwell");
    let d = deposits[0];
    assert_eq!(d.activity, Activity::Wait);
    assert_eq!((d.start.as_ms(), d.duration.as_ms()), (50_000, 150_000), "[start − wait, start)");
    assert_eq!(d.from, HtmJournalEndpoint::Relay(RelayId(5)));
    assert_eq!(d.to, HtmJournalEndpoint::Relay(RelayId(5)));
    assert_eq!(d.presentation_id, Some(1), "the active pickup leg's Trip ID");
    assert!(!d.carrying_cargo, "bot is awaiting the deposit, not yet carrying");
}

/// A zero-`wait` `HtmRelayPickupEvent` (on-time arrival) journals no
/// AwaitingDeposit row — there is no dwell to record.
#[test]
fn htm_relay_pickup_zero_wait_produces_no_row() {
    let r = 14;
    let events = vec![
        SimEvent::InitialPositions(InitialPositionsEvent {
            positions: vec![initial_pos(r, 50, RobotType::Htm)],
            htm_headings_rad: Vec::new(),
        }),
        htm_trip_start(r, TripPurpose::ToRelay, 1_000),
        SimEvent::HtmRelayPickup(HtmRelayPickupEvent {
            robot_id: RobotId(r),
            relay_id: RelayId(5),
            start: SimTime::ms(200_000),
            end: SimTime::ms(202_000),
            wait: SimDuration::ZERO,
            resident: false,
        }),
    ];
    let output = run_with_events_config(events, MetricsConfig::default());

    assert!(
        !output.htm_leg_log.iter().any(|x| x.wait_reason == Some(HtmWaitReason::AwaitingDeposit)),
        "zero wait → no AwaitingDeposit row"
    );
}

/// A typical VtmForward cycle (RailMove → Fork(Get) → RailMove → Fork(Put))
/// sums to the expected travel / fork-operation split, with no gap between
/// the four phases.
#[test]
fn vtm_forward_cycle_accumulates_duty() {
    let events = vec![
        SimEvent::InitialPositions(InitialPositionsEvent {
            positions: vec![initial_pos(0, 100, RobotType::Vtm)],
            htm_headings_rad: Vec::new(),
        }),
        rail_move(0, 0, 1_000, aisle(100), slot(101)),
        fork(0, 1_000, 3_000, slot(101), ForkOp::Get),
        rail_move(0, 3_000, 5_000, slot(101), relay(102)),
        fork(0, 5_000, 7_000, relay(102), ForkOp::Put),
    ];

    let output = run_with_events_config(events, MetricsConfig::default());
    let duty = output.robot_duty_totals.get(&RobotId(0)).expect("vtm 0 duty totals");

    assert_eq!(duty.traveling, SimDuration::ms(3_000)); // 1000 + 2000
    assert_eq!(duty.loading_unloading, SimDuration::ms(4_000)); // 2000 + 2000
    assert_eq!(duty.busy(), SimDuration::ms(7_000));

    // Timeline is gated off by default.
    assert!(output.robot_timelines.is_empty());

    // First/last seen markers track the bookend events.
    assert_eq!(output.robot_first_seen.get(&RobotId(0)), Some(&SimTime::ms(0)));
    assert_eq!(output.robot_last_seen.get(&RobotId(0)), Some(&SimTime::ms(7_000)));
}

/// A single RailMove `[800, 2200)` over `slice_width = 1000ms` is clipped
/// into three slice contributions: 200ms + 1000ms + 200ms, summing back to
/// the original 1400ms.
#[test]
fn interval_spanning_slice_boundary_clips_correctly() {
    let config = MetricsConfig { slice_width_ms: 1_000, ..MetricsConfig::default() };
    let events = vec![
        SimEvent::InitialPositions(InitialPositionsEvent {
            positions: vec![initial_pos(0, 100, RobotType::Vtm)],
            htm_headings_rad: Vec::new(),
        }),
        rail_move(0, 800, 2_200, aisle(100), slot(101)),
    ];
    let output = run_with_events_config(events, config);

    assert!(output.time_slices.len() >= 3);
    let slice_0 = output.time_slices[0].scalars.duty_per_robot.get(&0).expect("slice 0");
    let slice_1 = output.time_slices[1].scalars.duty_per_robot.get(&0).expect("slice 1");
    let slice_2 = output.time_slices[2].scalars.duty_per_robot.get(&0).expect("slice 2");

    assert_eq!(slice_0.traveling, SimDuration::ms(200));
    assert_eq!(slice_1.traveling, SimDuration::ms(1_000));
    assert_eq!(slice_2.traveling, SimDuration::ms(200));

    let summed = slice_0.traveling + slice_1.traveling + slice_2.traveling;
    assert_eq!(summed, SimDuration::ms(1_400));
    assert_eq!(output.robot_duty_totals.get(&RobotId(0)).unwrap().traveling, SimDuration::ms(1_400));
}

/// Each milestone bounds the interval the one before it opened, and the
/// interval's whole duration is booked to its kind. The stretch before a
/// robot's first milestone is a gap: journaled, not counted as duty.
#[test]
fn each_milestone_bounds_the_interval_the_previous_one_opened() {
    let events = vec![
        SimEvent::InitialPositions(InitialPositionsEvent {
            positions: vec![initial_pos(5, 50, RobotType::Htm)],
            htm_headings_rad: Vec::new(),
        }),
        htm_trip_start(5, TripPurpose::ToRelay, 10_000),
        htm_fork_get(5, 20_000, 25_000),
    ];
    let output = run_with_events_config(events, MetricsConfig::default());
    let duty = output.robot_duty_totals.get(&RobotId(5)).expect("htm 5 duty");

    // The gap [0, 10s) is not duty; travel [10s, 20s) = 10s; the fork
    // operation [20s, 25s) = 5s.
    assert_eq!(duty.traveling, SimDuration::ms(10_000));
    assert_eq!(duty.loading_unloading, SimDuration::ms(5_000));
    assert_eq!(duty.busy(), SimDuration::ms(15_000));
}

/// A zero-duration interval (`start == end`) must not touch any state:
/// no slices created, no totals updated, no first-seen marker set, no
/// timeline entry pushed.
#[test]
fn zero_duration_interval_is_noop() {
    let mut state = SimObserverState::new(MetricsConfig::default());
    state.record_interval(
        RobotId(0),
        BusyInterval { start: SimTime::ms(1_000), end: SimTime::ms(1_000), kind: ActivityKind::Travel },
    );
    assert!(state.output.robot_duty_totals.is_empty());
    assert!(state.output.robot_timelines.is_empty());
    assert!(state.output.robot_first_seen.is_empty());
    assert!(state.output.robot_last_seen.is_empty());
    assert!(state.slices().is_empty());
}

/// `SimEvent::VtmIdleReason` events are tallied by kind into both the
/// per-slice and cumulative counters. One event of each classification
/// across two slices verifies slice attribution and the discriminant.
#[test]
fn vtm_idle_reasons_aggregate_by_kind() {
    let config = MetricsConfig { slice_width_ms: 1_000, ..MetricsConfig::default() };
    let events = vec![
        SimEvent::VtmIdleReason(VtmIdleReasonEvent {
            robot_id: RobotId(0),
            vtm_cycle_id: VtmCycleId(0),
            at: SimTime::ms(500),
            reason: VtmIdleReason::Continuous,
            presentation_id: Some(PresentationId(10)),
            direction: VtmCycleDirection::Forward,
            relay_id: RelayId(0),
            geo_diverged: false,
        }),
        SimEvent::VtmIdleReason(VtmIdleReasonEvent {
            robot_id: RobotId(1),
            vtm_cycle_id: VtmCycleId(0),
            at: SimTime::ms(600),
            reason: VtmIdleReason::AwaitingDemand { gap: SimDuration::ms(200), relay_start: SimTime::ms(800) },
            presentation_id: Some(PresentationId(11)),
            direction: VtmCycleDirection::Forward,
            relay_id: RelayId(0),
            geo_diverged: false,
        }),
        SimEvent::VtmIdleReason(VtmIdleReasonEvent {
            robot_id: RobotId(2),
            vtm_cycle_id: VtmCycleId(0),
            at: SimTime::ms(700),
            reason: VtmIdleReason::AwaitingTote { gap: SimDuration::ms(150), relay_start: SimTime::ms(850) },
            presentation_id: Some(PresentationId(12)),
            direction: VtmCycleDirection::Forward,
            relay_id: RelayId(0),
            geo_diverged: false,
        }),
        SimEvent::VtmIdleReason(VtmIdleReasonEvent {
            robot_id: RobotId(0),
            vtm_cycle_id: VtmCycleId(0),
            at: SimTime::ms(1_500),
            reason: VtmIdleReason::AwaitingSlot { gap: SimDuration::ms(100), relay_start: SimTime::ms(1_600) },
            presentation_id: Some(PresentationId(13)),
            direction: VtmCycleDirection::Forward,
            relay_id: RelayId(0),
            geo_diverged: false,
        }),
        SimEvent::VtmIdleReason(VtmIdleReasonEvent {
            robot_id: RobotId(1),
            vtm_cycle_id: VtmCycleId(0),
            at: SimTime::ms(1_700),
            reason: VtmIdleReason::AwaitingHtmDeposit { gap: SimDuration::ms(50), deposit_time: SimTime::ms(1_750) },
            presentation_id: Some(PresentationId(14)),
            direction: VtmCycleDirection::Return,
            relay_id: RelayId(0),
            geo_diverged: false,
        }),
    ];
    let output = run_with_events_config(events, config);

    // Cumulative: one of each classification.
    assert_eq!(output.vtm_idle_reason_counts.get(&VtmIdleReasonKind::Continuous), Some(&1));
    assert_eq!(output.vtm_idle_reason_counts.get(&VtmIdleReasonKind::AwaitingDemand), Some(&1));
    assert_eq!(output.vtm_idle_reason_counts.get(&VtmIdleReasonKind::AwaitingTote), Some(&1));
    assert_eq!(output.vtm_idle_reason_counts.get(&VtmIdleReasonKind::AwaitingSlot), Some(&1));
    assert_eq!(output.vtm_idle_reason_counts.get(&VtmIdleReasonKind::AwaitingHtmDeposit), Some(&1));

    // Slice 0 [0, 1000ms): Continuous + the two forward floors.
    let slice_0 = &output.time_slices[0].scalars.vtm_idle_reason_counts;
    assert_eq!(slice_0.get(&VtmIdleReasonKind::Continuous), Some(&1));
    assert_eq!(slice_0.get(&VtmIdleReasonKind::AwaitingDemand), Some(&1));
    assert_eq!(slice_0.get(&VtmIdleReasonKind::AwaitingTote), Some(&1));
    assert!(slice_0.get(&VtmIdleReasonKind::AwaitingSlot).is_none());

    // Slice 1 [1000, 2000ms): the slot floor + the return deposit wait.
    let slice_1 = &output.time_slices[1].scalars.vtm_idle_reason_counts;
    assert_eq!(slice_1.get(&VtmIdleReasonKind::AwaitingSlot), Some(&1));
    assert_eq!(slice_1.get(&VtmIdleReasonKind::AwaitingHtmDeposit), Some(&1));
    assert!(slice_1.get(&VtmIdleReasonKind::Continuous).is_none());
}

/// `SimEvent::VtmCycle` events are appended to `vtm_cycle_log` verbatim — one
/// record per cycle, carrying the empty/loaded/alignment/transfer split, the
/// cycle direction, and the relay/rack/tote/chain context.
#[test]
fn vtm_cycles_append_to_log() {
    let events = vec![
        SimEvent::VtmCycle(VtmCycleEvent {
            vtm_id: RobotId(2),
            vtm_cycle_id: VtmCycleId(0),
            zone_id: ZoneId(5),
            direction: VtmCycleDirection::Forward,
            empty_leg_ms: 1_200,
            loaded_leg_ms: 3_400,
            fork_ms: 2_000,
            buffer_ms: 2_500,
            rack_alignment_ms: 500,
            relay_alignment_ms: 500,
            relay_transfer_ms: 2_000,
            total_cycle_ms: 9_600,
            sim_time_ms: 1_000,
            relay_id: RelayId(3),
            rack_id: RackId(8),
            tote_id: ToteId(7),
            presentation_id: Some(PresentationId(42)),
            pre_cycle_wait_ms: 600,
            wait_reason: VtmIdleReasonKind::AwaitingDemand,
            tote_source: ToteSource::Pick,
        }),
        SimEvent::VtmCycle(VtmCycleEvent {
            vtm_id: RobotId(2),
            vtm_cycle_id: VtmCycleId(0),
            zone_id: ZoneId(5),
            direction: VtmCycleDirection::Return,
            empty_leg_ms: 800,
            loaded_leg_ms: 3_000,
            fork_ms: 2_000,
            buffer_ms: 2_500,
            rack_alignment_ms: 500,
            relay_alignment_ms: 500,
            relay_transfer_ms: 2_000,
            total_cycle_ms: 8_800,
            sim_time_ms: 12_000,
            relay_id: RelayId(3),
            rack_id: RackId(8),
            tote_id: ToteId(7),
            presentation_id: Some(PresentationId(42)),
            pre_cycle_wait_ms: 0,
            wait_reason: VtmIdleReasonKind::Continuous,
            tote_source: ToteSource::Putaway,
        }),
    ];
    let output = run_with_events_config(events, MetricsConfig::default());

    assert_eq!(output.vtm_cycle_log.len(), 2);
    let fwd = &output.vtm_cycle_log[0];
    assert_eq!(fwd.vtm_id, 2);
    assert_eq!(fwd.zone_id, 5);
    assert_eq!(fwd.direction, VtmCycleDirection::Forward);
    assert_eq!(fwd.empty_leg_ms, 1_200);
    assert_eq!(fwd.loaded_leg_ms, 3_400);
    // `fork_ms` is the rack transfer and `buffer_ms` is the legacy aggregate
    // for relay alignment + relay transfer. The explicit fields preserve the
    // complete six-phase cycle without double-counting.
    assert_eq!(fwd.fork_ms, 2_000);
    assert_eq!(fwd.buffer_ms, 2_500);
    assert_eq!(fwd.rack_alignment_ms, 500);
    assert_eq!(fwd.relay_alignment_ms, 500);
    assert_eq!(fwd.relay_transfer_ms, 2_000);
    assert_eq!(fwd.total_cycle_ms, 9_600);
    assert_eq!(
        fwd.total_cycle_ms,
        fwd.empty_leg_ms + fwd.loaded_leg_ms + fwd.rack_alignment_ms + fwd.fork_ms + fwd.buffer_ms
    );
    assert_eq!(fwd.sim_time_ms, 1_000);
    assert_eq!(fwd.relay_id, 3);
    assert_eq!(fwd.rack_id, 8);
    assert_eq!(fwd.tote_id, 7);
    assert_eq!(fwd.presentation_id, Some(42));
    assert_eq!(fwd.pre_cycle_wait_ms, 600);
    assert_eq!(fwd.wait_reason, VtmIdleReasonKind::AwaitingDemand);
    assert_eq!(fwd.tote_source, ToteSource::Pick);

    let ret = &output.vtm_cycle_log[1];
    assert_eq!(ret.direction, VtmCycleDirection::Return);
    assert_eq!(ret.tote_source, ToteSource::Putaway);
    assert_eq!(ret.total_cycle_ms, 8_800);
    assert_eq!(
        ret.total_cycle_ms,
        ret.empty_leg_ms + ret.loaded_leg_ms + ret.rack_alignment_ms + ret.fork_ms + ret.buffer_ms
    );
}

#[test]
fn vtm_operations_preserve_boundary_payload_and_context() {
    let events = vec![
        SimEvent::VtmOperation(VtmOperationEvent {
            sim_time_ms: 4_000,
            vtm_id: RobotId(2),
            vtm_cycle_id: VtmCycleId(9),
            zone_id: ZoneId(5),
            kind: VtmOperationKind::PickupStorage,
            rack_id: RackId(8),
            rack_level: 3,
            relay_id: RelayId(3),
            tote_id: ToteId(7),
            payload: TotePayloadState::NonEmpty,
            presentation_id: Some(PresentationId(42)),
        }),
        SimEvent::VtmOperation(VtmOperationEvent {
            sim_time_ms: 8_000,
            vtm_id: RobotId(2),
            vtm_cycle_id: VtmCycleId(9),
            zone_id: ZoneId(5),
            kind: VtmOperationKind::DropRelay,
            rack_id: RackId(8),
            rack_level: 3,
            relay_id: RelayId(3),
            tote_id: ToteId(7),
            payload: TotePayloadState::NonEmpty,
            presentation_id: Some(PresentationId(42)),
        }),
    ];

    let output = run_with_events_config(events, MetricsConfig::default());

    assert_eq!(output.vtm_operation_log.len(), 2);
    let pickup = output.vtm_operation_log[0];
    assert_eq!(pickup.sim_time_ms, 4_000);
    assert_eq!(pickup.vtm_id, 2);
    assert_eq!(pickup.vtm_cycle_id, 9);
    assert_eq!(pickup.zone_id, 5);
    assert_eq!(pickup.kind, VtmOperationKind::PickupStorage);
    assert_eq!(pickup.rack_id, 8);
    assert_eq!(pickup.rack_level, Some(3));
    assert_eq!(pickup.relay_id, 3);
    assert_eq!(pickup.tote_id, 7);
    assert_eq!(pickup.payload, TotePayloadState::NonEmpty);
    assert_eq!(pickup.presentation_id, Some(42));
    assert_eq!(output.vtm_operation_log[1].kind, VtmOperationKind::DropRelay);
}

/// A forward VtmNav sequence (loaded RailMove slot→relay, alignment, then
/// Fork(Put)@relay) produces Travel, Alignment, and Mechanical journal rows, with the
/// endpoints resolved (Slot→Rack, Relay→Relay), tote/carrying flags set, and
/// the mechanical sub-kind noted by the fork endpoint (spec §3, DR-2/DR-3).
#[test]
fn vtm_nav_sequence_produces_travel_alignment_and_mechanical_legs() {
    let events = vec![
        SimEvent::InitialPositions(InitialPositionsEvent {
            positions: vec![initial_pos(0, 100, RobotType::Vtm)],
            htm_headings_rad: Vec::new(),
        }),
        // Loaded delivery leg: rack slot → relay, tote on board.
        rail_move_chain(0, 1_000, 3_000, slot(101), relay(102), Some(ToteId(7)), Some(PresentationId(42))),
        alignment(0, 3_000, 3_500, relay(102), Some(ToteId(7))),
        // Buffer-place at the relay.
        fork(0, 3_500, 5_500, relay(102), ForkOp::Put),
    ];
    let output = run_with_events_config(events, MetricsConfig::default());

    assert_eq!(output.vtm_leg_log.len(), 3, "Travel + Alignment + Mechanical");

    let travel = &output.vtm_leg_log[0];
    assert_eq!(travel.activity, Activity::Travel);
    assert_eq!(travel.start_ms, 1_000);
    assert_eq!(travel.duration_ms, 2_000);
    assert_eq!(travel.from, VtmJournalEndpoint::Rack(RackId(0)));
    assert_eq!(travel.to, VtmJournalEndpoint::Relay(RelayId(0)));
    assert!(travel.carrying_tote);
    assert_eq!(travel.tote_id, Some(7));
    assert_eq!(travel.presentation_id, Some(42));
    assert_eq!(travel.direction, VtmCycleDirection::Forward);

    let alignment = &output.vtm_leg_log[1];
    assert_eq!(alignment.activity, Activity::Alignment);
    assert_eq!(alignment.start_ms, 3_000);
    assert_eq!(alignment.duration_ms, 500);
    assert_eq!(alignment.from, VtmJournalEndpoint::Relay(RelayId(0)));
    assert!(alignment.carrying_tote);
    assert_eq!(alignment.notes, "align @ relay");

    let mech = &output.vtm_leg_log[2];
    assert_eq!(mech.activity, Activity::Mechanical);
    assert_eq!(mech.start_ms, 3_500);
    assert_eq!(mech.duration_ms, 2_000);
    assert_eq!(mech.from, VtmJournalEndpoint::Relay(RelayId(0)));
    assert_eq!(mech.distance_mm, 0);
    assert!(mech.carrying_tote);
    assert_eq!(mech.notes, "buffer-put @ relay");
}

/// A non-`Continuous` `VtmIdleReason` yields one Wait leg spanning
/// `[at − gap, at)` with the reason recorded; a `Continuous` reason (no gap)
/// yields no Wait leg (spec §3, DR-4 / §6).
#[test]
fn vtm_idle_reason_produces_wait_leg_from_gap() {
    let events = vec![
        SimEvent::VtmIdleReason(VtmIdleReasonEvent {
            robot_id: RobotId(0),
            vtm_cycle_id: VtmCycleId(0),
            at: SimTime::ms(5_000),
            reason: VtmIdleReason::AwaitingDemand { gap: SimDuration::ms(1_500), relay_start: SimTime::ms(5_000) },
            presentation_id: Some(PresentationId(42)),
            direction: VtmCycleDirection::Forward,
            relay_id: RelayId(9),
            geo_diverged: false,
        }),
        // Continuous: no gap → no Wait row.
        SimEvent::VtmIdleReason(VtmIdleReasonEvent {
            robot_id: RobotId(1),
            vtm_cycle_id: VtmCycleId(0),
            at: SimTime::ms(6_000),
            reason: VtmIdleReason::Continuous,
            presentation_id: Some(PresentationId(43)),
            direction: VtmCycleDirection::Return,
            relay_id: RelayId(0),
            geo_diverged: false,
        }),
    ];
    let output = run_with_events_config(events, MetricsConfig::default());

    assert_eq!(output.vtm_leg_log.len(), 1, "only the gap-bearing reason makes a Wait row");
    let wait = &output.vtm_leg_log[0];
    assert_eq!(wait.activity, Activity::Wait);
    assert_eq!(wait.start_ms, 3_500, "start = at − gap");
    assert_eq!(wait.duration_ms, 1_500);
    // The Wait row sits at the relay the VTM is waiting on, not a generic Aisle.
    assert_eq!(wait.from, VtmJournalEndpoint::Relay(RelayId(9)));
    assert_eq!(wait.to, VtmJournalEndpoint::Relay(RelayId(9)));
    assert_eq!(wait.wait_reason, Some(VtmIdleReasonKind::AwaitingDemand));
    assert_eq!(wait.presentation_id, Some(42));
    assert!(!wait.carrying_tote);
}

/// A populated `relay_zones` table stamps each VTM leg's relay endpoint(s) with
/// its zone: the loaded Travel leg (Rack→Relay) gets the zone only on `to`, and
/// the buffer-place Mechanical leg (at the relay) gets it on both `from` and
/// `to`. Non-relay endpoints (the rack) stay `None`.
#[test]
fn vtm_leg_relay_endpoints_carry_zone() {
    let config = MetricsConfig { relay_zones: [(RelayId(0), 3)].into_iter().collect(), ..MetricsConfig::default() };
    let events = vec![
        SimEvent::InitialPositions(InitialPositionsEvent {
            positions: vec![initial_pos(0, 100, RobotType::Vtm)],
            htm_headings_rad: Vec::new(),
        }),
        rail_move_chain(0, 1_000, 3_000, slot(101), relay(102), Some(ToteId(7)), Some(PresentationId(42))),
        fork(0, 3_000, 5_000, relay(102), ForkOp::Put),
    ];
    let output = run_with_events_config(events, config);

    let travel = &output.vtm_leg_log[0];
    assert_eq!(travel.from, VtmJournalEndpoint::Rack(RackId(0)));
    assert_eq!(travel.from_zone_id, None, "rack endpoint carries no zone");
    assert_eq!(travel.to, VtmJournalEndpoint::Relay(RelayId(0)));
    assert_eq!(travel.to_zone_id, Some(3), "relay endpoint carries its zone");

    let mech = &output.vtm_leg_log[1];
    assert_eq!(mech.from_zone_id, Some(3));
    assert_eq!(mech.to_zone_id, Some(3));
}

/// A VTM Wait leg sits at the relay it is waiting on, so both endpoints carry
/// that relay's zone.
#[test]
fn vtm_wait_leg_carries_relay_zone() {
    let config = MetricsConfig { relay_zones: [(RelayId(9), 7)].into_iter().collect(), ..MetricsConfig::default() };
    let events = vec![SimEvent::VtmIdleReason(VtmIdleReasonEvent {
        robot_id: RobotId(0),
        vtm_cycle_id: VtmCycleId(0),
        at: SimTime::ms(5_000),
        reason: VtmIdleReason::AwaitingDemand { gap: SimDuration::ms(1_500), relay_start: SimTime::ms(5_000) },
        presentation_id: Some(PresentationId(42)),
        direction: VtmCycleDirection::Forward,
        relay_id: RelayId(9),
        geo_diverged: false,
    })];
    let output = run_with_events_config(events, config);

    let wait = &output.vtm_leg_log[0];
    assert_eq!(wait.from, VtmJournalEndpoint::Relay(RelayId(9)));
    assert_eq!(wait.from_zone_id, Some(7));
    assert_eq!(wait.to_zone_id, Some(7));
}

/// An HTM Travel leg into a relay carries the relay's zone on `to`; its
/// Parking origin carries none. An empty `relay_zones` table leaves both blank.
#[test]
fn htm_leg_relay_endpoint_carries_zone() {
    let config = MetricsConfig { relay_zones: [(RelayId(0), 5)].into_iter().collect(), ..MetricsConfig::default() };
    let events = vec![
        SimEvent::InitialPositions(InitialPositionsEvent {
            positions: vec![initial_pos(4, 50, RobotType::Htm)],
            htm_headings_rad: Vec::new(),
        }),
        htm_trip_start(4, TripPurpose::ToRelay, 1_000),
        htm_nav(4, 50, 1_000, &[Orientation::North, Orientation::East]),
    ];
    let output = run_with_events_config(events, config);

    let travel = output
        .htm_leg_log
        .iter()
        .find(|r| r.activity == Activity::Travel && r.to == HtmJournalEndpoint::Relay(RelayId(0)))
        .expect("a Travel leg into the relay");
    assert_eq!(travel.from, HtmJournalEndpoint::Parking);
    assert_eq!(travel.from_zone_id, None, "parking origin carries no zone");
    assert_eq!(travel.to_zone_id, Some(5), "relay destination carries its zone");
}

/// Per-slice fleet aggregates (`vtm_duty_*_ms` / `htm_duty_*_ms`) accumulate
/// across multiple robots of the same type. These drive the line-chart
/// metrics (`MetricKind::VtmTraveling` etc.), which divide by `slice_width_ms`
/// to get the time-weighted "average # of robots in this state."
#[test]
fn fleet_duty_aggregates_sum_across_robots() {
    let events = vec![
        SimEvent::InitialPositions(InitialPositionsEvent {
            positions: vec![initial_pos(0, 100, RobotType::Vtm), initial_pos(1, 100, RobotType::Vtm)],
            htm_headings_rad: Vec::new(),
        }),
        // VTM 0: 1s RailMove + 2s Fork(Get) starting at t=0.
        rail_move(0, 0, 1_000, aisle(100), slot(101)),
        fork(0, 1_000, 3_000, slot(101), ForkOp::Get),
        // VTM 1: 2s RailMove + 1s Fork(Get) starting at t=0.
        rail_move(1, 0, 2_000, aisle(100), slot(101)),
        fork(1, 2_000, 3_000, slot(101), ForkOp::Get),
    ];

    let output = run_with_events_config(events, MetricsConfig::default());
    let slice_0 = &output.time_slices[0].scalars;

    // Fleet aggregates sum across both VTMs.
    assert_eq!(slice_0.vtm_duty_traveling_ms, 3_000); // 1000 + 2000
    assert_eq!(slice_0.vtm_duty_loading_unloading_ms, 3_000); // 2000 + 1000
    // No HTM activity.
    assert_eq!(slice_0.htm_duty_traveling_ms, 0);
    assert_eq!(slice_0.htm_duty_loading_unloading_ms, 0);
}

/// The `record_robot_timelines` flag gates the raw timeline storage:
/// totals are populated either way, but the per-bot `BusyInterval` list
/// is empty unless the flag is set.
#[test]
fn record_robot_timelines_flag_gates_storage() {
    let events = || {
        vec![
            SimEvent::InitialPositions(InitialPositionsEvent {
                positions: vec![initial_pos(0, 100, RobotType::Vtm)],
                htm_headings_rad: Vec::new(),
            }),
            rail_move(0, 0, 1_000, aisle(100), slot(101)),
            fork(0, 1_000, 3_000, slot(101), ForkOp::Get),
        ]
    };

    let off = run_with_events_config(events(), MetricsConfig::default());
    let on =
        run_with_events_config(events(), MetricsConfig { record_robot_timelines: true, ..MetricsConfig::default() });

    // Totals match regardless of the flag.
    assert_eq!(off.robot_duty_totals.get(&RobotId(0)).unwrap().busy(), SimDuration::ms(3_000));
    assert_eq!(on.robot_duty_totals.get(&RobotId(0)).unwrap().busy(), SimDuration::ms(3_000));

    // Timeline empty when off, populated when on.
    assert!(off.robot_timelines.is_empty());
    let timeline = on.robot_timelines.get(&RobotId(0)).expect("timeline present when flag on");
    assert_eq!(timeline.len(), 2);
    assert_eq!(timeline[0].kind, ActivityKind::Travel);
    assert_eq!(timeline[0].duration(), SimDuration::ms(1_000));
    assert_eq!(timeline[1].kind, ActivityKind::ForkOp);
    assert_eq!(timeline[1].duration(), SimDuration::ms(2_000));
}
