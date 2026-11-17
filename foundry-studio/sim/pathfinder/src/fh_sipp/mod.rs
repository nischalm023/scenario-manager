//! SIPP-A\* pathfinder that commits a journey in windows.
//!
//! Authoritative algorithm reference:
//! `sim/dessim/docs/fh-sipp-pathfinder-algo-draft.md`. Phase 1 contract:
//! `specs/putaway/turn-encoding-contract-phase1.md`.
//!
//! ## Module layout
//!
//! - [`pathfinder`] — `FHSippPathfinder` struct, constructors, accessors,
//!   heuristic, shared graph helpers.
//! - [`search`] — `Search` runner + `find_path_reserved` entry point.
//!   Owns the top-level loop, classification, and accepted-path emission.
//! - [`expand`] — per-state expansion dispatcher and goal-acceptance
//!   (`expand_state`, `try_accept_goal`).
//! - [`expand_rotate`] / [`expand_move`] / [`expand_wait`] — per-family
//!   successor emission. Each adds one method to `impl Search<'_>`.
//! - [`interface`] — `Pathfinder` trait impl (delegates to the inherent methods).
//! - [`allocator`] — `reserve_path` (refuses a target that is not a journey
//!   end, commits the path in windows, each one search and one commit).
//! - [`foreshortening`] — where a window ends short of the target: the
//!   queue cells feeding it and the held cell ahead.
//! - [`reconstruct`] — Waypoint chain reconstruction from `NodeMeta`.
//! - [`state`] / [`node`] / [`heap`] — search-state identity, per-state
//!   metadata, priority queue entry.

use desgraph::{CellId, WarehouseGraph};

use super::{DriveModel, Orientation};

mod allocator;
mod expand;
mod expand_move;
mod expand_rotate;
mod expand_wait;
mod foreshortening;
mod heap;
mod interface;
mod node;
mod pathfinder;
mod reconstruct;
mod search;
mod state;

// Tests live under `tests/`. Helpers, fixtures, and the `assert_trace!`
// macro are in `tests/helpers.rs`; each test file contains only
// `#[test]` functions.
#[cfg(test)]
mod tests;

pub use allocator::is_journey_end;
pub use pathfinder::FHSippPathfinder;

pub fn arrival_orientation(
    graph: &WarehouseGraph,
    from: CellId,
    to: CellId,
    drive: &dyn DriveModel,
) -> Option<Orientation> {
    let edge = graph.edge_between(from, to)?;
    reconstruct::edge_motion(graph, from, to, edge, drive).map(|motion| motion.arrival)
}
