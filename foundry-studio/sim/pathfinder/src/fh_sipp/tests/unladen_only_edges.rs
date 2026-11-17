//! HTM laden/unladen behavior across `HtmTraversal::UnladenOnly` edges.

use super::prelude::*;

/// Linear graph with the 0↔1 edge restricted to unladen HTM traffic.
///   Open[0] --R-- Storage[1] --- Open[2]
fn linear_unladen_only_at_left() -> WarehouseGraph {
    let mut b = WarehouseGraphBuilder::new();
    b.add_cell(cell(0, CellKind::Open, 0, 0));
    b.add_cell(cell(1, CellKind::Storage, 1, 0));
    b.add_cell(cell(2, CellKind::Open, 2, 0));

    let restricted = Edge {
        traversability: EdgeTraversability {
            vtm_traversal: true,
            htm_traversal: HtmTraversal::UnladenOnly,
            rtp_traversal: true,
        },
        distance_mm: 1000,
        curve: None,
        dest_only: None,
        vtm_zone_gate: None,
        htm_drive: HtmDriveDirection::Forward,
    };
    b.add_edge_bidi(c(0), c(1), restricted);
    b.add_edge_bidi(c(1), c(2), Edge::uniform(1000));
    b.build()
}

/// 2×3 grid with the 3↔4 edge restricted to unladen. Loaded HTM detours
/// via the top row.
///   [0]---[1]---[2]
///    |     |     |
///   [3]-x-[4]---[5]
fn detour_grid() -> WarehouseGraph {
    let mut b = WarehouseGraphBuilder::new();
    for y in 0..2u32 {
        for x in 0..3u32 {
            b.add_cell(cell(y * 3 + x, CellKind::Open, x, y));
        }
    }
    let open = Edge::uniform(1000);
    let restricted = Edge {
        traversability: EdgeTraversability {
            vtm_traversal: true,
            htm_traversal: HtmTraversal::UnladenOnly,
            rtp_traversal: true,
        },
        distance_mm: 1000,
        curve: None,
        dest_only: None,
        vtm_zone_gate: None,
        htm_drive: HtmDriveDirection::Forward,
    };
    b.add_edge_bidi(c(0), c(1), open);
    b.add_edge_bidi(c(1), c(2), open);
    b.add_edge_bidi(c(3), c(4), restricted);
    b.add_edge_bidi(c(4), c(5), open);
    b.add_edge_bidi(c(0), c(3), open);
    b.add_edge_bidi(c(1), c(4), open);
    b.add_edge_bidi(c(2), c(5), open);
    b.build()
}

#[test]
fn htm_unloaded_traverses_restricted_edge() {
    let graph = linear_unladen_only_at_left();
    let (pf, trace) = pathfinder_traced(&graph);

    let drive = &uniform_drive(RobotType::Htm, false);
    let result = pf
        .find_path(c(0), Orientation::East, c(2), drive, SimTime::ZERO)
        .expect("unladen HTM may cross UnladenOnly edge");

    use Orientation::{East, North, South};
    assert_trace!(
        trace,
        [
            Expand { cell: c(0), orientation: East, g: ms(0), step: 0 },
            Move(c(0), c(1), ms(1_000), ms(0)),
            Expand { cell: c(1), orientation: East, g: ms(1_000), step: 0 },
            Rotate(c(1), East, North, ms(2_000), ms(0)),
            Rotate(c(1), East, South, ms(2_000), ms(0)),
            Move(c(1), c(2), ms(2_000), ms(0)),
            Expand { cell: c(2), orientation: East, g: ms(2_000), step: 0 },
        ]
    );
    assert_waypoints!(
        result,
        [
            path_start(c(0), East, t(0)),
            mv(c(1), East, t(0), t(1_000)),
            mv(c(2), East, t(1_000), t(2_000)),
            path_end(c(2), East, t(2_000)),
        ]
    );
}

#[test]
fn htm_loaded_blocked_by_unladen_only_edge() {
    let graph = linear_unladen_only_at_left();
    let (pf, trace) = pathfinder_traced(&graph);

    let drive = &uniform_drive(RobotType::Htm, true);
    let result = pf.find_path(c(0), Orientation::East, c(2), drive, SimTime::ZERO);

    // Loaded HTM cannot cross the UnladenOnly edge to c(1). c(0) has no
    // other neighbors, so reachable_orients is empty and the search
    // exhausts after a single Expand.
    use Orientation::East;
    assert_trace!(trace, [Expand { cell: c(0), orientation: East, g: ms(0), step: 0 },]);
    assert!(matches!(result, Err(PathfinderError::Unreachable { .. })));
}

