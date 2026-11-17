//! Putaway station back-to-back arrival timing audit.
//!
//! Loads `minimal_putaway2.json` headless via the solver and prints the
//! first 20 HTM arrival/departure events at the putaway presentation cell.
//! Used to verify the cell-reservation handover semantics:
//!
//! - **Arrival** (`Move.end` waypoint at presentation cell): bot center
//!   reaches presentation cell.
//! - **Departure** (`Move.end` of next waypoint after presentation cell):
//!   bot center reaches the post-presentation cell — at this instant the
//!   bot has fully exited the presentation cell (trailing edge crosses
//!   the boundary).
//!
//! Once Presentation has its 12s dwell, consecutive arrivals at a saturated
//! station should be `PRESENTATION_DURATION + 2 × cell_traversal` apart
//! (one traversal for previous bot to clear, one for next bot to enter).

mod common;

use std::collections::{BTreeMap, BTreeSet};

use desgraph::{CellId, RobotType};
use dessim::events::{ForkOp, SimEvent, VtmEndpoint, VtmNavKind};
use dessim::pathfinding::WaypointAction;
use dessim::scenario::ScenarioConfig;
use native_modules as _;

#[derive(Clone, Debug)]
struct CellInterval {
    robot: u32,
    cell: u32,
    start_ms: u64,
    end_ms: u64,
}

#[derive(Clone, Debug)]
struct VtmPhase {
    robot: u32,
    start_ms: u64,
    end_ms: u64,
    label: String,
    cycle_kind_hint: Option<&'static str>,
}

#[derive(Clone, Debug)]
struct VtmCycle {
    robot: u32,
    start_ms: u64,
    end_ms: u64,
    kind: &'static str,
    gap_from_prev_ms: Option<u64>,
    labels: Vec<String>,
}

#[test]
fn putaway_first_10_arrivals_and_departures() {
    let (world, scenario, mut config) = common::load_world_spec("minimal_putaway2_spec.json");

    // --orders 0 equivalent: zero out any pick demand.
    config.max_orders = Some(0);
    let scenario = match scenario {
        ScenarioConfig::Generated(mut g) => {
            g.num_orders = 0;
            g.total_order_picks = 0;
            ScenarioConfig::Generated(g)
        }
        other => other,
    };

    let (events, _result, _metrics) = common::run_world_capture_all(&world, &scenario, config);

    // minimal_putaway2: 8-wide grid, presentation_cell=(4,8) → 8*8+4 = 68
    let presentation_cell = CellId(68);

    let station_cells = [65, 66, 67, 68];
    let intervals = htm_cell_intervals(&events, &station_cells);

    enum Action {
        Arrive,
        Depart,
    }

    let mut visits: Vec<(u64, u32, Action)> = intervals
        .iter()
        .filter(|i| i.cell == presentation_cell.0)
        .flat_map(|i| [(i.start_ms, i.robot, Action::Arrive), (i.end_ms, i.robot, Action::Depart)])
        .collect();

    visits.sort_by_key(|v| v.0);

    // FIXME(putaway-timing): replace this diagnostic printout with focused
    // assertions over exact arrival/departure cadence once the queue-depth
    // policy settles.
    eprintln!("\n=== Putaway presentation cell c({}) — first 20 HTM events ===", presentation_cell.0);
    eprintln!("{:>9}  {:<6}  {}", "time(ms)", "htm", "event");
    let mut prev_time: Option<u64> = None;
    for (t, htm, action) in visits.iter().take(20) {
        let label = match action {
            Action::Arrive => "ARRIVE  (center on presentation cell)",
            Action::Depart => "DEPART  (center on next cell after presentation)",
        };
        let delta = match prev_time {
            Some(prev) => format!("(+{}ms)", t.saturating_sub(prev)),
            None => String::new(),
        };
        eprintln!("{:>9}  r({:<3})  {}  {}", t, htm, label, delta);
        prev_time = Some(*t);
    }
    eprintln!("\nTotal events captured at c({}): {}", presentation_cell.0, visits.len());

    print_station_queue_timeline(&intervals, &station_cells);
    print_vtm_cycle_timeline(&events);
}

