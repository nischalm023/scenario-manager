//! `desvis route` — diagnose A* pathfinding between two cells.
//!
//! Usage:
//!   desvis route <map.json> <from> <to> [--robot htm|vtm|rtp] [--laden] [--kinematic]
//!
//! Runs A* and prints the path found (or "no path"). Either way, dumps:
//!   - All outbound edges from `from` with traversability flags
//!   - All inbound edges to `to` with traversability flags
//!   - If no path: a BFS reachability trace showing where traversal dies
//!
//! With `--kinematic`, also runs `find_path_reserved` with `MotionModel::HtmKinematic`
//! and an empty reservation table, then prints those waypoints (each Move shows its
//! own duration so accel/cruise/decel timing is visible).

use std::collections::{HashMap, HashSet, VecDeque};

use desgraph::{CellId, CellKind, DestGate, Edge, RobotType, WarehouseGraph};
use dessim::model::{RobotId, SimTime};
use dessim::pathfinding::{
    CollisionConfig, Constant, DriveModel, MotionModel, Pathfinder, ReservationTable, TableOracle, Trapezoidal,
    Velocity,
};
use dessim::robots::reference_htm;
use dessim::{FHSippPathfinder, Orientation, SimDuration};

const DEFAULT_SPEED_MM_S: u64 = 1500;

// Note: vtm_zone_gate is checked by the pathfinder using the destination's zone,
// not just the edge flag. We conservatively treat any vtm_zone_gate edge as blocked
// in the BFS (because we don't thread dest_zone through here).

