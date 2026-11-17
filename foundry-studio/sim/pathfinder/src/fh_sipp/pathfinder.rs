//! `FHSippPathfinder` — struct, constructors, configuration accessors,
//! heuristic, the rotate rule both searches share (`Inner::rotate_cost`), and
//! shared graph helpers (`rotation_permitted`, `is_holding_cell`).
//!
//! The actual A* search loops live in sibling modules:
//! - `super::search` — reservation-aware full-horizon SIPP-A\* (entry point
//!   plus the `Search<'a>` runner). Per-state expansion in
//!   `super::expand`, `super::expand_rotate`, `super::expand_move`,
//!   `super::expand_wait`.
//! - `super::interface` — spatial-only A\* (`Pathfinder` trait impl)

use std::collections::HashSet;
use std::sync::Arc;

use desgraph::WarehouseGraph;
use desgraph::types::{CellId, GridPos};
use desmap::WarehouseMap;

use super::super::Orientation;
use super::foreshortening::{QueueEntries, queue_entries};
use super::heap::BucketQueue;
use crate::SimDuration;
use crate::drive::DriveModel;
#[cfg(any(feature = "pathfinder-trace", test))]
use crate::pathfinder_trace::{TraceEvent, TraceHook};
#[cfg(any(feature = "pathfinder-trace", test))]
use crate::reservation::oracle::ReservationOracle;

/// Cardinal-direction iteration helper used during successor enumeration.
pub(super) const ALL_ORIENTATIONS: [Orientation; 4] =
    [Orientation::North, Orientation::East, Orientation::South, Orientation::West];

/// SIPP pathfinder that commits a journey in windows. Cheap to clone
/// (`Arc<Inner>`).
#[derive(Clone)]
pub struct FHSippPathfinder {
    pub(super) inner: Arc<Inner>,
}

pub(super) struct Inner {
    /// Owned snapshot of the warehouse map. The pathfinder reads its graph and
    /// the canonical cell-spacing (from `grid_bounds.cell_size_mm`) through
    /// this — cell spacing lives on the map, not on the pathfinder.
    pub(super) map: WarehouseMap,
    /// Dense lookup: `cell_positions[id.0]` → `GridPos`. Indexed by `CellId.0`.
    /// Grid cells are assigned IDs `row * width + col` and are fully dense,
    /// giving O(1) direct array access with no hashing in the hot heuristic.
    pub(super) cell_positions: Vec<Option<GridPos>>,
    pub(super) holding_cells: HashSet<CellId>,
    pub(super) speed_mm_per_sec: u64,
    pub(super) traverse_duration: SimDuration,
    /// Shared in-place 90° turn duration, reported by [`FHSippPathfinder::turn_duration`]; the
    /// search's turns come from each call's drive.
    pub(super) turn_duration: SimDuration,
    /// Shortest horizontal (column-to-column) edge in the graph, in mm.
    /// Both equal `grid_bounds.cell_size_mm` on a uniform map.
    pub(super) min_x_edge_mm: u64,
    /// Shortest vertical (row-to-row) edge in the graph, in mm.
    pub(super) min_y_edge_mm: u64,
    /// Every arc in the graph is axis-aligned AND at least as long as its
    /// coordinate span. When true, the gridless heuristic may charge
    /// MANHATTAN distance instead of straight-line: any path's arcs each
    /// cost at least their own |dx|+|dy| (one of the two is zero), so the
    /// summed cost is at least the endpoint L1 span — admissible, and far
    /// tighter than Euclid on L-shaped routes (a converted lattice map ran
    /// ~2x slower per search on straight-line h). One diagonal arc anywhere
    /// falsifies the argument, so the flag comes from a full load-time scan;
    /// DFW6's direct-stitch aisle crossings keep it false there.
    pub(super) rectilinear: bool,
    /// The distance whose crossing time bounds one move's f-increase in
    /// `find_path`, read off the graph by [`longest_move_jump_mm`]. Each call
    /// sizes its `BucketQueue` open list from it and the call's drive, by
    /// [`open_bucket_capacity`]; a fixed bucket count would be a correctness
    /// bug, not a tuning choice — see [`super::heap::BucketQueue`].
    pub(super) move_jump_mm: u64,
    /// The cells with an edge into each queue cell and each cell a queue
    /// feeds: where a window may end short of the target, and whether held
    /// queue cells seal a cell off (`super::foreshortening`).
    pub(super) queue_entries: QueueEntries,
    #[cfg(any(feature = "pathfinder-trace", test))]
    pub(super) trace_hook: Option<TraceHook>,
}

