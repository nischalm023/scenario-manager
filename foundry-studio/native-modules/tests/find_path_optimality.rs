//! Differential optimality check on the two `fh_sipp` open-set implementations.
//!
//! `find_path` (spatial-only) uses `BucketQueue`; `reserve_path` uses
//! `BinaryHeap<AStarEntry>`. Given an empty oracle and
//! `MotionModel::ConstantSpeed` the two solve the same shortest-path problem
//! over the same graph with the same action costs, so their travel times must
//! agree exactly. Any gap isolates an open-set ordering fault in whichever
//! structure reports the higher cost.
//!
//! Regression target: `BucketQueue`'s bucket count was a fixed 2048, but a
//! monotone-cursor ring only orders correctly while no single action raises `f`
//! by `capacity` or more. cencora_v3's HTM in-place rotate is 2100ms, so rotate
//! successors folded onto buckets the cursor had already passed and `find_path`
//! returned paths up to 3.8s worse than optimal on ~4.7% of HTM queries —
//! always pessimistic, silently. The bound is now sized per map in
//! `open_bucket_capacity`. Bisecting the rotate time pins the boundary at
//! exactly 2048ms: 2.040s was clean, 2.050s was not.
//!
//! This runs on the real cencora_v3 fixture on purpose. The bug was invisible on
//! the small hand-built graphs the `fh_sipp` unit tests use, because their
//! 1000ms turn duration fits inside the historical ring.

mod common;

use desgraph::types::{CellId, CellKind};
use dessim::model::RobotId;
use dessim::model::reservation::ReservationTable;
use dessim::model::reservation::oracle::TableOracle;
use dessim::pathfinding::{CollisionConfig, MotionModel, Orientation, Pathfinder, holding_cells, htm_estimate_drives};
use dessim::{FHSippPathfinder, SimDuration, SimTime};

/// Sampled Open→Open pairs. 500 reliably surfaced ~20 failures pre-fix
/// (~4.7% hit rate) and costs well under a second in release.
const PAIRS: usize = 500;

#[test]
fn find_path_matches_reserve_path_cost_on_cencora_v3() {
    let (world, _scenario, config) = common::load_world_spec("cencora_v3_spec.json");
    let map = world.map;

    let holding = holding_cells(&map);
    let pf = FHSippPathfinder::new(&map, (config.vtm.speed * 1000.0) as u64, SimDuration::seconds(1), holding);
    // Both searches run at constant speed (`cfg` below), so the empty estimate drive serves both.
    let [empty, _] = htm_estimate_drives(&config);

    let open: Vec<CellId> =
        map.graph.cells().filter(|c| c.kind == CellKind::Open && c.world_pos.is_some()).map(|c| c.id).collect();
    assert!(open.len() > 1000, "fixture no longer has a large Open region: {}", open.len());

    let oracle = TableOracle::new(ReservationTable::default());
    let cfg = CollisionConfig {
        self_robot: RobotId(0),
        predecessor: None,
        destination: None,
        sim_start: SimTime::ZERO,
        dest_dwell: Some(SimDuration::seconds(1)),
        collision_depth: 4,
        window_size: 0,
        replan_interval: 0,
        max_wait: SimDuration::seconds(120),
        max_iterations: 5_000_000,
        motion_model: MotionModel::ConstantSpeed,
    };

    // Fixed-seed LCG so the sampled pair set is identical run to run.
    let n = open.len() as u64;
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) % n
    };

    let mut compared = 0usize;
    let mut worst: Vec<String> = Vec::new();

    for _ in 0..PAIRS {
        let (a, b) = (open[next() as usize], open[next() as usize]);
        if a == b {
            continue;
        }
        let spatial = pf.find_path(a, Orientation::East, b, empty.as_ref(), SimTime::ZERO);
        let reserved = pf.find_path_reserved(a, Orientation::East, b, empty.as_ref(), &oracle, &cfg);
        let (Ok(sp), Ok((rp, _))) = (spatial, reserved) else { continue };
        compared += 1;
        if sp.travel_time != rp.travel_time {
            let gp = |c: CellId| map.graph.cell(c).and_then(|x| x.grid_pos).map(|g| (g.x, g.y));
            worst.push(format!(
                "{:?}{:?} -> {:?}{:?}: find_path={}ms reserve_path={}ms (delta {:+}ms)",
                a,
                gp(a),
                b,
                gp(b),
                sp.travel_time.as_ms(),
                rp.travel_time.as_ms(),
                sp.travel_time.as_ms() as i64 - rp.travel_time.as_ms() as i64,
            ));
        }
    }

    assert!(compared > 300, "only {} pairs were mutually reachable; sampling is broken", compared);
    assert!(
        worst.is_empty(),
        "{} of {} pairs disagree between find_path (BucketQueue) and reserve_path (BinaryHeap).\n\
         Check `open_bucket_capacity` in pathfinding/fh_sipp/pathfinder.rs: the bucket ring must be \
         strictly wider than the largest single-action f-increase (longest edge + heuristic step, or \
         the largest turn duration).\n{}",
        worst.len(),
        compared,
        worst.iter().take(10).cloned().collect::<Vec<_>>().join("\n"),
    );
}
