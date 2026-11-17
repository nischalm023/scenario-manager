//! Path: reconstruction, waypoint generation, direction computation.

use desgraph::types::{CellId, GridPos};
use desgraph::{Edge, WarehouseGraph};

use super::node::NodeActionKind;
use super::pathfinder::Inner;
use super::search::SearchEntry;
use super::state::SearchKey;

use super::super::{DriveModel, MotionModel, Orientation, Waypoint, WaypointAction};
use crate::{SimDuration, SimTime, Velocity};

pub(super) struct EdgeMotion {
    pub departure: Orientation,
    pub arrival: Orientation,
    pub distance_mm: u64,
    pub reverse: bool,
}

/// A curve changes facing during travel. Invalid/missing curve geometry makes
/// the edge unusable; it must never quietly become a straight chord.
pub(super) fn edge_motion(
    graph: &WarehouseGraph,
    from: CellId,
    to: CellId,
    edge: &Edge,
    drive: &dyn DriveModel,
) -> Option<EdgeMotion> {
    let reverse = drive.reverses_on(edge);
    let chassis = |heading: Orientation| if reverse { heading.opposite() } else { heading };
    let Some(arc) = edge.curve else {
        let direction = direction_between(graph, from, to);
        return Some(EdgeMotion {
            departure: chassis(direction),
            arrival: chassis(direction),
            distance_mm: edge.distance_mm,
            reverse,
        });
    };
    let from = graph.cell(from)?.world_pos?;
    let to = graph.cell(to)?.world_pos?;
    let distance = arc.length_mm(from, to);
    let heading = |[x, y]: [f64; 2]| {
        (x.is_finite() && y.is_finite() && x.hypot(y) > 0.0).then(|| Orientation::from_degrees(y.atan2(x).to_degrees()))
    };
    if !distance.is_finite() || distance <= 0.0 {
        return None;
    }
    Some(EdgeMotion {
        departure: chassis(heading(arc.start_tangent(from))?),
        arrival: chassis(heading(arc.end_tangent(to))?),
        distance_mm: distance.round().max(1.0) as u64,
        reverse,
    })
}

/// Determine the orientation required to travel from `from` to `to`.
pub(super) fn direction_between(graph: &WarehouseGraph, from: CellId, to: CellId) -> Orientation {
    let from_cell = graph.cell(from);
    let to_cell = graph.cell(to);

    if let (Some(fc), Some(tc)) = (from_cell, to_cell) {
        if let (Some(fg), Some(tg)) = (fc.grid_pos, tc.grid_pos) {
            return grid_direction(fg, tg);
        }
        if let (Some(fw), Some(tw)) = (fc.world_pos, tc.world_pos) {
            let dx = tw.x_mm - fw.x_mm;
            let dy = tw.y_mm - fw.y_mm;
            return if dx.abs() >= dy.abs() {
                if dx >= 0.0 { Orientation::East } else { Orientation::West }
            } else if dy >= 0.0 {
                Orientation::South
            } else {
                Orientation::North
            };
        }
    }

    Orientation::East
}

/// Direction from one grid position to an adjacent grid position.
/// Grid convention: +X = East, +Y = South (row 0 is top).
pub(super) fn grid_direction(from: GridPos, to: GridPos) -> Orientation {
    let dx = to.x as i64 - from.x as i64;
    let dy = to.y as i64 - from.y as i64;

    if dx > 0 {
        Orientation::East
    } else if dx < 0 {
        Orientation::West
    } else if dy > 0 {
        Orientation::South
    } else if dy < 0 {
        Orientation::North
    } else {
        Orientation::East
    }
}

/// Reconstruct an ordered list of SearchKeys via each entry's `parent`
/// pointer (start..goal). `lookup` resolves a key to its [`SearchEntry`] —
/// backed by the reservation search's arena+index or the spatial search's
/// node map.
fn reconstruct_key_chain(lookup: &impl Fn(SearchKey) -> Option<SearchEntry>, goal: SearchKey) -> Vec<SearchKey> {
    let mut chain = vec![goal];
    let mut current = goal;
    while let Some(prev) = lookup(current).and_then(|e| e.parent) {
        chain.push(prev);
        current = prev;
    }
    chain.reverse();
    chain
}

