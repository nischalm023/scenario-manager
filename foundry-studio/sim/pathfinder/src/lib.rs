//! Reservation-aware pathfinding for warehouse robots, pulled out of dessim:
//! orientation-aware A\* on the `WarehouseGraph`, the reservation table and
//! oracle it plans against, and the kinematics it prices moves with.
//!
//! The state space is `(CellId, Orientation, SafeIntervalId)` — each
//! physical cell has 4 orientation states, and SIPP dedup adds the
//! reservation-derived safe interval. Turning in place costs time. The drive
//! each call passes answers which edges and turns its robot may take.
//!
//! ## Architecture (SIPP, committed in windows)
//!
//! The active pathfinder is [`FHSippPathfinder`]. A `reserve_path` call
//! commits the journey in one or more windows of its path, each one
//! SIPP-A\* search and one atomic commit. Every successor is
//! reservation-validated against the oracle (cells and edges); each
//! window's `CommitPlan` is committed all-or-nothing by the oracle. There
//! is no optimistic tail: a window is always a whole committed path.
//!
//! Foreshortening: while another robot holds the target, or a queue cell on
//! the way to it, open-ended (its departure not yet scheduled), the window
//! ends on the deepest queue cell a search reaches, and the call awaits
//! (scheduler time) a commit touching a held cell on the robot's route before
//! the next window.
//!
//! Failure semantics: a search returns one of `NoStaticRoute`,
//! `UnreachableWithinHorizon`, or `DestinationExceedsMaxWait` per spec §4;
//! `reserve_path` returns one only once no window can be committed and no
//! open-ended hold on the target, or on a queue cell feeding it, on the
//! robot's route is left to await. Commit conflicts are
//! search/commit drift (a bug class) and surface as `CommitConflict`.
//!
//! ## Public API
//!
//! - `Pathfinder::find_path` — spatial-only A\* (no reservation checks).
//!   Used by `Cached` for travel-time estimates and ghost mode.
//! - `FHSippPathfinder::find_path_reserved` — SIPP-A\* against a
//!   `ReservationOracle`. Returns `(Path, SearchStats)`.
//! - `FHSippPathfinder::reserve_path` — per window, `find_path_reserved` +
//!   atomic `commit_path` against the oracle; each window that ends short
//!   of the target goes to the caller's [`WindowSink`] as it commits.
//! - `ReservationOracle` trait — injectable reservation checking + committing.
//! - `SearchStats` — per-run metrics for every A\* invocation.
//!
//! ## Key types
//!
//! - [`Orientation`] — cardinal direction a robot faces
//! - [`Path`] / [`Waypoint`] — planned path with per-action timing
//! - [`Pathfinder`] / [`FHSippPathfinder`] — owns graph topology and search
//! - [`SearchStats`] — per-run A\* metrics
//! - [`ReservationOracle`] — trait for reservation checking + committing
//! - [`CollisionConfig`] — parameters for reservation-aware pathfinding

mod drive;
mod error;
mod fh_sipp;
pub mod path;
pub mod pathfinder_trace;
pub mod reservation;
mod sequencer;
pub mod stats;
mod window;

pub use desunits::ids::RobotId;
pub use desunits::{SimDuration, SimTime, TimeRange};

pub use drive::kinematics::{HtmKinematics, KinematicParams};
pub use drive::{Constant, DriveModel, MotionKind, Trapezoidal};
pub use error::PathfinderError;
pub use fh_sipp::arrival_orientation;
pub use fh_sipp::{FHSippPathfinder, is_journey_end};
pub use sequencer::{Progress, RoundRobin, Sequencer, Turn, TurnGuard};
// The trapezoidal profile API the observer's `nav_handler` rasterizes HTM
// `Move` waypoints with: shared between the search and the observer.
pub use drive::kinematics::{TrapezoidalProfile, Velocity, trapezoid_position, trapezoidal_profile};
pub use path::{CommitPlan, MoveOp, Path, ReservationRequest, ReservedPath, Waypoint, WaypointAction};
pub use reservation::oracle::{CellStatus, GhostOracle, ReservationOracle, SafeIntervalId, TableOracle};
pub use reservation::table::{ReservationConflict, ReservationSnapshot, ReservationTable};
pub use reservation::{CellReservation, ReservationId};
pub use stats::SearchStats;
pub use window::WindowSink;

#[cfg(feature = "perf")]
macro_rules! perf_span {
    ($name:expr) => {
        let _span = tracing::info_span!($name).entered();
    };
}
#[cfg(not(feature = "perf"))]
macro_rules! perf_span {
    ($name:expr) => {};
}
pub(crate) use perf_span;

use async_trait::async_trait;