impl Inner {
    /// Convenience accessor for the embedded warehouse graph.
    pub(super) fn graph(&self) -> &WarehouseGraph {
        &self.map.graph
    }

    /// How long an in-place rotate from `from` to `to` takes, or `None` when
    /// the search has to reach `to` through an intermediate facing instead.
    /// The spatial and the reservation-aware search both call this, so a
    /// travel estimate gives a rotate the same time the reserved plan does.
    ///
    /// Between cardinal facings a rotate is one quarter turn, or a reversal in
    /// one action when the run states a 180° figure; unstated, the search
    /// composes a reversal from two quarter turns. A rotate to or from an
    /// off-cardinal heading costs its angle at the quarter-turn rate.
    pub(super) fn rotate_cost(drive: &dyn DriveModel, from: Orientation, to: Orientation) -> Option<SimDuration> {
        if from == to {
            return None;
        }
        let cardinal_pair = !matches!(from, Orientation::Heading(_)) && !matches!(to, Orientation::Heading(_));
        if cardinal_pair && from.turns_to(to) == 2 {
            return drive.reversal_duration_for();
        }
        Some(drive.rotation_cost(from.angle_millidegrees_to(to)))
    }
}

impl FHSippPathfinder {
    pub fn new(
        map: &WarehouseMap,
        speed_mm_per_sec: u64,
        turn_duration: SimDuration,
        holding_cells: HashSet<CellId>,
    ) -> Self {
        let cell_spacing_mm = map.cell_size_mm();
        let graph = &map.graph;
        let max_id = graph.cells().filter_map(|c| c.grid_pos.is_some().then_some(c.id.0)).max().unwrap_or(0);
        let mut cell_positions: Vec<Option<GridPos>> = vec![None; max_id as usize + 1];
        for c in graph.cells() {
            if let Some(gp) = c.grid_pos {
                cell_positions[c.id.0 as usize] = Some(gp);
            }
        }
        let traverse_duration = SimDuration::ms(cell_spacing_mm * 1000 / speed_mm_per_sec);
        // Shortest edge PER AXIS, not one global minimum. A global minimum would
        // charge the x term of the heuristic the smallest y pitch and vice versa,
        // deflating h over the whole map — on a calibrated cencora map only 12 of
        // 95 row pitches are 645mm and every column pitch is 850mm, so a global
        // 645 would understate every horizontal step by 24%, widening the A*
        // frontier everywhere for no accuracy gain. Per-axis minima are equally
        // admissible and stay tight.
        //
        // Zero-length edges are rejected rather than filtered out: an edge that
        // costs nothing to cross but still counts as a grid step would let h
        // exceed the true remaining cost, i.e. break admissibility. Same hazard
        // `AxisGeometry::from_centers` guards for row centres, so it is refused
        // here too instead of being silently skipped.
        let (min_x_edge_mm, min_y_edge_mm) = min_axis_edge_distances(graph, cell_spacing_mm);
        let rectilinear = graph_is_rectilinear(graph);
        let move_jump_mm = longest_move_jump_mm(graph, cell_spacing_mm);
        Self {
            inner: Arc::new(Inner {
                map: map.clone(),
                cell_positions,
                holding_cells,
                speed_mm_per_sec,
                traverse_duration,
                turn_duration,
                min_x_edge_mm,
                min_y_edge_mm,
                rectilinear,
                move_jump_mm,
                queue_entries: queue_entries(graph),
                #[cfg(any(feature = "pathfinder-trace", test))]
                trace_hook: None,
            }),
        }
    }

    #[cfg(any(feature = "pathfinder-trace", test))]
    pub fn with_trace_hook(
        mut self,
        hook: impl Fn(&TraceEvent, Option<&dyn ReservationOracle>) + Send + Sync + 'static,
    ) -> Self {
        Arc::get_mut(&mut self.inner).expect("with_trace_hook must be called before pathfinder is cloned").trace_hook =
            Some(Box::new(hook));
        self
    }

