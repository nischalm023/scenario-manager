//! Relay destinations and temporal bookings for idle HTMs. Physical collision
//! reservations remain owned by the navigation oracle; this registry prescreens
//! competing idle assignments. Chargers participate only in startup placement.

use std::cmp::Ordering;
use std::sync::{Arc, Mutex};

use desgraph::types::WorldPos;
use desgraph::{CellId, CellKind, HtmTraversal};
use rustc_hash::FxHashSet;

use crate::config::IdleRelaySelection;
use crate::utils::rng::Rng;
use desmap::WarehouseMap;

use crate::model::{RobotId, SimTime};
use crate::pathfinding::estimate::{cell_point_mm, manhattan_mm};

#[derive(Clone, Debug)]
struct ParkingCandidate {
    cell: CellId,
    /// Millimetre centre, by `estimate::cell_point_mm`'s rule (world position,
    /// else grid index scaled by the map's nominal cell size).
    point: WorldPos,
}

#[derive(Clone, Debug)]
struct ParkingReservation {
    cell: CellId,
    robot: RobotId,
    start: SimTime,
    end: SimTime, // SimTime::EOT = until retasked
}

#[derive(Clone)]
pub struct ParkingRegistry {
    inner: Arc<ParkingInner>,
}

struct ParkingInner {
    candidates: Vec<ParkingCandidate>,
    candidate_set: FxHashSet<CellId>,
    selection: IdleRelaySelection,
    seed: u64,
    reservations: Mutex<Vec<ParkingReservation>>,
}

impl ParkingRegistry {
    pub fn from_map(map: &WarehouseMap) -> Self {
        Self::with_policy(map, IdleRelaySelection::Nearest, 0)
    }

    pub fn with_policy(map: &WarehouseMap, selection: IdleRelaySelection, seed: u64) -> Self {
        let usable = usable_cells(map);
        let mut cells: Vec<_> = map
            .relays
            .iter()
            .map(|relay| relay.htm_access)
            .filter(|cell| usable.contains(cell) && map.graph.cell(*cell).is_some_and(|c| c.kind != CellKind::Charging))
            .collect();
        cells.sort_unstable();
        cells.dedup();
        let candidates: Vec<_> = cells
            .into_iter()
            .filter_map(|cell| cell_point_mm(map, cell).map(|point| ParkingCandidate { cell, point }))
            .collect();
        let candidate_set = candidates.iter().map(|c| c.cell).collect();
        Self {
            inner: Arc::new(ParkingInner {
                candidates,
                candidate_set,
                selection,
                seed,
                reservations: Mutex::new(Vec::new()),
            }),
        }
    }

