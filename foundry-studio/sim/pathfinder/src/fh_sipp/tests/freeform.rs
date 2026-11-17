//! Free-form (off-lattice) nodes: nodes carrying a world position but no
//! grid position, joined by explicit edges of any length and angle. A
//! dimensional (v4) map lowers its non-lattice content to exactly this
//! shape, so these tests pin what the search does with it.
//!
//! Two properties matter and are pinned here: such a node is REACHABLE
//! (the cardinal-snapping in `direction_between` lets the rotate/move
//! expanders cross a diagonal edge, at its true cost), and the heuristic
//! MEASURES it (straight-line over the speed ceiling) rather than
//! collapsing to zero and degenerating the search to Dijkstra.

use super::prelude::*;

use desgraph::graph::Cell;
use desgraph::types::{GridPos, WorldPos, ZoneId};

/// A node with a world position and NO grid position — what lowering emits
/// for a v4 `link`, and what `graph_builder` emits for a v3 junction cell.
fn free_cell(id: u32, x_mm: f64, y_mm: f64) -> Cell {
    Cell {
        id: c(id),
        kind: CellKind::Junction,
        zone: ZoneId(0),
        sector: None,
        grid_pos: None,
        world_pos: Some(WorldPos { x_mm, y_mm }),
        barcode: None,
        htm_rotation_forbidden: false,
        rack_orientation: None,
        station_id: None,
    }
}

/// A lattice node that also carries its world position, as the compiler
/// always emits (`graph_builder::add_grid_cells`).
fn placed_cell(id: u32, x: u32, y: u32, x_mm: f64, y_mm: f64) -> Cell {
    Cell {
        id: c(id),
        kind: CellKind::Open,
        zone: ZoneId(0),
        sector: None,
        grid_pos: Some(GridPos { x, y }),
        world_pos: Some(WorldPos { x_mm, y_mm }),
        barcode: None,
        htm_rotation_forbidden: false,
        rack_orientation: None,
        station_id: None,
    }
}

/// Three lattice cells in a row at 1000mm pitch, plus two free-form nodes
/// forming a diagonal bypass off the east end:
///
/// ```text
///   c0 --- c1 --- c2 --- c3(free, +1000 east)
///                          \ diagonal 5099mm
///                           c4(free, 1000 east + 5000 south)
/// ```
fn bypass_graph() -> WarehouseGraph {
    let mut b = WarehouseGraphBuilder::new();
    for i in 0..3u32 {
        b.add_cell(placed_cell(i, i, 0, i as f64 * 1000.0 + 500.0, 500.0));
    }
    b.add_cell(free_cell(3, 3500.0, 500.0));
    b.add_cell(free_cell(4, 4500.0, 5500.0));
    for i in 0..2u32 {
        b.add_edge_bidi(c(i), c(i + 1), Edge::uniform(1000));
    }
    // Lattice -> free-form, axis aligned.
    b.add_edge_bidi(c(2), c(3), Edge::uniform(1000));
    // The diagonal: hypot(1000, 5000).ceil().
    b.add_edge_bidi(c(3), c(4), Edge::uniform(5100));
    b.build()
}

/// A diagonal edge between off-lattice nodes is traversable: the required
/// orientation snaps to the dominant axis, the rotate expander can reach
/// that cardinal, and the move is charged the edge's true length. Heading
/// is quantized to 90 degrees; cost is not.
#[test]
fn diagonal_free_form_edges_are_traversable() {
    let graph = bypass_graph();
    let pf = pathfinder(&graph);

    // The diagonal runs 1000mm east and 5000mm south, so south dominates.
    assert_eq!(direction_between(&graph, c(3), c(4)), Orientation::South);

    let drive = &uniform_drive(RobotType::Htm, false);
    let path = pf.find_path(c(0), Orientation::East, c(4), drive, SimTime::ZERO).expect("free-form goal is reachable");
    let visited: Vec<CellId> = path.waypoints.iter().map(|w| w.cell).collect();
    assert_eq!(visited.first(), Some(&c(0)));
    assert_eq!(visited.last(), Some(&c(4)));
    assert!(visited.contains(&c(3)), "the route passes through the free-form node");

    // Travel time is driven by real edge lengths (3 x 1000mm + 5100mm at
    // 1000mm/s) plus the one 90-degree turn onto the diagonal (1s), not by
    // any nominal cell size.
    assert_eq!(path.travel_time, ms(3_000 + 5_100 + 1_000));
}