/// Build Waypoints from the came_from chain by reading NodeMeta for each
/// state. No g-delta arithmetic, no inference: each NodeMeta directly
/// names the action and its timestamps.
///
/// Per spec §5:
/// - Emit `PathStart` at index 0.
/// - For each subsequent state: if `action_start > prior_action_end`, emit
///   an explicit `Wait` waypoint covering the gap on the prior cell with
///   the prior orientation. Then emit the Move/Rotate/Wait.
/// - Explicit `Wait` NodeMeta (event-driven wait successor) emits a `Wait`
///   waypoint on the same cell and orientation. Adjacent gap-fill + explicit
///   waits at the same cell merge into a single Wait spanning the union.
/// - Terminal: always emit `PathEnd { time: last_action_end }`. The
///   terminal dwell (`dest_dwell`) is a reservation property and lives in
///   the commit plan, not in the path waypoints.
/// - Drop zero-width Move/Rotate/Wait.
pub(super) fn reconstruct_waypoints(
    lookup: impl Fn(SearchKey) -> Option<SearchEntry>,
    goal: SearchKey,
    cruise: Velocity,
    motion_model: MotionModel,
) -> Vec<Waypoint> {
    let key_chain = reconstruct_key_chain(&lookup, goal);
    if key_chain.is_empty() {
        return Vec::new();
    }

    let mut waypoints = Vec::with_capacity(key_chain.len() + 2);

    // Index 0: PathStart marker. NodeMeta.action_kind == Start.
    let start_key = key_chain[0];
    let start_meta = lookup(start_key)
        .map(|e| e.meta)
        .unwrap_or_else(|| panic!("reconstruct: missing SearchEntry for start state at cell {:?}", start_key.cell()));
    debug_assert_eq!(start_meta.action_kind, NodeActionKind::Start, "start state must have Start action_kind");
    waypoints.push(Waypoint {
        cell: start_key.cell(),
        orientation: start_key.orientation(),
        action: WaypointAction::PathStart { time: start_meta.action_end },
    });
    let mut prev_end = start_meta.action_end;
    let mut prev_cell = start_key.cell();
    let mut prev_orient = start_key.orientation();

    for i in 1..key_chain.len() {
        let curr = key_chain[i];
        let meta = lookup(curr)
            .map(|e| e.meta)
            .unwrap_or_else(|| panic!("reconstruct: missing SearchEntry for state at cell {:?}", curr.cell()));

        // Insert explicit Wait covering the gap, if any. The wait happens on
        // the prior cell with the prior orientation (the bot held station
        // there before the next action).
        if meta.action_start > prev_end {
            push_or_merge_wait(&mut waypoints, prev_cell, prev_orient, prev_end, meta.action_start);
        }

        match meta.action_kind {
            NodeActionKind::Start => {
                panic!("reconstruct: Start NodeMeta at non-zero index {} (cell {:?})", i, curr.cell());
            }
            NodeActionKind::Move => {
                if meta.action_end > meta.action_start {
                    // Velocity substitution is gated on motion mode, NOT on
                    // NodeMeta field values. In ConstantSpeed mode NodeMeta
                    // never carries velocities (always Velocity::ZERO), so we
                    // emit entry == exit == configured cruise speed. In
                    // HtmKinematic mode we emit the search's real trapezoidal
                    // profile endpoints verbatim — including legitimate (0, 0)
                    // for rest-to-rest peak profiles (e.g. goal-cell moves
                    // forced by `force_decel`, or the first move out of a
                    // Start/Rotate/Wait predecessor whose exit_velocity = 0).
                    // TODO: key on the drive's kind, not the run's mode.
                    let (entry_velocity_mm_s, exit_velocity_mm_s) = match motion_model {
                        MotionModel::ConstantSpeed => {
                            let v = cruise.mm_per_sec();
                            (v, v)
                        }
                        MotionModel::HtmKinematic => {
                            (meta.entry_velocity.mm_per_sec(), meta.exit_velocity.mm_per_sec())
                        }
                    };
                    debug_assert!(
                        entry_velocity_mm_s.is_finite() && entry_velocity_mm_s >= 0.0,
                        "non-finite or negative entry velocity: {entry_velocity_mm_s}"
                    );
                    debug_assert!(
                        exit_velocity_mm_s.is_finite() && exit_velocity_mm_s >= 0.0,
                        "non-finite or negative exit velocity: {exit_velocity_mm_s}"
                    );
                    waypoints.push(Waypoint {
                        cell: curr.cell(),
                        orientation: curr.orientation(),
                        action: WaypointAction::Move {
                            start: meta.action_start,
                            end: meta.action_end,
                            entry_velocity_mm_s,
                            exit_velocity_mm_s,
                        },
                    });
                }
            }
            NodeActionKind::Rotate => {
                if meta.action_end > meta.action_start {
                    waypoints.push(Waypoint {
                        cell: curr.cell(),
                        orientation: curr.orientation(),
                        action: WaypointAction::Rotate { start: meta.action_start, end: meta.action_end },
                    });
                }
            }
            NodeActionKind::Wait => {
                if meta.action_end > meta.action_start {
                    push_or_merge_wait(
                        &mut waypoints,
                        curr.cell(),
                        curr.orientation(),
                        meta.action_start,
                        meta.action_end,
                    );
                }
            }
        }

        prev_end = meta.action_end;
        prev_cell = curr.cell();
        prev_orient = curr.orientation();
    }

    // Terminal: always emit PathEnd at the last action's end time. Any
    // terminal dwell is consumed by the commit plan, not the path.
    waypoints.push(Waypoint {
        cell: prev_cell,
        orientation: prev_orient,
        action: WaypointAction::PathEnd { time: prev_end },
    });

    waypoints
}

