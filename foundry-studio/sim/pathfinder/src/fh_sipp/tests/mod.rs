//! Single-shot Phase 1 test suite for `FHSippPathfinder`.
//!
//! Conventions:
//! - Each test file is just `#[test]` functions; constructors, fixtures,
//!   and macros live in [`helpers`] and are bundled (with the trace
//!   prelude and the model/pathfinding/desgraph types every test
//!   reaches for) into [`prelude`]. Test files open with
//!   `use super::prelude::*;` and pull nothing else.
//! - `assert_waypoints!` narrates the returned path chronologically.
//! - `assert_trace!` locks in the search-decision sequence end-to-end.
//!   Both are exact-match. When the search over-explores (see e.g.
//!   `basic::grid_path_with_turn` and the L1 heuristic) the expansion
//!   fan-out is visible in the diff, so any heuristic / tie-break
//!   change has to confront it.
//!
//! Test conventions are pinned in `sim/dessim/docs/test-conventions.md`.

mod helpers;
mod prelude;
mod windows;

mod basic;
mod constant_speed_htm;
mod corridor;
mod curved_links;
mod delayed_move;
mod dest_dwell;
mod dest_only;
mod directional_lane;
mod foreshortening;
mod foreshortening_search;
mod freeform;
mod journey_end;
mod kinematic;
mod kinematic_goal_zero_velocity;
mod reverse_drive;
mod stacking;
mod turn;
mod unladen_only_edges;
mod vtm_traversal_flag;
mod wandering;
