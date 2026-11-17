//! Reservation-aware full-horizon SIPP-A\* search.
//!
//! Per spec §4 + FHSippPathfinder draft §Successor Expansion: every successor
//! is reservation-validated against the oracle (no windowed gating, no
//! spatial-only fallback past depth). Goal acceptance validates the terminal
//! reservation and continues the search if it fails (a later goal interval
//! may still satisfy).
//!
//! [`FHSippPathfinder::find_path_reserved`] is a thin entry point. It builds
//! a [`Search`] runner that owns the per-call mutable state (g_score,
//! came_from, node_meta, open heap, stats) plus the read-only context (pf,
//! oracle, config, horizon, precomputed goal-event times) and calls
//! [`Search::run`]. Per-state expansion lives in `super::expand`.

use crate::{MotionKind, MotionModel};
use std::cell::RefCell;
use std::collections::BinaryHeap;
use std::time::Instant;

use rustc_hash::{FxHashMap, FxHashSet};

use desgraph::types::{CellId, ZoneId};

#[cfg(any(feature = "pathfinder-trace", test))]
use super::super::pathfinder_trace::prelude::*;
use super::super::pathfinder_trace::trace_emit;
use super::super::stats::SearchStats;
use super::super::{CollisionConfig, DriveModel, Orientation, Path};
use super::expand::{UsefulOrients, useful_orients};
use super::heap::AStarEntry;
use super::node::{NodeActionKind, NodeMeta};
use super::pathfinder::{FHSippPathfinder, rotation_permitted};
use super::reconstruct::{edge_motion, reconstruct_waypoints};
use super::state::{SearchKey, SippState, SpeedBucket};

/// Per-search-state record. Combines the three previously-separate
/// FxHashMaps (g_score / came_from / node_meta) into one entry per state.
/// `key` lets the reservation search recover a state's identity from the
/// `u32` arena index carried in the open heap.
#[derive(Clone)]
pub(super) struct SearchEntry {
    pub key: SearchKey,
    pub g: SimDuration,
    pub parent: Option<SearchKey>,
    pub meta: NodeMeta,
}

/// Initial capacity for the per-search node table.
///
/// Hashbrown doubles bucket capacity on rehash, so starting at 1024
/// imposed three rehashes (1024 → 2048 → 4096 → 8192) on long-tail
/// searches whose state count exceeds 1000 — profile-attributed at
/// ~6-8% of total wall time across the relax sites. 8192 covers the
/// typical cencora_phase2 search without rehashing.
///
/// Cost: ~1 MB of slot allocation (8192 buckets × ~120 B). For
/// reservation-aware search this is paid once per thread and reused across
/// searches via [`SearchScratch`]; the spatial-only `Pathfinder::find_path`
/// still allocates it per call. Most slots stay empty for small searches, but
/// allocator-side that's a single bump-and-free, not per-slot init. The
/// wall-time tradeoff strongly favors the larger constant.
pub(super) const NODE_TABLE_INITIAL_CAPACITY: usize = 8192;

/// Initial capacity for the pooled open heap. Unlike the node table this is
/// just first-call smoothing: [`SearchScratch`] retains the high-water
/// capacity after the first few searches, so the steady state is realloc-free
/// regardless of this seed.
const OPEN_HEAP_INITIAL_CAPACITY: usize = 1024;

/// Relaxation primitive: insert-or-overwrite `candidate` into `nodes`
/// for `key`, but only if `candidate.g` improves on any existing entry.
/// Returns `true` if the caller should push the state onto the open
/// heap (i.e. either inserted fresh, or overwrote a strictly worse g).
///
/// Single `entry()` call computes the bucket once, replacing the
/// previous `get().unwrap_or(MAX)` + `insert()` pattern that hashed the
/// key twice per relaxation.
///
/// `candidate` is constructed eagerly at the call site so the
/// no-improvement branch wastes one stack `SearchEntry` build — far
/// cheaper than a second hash.
pub(super) fn relax_node(
    nodes: &mut FxHashMap<SearchKey, SearchEntry>,
    key: SearchKey,
    candidate: SearchEntry,
) -> bool {
    use std::collections::hash_map::Entry;
    match nodes.entry(key) {
        Entry::Occupied(mut occupied) => {
            if candidate.g < occupied.get().g {
                *occupied.get_mut() = candidate;
                true
            } else {
                false
            }
        }
        Entry::Vacant(vacant) => {
            vacant.insert(candidate);
            true
        }
    }
}

