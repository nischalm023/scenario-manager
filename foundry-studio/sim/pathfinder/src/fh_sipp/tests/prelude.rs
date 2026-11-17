//! Single-import bundle for fh_sipp test files.
//!
//! Each test file does `use super::prelude::*;` and gets:
//! - tiny constructors (`t`, `ms`, `c`, `r`)
//! - graph + pathfinder fixtures
//! - `TraceCollector` + `assert_trace!` + `assert_waypoints!`
//! - waypoint constructors (`path_start`, `mv`, `rotate`, `wait`, `path_end`)
//! - all trace event payload structs (via `pathfinder_trace::prelude`)
//! - the model / pathfinding / desgraph types every test reaches for

pub(super) use super::super::reconstruct::direction_between;
pub(super) use super::helpers::*;

pub(super) use crate::path::{CommitPlan, ReservationRequest};
pub(super) use crate::pathfinder_trace::prelude::*;
pub(super) use crate::{
    CollisionConfig, Constant, DriveModel, HtmKinematics, MotionModel, Orientation, Pathfinder, PathfinderError,
    Trapezoidal, WaypointAction,
};

pub(super) use crate::TimeRange;
pub(super) use crate::reservation::ReservationTable;
pub(super) use crate::reservation::oracle::{ReservationOracle, TableOracle};
pub(super) use crate::reservation::test::helpers::{CommitBlockingOracle, ScriptedConflicts};
pub(super) use crate::{RobotId, SimDuration, SimTime};
// Bare-name aliases for the sentinel constants — `MAX` for
// `SimDuration::MAX`, `EOT` for `SimTime::EOT`. Custom `Debug` on each type
// emits the bare-name form, so paste-back from panic output requires these
// in scope. (Rust does not let you `use Type::ASSOC_CONST` for struct
// associated constants — module-level aliases are the workaround.)
pub(super) const MAX: SimDuration = SimDuration::MAX;
pub(super) const EOT: SimTime = SimTime::EOT;

pub(super) use desgraph::graph::{DestGate, Edge, WarehouseGraphBuilder};
pub(super) use desgraph::types::{CellId, CellKind, EdgeTraversability, HtmTraversal, RobotType};
pub(super) use desgraph::{HtmDriveDirection, WarehouseGraph};