/// The heuristic must MEASURE distance to an off-lattice node. Returning
/// zero there is admissible but useless: it turns A* into Dijkstra exactly
/// where the graph is sparsest.
#[test]
fn heuristic_measures_off_lattice_distance() {
    let graph = bypass_graph();
    let pf = pathfinder(&graph);

    // c(0) at (500, 500) to c(4) at (4500, 5500): hypot(4000, 5000) =
    // 6403.1mm, at the 1000mm/s test speed.
    let drive = &uniform_drive(RobotType::Htm, false);
    let h = pf.heuristic_for_test(c(0), c(4), drive);
    assert_eq!(h, ms(6_403), "straight-line distance over the speed ceiling");

    // Admissible: never above the true optimal cost of that route.
    let drive = &uniform_drive(RobotType::Htm, false);
    let optimal = pf.find_path(c(0), Orientation::East, c(4), drive, SimTime::ZERO).expect("reachable").travel_time;
    assert!(h <= optimal, "heuristic {h:?} must not exceed the optimal {optimal:?}");

    // Both endpoints on the lattice keeps the exact L1 form — this is why
    // no pinned trace on a grid-only map moves.
    let drive = &uniform_drive(RobotType::Htm, false);
    let grid_h = pf.heuristic_for_test(c(0), c(2), drive);
    assert_eq!(grid_h, ms(2_000), "two 1000mm grid steps");
}

/// A node with neither grid nor world position keeps the zero fallback:
/// there is nothing to measure, and zero is the only admissible answer.
#[test]
fn positionless_nodes_keep_the_zero_heuristic() {
    let mut b = WarehouseGraphBuilder::new();
    b.add_cell(placed_cell(0, 0, 0, 500.0, 500.0));
    b.add_cell(cell(1, CellKind::Open, 1, 0));
    b.add_edge_bidi(c(0), c(1), Edge::uniform(1000));
    // c(1) from `helpers::cell` has a grid position but no world position,
    // so the pair still takes the L1 arm.
    let graph = b.build();
    let pf = pathfinder(&graph);
    let drive = &uniform_drive(RobotType::Htm, false);
    assert_eq!(pf.heuristic_for_test(c(0), c(1), drive), ms(1_000));

    let mut b = WarehouseGraphBuilder::new();
    b.add_cell(placed_cell(0, 0, 0, 500.0, 500.0));
    b.add_cell(Cell {
        id: c(1),
        kind: CellKind::Junction,
        zone: ZoneId(0),
        sector: None,
        grid_pos: None,
        world_pos: None,
        barcode: None,
        htm_rotation_forbidden: false,
        rack_orientation: None,
        station_id: None,
    });
    b.add_edge_bidi(c(0), c(1), Edge::uniform(1000));
    let graph = b.build();
    let pf = pathfinder(&graph);
    let drive = &uniform_drive(RobotType::Htm, false);
    assert_eq!(pf.heuristic_for_test(c(0), c(1), drive), SimDuration::ZERO, "nothing to measure",);
}

/// Free-form edges must not pollute the per-axis grid minima that the
/// lattice heuristic charges: the diagonal here is longer than the grid
/// pitch, but a shorter one must not deflate either axis.
#[test]
fn free_form_edges_do_not_enter_the_axis_minima() {
    let mut b = WarehouseGraphBuilder::new();
    b.add_cell(placed_cell(0, 0, 0, 500.0, 500.0));
    b.add_cell(placed_cell(1, 1, 0, 1500.0, 500.0));
    b.add_cell(free_cell(2, 1600.0, 600.0));
    b.add_edge_bidi(c(0), c(1), Edge::uniform(1000));
    // A very short free-form edge, well under the grid pitch.
    b.add_edge_bidi(c(1), c(2), Edge::uniform(50));
    let graph = b.build();
    let pf = pathfinder(&graph);

    // The lattice pair still charges the 1000mm grid pitch, not the 50mm
    // free-form edge.
    let drive = &uniform_drive(RobotType::Htm, false);
    assert_eq!(pf.heuristic_for_test(c(0), c(1), drive), ms(1_000));
}