use crate::PathfinderError;
use crate::reservation::oracle::ReservationOracle;
use crate::reservation::oracle::SafeIntervalId;
use crate::{SimDuration, SimTime, Velocity};

/// Per-search mutable scratch: the containers a single SIPP-A\* search
/// fills and drains. Bundled because they are only ever used together, and
/// pooled via [`SEARCH_SCRATCH`] so each search reuses the previous search's
/// allocations instead of building — and dropping — a fresh ~1 MB node table,
/// open heap, and caches on every `find_path_reserved` call.
///
/// Reuse is determinism-safe: every container is point-queried only
/// (`get` / `push` / `pop` by stable arena index / parent-walk in
/// `reconstruct_waypoints`), never iterated in an order-dependent way, so
/// capacity retained from a prior search cannot change results.
/// [`SearchScratch::reset`] restores logical emptiness (retaining capacity)
/// at the start of each search.
pub(super) struct SearchScratch {
    /// Append-only SIPP node store. Indices are stable for the lifetime of a
    /// single search; the open heap and `index` map both reference states by
    /// their arena index. Never removed individually — cleared wholesale on
    /// [`SearchScratch::reset`].
    pub arena: Vec<SearchEntry>,

    /// `SearchKey` → arena index, for decrease-key / dedup during expansion.
    pub index: FxHashMap<SearchKey, u32>,

    /// Min-f open set. `BinaryHeap` is a max-heap, so [`AStarEntry`]'s `Ord`
    /// is reversed (see `super::heap`); pop order depends only on that `Ord`,
    /// never on retained capacity.
    pub open: BinaryHeap<AStarEntry>,

    /// Lazy cache of `useful_orients` results within a search.
    /// `useful_orients` is deterministic in `(current_cell, current_orient)`
    /// given the per-search constants: the drive, `to` and `dest_zone`.
    /// SIPP-IP dedup produces multiple SearchKeys per `(cell, orient)`
    /// (distinct intervals, speed buckets, wait labels), so cache hits offset
    /// the per-expansion `may_traverse` + `direction_between` fan-out —
    /// profile-attributed at 4.1% self-time.
    pub useful_orients_cache: FxHashMap<(CellId, Orientation), UsefulOrients>,

    /// Scratch buffer for `oracle.entry_intervals_into` calls. Reused across
    /// all neighbor queries in a state expansion (and across all state
    /// expansions in the search). The previous per-call Vec alloc-and-drop
    /// pattern was profile-attributed at 1.9% of total wall time; reuse drops
    /// that to a single steady-state allocation.
    pub entry_intervals_scratch: Vec<SafeIntervalId>,
}

impl SearchScratch {
    fn new() -> Self {
        Self {
            arena: Vec::with_capacity(NODE_TABLE_INITIAL_CAPACITY),
            index: FxHashMap::with_capacity_and_hasher(NODE_TABLE_INITIAL_CAPACITY, Default::default()),
            open: BinaryHeap::with_capacity(OPEN_HEAP_INITIAL_CAPACITY),
            // ~hundreds of distinct (cell, orient) pairs per typical search.
            useful_orients_cache: FxHashMap::with_capacity_and_hasher(256, Default::default()),
            entry_intervals_scratch: Vec::with_capacity(16),
        }
    }

    /// Restore logical emptiness before reuse, retaining capacity in all
    /// containers — that is the point of pooling.
    fn reset(&mut self) {
        self.arena.clear();
        self.index.clear();
        self.open.clear();
        self.useful_orients_cache.clear();
        self.entry_intervals_scratch.clear();
    }
}

thread_local! {
    /// Pooled reservation-aware search scratch, one per thread. The live sim
    /// drives `find_path_reserved` from a single event-loop thread, so this is
    /// effectively one reused instance; other `Pathfinder: Send + Sync`
    /// callers (benches, desvis) each get their own.
    ///
    /// Borrowed non-re-entrantly: nothing a search calls runs another reserved
    /// search (`classify_exhaustion`'s static-route walk keeps its own
    /// containers), so this `RefCell` is never double-borrowed. Do not make
    /// `find_path_reserved` recursive without revisiting that invariant.
    static SEARCH_SCRATCH: RefCell<SearchScratch> = RefCell::new(SearchScratch::new());
}