    pub fn traverse_duration(&self) -> SimDuration {
        self.inner.traverse_duration
    }

    /// Test window onto [`Self::heuristic_for`] — the heuristic is the one
    /// piece of the search whose VALUE (not just its effect on the result)
    /// is worth asserting directly, since admissibility is a property of
    /// the number itself.
    #[cfg(test)]
    pub(crate) fn heuristic_for_test(&self, from: CellId, to: CellId, drive: &dyn DriveModel) -> SimDuration {
        self.heuristic_for(from, to, drive)
    }

    pub fn turn_duration(&self) -> SimDuration {
        self.inner.turn_duration
    }

    pub fn speed_mm_per_sec(&self) -> u64 {
        self.inner.speed_mm_per_sec
    }

    pub fn grid_dimensions(&self) -> (u32, u32) {
        let (mut max_x, mut max_y) = (0u32, 0u32);
        let mut any = false;
        for gp in self.inner.cell_positions.iter().filter_map(|x| x.as_ref()) {
            any = true;
            max_x = max_x.max(gp.x);
            max_y = max_y.max(gp.y);
        }
        if any { (max_x + 1, max_y + 1) } else { (0, 0) }
    }

    /// Admissible heuristic: L1 grid distance / speed, in ms.
    ///
    /// Uses the pathfinder's configured `speed_mm_per_sec` (typically the VTM
    /// max speed), shared across all robot types. The live search paths use
    /// the per-robot [`Self::heuristic_for`] instead; this shared-speed form
    /// is retained only as the documented constant-speed admissibility
    /// baseline in the kinematic tests.
    ///
    /// Deliberately still keyed on the nominal `traverse_duration` rather than
    /// the graph's shortest edge, unlike [`Self::heuristic_for`]: it documents
    /// the uniform-pitch baseline, and every caller is a uniform test fixture
    /// where the two coincide.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn heuristic(&self, from: CellId, to: CellId) -> SimDuration {
        let pos = &self.inner.cell_positions;
        match (pos.get(from.0 as usize).copied().flatten(), pos.get(to.0 as usize).copied().flatten()) {
            (Some(a), Some(b)) => {
                let dx = (a.x as i64 - b.x as i64).unsigned_abs();
                let dy = (a.y as i64 - b.y as i64).unsigned_abs();
                self.inner.traverse_duration * (dx + dy)
            }
            _ => SimDuration::ZERO,
        }
    }

    /// Admissible A* heuristic: L1 grid distance × the minimum time-per-cell for
    /// `drive`.
    ///
    /// Each step's bound is [`DriveModel::min_travel_time`], which never exceeds
    /// what the drive charges for that distance. The drive the search prices
    /// with is the one that bounds it: an HTM's constant speed is below its
    /// v_max, so bounding a trapezoid's moves with the constant speed would
    /// overestimate.
    ///
    /// The per-step *distance* is the graph's shortest edge ON THAT AXIS, not
    /// `grid_bounds.cell_size_mm`. Any path covering `k` grid steps traverses `k`
    /// edges, so charging each axis its own shortest edge is a lower bound on the
    /// true cost. Using `cell_size_mm` would overestimate
    /// — hence be inadmissible — on any map whose real pitch dips below the
    /// nominal, which is exactly what calibrated row geometry produces (cencora's
    /// relay pitch alternates 645/645/850 against an 850mm nominal). On a uniform
    /// map both axis minima equal `cell_size_mm`, so this is bit-identical to the
    /// previous behaviour and no pinned trace moves.
    ///
    /// When either endpoint has no `grid_pos` — a junction, or any node a
    /// dimensional map placed off the lattice — the L1 form has no grid
    /// steps to count, and falls back to the drive's bound on the
    /// straight-line distance (see [`Self::straight_line_h`]).
    pub(super) fn heuristic_for(&self, from: CellId, to: CellId, drive: &dyn DriveModel) -> SimDuration {
        let pos = &self.inner.cell_positions;
        match (pos.get(from.0 as usize).copied().flatten(), pos.get(to.0 as usize).copied().flatten()) {
            (Some(a), Some(b)) => {
                let x_step = drive.min_travel_time(self.inner.min_x_edge_mm);
                let y_step = drive.min_travel_time(self.inner.min_y_edge_mm);
                let dx = (a.x as i64 - b.x as i64).unsigned_abs();
                let dy = (a.y as i64 - b.y as i64).unsigned_abs();
                x_step * dx + y_step * dy
            }
            _ => self.straight_line_h(from, to, drive),
        }
    }

    /// Straight-line remaining time for a pair the grid form cannot measure:
    /// the drive's [`DriveModel::min_travel_time`] for the Euclidean world
    /// distance.
    ///
    /// Admissible because no path can be shorter than the straight line
    /// between its endpoints and no edge can be crossed faster than the
    /// drive's bound, provided every edge's `distance_mm` is at least the
    /// Euclidean distance it spans, which grid edges satisfy exactly
    /// (center-to-center) and lowered free-form edges satisfy by
    /// construction (derived Euclidean, rounded up).
    ///
    /// This replaces a `SimDuration::ZERO` that made the search degenerate
    /// to Dijkstra around any off-lattice node. Positions are read from the
    /// graph rather than a dense array because this arm is off the hot path
    /// by definition: on a pure-lattice map it never runs at all, which is
    /// why no pinned trace moves.
    fn straight_line_h(&self, from: CellId, to: CellId, drive: &dyn DriveModel) -> SimDuration {
        let graph = self.inner.graph();
        let (Some(a), Some(b)) = (graph.cell(from).and_then(|c| c.world_pos), graph.cell(to).and_then(|c| c.world_pos))
        else {
            return SimDuration::ZERO;
        };
        // On a rectilinear graph (see `Inner::rectilinear`) the L1 span is
        // itself a lower bound, and it is what the route actually walks.
        let dist_mm = if self.inner.rectilinear {
            (b.x_mm - a.x_mm).abs() + (b.y_mm - a.y_mm).abs()
        } else {
            (b.x_mm - a.x_mm).hypot(b.y_mm - a.y_mm)
        };
        // Truncating toward zero keeps this a lower bound under the
        // millimetre rounding that lowering applies to edge lengths.
        drive.min_travel_time(dist_mm as u64)
    }
}