pub(crate) fn run_route(map_path: &str, from: u32, to: u32, robot: RobotType, laden: bool, kinematic: bool) {
    let loaded = crate::world_cli::load(map_path);
    let map = loaded.world.map;

    // VTM keeps its configured speed; HTM/RTP measure with the reference M5F,
    // its loaded top speed and its kinematics, whatever robots the world has.
    let reference = reference_htm();
    let speed = match robot {
        RobotType::Vtm => (loaded.settings.vtm.speed * 1000.0) as u64,
        _ => reference.loaded.max_velocity_mm_s as u64,
    };
    let speed = if speed > 0 { speed } else { DEFAULT_SPEED_MM_S };
    let pf = FHSippPathfinder::new(&map, speed, SimDuration::seconds(1), HashSet::new());
    // The chosen robot's drives, constant for the spatial run and kinematic for the re-run: an
    // HTM's from the reference figures, a VTM or RTP at the speed the pathfinder is built with.
    let shared = Constant {
        speed: Velocity(speed as f64),
        turn_duration: SimDuration::seconds(1),
        reversal_duration: None,
        robot_type: robot,
        carrying_cargo: laden,
    };
    let (htm, htm_kinematic) = (Constant::htm(&reference, laden), Trapezoidal::htm(&reference, laden));
    let (drive, kinematic_drive): (&dyn DriveModel, &dyn DriveModel) = match robot {
        RobotType::Htm => (&htm, &htm_kinematic),
        RobotType::Vtm | RobotType::Rtp => (&shared, &shared),
    };

    let from_id = CellId(from);
    let to_id = CellId(to);

    // -- Cell summaries --
    let relay_cell_set: HashSet<u32> = map.relays.iter().map(|r| r.position.0).collect();
    let cell_summary = |id: CellId| {
        map.graph
            .cell(id)
            .map(|c| {
                let gp = c.grid_pos.map(|g| format!("({},{})", g.x, g.y)).unwrap_or_else(|| "?".to_string());
                let relay_tag = if c.kind == CellKind::Relay {
                    if relay_cell_set.contains(&id.0) { "  relay=yes" } else { "  relay=no" }
                } else {
                    ""
                };
                format!("cell {} {}  kind={:?}  zone={}{}", id.0, gp, c.kind, c.zone.0, relay_tag)
            })
            .unwrap_or_else(|| format!("cell {} (not found)", id.0))
    };

    println!("From: {}", cell_summary(from_id));
    println!("To:   {}", cell_summary(to_id));
    println!("Robot: {:?}  laden: {}", robot, laden);
    println!();

    // -- Outbound edges from `from` --
    println!("Outbound edges from {}:", from);
    for (tgt, edge) in map.graph.neighbors(from_id) {
        let tgt_gp = map
            .graph
            .cell(tgt)
            .and_then(|c| c.grid_pos)
            .map(|g| format!("({},{})", g.x, g.y))
            .unwrap_or_else(|| "?".to_string());
        let passable = is_passable(edge, robot, laden, to_id);
        println!(
            "  {} -> {} {}  htm={:?}  vtm={}  rtp={}  dest_only={:?}  vtm_zone_gate={:?}  [{}]",
            from,
            tgt.0,
            tgt_gp,
            edge.traversability.htm_traversal,
            edge.traversability.vtm_traversal,
            edge.traversability.rtp_traversal,
            edge.dest_only,
            edge.vtm_zone_gate,
            if passable { "PASSABLE" } else { "BLOCKED" },
        );
    }
    println!();

    // -- Inbound edges to `to` --
    println!("Inbound edges to {}:", to);
    // We have to scan all cells to find who points to `to`
    let mut found_any = false;
    let mut cell_ids: Vec<u32> = map.graph.cells().map(|c| c.id.0).collect();
    cell_ids.sort();
    for src_id in &cell_ids {
        let src = CellId(*src_id);
        for (tgt, edge) in map.graph.neighbors(src) {
            if tgt != to_id {
                continue;
            }
            found_any = true;
            let src_gp = map
                .graph
                .cell(src)
                .and_then(|c| c.grid_pos)
                .map(|g| format!("({},{})", g.x, g.y))
                .unwrap_or_else(|| "?".to_string());
            let passable = is_passable(edge, robot, laden, to_id);
            println!(
                "  {} {} -> {}  htm={:?}  vtm={}  rtp={}  dest_only={:?}  vtm_zone_gate={:?}  [{}]",
                src_id,
                src_gp,
                to,
                edge.traversability.htm_traversal,
                edge.traversability.vtm_traversal,
                edge.traversability.rtp_traversal,
                edge.dest_only,
                edge.vtm_zone_gate,
                if passable { "PASSABLE" } else { "BLOCKED" },
            );
        }
    }
    if !found_any {
        println!("  (none)");
    }
    println!();

    // -- Run A* --
    match pf.find_path(from_id, Orientation::East, to_id, drive, SimTime::ZERO) {
        Ok(result) => {
            println!("PATH FOUND  travel_time={}  {} waypoints", result.travel_time, result.waypoints.len());
            for (i, w) in result.waypoints.iter().enumerate() {
                let gp = map
                    .graph
                    .cell(w.cell)
                    .and_then(|c| c.grid_pos)
                    .map(|g| format!("({},{})", g.x, g.y))
                    .unwrap_or_else(|| "?".to_string());
                println!(
                    "  [{:>3}] cell={} {}  orient={:?}  action={:?}  dt={}",
                    i,
                    w.cell.0,
                    gp,
                    w.orientation,
                    w.action,
                    w.duration(),
                );
            }
        }
        Err(_) => {
            println!("NO PATH FOUND");
            println!();
            // BFS reachability from `from` to show where traversal dies
            bfs_reachability(&map.graph, from_id, to_id, robot, laden);
        }
    }

    // -- Optionally re-run with HTM kinematic motion model --
    if kinematic {
        let oracle = TableOracle::new(ReservationTable::default());
        let config = CollisionConfig {
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
            motion_model: MotionModel::HtmKinematic,
        };

        match pf.find_path_reserved(from_id, Orientation::East, to_id, kinematic_drive, &oracle, &config) {
            Ok((result, _stats)) => {
                println!();
                println!(
                    "KINEMATIC PATH FOUND  travel_time={}  {} waypoints",
                    result.travel_time,
                    result.waypoints.len()
                );
                for (i, w) in result.waypoints.iter().enumerate() {
                    let gp = map
                        .graph
                        .cell(w.cell)
                        .and_then(|c| c.grid_pos)
                        .map(|g| format!("({},{})", g.x, g.y))
                        .unwrap_or_else(|| "?".to_string());
                    println!(
                        "  [{:>3}] cell={} {}  orient={:?}  action={:?}  dt={}",
                        i,
                        w.cell.0,
                        gp,
                        w.orientation,
                        w.action,
                        w.duration(),
                    );
                }
            }
            Err(e) => {
                println!();
                println!("KINEMATIC: NO PATH FOUND ({:?})", e);
            }
        }
    }
}

fn is_passable(edge: &Edge, robot: RobotType, laden: bool, dest: CellId) -> bool {
    use desgraph::HtmTraversal;

    if let Some(gate) = edge.dest_only {
        if !gate.admits(dest) {
            return false;
        }
    }
    // vtm_zone_gate: conservatively block — pathfinder checks dest zone, we don't have it here
    if edge.vtm_zone_gate.is_some() {
        return false;
    }

    match robot {
        RobotType::Vtm => edge.traversability.vtm_traversal,
        RobotType::Htm => {
            if laden {
                edge.traversability.htm_traversal == HtmTraversal::Both
            } else {
                edge.traversability.htm_traversal != HtmTraversal::None
            }
        }
        RobotType::Rtp => edge.traversability.rtp_traversal,
    }
}

