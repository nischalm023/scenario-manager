//! Foreshortening: what `reserve_path` reads off the graph and the holds to
//! end a window short of the target. A target's queue is the queue cells
//! feeding it that a robot bound for it may drive; the map makes a queue's
//! edges destination-only, so a robot standing on one stops no traffic bound
//! elsewhere. Which of them a window ends on is decided by search, in the
//! allocator; this module names the candidates, the cells an await is for,
//! and the later start the target's search may take toward an awaited cell's
//! clearance (simtime).

use desgraph::WarehouseGraph;
use desgraph::types::{CellId, CellKind, ZoneId};
use rustc_hash::{FxHashMap, FxHashSet};
use smallvec::SmallVec;

use super::FHSippPathfinder;
use crate::reservation::oracle::ReservationOracle;
use crate::{CollisionConfig, DriveModel, GhostOracle, Orientation, Path, RobotId, SimTime};

/// The cells with an edge into each queue cell and into each cell a queue
/// cell feeds, read off the graph once: a queue's feeders, and every way in.
pub(super) type QueueEntries = FxHashMap<CellId, SmallVec<[CellId; 2]>>;

pub(super) fn queue_entries(graph: &WarehouseGraph) -> QueueEntries {
    let fed: FxHashSet<CellId> =
        graph.edges().filter(|(from, _, _)| is_queue_cell(graph, *from)).map(|(_, to, _)| to).collect();
    let mut entries = QueueEntries::default();
    for (from, to, _) in graph.edges() {
        if is_queue_cell(graph, to) || fed.contains(&to) {
            entries.entry(to).or_default().push(from);
        }
    }
    entries
}

fn is_queue_cell(graph: &WarehouseGraph, cell: CellId) -> bool {
    graph.cell(cell).is_some_and(|cell| cell.kind == CellKind::QueueCell)
}

/// A cell another robot holds open-ended: its departure is not scheduled.
fn held_by_another(oracle: &dyn ReservationOracle, cell: CellId, robot: RobotId) -> bool {
    oracle.open_ended_reservation(cell).is_some_and(|hold| hold.robot() != robot)
}

/// Where a foreshortened window from a robot's cell may end: whether another
/// robot holds the target itself open-ended, and the queue cells the window
/// may end on, nearest the target first.
pub(super) struct WindowEnds {
    pub(super) target_held: bool,
    pub(super) queue_cells: SmallVec<[CellId; 8]>,
}

impl FHSippPathfinder {
    /// Where a window from `at` may end short of `to`: the queue cells
    /// feeding `to` ahead of the robot that no other robot holds open-ended.
    /// Ahead of a robot in the queue is downstream along the queue's own
    /// edges, so neither a shortcut nor a reverse edge sends it back in the
    /// queue. `None` when neither the target nor a queue cell ahead is held
    /// open-ended by another robot: the journey is then one window, as
    /// without queues.
    pub(super) fn window_ends(
        &self,
        at: CellId,
        to: CellId,
        drive: &dyn DriveModel,
        oracle: &dyn ReservationOracle,
        robot: RobotId,
    ) -> Option<WindowEnds> {
        let queue = self.queue_cells_feeding(to, drive);
        let downstream =
            queue.iter().any(|(cell, _)| *cell == at).then(|| self.downstream_in_queue(at, to, &queue, drive));
        let ahead = || {
            queue
                .iter()
                .map(|(cell, _)| *cell)
                .filter(|cell| downstream.as_ref().is_none_or(|ahead| ahead.contains(cell)))
        };
        let target_held = held_by_another(oracle, to, robot);
        let held = target_held || ahead().any(|cell| held_by_another(oracle, cell, robot));
        held.then(|| WindowEnds {
            target_held,
            queue_cells: ahead().filter(|cell| !held_by_another(oracle, *cell, robot)).collect(),
        })
    }