fn htm_cell_intervals(events: &[SimEvent], tracked_cells: &[u32]) -> Vec<CellInterval> {
    let tracked: BTreeSet<u32> = tracked_cells.iter().copied().collect();
    let mut changes_by_robot: BTreeMap<u32, Vec<(u64, usize, u32)>> = BTreeMap::new();
    let mut seq = 0usize;

    for ev in events {
        let SimEvent::Nav(nav) = ev else { continue };
        if !matches!(nav.robot_type, RobotType::Htm) {
            continue;
        }
        for wp in &nav.waypoints {
            let time = match wp.action {
                WaypointAction::PathStart { time } => Some(time),
                WaypointAction::Move { end, .. } => Some(end),
                _ => None,
            };
            if let Some(time) = time {
                changes_by_robot.entry(nav.robot_id.0).or_default().push((time.as_ms(), seq, wp.cell.0));
                seq += 1;
            }
        }
    }

    let mut intervals = Vec::new();
    for (robot, mut changes) in changes_by_robot {
        changes.sort_by_key(|&(time, seq, _)| (time, seq));
        let mut current_cell: Option<u32> = None;
        let mut current_since = 0u64;
        for (time, _, cell) in changes {
            match current_cell {
                None => {
                    current_cell = Some(cell);
                    current_since = time;
                }
                Some(prev_cell) if prev_cell == cell => {}
                Some(prev_cell) => {
                    if tracked.contains(&prev_cell) && time > current_since {
                        intervals.push(CellInterval { robot, cell: prev_cell, start_ms: current_since, end_ms: time });
                    }
                    current_cell = Some(cell);
                    current_since = time;
                }
            }
        }
    }
    intervals.sort_by_key(|i| (i.start_ms, i.end_ms, i.cell, i.robot));
    intervals
}

fn print_station_queue_timeline(intervals: &[CellInterval], station_cells: &[u32]) {
    #[derive(Clone, Copy)]
    enum ChangeKind {
        Enter,
        Exit,
    }

    let mut changes: BTreeMap<u64, Vec<(ChangeKind, u32, u32)>> = BTreeMap::new();
    for interval in intervals {
        changes.entry(interval.start_ms).or_default().push((ChangeKind::Enter, interval.cell, interval.robot));
        changes.entry(interval.end_ms).or_default().push((ChangeKind::Exit, interval.cell, interval.robot));
    }

    let mut occupancy: BTreeMap<u32, Vec<u32>> = station_cells.iter().map(|&cell| (cell, Vec::new())).collect();
    let mut prev_time: Option<u64> = None;
    let mut empty_spans = Vec::new();
    let mut underfull_spans = Vec::new();
    let mut overfull_spans = Vec::new();
    let mut depth_ms: BTreeMap<usize, u64> = BTreeMap::new();
    let mut printed = 0usize;

    eprintln!("\n=== Station queue occupancy, cells {:?} ===", station_cells);
    eprintln!("{:>9}  {:>5}  {}", "time(ms)", "depth", station_occupancy_label(&occupancy, station_cells));

    for (time, mut at_time) in changes {
        if let Some(prev) = prev_time {
            if time > prev {
                let depth = station_depth(&occupancy);
                *depth_ms.entry(depth).or_default() += time - prev;
                if depth == 0 {
                    empty_spans.push((prev, time));
                }
                if depth < 3 {
                    underfull_spans.push((prev, time, depth));
                }
                if depth > 3 {
                    overfull_spans.push((prev, time, depth));
                }
            }
        }

        at_time.sort_by_key(|(kind, cell, robot)| {
            let order = match kind {
                ChangeKind::Exit => 0,
                ChangeKind::Enter => 1,
            };
            (order, *cell, *robot)
        });
        for (kind, cell, robot) in at_time {
            let robots = occupancy.entry(cell).or_default();
            match kind {
                ChangeKind::Enter => {
                    if !robots.contains(&robot) {
                        robots.push(robot);
                        robots.sort_unstable();
                    }
                }
                ChangeKind::Exit => {
                    robots.retain(|&r| r != robot);
                }
            }
        }

        let depth = station_depth(&occupancy);
        if printed < 80 {
            eprintln!("{:>9}  {:>5}  {}", time, depth, station_occupancy_label(&occupancy, station_cells));
            printed += 1;
        }
        prev_time = Some(time);
    }

    eprintln!("\nEmpty station-corridor spans > 1000ms:");
    for (start, end) in empty_spans.iter().copied().filter(|(s, e)| e - s > 1000).take(20) {
        eprintln!("  {:>9}..{:>9}  ({}ms)", start, end, end - start);
    }

    eprintln!("\nUnderfull station-corridor spans > 1000ms (depth < 3):");
    for (start, end, depth) in underfull_spans.iter().copied().filter(|(s, e, _)| e - s > 1000).take(30) {
        eprintln!("  {:>9}..{:>9}  depth={} ({}ms)", start, end, depth, end - start);
    }

    eprintln!("\nOverfull station-corridor spans (depth > 3):");
    for (start, end, depth) in overfull_spans.into_iter().take(20) {
        eprintln!("  {:>9}..{:>9}  depth={} ({}ms)", start, end, depth, end - start);
    }

    eprintln!("\nDepth distribution over tracked station cells:");
    let total_ms: u64 = depth_ms.values().sum();
    for (depth, ms) in depth_ms {
        let pct = if total_ms > 0 { (ms as f64 / total_ms as f64) * 100.0 } else { 0.0 };
        eprintln!("  depth={} {:>9}ms ({:>5.1}%)", depth, ms, pct);
    }
}