/// BFS from `start`, showing cells reached and the frontier where traversal stops.
fn bfs_reachability(graph: &WarehouseGraph, start: CellId, target: CellId, robot: RobotType, laden: bool) {
    println!("BFS reachability from {} (robot={:?} laden={}):", start.0, robot, laden);

    let mut visited: HashSet<u32> = HashSet::new();
    let mut queue: VecDeque<u32> = VecDeque::new();
    // Track why each cell was blocked (for frontier reporting)
    let mut blocked: HashMap<u32, Vec<String>> = HashMap::new();

    visited.insert(start.0);
    queue.push_back(start.0);

    while let Some(cur) = queue.pop_front() {
        let cur_id = CellId(cur);
        for (tgt, edge) in graph.neighbors(cur_id) {
            if visited.contains(&tgt.0) {
                continue;
            }
            if is_passable(edge, robot, laden, target) {
                visited.insert(tgt.0);
                queue.push_back(tgt.0);
            } else {
                // Record why this edge is blocked
                let reason = block_reason(edge, robot, laden, target);
                blocked.entry(tgt.0).or_default().push(format!("from {} [{}]", cur, reason));
            }
        }
    }

    // Sort for stable output
    let mut reachable: Vec<u32> = visited.iter().copied().collect();
    reachable.sort();
    println!("  Reachable cells ({}):", reachable.len());
    for id in &reachable {
        let gp = graph
            .cell(CellId(*id))
            .and_then(|c| c.grid_pos)
            .map(|g| format!("({},{})", g.x, g.y))
            .unwrap_or_else(|| "?".to_string());
        let marker = if CellId(*id) == target { " <-- TARGET" } else { "" };
        println!("    {} {}{}", id, gp, marker);
    }

    if !visited.contains(&target.0) {
        println!();
        println!("  Target {} NOT reachable.", target.0);
        println!("  Blocked approaches to target:");
        if let Some(reasons) = blocked.get(&target.0) {
            for r in reasons {
                println!("    {}", r);
            }
        } else {
            println!("    (no edges attempted — no neighbor of any reachable cell points to target)");
        }

        // Also show any frontier cells (reachable neighbors of unreachable cells)
        println!();
        println!("  Frontier (reachable cells with blocked outbound edges):");
        let mut frontier: Vec<(u32, u32, String)> = Vec::new(); // (src, tgt, reason)
        for &src in &reachable {
            for (tgt, edge) in graph.neighbors(CellId(src)) {
                if !visited.contains(&tgt.0) {
                    let reason = block_reason(edge, robot, laden, target);
                    frontier.push((src, tgt.0, reason));
                }
            }
        }
        frontier.sort();
        frontier.dedup();
        for (src, tgt, reason) in &frontier {
            let src_gp = graph
                .cell(CellId(*src))
                .and_then(|c| c.grid_pos)
                .map(|g| format!("({},{})", g.x, g.y))
                .unwrap_or_else(|| "?".to_string());
            let tgt_gp = graph
                .cell(CellId(*tgt))
                .and_then(|c| c.grid_pos)
                .map(|g| format!("({},{})", g.x, g.y))
                .unwrap_or_else(|| "?".to_string());
            println!("    {} {} -> {} {}  [{}]", src, src_gp, tgt, tgt_gp, reason);
        }
    }
}

fn block_reason(edge: &Edge, robot: RobotType, laden: bool, dest: CellId) -> String {
    if let Some(gate) = edge.dest_only {
        if !gate.admits(dest) {
            return match gate {
                DestGate::Cell(cell) => format!("dest_only={}", cell.0),
                DestGate::Handovers([a, b]) => format!("dest_only={} or {}", a.0, b.0),
            };
        }
    }
    if let Some(z) = edge.vtm_zone_gate {
        return format!("vtm_zone_gate={}", z.0);
    }
    match robot {
        RobotType::Vtm => "vtm_traversal=false".to_string(),
        RobotType::Htm => {
            if laden {
                format!("htm_traversal={:?} (need Both for laden)", edge.traversability.htm_traversal)
            } else {
                format!("htm_traversal=None (unladen blocked)")
            }
        }
        RobotType::Rtp => "rtp_traversal=false".to_string(),
    }
}
