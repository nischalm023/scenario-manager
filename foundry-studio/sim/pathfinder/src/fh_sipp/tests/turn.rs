//! Within the fh_sipp tests, `turn` pins the in-place rotate: where its
//! waypoint window sits, and what a reversal costs in both searches and the
//! travel estimates built on them, when a run states a 180° figure (one rotate
//! at that figure) and when it does not (two 90° turns).
//!
//! The window test is the regression guard for bug c891118: the rotate
//! waypoint window must be `[turn_start, turn_end)`, not the
//! post-rotate-shifted window the pre-pivot windowed allocator emitted.
//!
//! Pre-pivot bug: A* validated the rotate at `[turn_start, turn_start
//! + turn_duration)`, but `reconstruct_waypoints` encoded the Rotate
//! with `start = sim_start + curr_g`, where `curr_g` was the *end* of
//! the rotate. The recorded window was shifted forward by
//! `turn_duration`, so a later robot's reservation sitting inside the
//! shifted window but outside the actual rotate triggered a phantom
//! `commit_path` conflict.
//!
//! Under single-shot Phase 1, `Rotate { start, end }` is read straight
//! from `NodeMeta` at reconstruction, so the phantom-conflict path is
//! structurally impossible. The fixture below would have surfaced the
//! pre-pivot bug; under the new contract, `reserve_path` returns a
//! valid plan and commits cleanly.

use super::prelude::*;
use crate::Path;

#[test]
fn bug_c891118_turn_waypoint_phantom_conflict() {
    // 2-cell graph, cells named 1 and 2 to match the historical bug
    // report. 6000 mm/s + 1000 mm cells → 166 ms traversal; turn = 1 s.
    let mut b = WarehouseGraphBuilder::new();
    b.add_cell(cell(1, CellKind::Open, 0, 0));
    b.add_cell(cell(2, CellKind::Open, 1, 0));
    b.add_edge_bidi(c(1), c(2), Edge::uniform(1000));
    let graph = b.build();

    let trace = TraceCollector::new();
    let pf = pathfinder_speed(&graph, 6000).with_trace_hook(trace.hook());

    // r(1) holds cell 1 at [1500, 1700) — past r(2)'s real cell-1 exit
    // (rotate ends at 1000, move trailing edge at 1166), but inside the
    // *shifted* rotate window [1000, 2000) that the pre-pivot bug
    // would have emitted.
    let oracle = TableOracle::new(ReservationTable::default());
    oracle
        .commit_path(r(1), &CommitPlan::single(ReservationRequest::spanning(c(1), t(1_500), Some(t(1_700)))), None)
        .expect("r(1) single-cell hold should commit");

    let config = CollisionConfig {
        self_robot: r(2),
        predecessor: None,
        destination: None,
        sim_start: t(0),
        dest_dwell: None,
        collision_depth: 4,
        window_size: 0,
        replan_interval: 0,
        max_wait: SimDuration::seconds(30),
        max_iterations: 100,
        motion_model: MotionModel::ConstantSpeed,
    };
    let drive = &drive_speed_uniform(6000, RobotType::Htm, false);
    let result = pf
        .reserve_path_without_waiting(c(1), Orientation::North, c(2), drive, &oracle, &config)
        .expect("rotate at [0, 1000) does not conflict with r(1) [1500, 1700)");

    use Orientation::{East, North};
    // Reservation-aware search prunes rotations to goal-useful
    // orientations only (East), so North→West never appears.
    assert_trace!(
        trace,
        [
            Expand { cell: c(1), orientation: North, g: ms(0), step: 0 },
            Rotate(c(1), North, East, ms(1_000), ms(0)),
            Expand { cell: c(1), orientation: East, g: ms(1_000), step: 0 },
            EntryIntervals { cell: c(2), intervals: vec![(t(0), None)] },
            Move(c(1), c(2), ms(1_166), ms(0)),
            Expand { cell: c(2), orientation: East, g: ms(1_166), step: 0 },
        ]
    );
    // The regression assertion: the Rotate waypoint window is
    // [0, 1000), not the pre-pivot-shifted [1000, 2000). With
    // `dest_dwell = None`, the path closes with a `PathEnd` marker
    // rather than a trailing `Wait`.
    assert_waypoints!(
        result.path,
        [
            path_start(c(1), North, t(0)),
            rotate(c(1), East, t(0), t(1_000)),
            mv_v(c(2), East, t(1_000), t(1_166), 6000.0),
            path_end(c(2), East, t(1_166)),
        ]
    );
}

/// The start and end of every rotate in a plan, plus the plan's total time.
fn rotates_and_total(path: &Path) -> (Vec<(SimTime, SimTime)>, SimDuration) {
    let rotates = path
        .waypoints
        .iter()
        .filter_map(|w| match w.action {
            WaypointAction::Rotate { start, end } => Some((start, end)),
            _ => None,
        })
        .collect();
    (rotates, path.travel_time)
}