/// The planning interface a flow drives a robot through. Every call takes the
/// drive the robot moves with, which prices its moves and turns and answers
/// which edges it may take, where it may turn in place and where it drives in
/// reverse; the caller picks it. `reserve_path` awaits inside the pathfinder
/// for the floor to change instead of its caller looping around the call:
/// while another robot holds the target, or a queue cell on the way to it,
/// open-ended, it commits the journey in windows that each end on a queue
/// cell (`WindowSink`), and it returns once a window ends on the target.
/// `find_path` is the empty-floor route, kept for estimates, and stays
/// synchronous.
#[async_trait(?Send)]
pub trait Pathfinder: Send + Sync {
    fn find_path(
        &self,
        from: CellId,
        from_orientation: Orientation,
        to: CellId,
        drive: &dyn DriveModel,
        start: SimTime,
    ) -> Result<Path, PathfinderError>;

    /// Commits the journey from `from` to `to`, window by window, and
    /// returns the window that ends on `to`. Its limits:
    /// - Once a window has committed, a call that returns an error, or a
    ///   future dropped (it is not cancel-safe), leaves the robot holding its
    ///   last window's cell, open-ended.
    /// - A call that awaits goes on only once a commit touches a cell it
    ///   awaits (scheduler time); without one, as when two robots each await
    ///   the other's cell, the robot waits on its cell without end (simtime).
    /// - When several robots await one cell, the executor's polling order
    ///   decides which commits onto it first.
    ///
    /// A sink that refuses the wait on `from` (`WindowSink::allows_origin_wait`)
    /// gets `PathfinderError::OriginWaitRefused`, nothing committed, where the
    /// call would otherwise await (scheduler time) with its robot waiting on
    /// `from` (simtime).
    async fn reserve_path(
        &self,
        from: CellId,
        from_orientation: Orientation,
        to: CellId,
        drive: &dyn DriveModel,
        oracle: &dyn ReservationOracle,
        config: &CollisionConfig,
        windows: &mut dyn WindowSink,
    ) -> Result<ReservedPath, PathfinderError>;

    fn traverse_duration(&self) -> SimDuration;

    /// Honest free-travel time over a previously-found spatial path, driven
    /// by `drive`. Default impl returns the path's recorded constant-speed
    /// travel time. FH-SIPP walks the path with the supplied drive in both
    /// motion models, so an estimate's search speed does not set execution
    /// quotes. Used by demand-side quote computation.
    fn kinematic_travel_time(&self, path: &Path, _drive: &dyn DriveModel) -> SimDuration {
        path.travel_time
    }
}

use desgraph::CellId;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Orientation
// ---------------------------------------------------------------------------

mod orientation;
pub use orientation::{Heading, Orientation};

// ---------------------------------------------------------------------------
// MotionModel
// ---------------------------------------------------------------------------

/// Motion model selector for reservation-aware pathfinding.
///
/// `ConstantSpeed` (default): single-rate move duration = `cell_length / speed_mm_per_sec`.
/// Phase 1 behavior. Cheap, single-bucket search.
///
/// `HtmKinematic`: trapezoidal accel/cruise/decel for HTM robots. VTMs continue
/// using constant speed. Doubles per-move branching factor (cruise vs decelerate-
/// to-zero target velocities). Required for accurate rotate timing in dense
/// reservation conditions — vanilla SIPP is incomplete under bounded acceleration
/// (Ali & Yakovlev, AAAI 2023).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionModel {
    #[default]
    ConstantSpeed,
    HtmKinematic,
}

impl FromStr for MotionModel {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "constant_speed" | "constant" => Ok(MotionModel::ConstantSpeed),
            "htm_kinematic" | "kinematic" | "htm" => Ok(MotionModel::HtmKinematic),
            _ => Err(format!("unknown MotionModel: {s} (try constant_speed or htm_kinematic)")),
        }
    }
}

// ---------------------------------------------------------------------------
// CollisionConfig
// ---------------------------------------------------------------------------

