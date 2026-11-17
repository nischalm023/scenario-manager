//! Convert planned movement to observer distance and bot-plan occupancy facts.
//! Both projections follow the same physical edge and timing data as the path.

use desgraph::CellId;
use desmap::WarehouseMap;

use crate::model::SimDuration;
use crate::pathfinding::{Path, ReservationRequest, Waypoint, WaypointAction};

/// Total physical distance in mm: the sum of the traversed edges'
/// `distance_mm`. On uniform grids every edge is one cell, so this equals
/// the old Move-count × cell-size figure; it diverges (correctly) on
/// junction edges and non-grid dock edges, which carry their own physical
/// lengths. A Move with no graph edge (planner paths never produce one)
/// falls back to one cell-size.
///
/// Deliberately-uniform siblings (per-edge timing already flows through the
/// planner; these are estimates/animation only): `kinematic_walk_time`
/// (demand-side quotes) and `observer::nav_kinematic` position rasterization
/// both still charge `cell_spacing_mm` per Move.
pub(crate) fn move_distance(path: &Path, map: &WarehouseMap) -> u64 {
    let cell_size = map.cell_size_mm();
    let mut previous: Option<CellId> = None;
    let mut total = 0u64;
    for waypoint in &path.waypoints {
        if matches!(waypoint.action, WaypointAction::Move { .. }) {
            let edge_distance = previous.and_then(|from| map.graph.edge_between(from, waypoint.cell));
            debug_assert!(
                previous.is_none() || edge_distance.is_some(),
                "Move {:?} -> {:?} traverses no graph edge — path replayed over a stale graph?",
                previous,
                waypoint.cell,
            );
            total += edge_distance.map(|edge| edge.distance_mm).unwrap_or(cell_size);
        }
        previous = Some(waypoint.cell);
    }
    total
}

/// Compose ReservationRequests from a Waypoint chain for BotPlan storage.
///
/// One ReservationRequest per cell occupancy block. The chain shape is
/// `PathStart [Move|Rotate|Wait]* PathEnd`. The start cell occupies from
/// PathStart.time to first Move.end (or PathEnd.time if no Move). Each
/// subsequent cell occupies from Move-into-it.start to next Move-out.end.
///
/// Terminal: `dest_dwell = None` or `Some(MAX)` → final cell hold is
/// open-ended. `Some(d)` finite → final cell hold ends at
/// `PathEnd.time + d`. The dwell is a reservation property, applied here,
/// not encoded in the path waypoints.
pub(crate) fn commit_plan_requests(waypoints: &[Waypoint], dest_dwell: Option<SimDuration>) -> Vec<ReservationRequest> {
    use crate::model::SimDuration;
    use crate::pathfinding::{ReservationRequest, WaypointAction};
    if waypoints.is_empty() {
        return Vec::new();
    }
    let start_time = match &waypoints[0].action {
        WaypointAction::PathStart { time } => *time,
        _ => waypoints[0].action.start_time(),
    };
    let mut requests: Vec<ReservationRequest> = Vec::new();
    let mut cur_cell = waypoints[0].cell;
    let mut cur_start = start_time;

    for wp in waypoints.iter().skip(1) {
        match &wp.action {
            WaypointAction::Move { start, end, .. } => {
                // Close prior cell at this Move's end (= action-ready at dest;
                // straddle is captured by the next request opening at *start).
                requests.push(ReservationRequest::spanning(cur_cell, cur_start, Some(*end)));
                cur_cell = wp.cell;
                cur_start = *start;
            }
            WaypointAction::Rotate { .. } | WaypointAction::Wait { .. } => {
                // Same-cell action — composition continues.
            }
            WaypointAction::PathStart { .. } => {
                debug_assert!(false, "PathStart at non-zero index");
            }
            WaypointAction::PathEnd { time } => {
                // None and Some(MAX) → open-ended hold. Every other
                // Some(d) — including Some(SimDuration::ZERO) — is
                // finite, matching search admissibility in
                // expand.rs::try_accept_goal.
                let final_end = dest_dwell.filter(|d| *d != SimDuration::MAX).map(|d| time.saturating_add(d));
                requests.push(ReservationRequest::spanning(cur_cell, cur_start, final_end));
                return requests;
            }
        }
    }

    debug_assert!(false, "commit_plan_requests: waypoints must end with PathEnd");
    requests
}

#[cfg(test)]
mod move_distance_tests {
    use std::collections::BTreeSet;

    use desgraph::{Cell, CellId, CellKind, Edge, GridPos, WarehouseGraphBuilder, ZoneId};
    use desmap::{GridBounds, SystemType, WarehouseMap, WarehouseMapBuilder};

    use super::move_distance;
    use crate::model::{SimDuration, SimTime};
    use crate::pathfinding::{Orientation, Path, Waypoint, WaypointAction};

    fn map_with_edges(edges: &[(u32, u32, u64)]) -> WarehouseMap {
        let mut builder = WarehouseGraphBuilder::new();
        let cells: BTreeSet<u32> = edges.iter().flat_map(|&(a, b, _)| [a, b]).collect();
        for id in cells {
            builder.add_cell(Cell {
                id: CellId(id),
                kind: CellKind::Open,
                zone: ZoneId(0),
                sector: None,
                grid_pos: Some(GridPos { x: id, y: 0 }),
                world_pos: None,
                barcode: None,
                htm_rotation_forbidden: false,
                rack_orientation: None,
                station_id: None,
            });
        }
        for &(a, b, distance) in edges {
            builder.add_edge_bidi(CellId(a), CellId(b), Edge::uniform(distance));
        }
        WarehouseMapBuilder::new(SystemType::Relay)
            .grid_bounds(GridBounds { width: 8, depth: 1, cell_size_mm: 1000.0 })
            .graph(builder.build())
            .build()
    }

    fn walk(cells: &[u32]) -> Path {
        let t = SimTime::ZERO;
        let mut waypoints = vec![Waypoint {
            cell: CellId(cells[0]),
            orientation: Orientation::East,
            action: WaypointAction::PathStart { time: t },
        }];
        for &cell in &cells[1..] {
            waypoints.push(Waypoint {
                cell: CellId(cell),
                orientation: Orientation::East,
                action: WaypointAction::Move { start: t, end: t, entry_velocity_mm_s: 0.0, exit_velocity_mm_s: 0.0 },
            });
        }
        Path { waypoints, travel_time: SimDuration::ZERO }
    }

    #[test]
    fn uniform_edges_match_the_old_hop_count_formula() {
        let map = map_with_edges(&[(0, 1, 1000), (1, 2, 1000)]);
        assert_eq!(move_distance(&walk(&[0, 1, 2]), &map), 2 * map.cell_size_mm());
    }

    #[test]
    fn non_uniform_edges_contribute_their_own_length() {
        // A junction-style short edge: the sum reflects physical lengths,
        // not hop count x cell size.
        let map = map_with_edges(&[(0, 1, 1000), (1, 2, 718)]);
        assert_eq!(move_distance(&walk(&[0, 1, 2]), &map), 1718);
    }
}
