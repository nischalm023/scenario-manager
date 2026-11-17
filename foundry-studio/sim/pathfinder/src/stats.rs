//! Per-run A* search statistics.
//!
//! Returned alongside the path from `find_path` so callers can measure
//! pathfinder performance precisely. Each window of a `reserve_path` call is
//! one search, so these are a window's statistics; `reserve_path` returns
//! the statistics of the window that ends on the target.
//!
//! On small maps, every field should be inspectable in tests:
//! "A* expanded exactly 12 states, queried earliest_available 8 times,
//!  emitted 2 pause nodes, peak open-heap 5."

/// Statistics from a single `find_path` invocation.
#[derive(Clone, Debug, Default)]
pub struct SearchStats {
    // --- A* core ---
    /// Total states popped from the open set.
    pub states_expanded: u64,
    /// Total states pushed to the open set (including duplicates).
    pub states_pushed: u64,
    /// A* iterations (pop + expand cycles).
    pub iterations: u64,
    /// Maximum size the open heap reached during the search. Useful for
    /// per-call memory budget and "this query blew up" detection under
    /// full-horizon search.
    pub peak_open_size: u64,

    // --- Successor generation ---
    /// Total successors pushed: Move + Rotate + explicit Wait. A bookkeeping
    /// total for Step 5 metrics; `pause_nodes_emitted` and
    /// `wait_successors_emitted` are informative subsets of this count.
    pub successors_emitted: u64,
    /// Move successors skipped due to edge traversability.
    pub moves_filtered_traversability: u64,
    /// Turn successors skipped in timed mode (handled as explicit states).
    pub turns_skipped_timed: u64,

    // --- Reservation checks ---
    /// Times `earliest_available` was called on the oracle.
    pub reservation_queries: u64,
    /// Times `entry_intervals` was called on the oracle. Tracks the cost of
    /// canonical safe-interval enumeration separately from exact admission
    /// checks (`reservation_queries`).
    pub entry_intervals_calls: u64,
    /// Queries that returned Some(t) == after (no conflict).
    pub reservation_free: u64,
    /// Queries that returned Some(t) > after (delayed / pause node).
    pub reservation_delayed: u64,
    /// Queries that returned None (permanent hold).
    pub reservation_permanent: u64,

    // --- Wait / pause nodes ---
    /// Move successors that included an implicit wait at the source cell
    /// (`m_in_start > current_g_t`). Subset of `successors_emitted`.
    pub pause_nodes_emitted: u64,
    /// Wait-in-place nodes emitted (last resort when all neighbors blocked).
    pub wait_in_place_emitted: u64,
    /// Source-cell hold checks that failed (robot can't wait at current cell).
    pub source_hold_blocked: u64,
    /// Legacy windowed-SIPP metric. Counted only by the frozen
    /// `windowed_sipp` reference module; fh_sipp single-shot uses
    /// `action_ready_past_horizon` instead.
    pub delayed_move_past_wait_limit: u64,
    /// Successors rejected because their action-ready end time would exceed
    /// `sim_start + max_wait`. Per spec §4: explicit Wait, Rotate, Move
    /// successors and goal acceptance must all stay inside the planning
    /// horizon. Counts Move m_in_end > horizon, Rotate action_end > horizon,
    /// and goal-acceptance current_g > horizon. Wait-end-past-horizon is
    /// counted separately in `wait_successors_rejected_horizon`.
    pub action_ready_past_horizon: u64,

    // --- Event-driven Wait successors (single-shot SIPP) ---
    /// Goal-cell event times considered as wait targets across the search
    /// (current_g_t + interval expansions).
    pub wait_events_considered: u64,
    /// Explicit Wait successors actually pushed to the heap. Subset of
    /// `successors_emitted`.
    pub wait_successors_emitted: u64,
    /// Wait candidates rejected because the wait would exit the source-cell
    /// safe interval (predecessor's current SippState.interval).
    pub wait_successors_rejected_source_interval: u64,
    /// Wait candidates rejected because the wait would exceed
    /// `sim_start + max_wait` (planning horizon).
    pub wait_successors_rejected_horizon: u64,

    // --- Edge validation (single-shot) ---
    /// Move successors rejected because the directed edge had a conflicting
    /// reservation over `[m_in_start, m_in_end)`.
    pub edge_conflicts_rejected: u64,

    // --- Legacy windowed-SIPP fields ---
    /// The collision_depth value passed in `CollisionConfig`. Retained as
    /// a passthrough for legacy consumers; not consulted by the single-shot
    /// search.
    pub collision_depth: u32,
    /// Always 0 under single-shot search (no spatial-only fallback past
    /// depth). Retained for legacy consumers.
    pub cells_beyond_depth: u64,

    // --- Commit (filled by reserve_path, not find_path) ---
    /// Total cells in the planned path.
    pub path_length: usize,
    /// Cells successfully committed by commit_path.
    pub cells_committed: usize,
    /// Whether the optimistic tail (beyond C) committed without conflict.
    pub optimistic_tail_committed: bool,

    // --- Timing ---
    /// Wall-clock duration of the A* search (microseconds).
    pub search_duration_us: u64,
}
