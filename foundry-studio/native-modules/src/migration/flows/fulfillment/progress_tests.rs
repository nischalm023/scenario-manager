//! An order-area route's one boundary crossing, heard window by window: the
//! area is cell 10, entered from 9 and left to 11.

use desgraph::CellId;
use desmap::OrderHtmArea;

use super::BoundaryCrossing;
use crate::flows::transport::windows::RouteWindow;
use crate::model::SimTime;
use crate::pathfinding::{Orientation, Path, Waypoint, WaypointAction};

fn area() -> OrderHtmArea {
    OrderHtmArea { entry: CellId(9), egress: CellId(11), resident_cells: vec![CellId(10)] }
}

/// A window through `cells`, one second a move, from `start` seconds.
fn window(cells: &[u32], start: u64) -> Path {
    let at = |step: u64| SimTime::seconds(start + step);
    let mut waypoints = vec![Waypoint {
        cell: CellId(cells[0]),
        orientation: Orientation::East,
        action: WaypointAction::PathStart { time: at(0) },
    }];
    for (step, &cell) in (1..).zip(&cells[1..]) {
        waypoints.push(Waypoint {
            cell: CellId(cell),
            orientation: Orientation::East,
            action: WaypointAction::Move {
                start: at(step - 1),
                end: at(step),
                entry_velocity_mm_s: 0.0,
                exit_velocity_mm_s: 0.0,
            },
        });
    }
    let end = at(cells.len() as u64 - 1);
    waypoints.push(Waypoint {
        cell: CellId(*cells.last().unwrap()),
        orientation: Orientation::East,
        action: WaypointAction::PathEnd { time: end },
    });
    Path { waypoints, travel_time: end - at(0) }
}

#[test]
fn an_exit_crossed_in_an_earlier_window_is_heard_once_and_the_last_window_needs_none() {
    let mut exit = BoundaryCrossing::exit(area());
    let crossed = exit.in_window(&window(&[10, 11, 12], 0), RouteWindow::ShortOfTarget).unwrap();
    assert_eq!(crossed, Some(SimTime::seconds(1)), "the exit counts once the HTM has crossed into 11");
    // The barrier passes; the last window starts later from where the first ended.
    let last = exit.in_window(&window(&[12, 13], 40), RouteWindow::Last).unwrap();
    assert_eq!(last, None, "the last window crosses nothing and is not an error");
}

#[test]
fn a_route_whose_last_window_commits_without_its_crossing_is_an_error() {
    let mut entry = BoundaryCrossing::entry(area());
    assert_eq!(entry.in_window(&window(&[6, 7, 8], 0), RouteWindow::ShortOfTarget).unwrap(), None);
    assert!(entry.in_window(&window(&[8, 9], 30), RouteWindow::Last).is_err(), "the route never entered the area");
}

#[test]
fn a_crossing_heard_again_in_a_later_window_is_an_error() {
    // Each window alone crosses once; the route as a whole crosses twice.
    let mut exit = BoundaryCrossing::exit(area());
    exit.in_window(&window(&[10, 11], 0), RouteWindow::ShortOfTarget).unwrap();
    assert!(exit.in_window(&window(&[10, 11], 30), RouteWindow::Last).is_err(), "it remembers the first crossing");
}