impl FHSippPathfinder {
    /// Reservation-aware pathfinding using full-horizon SIPP-A\*.
    pub fn find_path_reserved(
        &self,
        from: CellId,
        from_orientation: Orientation,
        to: CellId,
        drive: &dyn DriveModel,
        oracle: &dyn ReservationOracle,
        config: &CollisionConfig,
    ) -> Result<(Path, SearchStats), PathfinderError> {
        self.find_window_reserved(from, from_orientation, to, to, drive, oracle, config)
    }

    /// The same search to `to` for a robot bound for `bound_for`: the edges a
    /// destination gates are the ones that admit `bound_for`, so a window can
    /// end on a queue cell whose gates admit only robots bound for the cell
    /// it feeds.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn find_window_reserved(
        &self,
        from: CellId,
        from_orientation: Orientation,
        to: CellId,
        bound_for: CellId,
        drive: &dyn DriveModel,
        oracle: &dyn ReservationOracle,
        config: &CollisionConfig,
    ) -> Result<(Path, SearchStats), PathfinderError> {
        if from == to {
            return Err(PathfinderError::ZeroLength { cell: from });
        }
        SEARCH_SCRATCH.with(|cell| {
            let mut scratch = cell.borrow_mut();
            scratch.reset();
            let search = {
                crate::perf_span!("fh_sipp_search_new");
                Search::new(self, oracle, config, from, from_orientation, to, bound_for, drive, &mut scratch)?
            };
            {
                crate::perf_span!("fh_sipp_search_run");
                search.run()
            }
        })
    }

    /// Whether any route leads from `from`, facing `from_orientation`, to `to`
    /// on an empty floor, which tells a failed reserved search that no wait
    /// would help (`NoStaticRoute`). It takes the spatial search's successors
    /// (`find_path`) through the same helpers, asking `drive` only which edges
    /// and rotations it admits, and prices none of them; it visits each (cell,
    /// facing) once.
    ///
    /// `find_path` makes a reversal in one rotation only when the drive states
    /// one, and otherwise through quarter turns, which lead to the same facings
    /// an edge departs along. The walk counts it as one rotation, so its answer
    /// does not depend on whether the drive states one.
    fn static_route_exists(
        &self,
        from: CellId,
        from_orientation: Orientation,
        to: CellId,
        bound_for: CellId,
        drive: &dyn DriveModel,
    ) -> bool {
        // `find_path` refuses a route of zero length.
        if from == to {
            return false;
        }
        let graph = self.inner.graph();
        let dest_zone = graph.cell(bound_for).map(|c| c.zone).unwrap_or(ZoneId(0));
        let mut visited: FxHashSet<(CellId, Orientation)> = FxHashSet::default();
        visited.insert((from, from_orientation));
        let mut frontier = vec![(from, from_orientation)];
        while let Some((cell, orientation)) = frontier.pop() {
            if cell == to {
                return true;
            }
            let neighbors = graph.neighbors(cell);
            if rotation_permitted(graph, cell, drive) {
                let useful = useful_orients(graph, &neighbors, cell, orientation, drive, bound_for, dest_zone);
                for facing in useful.iter() {
                    if visited.insert((cell, facing)) {
                        frontier.push((cell, facing));
                    }
                }
            }
            for &(neighbor, edge) in neighbors.iter() {
                if !drive.may_traverse(edge, bound_for, dest_zone) {
                    continue;
                }
                let Some(motion) = edge_motion(graph, cell, neighbor, edge, drive) else {
                    continue;
                };
                if orientation == motion.departure && visited.insert((neighbor, motion.arrival)) {
                    frontier.push((neighbor, motion.arrival));
                }
            }
        }
        false
    }
}