/// Whether every arc is axis-aligned and no shorter than its coordinate
/// span -- the two facts that make Manhattan h admissible (see
/// [`Inner::rectilinear`]). Positionless endpoints keep the flag false:
/// an arc we cannot measure is an arc we cannot vouch for.
fn graph_is_rectilinear(graph: &WarehouseGraph) -> bool {
    for cell in graph.cells() {
        let Some(a) = cell.world_pos else { return false };
        for (n, e) in graph.neighbors(cell.id) {
            let Some(b) = graph.cell(n).and_then(|c| c.world_pos) else { return false };
            let (dx, dy) = ((b.x_mm - a.x_mm).abs(), (b.y_mm - a.y_mm).abs());
            if dx > 0.5 && dy > 0.5 {
                return false;
            }
            if (e.distance_mm as f64) < dx.max(dy) - 0.5 {
                return false;
            }
        }
    }
    true
}

/// Shortest cardinal edge along each grid axis, in mm, as `(x, y)`.
///
/// Only edges between two cells that both carry a `grid_pos` are classified, and
/// only when they differ on exactly one axis: a junction-cell edge or a diagonal
/// has no axis to charge, and folding it into either minimum would deflate that
/// axis for the whole map.
///
/// Falls back to `nominal_mm` per axis when nothing qualifies, which covers an
/// empty graph and any map that is pure junctions.
///
/// A zero-length edge between two distinct grid cells is a broken map, not a free
/// move: it would let the heuristic charge a step the search can cross for
/// nothing, exceeding the true remaining cost and costing admissibility. Since
/// `new` cannot return an error, it is clamped to 1mm and announced, rather than
/// skipped silently.
fn min_axis_edge_distances(graph: &WarehouseGraph, nominal_mm: u64) -> (u64, u64) {
    let (mut min_x, mut min_y) = (None::<u64>, None::<u64>);
    let mut zero_len = 0usize;
    for (from, to, edge) in graph.edges() {
        let (Some(a), Some(b)) = (graph.cell(from).and_then(|c| c.grid_pos), graph.cell(to).and_then(|c| c.grid_pos))
        else {
            continue;
        };
        let d = if edge.distance_mm == 0 {
            zero_len += 1;
            1
        } else {
            edge.distance_mm
        };
        match (a.x != b.x, a.y != b.y) {
            (true, false) => min_x = Some(min_x.map_or(d, |m: u64| m.min(d))),
            (false, true) => min_y = Some(min_y.map_or(d, |m: u64| m.min(d))),
            _ => {}
        }
    }
    if zero_len > 0 {
        eprintln!(
            "[pathfinder] {zero_len} cardinal edge(s) have distance_mm = 0 between distinct grid cells; \
             treating them as 1mm to keep the A* heuristic admissible. Check the map's edge_exceptions."
        );
    }
    (min_x.unwrap_or(nominal_mm), min_y.unwrap_or(nominal_mm))
}