    /// The cells of `to`'s queue strictly downstream of `at`, itself one of
    /// them: reachable from `at` along the queue's edges a robot bound for
    /// `to` may drive, and unable to reach `at` along them.
    fn downstream_in_queue(
        &self,
        at: CellId,
        to: CellId,
        queue: &[(CellId, u32)],
        drive: &dyn DriveModel,
    ) -> SmallVec<[CellId; 8]> {
        let graph = self.inner.graph();
        let dest_zone = graph.cell(to).map(|cell| cell.zone).unwrap_or(ZoneId(0));
        let queued = |cell: CellId| queue.iter().any(|(queued, _)| *queued == cell);
        let drivable = |from: CellId, into: CellId| {
            graph.edge_between(from, into).is_some_and(|edge| drive.may_traverse(edge, to, dest_zone))
        };
        let reached = |step: &dyn Fn(CellId) -> SmallVec<[CellId; 4]>| {
            let mut reached: SmallVec<[CellId; 8]> = SmallVec::new();
            let (mut next, mut from) = (0, at);
            loop {
                for cell in step(from) {
                    if cell != at && queued(cell) && !reached.contains(&cell) {
                        reached.push(cell);
                    }
                }
                let Some(&cell) = reached.get(next) else { return reached };
                next += 1;
                from = cell;
            }
        };
        let forward = reached(&|from| {
            graph.neighbors(from).into_iter().map(|(into, _)| into).filter(|&into| drivable(from, into)).collect()
        });
        let back = reached(&|into| {
            let feeders = self.inner.queue_entries.get(&into).into_iter().flatten().copied();
            feeders.filter(|&from| drivable(from, into)).collect()
        });
        forward.into_iter().filter(|cell| !back.contains(cell)).collect()
    }

    /// The cells an await from `at` is for: every cell on the robot's
    /// empty-floor route to `to` that another robot holds open-ended, since a
    /// commit bounding any of them may let the journey on. Empty when that
    /// route crosses no such hold, so no commit the await could hear would.
    pub(super) fn held_on_route(
        &self,
        at: CellId,
        facing: Orientation,
        to: CellId,
        drive: &dyn DriveModel,
        oracle: &dyn ReservationOracle,
        config: &CollisionConfig,
    ) -> SmallVec<[CellId; 4]> {
        let route = self.route_cells(at, facing, to, drive, config);
        route.into_iter().filter(|&cell| held_by_another(oracle, cell, config.self_robot)).collect()
    }

    /// Of the cells `awaited`, those on the robot's empty-floor route from
    /// `at` to `to`, `at` aside: the cells whose clearance the target's search
    /// may start toward. Only the route decides, not the cell's hold.
    pub(super) fn awaited_on_route(
        &self,
        awaited: &[CellId],
        at: CellId,
        facing: Orientation,
        to: CellId,
        drive: &dyn DriveModel,
        config: &CollisionConfig,
    ) -> SmallVec<[CellId; 4]> {
        if awaited.is_empty() {
            return SmallVec::new();
        }
        let route = self.route_cells(at, facing, to, drive, config);
        awaited.iter().copied().filter(|cell| route.contains(cell)).collect()
    }

    /// The cells of the robot's empty-floor route from `at` to `to`, `at`
    /// aside, each once; none when no route exists.
    fn route_cells(
        &self,
        at: CellId,
        facing: Orientation,
        to: CellId,
        drive: &dyn DriveModel,
        config: &CollisionConfig,
    ) -> SmallVec<[CellId; 8]> {
        let mut cells: SmallVec<[CellId; 8]> = SmallVec::new();
        let Some(route) = self.empty_floor_route(at, facing, to, to, drive, config) else {
            return cells;
        };
        for cell in route.waypoints.iter().map(|waypoint| waypoint.cell) {
            if cell != at && !cells.contains(&cell) {
                cells.push(cell);
            }
        }
        cells
    }

