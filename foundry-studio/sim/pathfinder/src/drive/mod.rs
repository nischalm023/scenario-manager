//! How a robot drives on the floor, as the search asks it: a [`DriveModel`] prices a robot's
//! moves and turns and says which edges and turns the robot may take, so the search holds
//! neither kinematic parameters nor admissibility rules of its own. [`Constant`] and
//! [`Trapezoidal`] are the two models runs use today.

pub(crate) mod kinematics;
mod model;

pub use model::{Constant, Trapezoidal};

use desgraph::types::{CellId, ZoneId};
use desgraph::{Cell, Edge};

use crate::SimDuration;
use kinematics::Velocity;

/// Whether a model's costs depend on the speed a move enters at. A pathfinder that assumes
/// linear costs accepts only `Constant`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotionKind {
    Constant,
    Kinematic,
}

/// One robot type in one load state, driving on the floor.
pub trait DriveModel: Send + Sync {
    fn kind(&self) -> MotionKind;

    // TODO: remove once the cruise target and waypoint speeds come from other queries.
    fn max_speed(&self) -> Velocity;

    // TODO: rename
    fn kinematic_cost(
        &self,
        entry_v: Velocity,
        target_exit_v: Velocity,
        cell_length_mm: u64,
    ) -> (SimDuration, Velocity);

    /// A turn in place by `angle_millidegrees`, at the robot's turning rate.
    // TODO: angle unit type
    fn rotation_cost(&self, angle_millidegrees: u32) -> SimDuration;

    /// A 180° turn in place in one action, when the robot has one; `None` leaves it to two quarter
    /// turns.
    // TODO: rename
    fn reversal_duration_for(&self) -> Option<SimDuration>;

    /// A lower bound on the time to cover `distance_mm`, for the search's heuristic: never more
    /// than `kinematic_cost` charges for that distance.
    fn min_travel_time(&self, distance_mm: u64) -> SimDuration;

    /// Whether this robot may take `edge` on its way to `dest`, which lies in `dest_zone`. The
    /// search asks before it takes any edge: the edge's traversal flags must admit the robot's
    /// type in its load state, the edge's destination gate must admit `dest`, and a VTM may
    /// cross a zone-gated edge only toward a destination in that zone.
    fn may_traverse(&self, edge: &Edge, dest: CellId, dest_zone: ZoneId) -> bool;

    /// Whether this robot may turn in place on `cell`.
    fn may_rotate_on(&self, cell: &Cell) -> bool;

    /// Whether this robot drives `edge` in reverse, its chassis facing away from its travel.
    fn reverses_on(&self, edge: &Edge) -> bool;
}