    pub fn candidate_cells(&self) -> impl Iterator<Item = CellId> + '_ {
        self.inner.candidates.iter().map(|candidate| candidate.cell)
    }

    pub fn selection(&self) -> IdleRelaySelection {
        self.inner.selection
    }

    /// Find nearest idle relay cell free at `at_ms` for `robot`.
    /// A cell is free if no OTHER robot's reservation overlaps `at_ms`.
    /// Returns None if no idle relay cell is available — caller must handle.
    /// Never returns a non-idle relay cell (invariant I1).
    pub fn find(
        &self,
        robot_pos: CellId,
        robot_point: Option<WorldPos>,
        at: SimTime,
        robot: RobotId,
    ) -> Option<CellId> {
        let reservations = self.inner.reservations.lock().unwrap();

        // For MAX-dwell parking allocation a cell is unavailable iff any
        // OTHER robot's reservation extends beyond `at` — two open-ended
        // holds on the same cell can't coexist regardless of relative
        // start times. The narrower "currently occupied" check
        // (`r.start <= at && r.end > at`) misses future open-ended
        // reservations and lets the registry hand out a cell whose
        // pathfinder commit will then fail with `DestinationExceedsMaxWait`.
        let is_occupied =
            |cell: CellId| -> bool { reservations.iter().any(|r| r.cell == cell && r.robot != robot && r.end > at) };

        if self.inner.candidate_set.contains(&robot_pos) && !is_occupied(robot_pos) {
            return Some(robot_pos);
        }

        let pos = robot_point?;

        let mut best: Option<(CellId, f64)> = None;
        for c in &self.inner.candidates {
            if is_occupied(c.cell) {
                continue;
            }
            let dist = manhattan_mm(pos, c.point);
            if best.is_none() || dist < best.unwrap().1 {
                best = Some((c.cell, dist));
            }
        }

        best.map(|(cell, _)| cell)
    }

    /// Free idle relays ordered by locality or a reproducible random permutation.
    /// Excludes cells occupied by other robots at `at_ms`.
    /// Used for prescreen — actual selection confirmed by A* via commit_segments.
    /// No cap: the A* loop in commit_segments exits on first success.
    pub fn find_candidates(
        &self,
        robot_pos: CellId,
        robot_point: Option<WorldPos>,
        at: SimTime,
        robot: RobotId,
    ) -> Vec<CellId> {
        let mut reservations = self.inner.reservations.lock().unwrap();

        // GC: prune stale reservations (end in the past). Keeps the
        // Vec bounded to O(active bots) instead of O(total tasks).
        reservations.retain(|r| r.end > at);

        // Same MAX-dwell semantics as `find` — any other-robot reservation
        // extending past `at` blocks the cell.
        let is_occupied =
            |cell: CellId| -> bool { reservations.iter().any(|r| r.cell == cell && r.robot != robot && r.end > at) };

        let Some(pos) = robot_point else { return vec![] };

        let mut ranked: Vec<(CellId, f64)> = self
            .inner
            .candidates
            .iter()
            .filter(|c| !is_occupied(c.cell))
            .map(|c| (c.cell, manhattan_mm(pos, c.point)))
            .collect();

        ranked.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal).then(a.0.cmp(&b.0)));
        if self.selection() == IdleRelaySelection::Random {
            // Stable for the same proposal, independent of other tasks' attempts.
            let mut rng = Rng::new(self.inner.seed ^ u64::from(robot.0).rotate_left(23) ^ at.as_ms().rotate_left(41));
            for index in (1..ranked.len()).rev() {
                ranked.swap(index, rng.index(index + 1));
            }
        }

        // If robot is already at an idle relay cell, put it first.
        if self.inner.candidate_set.contains(&robot_pos) && !is_occupied(robot_pos) {
            ranked.retain(|&(c, _)| c != robot_pos);
            ranked.insert(0, (robot_pos, 0.0));
        }

        ranked.into_iter().map(|(c, _)| c).collect()
    }

    /// Reserve an idle relay cell from `arrival` to EOT (end of simulation).
    /// Evicts any prior entry for this robot — one bot, one idle relay cell.
    pub fn reserve(&self, cell: CellId, robot: RobotId, arrival: SimTime) {
        let mut res = self.inner.reservations.lock().unwrap();
        res.retain(|r| r.robot != robot);
        res.push(ParkingReservation { cell, robot, start: arrival, end: SimTime::EOT });
    }

    /// Cancel: truncate end to `at`. Removes if `at <= start`.
    pub fn cancel(&self, cell: CellId, robot: RobotId, at: SimTime) {
        let mut res = self.inner.reservations.lock().unwrap();
        res.retain_mut(|r| {
            if r.cell == cell && r.robot == robot {
                if at <= r.start {
                    return false;
                }
                r.end = at;
            }
            true
        });
    }

    pub fn candidate_count(&self) -> usize {
        self.inner.candidates.len()
    }

    pub fn is_candidate(&self, cell: CellId) -> bool {
        self.inner.candidate_set.contains(&cell)
    }
}

/// Positioned cells with an HTM approach and departure. The pathfinder still
/// checks the complete route and live reservations when assigning a destination.
pub(crate) fn usable_cells(map: &WarehouseMap) -> FxHashSet<CellId> {
    let mut incoming = FxHashSet::default();
    let mut outgoing = FxHashSet::default();
    for (from, to, edge) in map.graph.edges() {
        if from == to || edge.traversability.htm_traversal == HtmTraversal::None {
            continue;
        }
        outgoing.insert(from);
        if edge.dest_only.is_none_or(|gate| gate.admits(to)) {
            incoming.insert(to);
        }
    }
    incoming
        .into_iter()
        .filter(|cell| {
            outgoing.contains(cell)
                && cell_point_mm(map, *cell).is_some_and(|point| point.x_mm.is_finite() && point.y_mm.is_finite())
        })
        .collect()
}