/// Push a Wait waypoint, merging with the previous waypoint if it is also a
/// Wait on the same cell+orientation that ends exactly when this one starts.
/// Per spec §5: gap-fill Wait + explicit event-driven Wait at the same cell
/// must collapse into a single Wait spanning the union window.
fn push_or_merge_wait(
    waypoints: &mut Vec<Waypoint>,
    cell: CellId,
    orientation: Orientation,
    start: SimTime,
    end: SimTime,
) {
    if let Some(last) = waypoints.last_mut() {
        if last.cell == cell
            && last.orientation == orientation
            && let WaypointAction::Wait { start: prev_start, end: prev_end } = last.action
            && prev_end == start
        {
            last.action = WaypointAction::Wait { start: prev_start, end };
            return;
        }
    }
    waypoints.push(Waypoint { cell, orientation, action: WaypointAction::Wait { start, end } });
}

/// Price the spatial route with its execution drive. Its search may have used
/// a different cruise or turning rate; preserve the route, not those prices.
/// Each edge uses the same distance and integer rounding as the search.
pub(crate) fn constant_walk_time(
    waypoints: &[Waypoint],
    drive: &dyn DriveModel,
    graph: &WarehouseGraph,
) -> SimDuration {
    waypoints.windows(2).fold(SimDuration::ZERO, |total, pair| {
        let [previous, waypoint] = pair else { unreachable!() };
        let cost = match waypoint.action {
            WaypointAction::Move { .. } => {
                let edge =
                    graph.edge_between(previous.cell, waypoint.cell).expect("a spatial path follows graph edges");
                let motion = edge_motion(graph, previous.cell, waypoint.cell, edge, drive)
                    .expect("a spatial path has valid geometry");
                drive.kinematic_cost(Velocity::ZERO, Velocity::ZERO, motion.distance_mm).0
            }
            WaypointAction::Rotate { .. } => {
                let angle = previous.orientation.angle_millidegrees_to(waypoint.orientation);
                Inner::rotate_cost(drive, previous.orientation, waypoint.orientation)
                    .unwrap_or_else(|| drive.rotation_cost(angle))
            }
            WaypointAction::Wait { start, end } => end - start,
            WaypointAction::PathStart { .. } | WaypointAction::PathEnd { .. } => SimDuration::ZERO,
        };
        total.saturating_add(cost)
    })
}

/// Walk a previously-computed spatial path under the trapezoidal kinematic
/// profile and return the honest free-travel time. Used by demand-side quote
/// computation when the quote's drive is kinematic.
///
/// Move target velocity is `v_max` if the next non-trivial waypoint is also a
/// Move (cruise through), `0` otherwise (decelerate before Rotate / Wait /
/// PathEnd). Rotates and Waits force the bot back to `v=0`.
pub(crate) fn kinematic_walk_time(
    waypoints: &[Waypoint],
    drive: &dyn DriveModel,
    graph: &WarehouseGraph,
) -> SimDuration {
    let mut total = SimDuration::ZERO;
    let mut entry_v = Velocity::ZERO;

    for (i, wp) in waypoints.iter().enumerate() {
        match wp.action {
            WaypointAction::Move { .. } => {
                let from = waypoints[i - 1].cell;
                let edge = graph.edge_between(from, wp.cell).expect("a spatial path follows graph edges");
                let motion = edge_motion(graph, from, wp.cell, edge, drive).expect("a spatial path has valid geometry");
                let next_reverse = waypoints.get(i + 1).and_then(|next| {
                    matches!(next.action, WaypointAction::Move { .. }).then(|| {
                        graph
                            .edge_between(wp.cell, next.cell)
                            .and_then(|edge| edge_motion(graph, wp.cell, next.cell, edge, drive))
                            .is_some_and(|motion| motion.reverse)
                    })
                });
                let target_v =
                    if !motion.reverse && next_reverse == Some(false) { drive.max_speed() } else { Velocity::ZERO };
                debug_assert!(!motion.reverse || entry_v == Velocity::ZERO);
                let (cost, exit_v) = drive.kinematic_cost(entry_v, target_v, motion.distance_mm);
                total = total.saturating_add(cost);
                entry_v = exit_v;
            }
            WaypointAction::Rotate { start, end } | WaypointAction::Wait { start, end } => {
                total = total.saturating_add(end - start);
                entry_v = Velocity::ZERO;
            }
            WaypointAction::PathStart { .. } | WaypointAction::PathEnd { .. } => {}
        }
    }
    total
}
