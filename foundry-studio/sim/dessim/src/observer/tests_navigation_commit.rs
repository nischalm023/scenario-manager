//! Committed robot movement is immutable, including movement ahead of the clock.
//! A slice made drawable stands each robot where its committed movement left it.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use desgraph::{CellId, RobotType, WorldPos};

use super::state::SimObserverState;
use super::test_models::legacy_models;
use super::time_slice::MetricsConfig;
use crate::config::SimConfig;
use crate::events::{InitialPositionsEvent, NavEventData, SimEvent};
use crate::model::{RobotId, SimTime};
use crate::pathfinding::{Orientation, Waypoint, WaypointAction};

fn state() -> SimObserverState {
    let mut state = SimObserverState::new(MetricsConfig {
        slice_width_ms: 1000,
        htm_models: legacy_models(&SimConfig::default(), [RobotId(7)]),
        cell_positions: Arc::new((0..4).map(|x| WorldPos { x_mm: x as f64 * 1000.0, y_mm: 0.0 }).collect()),
        ..MetricsConfig::default()
    });
    state.dispatch_event(&SimEvent::InitialPositions(InitialPositionsEvent {
        positions: vec![(RobotId(7), CellId(0), RobotType::Htm, WorldPos::default())],
        htm_headings_rad: Vec::new(),
    }));
    state
}

fn nav(from: u32, start: u64, end: u64) -> NavEventData {
    NavEventData {
        robot_id: RobotId(7),
        robot_type: RobotType::Htm,
        time: SimTime::ms(start),
        distance_mm: 1000,
        carrying_cargo: false,
        waypoints: vec![
            Waypoint {
                cell: CellId(from),
                orientation: Orientation::East,
                action: WaypointAction::PathStart { time: SimTime::ms(start) },
            },
            Waypoint {
                cell: CellId(from + 1),
                orientation: Orientation::East,
                action: WaypointAction::Move {
                    start: SimTime::ms(start),
                    end: SimTime::ms(end),
                    entry_velocity_mm_s: 1000.0,
                    exit_velocity_mm_s: 1000.0,
                },
            },
            Waypoint {
                cell: CellId(from + 1),
                orientation: Orientation::East,
                action: WaypointAction::PathEnd { time: SimTime::ms(end) },
            },
        ],
    }
}

#[test]
fn overlapping_future_navigation_is_rejected_without_rewriting_committed_facts() {
    let mut state = state();
    state.handle_nav(&nav(0, 10_000, 12_000));
    let before = rmp_serde::to_vec_named(state.slices()).unwrap();
    let result = catch_unwind(AssertUnwindSafe(|| state.handle_nav(&nav(0, 11_000, 13_000))));
    assert!(result.is_err());
    assert_eq!(rmp_serde::to_vec_named(state.slices()).unwrap(), before);
    let travelled: u64 = state.slices().iter().map(|slice| slice.scalars.htm_travel_distance_mm).sum();
    assert_eq!(travelled, 1000);
}

#[test]
fn adjacent_commits_and_a_later_departure_stand_the_robot_between_them() {
    let mut state = state();
    state.handle_nav(&nav(0, 0, 1000));
    let first = state.slices()[0].bot_locations[&7].clone();
    state.handle_nav(&nav(1, 1000, 2000));
    state.handle_nav(&nav(2, 5500, 6500));
    state.extend_slices_through(8);
    for index in 0..state.slices().len() {
        state.make_drawable(index);
    }
    assert_eq!(
        rmp_serde::to_vec_named(&state.slices()[0].bot_locations[&7]).unwrap(),
        rmp_serde::to_vec_named(&first).unwrap()
    );
    assert_eq!(state.slices()[4].bot_locations[&7].start_cell, 2);
    assert_eq!(state.slices()[7].bot_locations[&7].start_cell, 3);
    let travelled: u64 = state.slices().iter().map(|slice| slice.scalars.htm_travel_distance_mm).sum();
    assert_eq!(travelled, 3000);
}