impl Default for ParkingRegistry {
    fn default() -> Self {
        Self {
            inner: Arc::new(ParkingInner {
                candidates: Vec::new(),
                candidate_set: FxHashSet::default(),
                selection: IdleRelaySelection::Nearest,
                seed: 0,
                reservations: Mutex::new(Vec::new()),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use desgraph::types::SystemType;
    use desgraph::{BlockId, Cell, Edge, GridPos, RelayId, WarehouseGraphBuilder, ZoneId};
    use desmap::{BlockSide, RelayPosition, VtmPosition, WarehouseMapBuilder};

    /// 1,000mm pitch: the old grid coordinates, now stated in millimetres, so
    /// every distance the assertions rely on is numerically unchanged.
    fn cell(id: u32, kind: CellKind, x: u32, y: u32) -> Cell {
        Cell {
            id: CellId(id),
            kind,
            zone: ZoneId(0),
            sector: None,
            grid_pos: Some(GridPos { x, y }),
            world_pos: Some(WorldPos { x_mm: x as f64 * 1000.0, y_mm: y as f64 * 1000.0 }),
            barcode: None,
            htm_rotation_forbidden: false,
            rack_orientation: None,
            station_id: None,
        }
    }

    fn build_simple_map() -> WarehouseMap {
        let mut b = WarehouseGraphBuilder::with_capacity(9, 24);
        b.add_cell(cell(0, CellKind::Open, 0, 0));
        b.add_cell(cell(1, CellKind::Highway, 1, 0));
        b.add_cell(cell(2, CellKind::Open, 2, 0));
        b.add_cell(cell(3, CellKind::Open, 0, 1));
        b.add_cell(cell(4, CellKind::Open, 1, 1));
        b.add_cell(cell(5, CellKind::Open, 2, 1));
        b.add_cell(cell(6, CellKind::Open, 0, 2));
        b.add_cell(cell(7, CellKind::Open, 1, 2));
        b.add_cell(cell(8, CellKind::Presentation, 2, 2));
        let e = Edge::uniform(1000);
        for row in 0..3u32 {
            for col in 0..2u32 {
                let a = CellId(row * 3 + col);
                let b_id = CellId(row * 3 + col + 1);
                b.add_edge(a, b_id, e);
                b.add_edge(b_id, a, e);
            }
        }
        for row in 0..2u32 {
            for col in 0..3u32 {
                let a = CellId(row * 3 + col);
                let b_id = CellId((row + 1) * 3 + col);
                b.add_edge(a, b_id, e);
                b.add_edge(b_id, a, e);
            }
        }
        let mut builder = WarehouseMapBuilder::new(SystemType::Relay).graph(b.build());
        for id in [3, 6, 7] {
            builder = builder.add_relay(RelayPosition {
                id: RelayId(id),
                position: CellId(id),
                htm_access: CellId(id),
                vtm_access: VtmPosition::Cell(CellId(0)),
                block: BlockId(0),
                side: BlockSide::Left,
            });
        }
        builder.build()
    }

    #[test]
    fn only_includes_relay_access_cells() {
        let map = build_simple_map();
        let reg = ParkingRegistry::from_map(&map);
        assert!(reg.is_candidate(CellId(3)));
        assert!(reg.is_candidate(CellId(6)));
        assert!(reg.is_candidate(CellId(7)));
        assert!(!reg.is_candidate(CellId(0)));
        assert_eq!(reg.candidate_count(), 3);
    }

    fn t(ms: u64) -> SimTime {
        SimTime::ms(ms)
    }

    #[test]
    fn find_nearest_free() {
        let map = build_simple_map();
        let reg = ParkingRegistry::from_map(&map);
        let c = reg.find(CellId(6), Some(WorldPos { x_mm: 0.0, y_mm: 2000.0 }), t(0), RobotId(1));
        assert_eq!(c, Some(CellId(6)));
    }

    #[test]
    fn find_avoids_occupied() {
        let map = build_simple_map();
        let reg = ParkingRegistry::from_map(&map);
        reg.reserve(CellId(6), RobotId(1), t(0));
        let c = reg.find(CellId(0), Some(WorldPos { x_mm: 0.0, y_mm: 1000.0 }), t(500), RobotId(2));
        assert_ne!(c, Some(CellId(6)));
        assert!(reg.is_candidate(c.unwrap()));
    }

    #[test]
    fn temporal_availability() {
        let map = build_simple_map();
        let reg = ParkingRegistry::from_map(&map);
        reg.reserve(CellId(6), RobotId(1), t(0));
        reg.cancel(CellId(6), RobotId(1), t(5000));
        let c = reg.find(CellId(6), Some(WorldPos { x_mm: 0.0, y_mm: 2000.0 }), t(5000), RobotId(2));
        assert_eq!(c, Some(CellId(6)));
        let c = reg.find(CellId(6), Some(WorldPos { x_mm: 0.0, y_mm: 2000.0 }), t(3000), RobotId(2));
        assert_ne!(c, Some(CellId(6)));
    }

    #[test]
    fn cancel_before_start_removes() {
        let map = build_simple_map();
        let reg = ParkingRegistry::from_map(&map);
        reg.reserve(CellId(6), RobotId(1), t(5000));
        reg.cancel(CellId(6), RobotId(1), t(3000));
        let c = reg.find(CellId(6), Some(WorldPos { x_mm: 0.0, y_mm: 2000.0 }), t(0), RobotId(2));
        assert_eq!(c, Some(CellId(6)));
    }

    /// Regression for the parking-conflict surfaced by `minimal_putaway2`:
    /// `find` must reject a cell whose only existing reservation is a
    /// future open-ended hold by another robot. The pre-fix predicate
    /// (`r.start <= at && r.end > at`) considered such cells "free" at
    /// `at` and let the registry hand them out, after which the
    /// pathfinder rejected the resulting MAX-dwell commit with
    /// `DestinationExceedsMaxWait`.
    #[test]
    fn future_open_ended_reservation_blocks_find() {
        let map = build_simple_map();
        let reg = ParkingRegistry::from_map(&map);
        // RobotId(1)'s parking arrival is at t=10_000 (future from the
        // perspective of the t=5_000 query below).
        reg.reserve(CellId(6), RobotId(1), t(10_000));
        // RobotId(2) queries at t=5_000 — well before RobotId(1)'s
        // reservation starts. Under the pre-fix predicate this returned
        // CellId(6) (cell looked free at the instant t=5_000); under
        // the corrected MAX-dwell predicate it must skip CellId(6).
        let c = reg.find(CellId(6), Some(WorldPos { x_mm: 0.0, y_mm: 2000.0 }), t(5_000), RobotId(2));
        assert_ne!(c, Some(CellId(6)), "future open-ended hold must block parking allocation");
        assert!(c.map(|c| reg.is_candidate(c)).unwrap_or(true));
    }

    /// Companion test: once the prior reservation is canceled, the cell
    /// becomes available again. This is the un-park lifecycle path —
    /// `AvailableHtmRegistry::take` calls `parking.cancel(cell, robot,
    /// action_ready_at)` to clear the entry as the bot starts a new
    /// task.
    #[test]
    fn cancel_unblocks_future_holder() {
        let map = build_simple_map();
        let reg = ParkingRegistry::from_map(&map);
        reg.reserve(CellId(6), RobotId(1), t(10_000));
        reg.cancel(CellId(6), RobotId(1), t(10_000));
        let c = reg.find(CellId(6), Some(WorldPos { x_mm: 0.0, y_mm: 2000.0 }), t(5_000), RobotId(2));
        assert_eq!(c, Some(CellId(6)), "canceled reservation must not block new allocation");
    }
    #[test]
    fn random_candidates_are_repeatable_and_exclude_reserved_relays_and_chargers() {
        let mut map = build_simple_map();
        // A charger is not an idle relay, even if malformed relay metadata also names it.
        let mut graph = WarehouseGraphBuilder::new();
        for cell in map.graph.cells() {
            let mut cell = cell.clone();
            if cell.id == CellId(7) {
                cell.kind = CellKind::Charging;
            }
            graph.add_cell(cell);
        }
        for (from, to, edge) in map.graph.edges() {
            graph.add_edge(from, to, *edge);
        }
        map.graph = graph.build();
        let reg = ParkingRegistry::with_policy(&map, IdleRelaySelection::Random, 42);
        reg.reserve(CellId(6), RobotId(2), t(10_000));
        let query = || reg.find_candidates(CellId(0), cell_point_mm(&map, CellId(0)), t(500), RobotId(1));
        assert_eq!(query(), vec![CellId(3)]);
        assert_eq!(query(), query());
    }
}