/// The distance whose crossing time bounds how far one move raises f in
/// `find_path`: the longest edge in the graph plus the largest drop in the
/// heuristic one move can cause. Per-edge `distance_mm` is already
/// non-uniform, so this reads the graph rather than assuming `cell_size_mm`.
fn longest_move_jump_mm(graph: &WarehouseGraph, cell_spacing_mm: u64) -> u64 {
    let longest_edge_mm = graph.edges().map(|(_, _, e)| e.distance_mm).max().unwrap_or(cell_spacing_mm);
    // The heuristic term of a move's f-jump is |h(next) - h(cur)|, and a
    // metric heuristic can drop by the full edge length -- NOT by one cell
    // pitch. On a uniform lattice the two coincide, which is how the pitch
    // bound survived v3; a world map's stitch edges exceed the pitch and the
    // ring under-sizes, wraps, and quietly returns suboptimal paths (caught
    // by find_path_optimality on the converted cencora_v3). Bound by the
    // edge itself, floored at the pitch for degenerate graphs.
    let h_step_mm = longest_edge_mm.max(cell_spacing_mm);
    longest_edge_mm.saturating_add(h_step_mm)
}

/// Bucket count for `find_path`'s open list, sized for the largest f-increase
/// any single action on this map can produce when `drive` prices it.
///
/// `f = g + h`, so an action raising g by `c` raises f by `c + (h_after -
/// h_before)`:
/// - **Move** — `h` shifts by at most one heuristic grid step, so the jump is
///   bounded by `edge_ms + heuristic_step_ms`: the time to cover
///   `move_jump_mm` ([`longest_move_jump_mm`]) at the drive's speed.
/// - **Rotate** — same cell, so `h` is unchanged and the jump is exactly the
///   rotate's duration: a quarter turn, or a stated reversal. This is the
///   term that broke cencora_v3 (2100ms > 2048).
///
/// Only constant drives come here: `find_path` refuses a kinematic one, and
/// `reserve_path`, where kinematic searches run, uses `BinaryHeap` instead.
pub(super) fn open_bucket_capacity(move_jump_mm: u64, drive: &dyn DriveModel) -> usize {
    let speed_mm_s = (drive.max_speed().mm_per_sec() as u64).max(1);
    let move_jump_ms = move_jump_mm.saturating_mul(1000) / speed_mm_s;
    let quarter_turn = drive.rotation_cost(90_000);
    // TODO: cover a turn to or from an off-cardinal heading, which costs up to two quarter turns.
    let turn_jump = drive.reversal_duration_for().unwrap_or(quarter_turn).max(quarter_turn);
    BucketQueue::capacity_for(move_jump_ms.max(turn_jump.as_ms()))
}

/// Crate-internal helper for `allocator.rs` to query holding-cell membership.
pub(super) fn is_holding_cell(pf: &FHSippPathfinder, cell: CellId) -> bool {
    pf.inner.holding_cells.contains(&cell)
}

/// Whether the robot `drive` moves may rotate in place on `cell`, as the drive
/// answers for that cell. A cell the graph does not hold forbids nothing.
pub(super) fn rotation_permitted(graph: &WarehouseGraph, cell: CellId, drive: &dyn DriveModel) -> bool {
    graph.cell(cell).is_none_or(|cell| drive.may_rotate_on(cell))
}