/// Per-call SIPP-A\* runner. Bundles the read-only context (pf, oracle,
/// config, horizon, precomputed goal-event source) with the mutable search
/// state (g_score, came_from, node_meta, open heap, stats). Per-state
/// expansion logic is implemented in `super::expand`.
pub(super) struct Search<'a> {
    pub pf: &'a FHSippPathfinder,
    pub oracle: &'a dyn ReservationOracle,
    pub config: &'a CollisionConfig,
    pub from: CellId,
    pub from_orientation: Orientation,
    pub to: CellId,
    /// The cell the robot is bound for, which destination gates read: `to`,
    /// except for a window that ends on a queue cell feeding it.
    pub bound_for: CellId,
    /// How this robot drives, as the caller picked it: it prices every move
    /// and turn the search makes, and says which edges and turns the robot may
    /// take.
    pub drive: &'a dyn DriveModel,
    pub dest_zone: ZoneId,
    pub horizon_t: SimTime,
    pub goal_event_times: Vec<SimTime>,

    /// Pooled mutable search containers (the arena node store + `index` map,
    /// the open heap, and the `useful_orients` / `entry_intervals` caches),
    /// borrowed from the thread-local [`SEARCH_SCRATCH`] for the duration of
    /// one search. Reset before use — see [`SearchScratch`].
    pub scratch: &'a mut SearchScratch,
    pub stats: SearchStats,

    /// Whether ANY goal-cell safe interval in `[sim_start, horizon_t]`
    /// structurally admits the requested terminal dwell. Computed once at
    /// `Search::new` from the same `goal_intervals` query that feeds
    /// `goal_event_times`. Used by `classify_exhaustion` to declare
    /// `DestinationExceedsMaxWait` only when this is `false` — proving
    /// goal-cell capacity is the bottleneck regardless of route timing.
    pub terminal_dwell_possible_in_horizon: bool,

    pub start_instant: Instant,
}

impl<'a> Search<'a> {
    /// Initialize the search. Computes the start cell's safe interval (with
    /// predecessor excluded so the bot's own open hold doesn't self-conflict)
    /// and the bounded goal-event source for event-driven Wait successors.
    pub fn new(
        pf: &'a FHSippPathfinder,
        oracle: &'a dyn ReservationOracle,
        config: &'a CollisionConfig,
        from: CellId,
        from_orientation: Orientation,
        to: CellId,
        bound_for: CellId,
        drive: &'a dyn DriveModel,
        scratch: &'a mut SearchScratch,
    ) -> Result<Self, PathfinderError> {
        let mut stats = SearchStats::default();
        let horizon_t = config.sim_start + config.max_wait;

        stats.entry_intervals_calls += 1;
        let start_intervals = oracle.entry_intervals(from, config.sim_start, horizon_t, config.exclusion_for(from));
        let start_interval = start_intervals
            .iter()
            .copied()
            .find(|(s, e)| *s <= config.sim_start && e.map(|end| config.sim_start < end).unwrap_or(true))
            .ok_or(PathfinderError::UnreachableWithinHorizon { max_wait: config.max_wait, from, to })?;

        let start_sipp = SippState {
            cell: from,
            orientation: from_orientation,
            interval: start_interval,
            speed_bucket: SpeedBucket::ZERO,
        };
        let start_key = SearchKey::ordinary(start_sipp);

        let start_g = SimDuration::ZERO;
        scratch.arena.push(SearchEntry {
            key: start_key,
            g: start_g,
            parent: None,
            meta: NodeMeta {
                action_kind: NodeActionKind::Start,
                action_start: config.sim_start,
                action_end: config.sim_start,
                source_cell: None,
                interval: start_interval,
                entry_velocity: Velocity::ZERO,
                exit_velocity: Velocity::ZERO,
            },
        });
        scratch.index.insert(start_key, 0);
        let h = pf.heuristic_for(from, to, drive);
        scratch.open.push(AStarEntry { f: h.as_ms() as u32, g: start_g.as_ms() as u32, idx: 0 });
        stats.states_pushed += 1;
        stats.peak_open_size = stats.peak_open_size.max(scratch.open.len() as u64);

        // Bounded event-driven Wait source. Per spec §4 "Wait successor
        // (event-driven)" + FHSippPathfinder draft §Event-Driven Wait
        // Successors: goal-cell safe-interval starts that admit the requested
        // terminal dwell, derived with the same gap semantics as
        // `entry_intervals` and `earliest_available`. Computed once — the
        // reservation table is static for the duration of a single search.
        //
        // The same query also feeds `terminal_dwell_possible_in_horizon`:
        // can the destination cell accept the requested terminal reservation
        // before `sim_start + max_wait`? Used by classify_exhaustion to
        // distinguish `DestinationExceedsMaxWait` (destination cannot accept the
        // terminal hold under this task's max_wait policy) from `UnreachableWithinHorizon`
        // (destination can accept it, but no route-timed plan was found).
        stats.entry_intervals_calls += 1;
        let goal_intervals = oracle.entry_intervals(to, config.sim_start, horizon_t, config.exclusion_for(to));
        let interval_admits_dwell = |free_start: SimTime, free_end: Option<SimTime>| -> bool {
            match config.dest_dwell {
                None => free_end.is_none(),
                Some(d) if d == SimDuration::MAX => free_end.is_none(),
                Some(d) => match free_end {
                    None => true,
                    Some(end) => {
                        let entry = if free_start >= config.sim_start { free_start } else { config.sim_start };
                        entry < end && (end - entry) >= pf.inner.traverse_duration + d
                    }
                },
            }
        };
        let terminal_dwell_possible_in_horizon = goal_intervals.iter().any(|&(fs, fe)| interval_admits_dwell(fs, fe));
        let goal_event_times: Vec<SimTime> = goal_intervals
            .iter()
            .filter_map(|&(free_start, free_end)| {
                if free_start <= config.sim_start || free_start > horizon_t {
                    return None;
                }
                interval_admits_dwell(free_start, free_end).then_some(free_start)
            })
            .collect();

        let dest_zone = pf.inner.graph().cell(bound_for).map(|c| c.zone).unwrap_or(ZoneId(0));

        Ok(Search {
            pf,
            oracle,
            config,
            from,
            from_orientation,
            to,
            bound_for,
            drive,
            dest_zone,
            horizon_t,
            goal_event_times,
            scratch,
            stats,
            terminal_dwell_possible_in_horizon,
            start_instant: Instant::now(),
        })
    }