fn station_depth(occupancy: &BTreeMap<u32, Vec<u32>>) -> usize {
    occupancy.values().map(Vec::len).sum()
}

fn station_occupancy_label(occupancy: &BTreeMap<u32, Vec<u32>>, station_cells: &[u32]) -> String {
    station_cells
        .iter()
        .map(|cell| {
            let robots = occupancy.get(cell).map(Vec::as_slice).unwrap_or(&[]);
            if robots.is_empty() {
                format!("c{}=.", cell)
            } else {
                let robots = robots.iter().map(|r| format!("r{}", r)).collect::<Vec<_>>().join("/");
                format!("c{}={}", cell, robots)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn print_vtm_cycle_timeline(events: &[SimEvent]) {
    let all_vtms: Vec<u32> = events
        .iter()
        .find_map(|ev| {
            let SimEvent::InitialPositions(init) = ev else { return None };
            Some(
                init.positions
                    .iter()
                    .filter(|(_, _, robot_type, _)| matches!(robot_type, RobotType::Vtm))
                    .map(|(robot_id, _, _, _)| robot_id.0)
                    .collect::<Vec<_>>(),
            )
        })
        .unwrap_or_default();

    let mut phases_by_robot: BTreeMap<u32, Vec<VtmPhase>> = BTreeMap::new();
    for ev in events {
        let SimEvent::VtmNav(vtm) = ev else { continue };
        let (label, hint) = vtm_phase_label(&vtm.kind);
        phases_by_robot.entry(vtm.robot_id.0).or_default().push(VtmPhase {
            robot: vtm.robot_id.0,
            start_ms: vtm.kind.start().as_ms(),
            end_ms: vtm.kind.end().as_ms(),
            label,
            cycle_kind_hint: hint,
        });
    }

    let mut cycles = Vec::new();
    for (_robot, mut phases) in phases_by_robot {
        phases.sort_by_key(|p| (p.start_ms, p.end_ms, p.label.clone()));
        let mut current: Option<VtmCycle> = None;
        let mut last_cycle_end: Option<u64> = None;

        for phase in phases {
            let starts_new = current.as_ref().map_or(true, |c| c.end_ms != phase.start_ms || c.labels.len() >= 4);
            if starts_new {
                if let Some(cycle) = current.take() {
                    last_cycle_end = Some(cycle.end_ms);
                    cycles.push(cycle);
                }
                let gap = last_cycle_end.map(|end| phase.start_ms.saturating_sub(end));
                current = Some(VtmCycle {
                    robot: phase.robot,
                    start_ms: phase.start_ms,
                    end_ms: phase.end_ms,
                    kind: phase.cycle_kind_hint.unwrap_or("?"),
                    gap_from_prev_ms: gap,
                    labels: vec![phase.label],
                });
            } else if let Some(cycle) = &mut current {
                cycle.end_ms = phase.end_ms;
                if cycle.kind == "?" {
                    cycle.kind = phase.cycle_kind_hint.unwrap_or("?");
                }
                cycle.labels.push(phase.label);
            }
        }
        if let Some(cycle) = current.take() {
            cycles.push(cycle);
        }
    }

    cycles.sort_by_key(|c| (c.start_ms, c.robot));

    let mut cycle_counts: BTreeMap<u32, usize> = all_vtms.iter().map(|&id| (id, 0)).collect();
    for cycle in &cycles {
        *cycle_counts.entry(cycle.robot).or_default() += 1;
    }
    eprintln!("\nVTM cycle counts:");
    for (robot, count) in &cycle_counts {
        eprintln!("  r({}) cycles={}", robot, count);
    }

    eprintln!("\n=== VTM cycles (first 60) ===");
    eprintln!("{:>9}  {:>9}  {:<4}  {:<3}  {:>8}  phases", "start", "end", "vtm", "dir", "gap");
    for cycle in cycles.iter().take(60) {
        let gap = cycle.gap_from_prev_ms.map(|g| g.to_string()).unwrap_or_else(|| "-".to_string());
        eprintln!(
            "{:>9}  {:>9}  r({:<2})  {:<3}  {:>8}  {}",
            cycle.start_ms,
            cycle.end_ms,
            cycle.robot,
            cycle.kind,
            gap,
            cycle.labels.join(" | ")
        );
    }

    eprintln!("\nVTM idle gaps > 1000ms:");
    for cycle in cycles.iter().filter(|c| c.gap_from_prev_ms.unwrap_or(0) > 1000).take(30) {
        eprintln!(
            "  r({}) gap={}ms before {} cycle at {}ms",
            cycle.robot,
            cycle.gap_from_prev_ms.unwrap(),
            cycle.kind,
            cycle.start_ms
        );
    }
}

fn vtm_phase_label(kind: &VtmNavKind) -> (String, Option<&'static str>) {
    match kind {
        VtmNavKind::Fork { op, at, start, end, .. } => {
            let endpoint = endpoint_label(at);
            let op = match op {
                ForkOp::Get => "get",
                ForkOp::Put => "put",
            };
            let hint = match (op, at) {
                ("get", VtmEndpoint::Slot { .. }) => Some("fwd"),
                ("get", VtmEndpoint::Relay { .. }) => Some("ret"),
                _ => None,
            };
            (format!("{}@{}:{}..{}", op, endpoint, start.as_ms(), end.as_ms()), hint)
        }
        VtmNavKind::Alignment { at, start, end, .. } => {
            (format!("align@{}:{}..{}", endpoint_label(at), start.as_ms(), end.as_ms()), None)
        }
        VtmNavKind::RailMove { from, to, start, end, .. } => {
            (format!("rail {}->{}:{}..{}", endpoint_label(from), endpoint_label(to), start.as_ms(), end.as_ms()), None)
        }
    }
}

fn endpoint_label(endpoint: &VtmEndpoint) -> &'static str {
    match endpoint {
        VtmEndpoint::Aisle { .. } => "aisle",
        VtmEndpoint::Relay { .. } => "relay",
        VtmEndpoint::Slot { .. } => "slot",
    }
}
