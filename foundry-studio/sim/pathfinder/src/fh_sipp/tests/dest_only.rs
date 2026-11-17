//! `dest_only` edges: through-traffic detours, target HTM passes.

use super::prelude::*;

#[test]
fn dest_only_forces_through_traffic_to_detour() {
    // [0] --d-- [1] --- [2]
    //  |                 |
    // [3] ------------- [4]
    // 0→1 is dest_only=c(1). Reverse 1→0 is unrestricted.
    let mut b = WarehouseGraphBuilder::new();
    b.add_cell(cell(0, CellKind::Open, 0, 0));
    b.add_cell(cell(1, CellKind::Open, 1, 0));
    b.add_cell(cell(2, CellKind::Open, 2, 0));
    b.add_cell(cell(3, CellKind::Open, 0, 1));
    b.add_cell(cell(4, CellKind::Open, 2, 1));

    let dest_only_edge = Edge {
        traversability: EdgeTraversability::fully_traversable(),
        distance_mm: 1000,
        curve: None,
        dest_only: Some(DestGate::Cell(c(1))),
        vtm_zone_gate: None,
        htm_drive: HtmDriveDirection::Forward,
    };
    let open = Edge::uniform(1000);

    b.add_edge(c(0), c(1), dest_only_edge);
    b.add_edge(c(1), c(0), open);
    b.add_edge_bidi(c(1), c(2), open);
    b.add_edge_bidi(c(0), c(3), open);
    b.add_edge_bidi(c(3), c(4), open);
    b.add_edge_bidi(c(4), c(2), open);

    let graph = b.build();
    let (pf, trace) = pathfinder_traced(&graph);

    let drive = &uniform_drive(RobotType::Htm, false);
    let result = pf
        .find_path(c(0), Orientation::East, c(2), drive, SimTime::ZERO)
        .expect("through-traffic detours around dest_only edge");

    // dest_only on 0→1 forces the detour 0→3→4→2; the geometry leaves no
    // alternate. Trace + path + travel_time pinned per test-conventions.md.
    // (Migrated when BucketQueue's cursor learned to REWIND — bisected: with
    // only the rewind disabled this trace reverts to its old 24 events, while
    // with only the seeding disabled the new 19-event order stands, so the
    // rewind is the cause and the seeding alone changes nothing here. The old
    // order interleaved the dead-end backtrack states — c(3) North, Move back
    // to c(0), c(0) West — into the pop order ahead of the goal; a cursor
    // that can walk back to the true minimum pops the goal-ward frontier
    // first and those five expansions vanish. Same route, same travel_time.)
    use Orientation::{East, North, South, West};
    assert_trace!(
        trace,
        [
            Expand { cell: c(0), orientation: East, g: ms(0), step: 0 },
            Rotate(c(0), East, South, ms(1_000), ms(0)),
            Expand { cell: c(0), orientation: South, g: ms(1_000), step: 0 },
            Move(c(0), c(3), ms(2_000), ms(0)),
            Expand { cell: c(3), orientation: South, g: ms(2_000), step: 0 },
            Rotate(c(3), South, East, ms(3_000), ms(0)),
            Rotate(c(3), South, West, ms(3_000), ms(0)),
            Expand { cell: c(3), orientation: West, g: ms(3_000), step: 0 },
            Rotate(c(3), West, North, ms(4_000), ms(0)),
            Expand { cell: c(3), orientation: East, g: ms(3_000), step: 0 },
            Move(c(3), c(4), ms(4_000), ms(0)),
            Expand { cell: c(4), orientation: East, g: ms(4_000), step: 0 },
            Rotate(c(4), East, North, ms(5_000), ms(0)),
            Rotate(c(4), East, South, ms(5_000), ms(0)),
            Expand { cell: c(4), orientation: South, g: ms(5_000), step: 0 },
            Rotate(c(4), South, West, ms(6_000), ms(0)),
            Expand { cell: c(4), orientation: North, g: ms(5_000), step: 0 },
            Move(c(4), c(2), ms(6_000), ms(0)),
            Expand { cell: c(2), orientation: North, g: ms(6_000), step: 0 },
        ]
    );
    assert_waypoints!(
        result,
        [
            path_start(c(0), East, t(0)),
            rotate(c(0), South, t(0), t(1_000)),
            mv(c(3), South, t(1_000), t(2_000)),
            rotate(c(3), East, t(2_000), t(3_000)),
            mv(c(4), East, t(3_000), t(4_000)),
            rotate(c(4), North, t(4_000), t(5_000)),
            mv(c(2), North, t(5_000), t(6_000)),
            path_end(c(2), North, t(6_000)),
        ]
    );
    assert_eq!(result.travel_time, ms(6_000));
}

#[test]
fn dest_only_allows_target_htm() {
    let mut b = WarehouseGraphBuilder::new();
    b.add_cell(cell(0, CellKind::Open, 0, 0));
    b.add_cell(cell(1, CellKind::Open, 1, 0));
    b.add_cell(cell(2, CellKind::Open, 2, 0));

    let dest_only_edge = Edge {
        traversability: EdgeTraversability::fully_traversable(),
        distance_mm: 1000,
        curve: None,
        dest_only: Some(DestGate::Cell(c(1))),
        vtm_zone_gate: None,
        htm_drive: HtmDriveDirection::Forward,
    };
    let open = Edge::uniform(1000);
    b.add_edge(c(0), c(1), dest_only_edge);
    b.add_edge(c(1), c(0), open);
    b.add_edge_bidi(c(1), c(2), open);

    let graph = b.build();
    let (pf, trace) = pathfinder_traced(&graph);

    let drive = &uniform_drive(RobotType::Htm, false);
    let result = pf
        .find_path(c(0), Orientation::East, c(1), drive, SimTime::ZERO)
        .expect("HTM targeting the dest_only cell may cross the edge");

    use Orientation::East;
    assert_trace!(
        trace,
        [
            Expand { cell: c(0), orientation: East, g: ms(0), step: 0 },
            Move(c(0), c(1), ms(1_000), ms(0)),
            Expand { cell: c(1), orientation: East, g: ms(1_000), step: 0 },
        ]
    );
    assert_waypoints!(
        result,
        [path_start(c(0), East, t(0)), mv(c(1), East, t(0), t(1_000)), path_end(c(1), East, t(1_000)),]
    );
}