    /// Run the search to completion. Returns the accepted path + stats on
    /// success, or one of `IterationLimit` / `NoStaticRoute` /
    /// `UnreachableWithinHorizon` / `DestinationExceedsMaxWait` on failure.
    pub fn run(mut self) -> Result<(Path, SearchStats), PathfinderError> {
        while let Some(AStarEntry { idx, g: popped_g, .. }) = self.scratch.open.pop() {
            self.stats.iterations += 1;
            if self.stats.iterations > self.config.max_iterations {
                return Err(PathfinderError::IterationLimit { from: self.from, to: self.to });
            }

            // Stale heap entry? A cheaper path to this state was found and
            // pushed after this entry; its arena `g` now beats the popped one.
            let (current, current_g, current_meta) = {
                let entry = &self.scratch.arena[idx as usize];
                if popped_g > entry.g.as_ms() as u32 {
                    continue;
                }
                (entry.key, entry.g, entry.meta)
            };
            self.stats.states_expanded += 1;
            trace_emit!(
                self.pf.inner,
                Expand { cell: current.cell(), orientation: current.orientation(), g: current_g, step: 0 },
                self.oracle
            );

            // Goal-acceptance attempt. Returns the accepted path on success;
            // `None` if validation rejects this goal state (search continues).
            if current.cell() == self.to {
                let accepted = {
                    crate::perf_span!("fh_sipp_goal_accept");
                    self.try_accept_goal(current, current_g, current_meta)
                };
                if let Some(path) = accepted {
                    return Ok((path, self.stats));
                }
                continue;
            }

            self.expand_state(current, current_g, current_meta);
        }

        trace_emit!(
            self.pf.inner,
            Exhausted {
                from: self.from,
                to: self.to,
                iterations: self.stats.iterations,
                expanded: self.stats.states_expanded,
                turns_skipped: self.stats.turns_skipped_timed,
                src_hold_blocked: self.stats.source_hold_blocked,
                delayed_count: self.stats.reservation_delayed,
                permanent_blocked: self.stats.reservation_permanent,
            },
            self.oracle
        );
        self.stats.search_duration_us = self.start_instant.elapsed().as_micros() as u64;
        Err(self.classify_exhaustion())
    }

