//! VTM eligibility under the `vtm_traversal` edge flag.

use super::prelude::*;

/// 4-cell linear graph with every edge flagged `vtm_traversal: false`.
///   Open[0] -- Highway[1] -- Open[2] -- Storage[3]
fn linear_vtm_blocked() -> WarehouseGraph {
    let mut b = WarehouseGraphBuilder::new();
    b.add_cell(cell(0, CellKind::Open, 0, 0));
    b.add_cell(cell(1, CellKind::Highway, 1, 0));
    b.add_cell(cell(2, CellKind::Open, 2, 0));
    b.add_cell(cell(3, CellKind::Storage, 3, 0));

    let vtm_blocked = Edge {
        traversability: EdgeTraversability {
            vtm_traversal: false,
            htm_traversal: HtmTraversal::Both,
            rtp_traversal: true,
        },
        distance_mm: 1000,
        curve: None,
        dest_only: None,
        vtm_zone_gate: None,
        htm_drive: HtmDriveDirection::Forward,
    };
    b.add_edge_bidi(c(0), c(1), vtm_blocked);
    b.add_edge_bidi(c(1), c(2), vtm_blocked);
    b.add_edge_bidi(c(2), c(3), vtm_blocked);
    b.build()
}

#[test]
fn vtm_blocked_to_open_target() {
    let graph = linear_vtm_blocked();
    let (pf, trace) = pathfinder_traced(&graph);

    let drive = &uniform_drive(RobotType::Vtm, false);
    let result = pf.find_path(c(0), Orientation::East, c(2), drive, SimTime::ZERO);

    // VTM cannot cross the blocked edge to c(1); reachable_orients at
    // c(0) is empty so the search exhausts after one Expand.
    use Orientation::East;
    assert_trace!(trace, [Expand { cell: c(0), orientation: East, g: ms(0), step: 0 },]);
    assert!(matches!(result, Err(PathfinderError::Unreachable { .. })));
}

#[test]
fn vtm_blocked_to_storage_target() {
    // Same trace shape as `vtm_blocked_to_open_target` — the search
    // exhausts at c(0) before any neighbor cell is reached, so the
    // target downstream of the blocked edges is irrelevant. The
    // duplicated assertion is the point: distinct scenarios that share
    // a trace should both pin it; merging would erase the distinction.
    let graph = linear_vtm_blocked();
    let (pf, trace) = pathfinder_traced(&graph);

    let drive = &uniform_drive(RobotType::Vtm, false);
    let result = pf.find_path(c(0), Orientation::East, c(3), drive, SimTime::ZERO);

    use Orientation::East;
    assert_trace!(trace, [Expand { cell: c(0), orientation: East, g: ms(0), step: 0 },]);
    assert!(matches!(result, Err(PathfinderError::Unreachable { .. })));
}

#[test]
fn htm_passes_when_vtm_flag_off() {
    let graph = linear_vtm_blocked();
    let (pf, trace) = pathfinder_traced(&graph);

    let drive = &uniform_drive(RobotType::Htm, false);
    let result = pf
        .find_path(c(0), Orientation::East, c(3), drive, SimTime::ZERO)
        .expect("HTM is unaffected by vtm_traversal flag");

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
            Rotate(c(2), East, North, ms(3_000), ms(0)),
            Rotate(c(2), East, South, ms(3_000), ms(0)),
            Move(c(2), c(3), ms(3_000), ms(0)),
            Expand { cell: c(3), orientation: East, g: ms(3_000), step: 0 },
        ]
    );
    assert_waypoints!(
        result,
        [
            path_start(c(0), East, t(0)),
            mv(c(1), East, t(0), t(1_000)),
            mv(c(2), East, t(1_000), t(2_000)),
            mv(c(3), East, t(2_000), t(3_000)),
            path_end(c(3), East, t(3_000)),
        ]
    );
}

#[test]
fn vtm_traverses_when_flag_set() {
    let graph = corridor(3);
    let (pf, trace) = pathfinder_traced(&graph);

    let drive = &uniform_drive(RobotType::Vtm, false);
    let result = pf
        .find_path(c(0), Orientation::East, c(2), drive, SimTime::ZERO)
        .expect("VTM may cross fully-traversable edges");

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
