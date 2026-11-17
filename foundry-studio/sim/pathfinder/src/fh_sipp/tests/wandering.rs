//! Wandering-detour regression: under single-shot SIPP every Move successor
//! validates against the oracle, so the search cannot manufacture an
//! equal-cost dip via the bidi side cell. The pre-pivot windowed allocator
//! priced the wander's tail beyond `collision_depth` against the spatial
//! graph only, producing a g-tie decided by heap order rather than by the
//! cost model. The bug class is structurally absent now; this test pins
//! the direct wait-and-go path against any future regression.

use super::prelude::*;

/// Topology with a one-way main corridor and a single bidi dip:
///
/// ```text
///   c0(G) ←── c1 ←── c2(S)
///             ↕
///             c3
/// ```
///
/// G = c0 (charging, only inflow from c1). S = c2 (one-way to c1). The
/// `c1↔c3` bidi edge is what made any wandering pattern reachable at all.
fn dip_graph() -> WarehouseGraph {
    let mut b = WarehouseGraphBuilder::new();
    b.add_cell(cell(0, CellKind::Charging, 0, 0));
    b.add_cell(cell(1, CellKind::Open, 1, 0));
    b.add_cell(cell(2, CellKind::Open, 2, 0));
    b.add_cell(cell(3, CellKind::Open, 1, 1));
    b.add_edge(c(1), c(0), Edge::uniform(1000));
    b.add_edge(c(2), c(1), Edge::uniform(1000));
    b.add_edge_bidi(c(1), c(3), Edge::uniform(1000));
    b.build()
}

#[test]
fn waits_at_predecessor_when_goal_busy_no_wander() {
    let graph = dip_graph();
    let (pf, trace) = pathfinder_traced_holding(&graph, [c(0)].into_iter().collect());
    let oracle = TableOracle::new(ReservationTable::default());

    // r1 occupies the goal cell c0 for [0, 7000). r2 plans c2 → c0 with
    // open-ended dwell; the only feasible plan waits at c1 until r1 clears,
    // then steps into c0 at t=8000.
    oracle
        .commit_path(r(1), &CommitPlan::single(ReservationRequest::spanning(c(0), t(0), Some(t(7_000)))), None)
        .expect("r1 single-cell hold should commit");

    let config = CollisionConfig {
        self_robot: r(2),
        predecessor: None,
        destination: None,
        sim_start: t(0),
        dest_dwell: Some(MAX),
        collision_depth: 3,
        window_size: 0,
        replan_interval: 0,
        max_wait: SimDuration::seconds(30),
        max_iterations: 500,
        motion_model: MotionModel::ConstantSpeed,
    };
    let drive = &uniform_drive(RobotType::Htm, false);
    let reserved = pf
        .reserve_path_without_waiting(c(2), Orientation::West, c(0), drive, &oracle, &config)
        .expect("direct wait-and-go path");

    use Orientation::{East, North, South, West};
    // Search-decision shape: c2 expands and emits Move-into-c1; c1 (West)
    // expands and emits the delayed Move-into-c0 at g=8000 (wait 6s for
    // r1 to clear). The c1 South rotation kicks off the spatial dip
    // through c3 (steps 7-25); the dip's c1→c0 push is silently filtered
    // by SIPP dedup (the direct route's `(c0, West, (7000, None))` at
    // g=8000 already lives in the open set with the same priority, so
    // the wander cannot price below it). Step 27 pops the goal from the
    // direct route. The dip cell c3 appears in the trace but never in
    // the returned path — the regression guard.
    assert_trace!(
        trace,
        [
            Expand { cell: c(2), orientation: West, g: ms(0), step: 0 },
            EntryIntervals { cell: c(1), intervals: [(t(0), None)].into() },
            Move(c(2), c(1), ms(1_000), ms(0)),
            Expand { cell: c(1), orientation: West, g: ms(1_000), step: 0 },
            Rotate(c(1), West, South, ms(2_000), ms(0)),
            EntryIntervals { cell: c(0), intervals: [(t(7_000), None)].into() },
            Move(c(1), c(0), ms(8_000), ms(6_000)),
            Expand { cell: c(1), orientation: South, g: ms(2_000), step: 0 },
            EntryIntervals { cell: c(3), intervals: [(t(0), None)].into() },
            Move(c(1), c(3), ms(3_000), ms(0)),
            Expand { cell: c(3), orientation: South, g: ms(3_000), step: 0 },
            Rotate(c(3), South, East, ms(4_000), ms(0)),
            Rotate(c(3), South, West, ms(4_000), ms(0)),
            Expand { cell: c(3), orientation: East, g: ms(4_000), step: 0 },
            Rotate(c(3), East, North, ms(5_000), ms(0)),
            Expand { cell: c(3), orientation: West, g: ms(4_000), step: 0 },
            Expand { cell: c(3), orientation: North, g: ms(5_000), step: 0 },
            EntryIntervals { cell: c(1), intervals: [(t(0), None)].into() },
            Move(c(3), c(1), ms(6_000), ms(0)),
            Expand { cell: c(1), orientation: North, g: ms(6_000), step: 0 },
            Rotate(c(1), North, East, ms(7_000), ms(0)),
            Expand { cell: c(1), orientation: West, g: ms(7_000), step: 0 },
            EntryIntervals { cell: c(0), intervals: [(t(7_000), None)].into() },
            Expand { cell: c(1), orientation: South, g: ms(7_000), step: 0 },
            EntryIntervals { cell: c(3), intervals: [(t(0), None)].into() },
            Expand { cell: c(1), orientation: East, g: ms(7_000), step: 0 },
            Expand { cell: c(1), orientation: North, g: ms(7_000), step: 0 },
            Expand { cell: c(0), orientation: West, g: ms(8_000), step: 0 },
        ]
    );
    // Direct wait-and-go: step into c1, hold there until r1 clears c0,
    // step into c0 at t=8000. The dip cell c3 is absent — exactly the
    // regression guard.
    assert_waypoints!(
        reserved.path,
        [
            path_start(c(2), West, t(0)),
            mv(c(1), West, t(0), t(1_000)),
            wait(c(1), West, t(1_000), t(7_000)),
            mv(c(0), West, t(7_000), t(8_000)),
            path_end(c(0), West, t(8_000)),
        ]
    );
    assert_eq!(reserved.path.travel_time, ms(8_000));
}