    /// Classify search exhaustion per spec §4 / FHSippPathfinder draft §Failure
    /// Semantics. Precedence:
    ///
    /// 1. `NoStaticRoute` if no route exists even on an empty floor
    ///    (`static_route_exists`) — proven structural failure, no horizon or
    ///    schedule change would help.
    /// 2. `UnreachableWithinHorizon` if any horizon-driven rejection was
    ///    observed during the search (preferred when uncertain).
    /// 3. `DestinationExceedsMaxWait` only when destination-cell interval
    ///    enumeration proves the cell cannot accept the requested terminal
    ///    reservation before `sim_start + max_wait`.
    /// 4. `UnreachableWithinHorizon` as the conservative catch-all.
    fn classify_exhaustion(&self) -> PathfinderError {
        let static_reachable =
            self.pf.static_route_exists(self.from, self.from_orientation, self.to, self.bound_for, self.drive);
        if !static_reachable {
            return PathfinderError::NoStaticRoute { from: self.from, to: self.to };
        }
        let had_horizon_rejection =
            self.stats.action_ready_past_horizon > 0 || self.stats.wait_successors_rejected_horizon > 0;
        if had_horizon_rejection {
            return PathfinderError::UnreachableWithinHorizon {
                max_wait: self.config.max_wait,
                from: self.from,
                to: self.to,
            };
        }
        if !self.terminal_dwell_possible_in_horizon {
            return PathfinderError::DestinationExceedsMaxWait { from: self.from, to: self.to };
        }
        PathfinderError::UnreachableWithinHorizon { max_wait: self.config.max_wait, from: self.from, to: self.to }
    }

    /// Build the accepted path from `came_from` + `node_meta`. Used by goal
    /// acceptance to assemble the return value.
    pub(super) fn build_accepted_path(&mut self, goal: SearchKey, current_g: SimDuration) -> Path {
        // A constant drive's speed is substituted as entry == exit velocity on
        // every Move waypoint, so visualized velocities match the cell cost. In
        // HtmKinematic mode reconstruct ignores it (velocities come from
        // NodeMeta), so resolving it unconditionally is safe.
        let waypoints = {
            let (arena, index) = (&self.scratch.arena, &self.scratch.index);
            reconstruct_waypoints(
                |k| index.get(&k).map(|&i| arena[i as usize].clone()),
                goal,
                self.drive.max_speed(),
                match self.drive.kind() {
                    MotionKind::Constant => MotionModel::ConstantSpeed,
                    MotionKind::Kinematic => MotionModel::HtmKinematic,
                },
            )
        };
        self.stats.path_length = waypoints.len();
        self.stats.search_duration_us = self.start_instant.elapsed().as_micros() as u64;
        Path { waypoints, travel_time: current_g }
    }

    /// Relax `key` to cost `next_g` with the given parent and metadata. If
    /// the state is new or this improves its best `g`, update the arena and
    /// push an open-heap entry ordered by `f = next_g + h` (with `h` the goal
    /// heuristic from `key.cell()`). Returns `true` when it pushed, so callers
    /// can gate per-family stats and trace emission.
    pub(super) fn relax(&mut self, key: SearchKey, next_g: SimDuration, parent: SearchKey, meta: NodeMeta) -> bool {
        let existing_idx = self.scratch.index.get(&key).copied();
        let existing_g = existing_idx.map(|i| self.scratch.arena[i as usize].g).unwrap_or(SimDuration::MAX);
        if next_g >= existing_g {
            return false;
        }
        let entry = SearchEntry { key, g: next_g, parent: Some(parent), meta };
        let idx = match existing_idx {
            Some(i) => {
                self.scratch.arena[i as usize] = entry;
                i
            }
            None => {
                let i = self.scratch.arena.len() as u32;
                self.scratch.arena.push(entry);
                self.scratch.index.insert(key, i);
                i
            }
        };
        let h = self.pf.heuristic_for(key.cell(), self.to, self.drive);
        self.scratch.open.push(AStarEntry { f: (next_g + h).as_ms() as u32, g: next_g.as_ms() as u32, idx });
        self.stats.peak_open_size = self.stats.peak_open_size.max(self.scratch.open.len() as u64);
        self.stats.states_pushed += 1;
        true
    }
}