#[test]
fn unloaded_takes_direct_path() {
    let graph = detour_grid();
    let (pf, trace) = pathfinder_traced(&graph);

    let drive = &uniform_drive(RobotType::Htm, false);
    let result = pf
        .find_path(c(3), Orientation::East, c(5), drive, SimTime::ZERO)
        .expect("unladen HTM crosses restricted edge directly");

    use Orientation::{East, North, South};
    assert_trace!(
        trace,
        [
            Expand { cell: c(3), orientation: East, g: ms(0), step: 0 },
            Rotate(c(3), East, North, ms(1_000), ms(0)),
            Move(c(3), c(4), ms(1_000), ms(0)),
            Expand { cell: c(4), orientation: East, g: ms(1_000), step: 0 },
            Rotate(c(4), East, North, ms(2_000), ms(0)),
            Rotate(c(4), East, South, ms(2_000), ms(0)),
            Move(c(4), c(5), ms(2_000), ms(0)),
            Expand { cell: c(5), orientation: East, g: ms(2_000), step: 0 },
        ]
    );
    assert_waypoints!(
        result,
        [
            path_start(c(3), East, t(0)),
            mv(c(4), East, t(0), t(1_000)),
            mv(c(5), East, t(1_000), t(2_000)),
            path_end(c(5), East, t(2_000)),
        ]
    );
    assert_eq!(result.travel_time, ms(2_000));
}

#[test]
fn loaded_takes_detour_via_top_row() {
    let graph = detour_grid();
    let (pf, trace) = pathfinder_traced(&graph);

    let drive = &uniform_drive(RobotType::Htm, true);
    let result = pf
        .find_path(c(3), Orientation::East, c(5), drive, SimTime::ZERO)
        .expect("laden HTM must detour around restricted edge");

    // The c(3)↔c(4) edge is restricted to unladen HTM, so a laden HTM has
    // exactly one detour: the top row 3→0→1→2→5. Pin the chain. Exploration
    // order is left to the search (don't pin `assert_trace!`).
    use Orientation::{East, North, South, West};
    // The detour itself: rotate at c(3) to head north, traverse the top row
    // east-west-east, rotate at c(2) to head south, drop into c(5).
    assert_trace!(
        trace,
        [
            Expand { cell: c(3), orientation: East, g: ms(0), step: 0 },
            Rotate(c(3), East, North, ms(1_000), ms(0)),
            Expand { cell: c(3), orientation: North, g: ms(1_000), step: 0 },
            Move(c(3), c(0), ms(2_000), ms(0)),
            Expand { cell: c(0), orientation: North, g: ms(2_000), step: 0 },
            Rotate(c(0), North, East, ms(3_000), ms(0)),
            Rotate(c(0), North, West, ms(3_000), ms(0)),
            Expand { cell: c(0), orientation: West, g: ms(3_000), step: 0 },
            Rotate(c(0), West, South, ms(4_000), ms(0)),
            Expand { cell: c(0), orientation: East, g: ms(3_000), step: 0 },
            Move(c(0), c(1), ms(4_000), ms(0)),
            Expand { cell: c(1), orientation: East, g: ms(4_000), step: 0 },
            Rotate(c(1), East, North, ms(5_000), ms(0)),
            Rotate(c(1), East, South, ms(5_000), ms(0)),
            Move(c(1), c(2), ms(5_000), ms(0)),
            Expand { cell: c(2), orientation: East, g: ms(5_000), step: 0 },
            Rotate(c(2), East, North, ms(6_000), ms(0)),
            Rotate(c(2), East, South, ms(6_000), ms(0)),
            Expand { cell: c(2), orientation: South, g: ms(6_000), step: 0 },
            Rotate(c(2), South, West, ms(7_000), ms(0)),
            Move(c(2), c(5), ms(7_000), ms(0)),
            Expand { cell: c(5), orientation: South, g: ms(7_000), step: 0 },
        ]
    );
    assert_waypoints!(
        result,
        [
            path_start(c(3), East, t(0)),
            rotate(c(3), North, t(0), t(1_000)),
            mv(c(0), North, t(1_000), t(2_000)),
            rotate(c(0), East, t(2_000), t(3_000)),
            mv(c(1), East, t(3_000), t(4_000)),
            mv(c(2), East, t(4_000), t(5_000)),
            rotate(c(2), South, t(5_000), t(6_000)),
            mv(c(5), South, t(6_000), t(7_000)),
            path_end(c(5), South, t(7_000)),
        ]
    );
    assert_eq!(result.travel_time, ms(7_000));
}