/// Parameters for reservation-aware pathfinding, for a whole journey: each
/// window of its path is searched with them.
///
/// Passed alongside the `ReservationOracle` to
/// `FHSippPathfinder::reserve_path`. Per-search parameters; per-robot
/// timing comes from the drive passed beside it.
#[derive(Clone)]
pub struct CollisionConfig {
    /// The robot being planned for.
    pub self_robot: RobotId,
    /// Predecessor's open-ended hold at `from`, if any.
    ///
    /// Scope and semantics:
    /// - Applies at **cell 0** (`from`) of the planned path. The oracle
    ///   matches predecessors by reservation ID, so exclusion is a no-op
    ///   on any other cell.
    /// - The search excludes the predecessor from self-conflict checks on
    ///   its own cell; the planning state still uses the canonical safe
    ///   interval that contains `sim_start`.
    /// - `commit_path` **finalizes** the predecessor (closes its
    ///   open-ended window to the first move-out's `Move.end`) atomically
    ///   with the rest of the commit, rather than creating a new
    ///   reservation for cell 0. Direct handoff — no release/recreate.
    pub predecessor: Option<CellReservation>,
    /// The robot's own promoted destination row on the path's goal cell —
    /// the third reservation shape (task-model-v2 temporal model item 5):
    /// excluded AND requested. The search plans INTO the row (its
    /// open-ended hold must not block its own owner), and the commit
    /// subsumes it — moving its start right to the actual arrival,
    /// preserving id and open end — instead of creating a fresh goal
    /// reservation.
    ///
    /// Search-side only in this cut: the commit-side subsume wiring lands
    /// when family services start authoring destination rows at
    /// promotion. Until then every caller passes `None`; passing `Some`
    /// today would plan correctly and then CommitConflict at commit.
    pub destination: Option<CellReservation>,
    /// Absolute sim time when this path starts. Every window starts no earlier
    /// than the caller's barrier ([`WindowSink::barrier`]), and a later one no
    /// earlier than the robot's arrival on the queue cell it stands on.
    pub sim_start: SimTime,
    /// Dwell time at the destination cell. `None` = open-ended (successor
    /// task closes the reservation). `Some(d)` finite = finite hold for
    /// `d` after arrival. `Some(MAX)` requests indefinite holding and requires
    /// an eligible relay access or charging destination.
    pub dest_dwell: Option<SimDuration>,
    /// **Legacy** windowed-pathfinder field — ignored by
    /// `FHSippPathfinder`. Retained on the struct so production call sites
    /// don't need updates; remove once those callers are migrated.
    pub collision_depth: u32,
    /// **Legacy** windowed-pathfinder field — ignored. Not the window of a
    /// path that `reserve_path` commits when it foreshortens a journey.
    pub window_size: u32,
    /// **Legacy** windowed-pathfinder field — ignored.
    pub replan_interval: u32,
    /// Planning horizon of each window's search: it rejects any successor
    /// (Move/Rotate/Wait) whose action-ready time exceeds `sim_start +
    /// max_wait`. Reaching this cap returns `UnreachableWithinHorizon` or
    /// `DestinationExceedsMaxWait`, depending on where the policy cap binds.
    /// It never bounds the await for an open-ended hold between windows.
    pub max_wait: SimDuration,
    /// Maximum A\* iterations before returning `IterationLimit`.
    pub max_iterations: u64,
    /// Motion model selector. Defaults to `ConstantSpeed` (Phase 1 behavior).
    /// The caller picks the drive it passes beside this config to match; the
    /// search reads it only to decide whether a ghost oracle may take the
    /// spatial route and how to write waypoint speeds.
    pub motion_model: MotionModel,
}

impl CollisionConfig {
    /// The reservation to exclude from availability checks on `cell`:
    /// the robot's own destination row on its cell, the predecessor hold
    /// everywhere else. Exclusion matches by reservation id, which only
    /// bites on the row's own cell — selecting per cell is equivalent to
    /// excluding both rows everywhere, without widening the oracle
    /// signatures. When the predecessor IS the destination row (adoption:
    /// the robot already rests on the goal cell), the arms coincide.
    pub fn exclusion_for(&self, cell: CellId) -> Option<&CellReservation> {
        match &self.destination {
            Some(dest) if dest.cell() == cell => Some(dest),
            _ => self.predecessor.as_ref(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

// TODO move tests to pathfindings/tests/
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orientation_turns_to_self_is_zero() {
        for &o in &[Orientation::North, Orientation::East, Orientation::South, Orientation::West] {
            assert_eq!(o.turns_to(o), 0);
        }
    }

    #[test]
    fn orientation_90_degree_turns() {
        assert_eq!(Orientation::North.turns_to(Orientation::East), 1);
        assert_eq!(Orientation::East.turns_to(Orientation::South), 1);
        assert_eq!(Orientation::South.turns_to(Orientation::West), 1);
        assert_eq!(Orientation::West.turns_to(Orientation::North), 1);
        assert_eq!(Orientation::East.turns_to(Orientation::North), 1);
        assert_eq!(Orientation::South.turns_to(Orientation::East), 1);
        assert_eq!(Orientation::West.turns_to(Orientation::South), 1);
        assert_eq!(Orientation::North.turns_to(Orientation::West), 1);
    }

    #[test]
    fn orientation_180_degree_turns() {
        assert_eq!(Orientation::North.turns_to(Orientation::South), 2);
        assert_eq!(Orientation::South.turns_to(Orientation::North), 2);
        assert_eq!(Orientation::East.turns_to(Orientation::West), 2);
        assert_eq!(Orientation::West.turns_to(Orientation::East), 2);
    }
}
