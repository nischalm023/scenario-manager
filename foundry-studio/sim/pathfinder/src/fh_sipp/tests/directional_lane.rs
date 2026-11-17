//! Eastbound-only lane: HTM heading west must detour through an
//! adjacent row.

use super::prelude::*;

#[test]
fn directional_lane_blocks_wrong_direction() {
    // 5×3 grid, row 1 is eastbound: westward edges set HtmTraversal::None.
    let w = 5u32;
    let mut b = WarehouseGraphBuilder::new();
    for y in 0..3u32 {
        for x in 0..w {
            b.add_cell(cell(y * w + x, CellKind::Open, x, y));
        }
    }

    let open = Edge::uniform(1000);
    let eastward_only = Edge {
        traversability: EdgeTraversability {
            vtm_traversal: true,
            htm_traversal: HtmTraversal::None,
            rtp_traversal: true,
        },
        distance_mm: 1000,
        curve: None,
        dest_only: None,
        vtm_zone_gate: None,
        htm_drive: HtmDriveDirection::Forward,
    };

    for y in 0..3u32 {
        for x in 0..w {
            let id = c(y * w + x);
            if x + 1 < w {
                let right = c(y * w + x + 1);
                if y == 1 {
                    b.add_edge(id, right, open);
                    b.add_edge(right, id, eastward_only);
                } else {
                    b.add_edge_bidi(id, right, open);
                }
            }
            if y + 1 < 3 {
                let down = c((y + 1) * w + x);
                b.add_edge_bidi(id, down, open);
            }
        }
    }
    let graph = b.build();
    let (pf, trace) = pathfinder_traced(&graph);

    // From (4,1) west to (0,1): cannot go directly along row 1 against
    // the eastbound flow; A* detours through row 0 or row 2. With
    // `useful_orients` pruning, the trace shrinks (intermediate cells on
    // the detour drop their non-detour-aligned rotates), but the L1
    // row-vs-row tie still keeps the search exploring both rows in
    // parallel and bidi-corridor cells emit perpendicular intermediates
    // for the U-turn case.
    let drive = &uniform_drive(RobotType::Htm, false);
    let result = pf
        .find_path(c(w + 4), Orientation::West, c(w), drive, SimTime::ZERO)
        .expect("HTM detours around eastbound-only row");

    // Row-0 (north) and row-2 (south) detours are equal-cost mirrors;
    // current trace pins the row-0 chain.
    use Orientation::{East, North, South, West};
    assert_trace!(
        trace,
        [
            Expand { cell: c(9), orientation: West, g: ms(0), step: 0 },
            Rotate(c(9), West, North, ms(1_000), ms(0)),
            Rotate(c(9), West, South, ms(1_000), ms(0)),
            Expand { cell: c(9), orientation: South, g: ms(1_000), step: 0 },
            Rotate(c(9), South, East, ms(2_000), ms(0)),
            Move(c(9), c(14), ms(2_000), ms(0)),
            Expand { cell: c(9), orientation: North, g: ms(1_000), step: 0 },
            Move(c(9), c(4), ms(2_000), ms(0)),
            Expand { cell: c(9), orientation: East, g: ms(2_000), step: 0 },
            Expand { cell: c(4), orientation: North, g: ms(2_000), step: 0 },
            Rotate(c(4), North, East, ms(3_000), ms(0)),
            Rotate(c(4), North, West, ms(3_000), ms(0)),
            Expand { cell: c(14), orientation: South, g: ms(2_000), step: 0 },
            Rotate(c(14), South, East, ms(3_000), ms(0)),
            Rotate(c(14), South, West, ms(3_000), ms(0)),
            Expand { cell: c(14), orientation: West, g: ms(3_000), step: 0 },
            Rotate(c(14), West, North, ms(4_000), ms(0)),
            Move(c(14), c(13), ms(4_000), ms(0)),
            Expand { cell: c(13), orientation: West, g: ms(4_000), step: 0 },
            Rotate(c(13), West, North, ms(5_000), ms(0)),
            Rotate(c(13), West, South, ms(5_000), ms(0)),
            Move(c(13), c(12), ms(5_000), ms(0)),
            Expand { cell: c(12), orientation: West, g: ms(5_000), step: 0 },
            Rotate(c(12), West, North, ms(6_000), ms(0)),
            Rotate(c(12), West, South, ms(6_000), ms(0)),
            Move(c(12), c(11), ms(6_000), ms(0)),
            Expand { cell: c(11), orientation: West, g: ms(6_000), step: 0 },
            Rotate(c(11), West, North, ms(7_000), ms(0)),
            Rotate(c(11), West, South, ms(7_000), ms(0)),
            Move(c(11), c(10), ms(7_000), ms(0)),
            Expand { cell: c(10), orientation: West, g: ms(7_000), step: 0 },
            Rotate(c(10), West, North, ms(8_000), ms(0)),
            Rotate(c(10), West, South, ms(8_000), ms(0)),
            Expand { cell: c(14), orientation: East, g: ms(3_000), step: 0 },
            Expand { cell: c(4), orientation: West, g: ms(3_000), step: 0 },
            Rotate(c(4), West, South, ms(4_000), ms(0)),
            Move(c(4), c(3), ms(4_000), ms(0)),
            Expand { cell: c(3), orientation: West, g: ms(4_000), step: 0 },
            Rotate(c(3), West, North, ms(5_000), ms(0)),
            Rotate(c(3), West, South, ms(5_000), ms(0)),
            Move(c(3), c(2), ms(5_000), ms(0)),
            Expand { cell: c(2), orientation: West, g: ms(5_000), step: 0 },
            Rotate(c(2), West, North, ms(6_000), ms(0)),
            Rotate(c(2), West, South, ms(6_000), ms(0)),
            Move(c(2), c(1), ms(6_000), ms(0)),
            Expand { cell: c(1), orientation: West, g: ms(6_000), step: 0 },
            Rotate(c(1), West, North, ms(7_000), ms(0)),
            Rotate(c(1), West, South, ms(7_000), ms(0)),
            Move(c(1), c(0), ms(7_000), ms(0)),
            Expand { cell: c(0), orientation: West, g: ms(7_000), step: 0 },
            Rotate(c(0), West, North, ms(8_000), ms(0)),
            Rotate(c(0), West, South, ms(8_000), ms(0)),
            Expand { cell: c(4), orientation: East, g: ms(3_000), step: 0 },
            Expand { cell: c(0), orientation: South, g: ms(8_000), step: 0 },
            Rotate(c(0), South, East, ms(9_000), ms(0)),
            Move(c(0), c(5), ms(9_000), ms(0)),
            Expand { cell: c(5), orientation: South, g: ms(9_000), step: 0 },
        ]
    );
    assert_waypoints!(
        result,
        [
            path_start(c(9), West, t(0)),
            rotate(c(9), North, t(0), t(1_000)),
            mv(c(4), North, t(1_000), t(2_000)),
            rotate(c(4), West, t(2_000), t(3_000)),
            mv(c(3), West, t(3_000), t(4_000)),
            mv(c(2), West, t(4_000), t(5_000)),
            mv(c(1), West, t(5_000), t(6_000)),
            mv(c(0), West, t(6_000), t(7_000)),
            rotate(c(0), South, t(7_000), t(8_000)),
            mv(c(5), South, t(8_000), t(9_000)),
            path_end(c(5), South, t(9_000)),
        ]
    );
    assert_eq!(result.travel_time, ms(9_000));
}