/// Reserve the reversal an HTM must make to reach the other end of a two-cell
/// corridor it is facing away from.
fn reversal_plan(htm: HtmKinematics, motion_model: MotionModel) -> Path {
    let graph = corridor(2);
    let pf = pathfinder_kinematics(&graph);
    let oracle = TableOracle::new(ReservationTable::default());
    let config = CollisionConfig {
        self_robot: r(1),
        predecessor: None,
        destination: None,
        sim_start: t(0),
        dest_dwell: None,
        collision_depth: 4,
        window_size: 0,
        replan_interval: 0,
        max_wait: SimDuration::seconds(30),
        max_iterations: 100,
        motion_model,
    };
    let drive: &dyn DriveModel = match motion_model {
        MotionModel::ConstantSpeed => &Constant::htm(&htm, false),
        MotionModel::HtmKinematic => &Trapezoidal::htm(&htm, false),
    };
    pf.reserve_path_without_waiting(c(1), Orientation::East, c(0), drive, &oracle, &config)
        .expect("an empty corridor admits the reversal")
        .path
}

/// The same reversal as the spatial search plans it, which is the search
/// behind every travel estimate and a Ghost run's moves.
fn spatial_reversal_plan(htm: HtmKinematics) -> Path {
    let pf = pathfinder_kinematics(&corridor(2));
    let drive = &Constant::htm(&htm, false);
    pf.find_path(c(1), Orientation::East, c(0), drive, t(0)).expect("the corridor is connected")
}

/// A stated 180° figure (docs/walmart-facts.md section 6b gives one) is worth
/// stating only if the HTM then reverses in one rotate that takes that long;
/// with no figure stated, the same reversal takes two 90° turns.
#[test]
fn a_stated_180_figure_makes_a_reversal_one_rotate_action() {
    let quarter_turns_only = HtmKinematics::uniform(1000, ms(1_000));
    let (rotates, travel) = rotates_and_total(&reversal_plan(quarter_turns_only, MotionModel::ConstantSpeed));
    assert_eq!(rotates, vec![(t(0), t(1_000)), (t(1_000), t(2_000))]);
    assert_eq!(travel, ms(3_000), "two 1 s turns then the 1 s crossing");

    let with_180 = HtmKinematics { empty_reversal: Some(ms(1_400)), ..quarter_turns_only };
    let (rotates, travel) = rotates_and_total(&reversal_plan(with_180, MotionModel::ConstantSpeed));
    assert_eq!(rotates, vec![(t(0), t(1_400))], "one rotate, lasting the stated figure");
    assert_eq!(travel, ms(2_400));
}

/// The spatial search plans the reversal the same way.
#[test]
fn the_spatial_search_makes_a_stated_reversal_one_rotate_action_too() {
    let quarter_turns_only = HtmKinematics::uniform(1000, ms(1_000));
    let (rotates, travel) = rotates_and_total(&spatial_reversal_plan(quarter_turns_only));
    assert_eq!(rotates, vec![(t(0), t(1_000)), (t(1_000), t(2_000))]);
    assert_eq!(travel, ms(3_000), "two 1 s turns then the 1 s crossing");

    let with_180 = HtmKinematics { empty_reversal: Some(ms(1_400)), ..quarter_turns_only };
    let (rotates, travel) = rotates_and_total(&spatial_reversal_plan(with_180));
    assert_eq!(rotates, vec![(t(0), t(1_400))], "one rotate, lasting the stated figure");
    assert_eq!(travel, ms(2_400));
}

/// HTM selection and the departure times set from it read travel estimates
/// off dessim's path cache — a spatial `find_path` priced by
/// `kinematic_travel_time` — so the estimate for a route that opens with a
/// stated reversal has to match the reserved plan's time, in either motion model.
#[test]
fn a_travel_estimate_prices_a_stated_reversal_as_the_reserved_plan_does() {
    let htm = HtmKinematics { empty_reversal: Some(ms(1_400)), ..HtmKinematics::uniform(1000, ms(1_000)) };
    for motion_model in [MotionModel::ConstantSpeed, MotionModel::HtmKinematic] {
        let pf = pathfinder_kinematics(&corridor(2));
        let drive = &Constant::htm(&htm, false);
        let path =
            pf.find_path(c(1), Orientation::East, c(0), drive, SimTime::ZERO).expect("the corridor is connected");
        let drive: &dyn DriveModel = match motion_model {
            MotionModel::ConstantSpeed => &Constant::htm(&htm, false),
            MotionModel::HtmKinematic => &Trapezoidal::htm(&htm, false),
        };
        let estimate = pf.kinematic_travel_time(&path, drive);
        assert_eq!(estimate, reversal_plan(htm, motion_model).travel_time, "{motion_model:?}");
    }
}