    /// A later start for the target's search, toward an awaited `cell`'s
    /// clearance (the start of its unbounded free interval, simtime): when that
    /// clearance is at or beyond the horizon and the empty-floor travel to the
    /// cell is shorter than `max_wait`, the clearance less that travel, so the
    /// wait on the robot's cell does not spend the window's horizon; else
    /// `config`'s. Navigate applies the same rule to its chosen target before
    /// the call, for a target already bounded that far ahead; each only moves
    /// a start later, so the later of the two stands.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn start_toward_clearance(
        &self,
        at: CellId,
        facing: Orientation,
        cell: CellId,
        to: CellId,
        drive: &dyn DriveModel,
        oracle: &dyn ReservationOracle,
        config: &CollisionConfig,
    ) -> SimTime {
        let start = config.sim_start;
        let clearance = oracle
            .entry_intervals(cell, start, SimTime::EOT, config.exclusion_for(cell))
            .into_iter()
            .find_map(|(free, end)| end.is_none().then_some(free.max(start)))
            .filter(|clearance| *clearance >= start + config.max_wait);
        let travel = clearance
            .and_then(|_| self.empty_floor_route(at, facing, cell, to, drive, config))
            .map(|route| route.travel_time);
        match (clearance, travel) {
            (Some(clearance), Some(travel)) if travel < config.max_wait => start.max(clearance.saturating_sub(travel)),
            _ => start,
        }
    }

    /// The route from `at` to `to` on an empty floor, for a robot bound for
    /// `bound_for`, priced by the robot's own drive.
    fn empty_floor_route(
        &self,
        at: CellId,
        facing: Orientation,
        to: CellId,
        bound_for: CellId,
        drive: &dyn DriveModel,
        config: &CollisionConfig,
    ) -> Option<Path> {
        let unheld = CollisionConfig { predecessor: None, destination: None, dest_dwell: None, ..config.clone() };
        let found = self.find_window_reserved(at, facing, to, bound_for, drive, &GhostOracle, &unheld);
        found.ok().map(|(route, _)| route)
    }

    /// Whether every way into `into` (`to` or a cell of its queue) comes from
    /// a cell another robot holds open-ended from simtime `start` on, directly
    /// or through queue cells, so no search from `start` reaches `into`. Walked
    /// back from `into` over `queue_entries`: a way in from a cell so held is
    /// sealed, a queue cell or not; one from another queue cell walks on; one
    /// from `at` or another cell outside the queue leaves `into` open.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn sealed_off(
        &self,
        at: CellId,
        into: CellId,
        to: CellId,
        drive: &dyn DriveModel,
        oracle: &dyn ReservationOracle,
        robot: RobotId,
        start: SimTime,
    ) -> bool {
        let graph = self.inner.graph();
        let dest_zone = graph.cell(to).map(|cell| cell.zone).unwrap_or(ZoneId(0));
        if !self.inner.queue_entries.contains_key(&into) {
            return false;
        }
        let mut walked: SmallVec<[CellId; 8]> = SmallVec::new();
        walked.push(into);
        let mut next = 0;
        while let Some(&cell) = walked.get(next) {
            next += 1;
            for &from in self.inner.queue_entries.get(&cell).into_iter().flatten() {
                let drivable =
                    graph.edge_between(from, cell).is_some_and(|edge| drive.may_traverse(edge, to, dest_zone));
                let sealing = oracle
                    .open_ended_reservation(from)
                    .is_some_and(|hold| hold.robot() != robot && hold.window().start() <= start);
                if !drivable || sealing || walked.contains(&from) {
                    continue;
                }
                if from == at || !is_queue_cell(graph, from) {
                    return false;
                }
                walked.push(from);
            }
        }
        true
    }

    /// The target's queue, nearest cell first: each queue cell with its count
    /// of moves to `to` through queue cells a robot bound for `to` may drive.
    fn queue_cells_feeding(&self, to: CellId, drive: &dyn DriveModel) -> SmallVec<[(CellId, u32); 8]> {
        let graph = self.inner.graph();
        let dest_zone = graph.cell(to).map(|cell| cell.zone).unwrap_or(ZoneId(0));
        let mut queue: SmallVec<[(CellId, u32); 8]> = SmallVec::new();
        let mut next = 0;
        let mut ahead = (to, 0);
        loop {
            for &feeder in self.inner.queue_entries.get(&ahead.0).into_iter().flatten() {
                let drivable =
                    graph.edge_between(feeder, ahead.0).is_some_and(|edge| drive.may_traverse(edge, to, dest_zone));
                if drivable
                    && feeder != to
                    && is_queue_cell(graph, feeder)
                    && !queue.iter().any(|(cell, _)| *cell == feeder)
                {
                    queue.push((feeder, ahead.1 + 1));
                }
            }
            let Some(&cell) = queue.get(next) else {
                return queue;
            };
            next += 1;
            ahead = cell;
        }
    }
}
