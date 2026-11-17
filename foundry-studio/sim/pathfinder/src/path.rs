//! Within `pathfinding`, `path` is the SEARCH OUTPUT AND COMMIT INTENT layer:
//! what a completed search produced ([`Path`], [`Waypoint`],
//! [`WaypointAction`] — cell-by-cell geometry with per-action timing), and
//! what the reservation table is being asked to hold as a result
//! ([`ReservationRequest`], [`MoveOp`], [`CommitPlan`]).
//!
//! `CommitPlan` is the contract between the pathfinder and
//! `model::reservation`: the pathfinder composes one, the table applies it
//! all-or-nothing. Nothing here searches or commits — `fh_sipp/` owns the
//! former, the oracle the latter.

use std::fmt;

use desgraph::CellId;

use super::Orientation;
use super::SearchStats;
use crate::TimeRange;
use crate::reservation::cell_reservation::CellReservation;
use crate::{SimDuration, SimTime};

/// Result of a successful pathfinding query.
#[derive(Clone, Debug)]
pub struct Path {
    pub waypoints: Vec<Waypoint>,
    /// Action-ready time at goal minus PathStart.time. Excludes destination dwell.
    pub travel_time: SimDuration,
}

impl Path {
    pub fn empty() -> Self {
        Self { waypoints: vec![], travel_time: SimDuration::ZERO }
    }
}

/// A single phase-slice of the bot's action progression.
///
/// Waypoints are contiguous in time on possibly-different cells. Adjacent
/// waypoints' windows must not overlap at the chain level. A `Waypoint`
/// encodes a single physical action (Move/Rotate/Wait) or a chain marker
/// (PathStart/PathEnd). Reservation composition derives separate
/// `ReservationRequest`s; see `CommitPlan`.
#[derive(Clone, PartialEq)]
pub struct Waypoint {
    pub cell: CellId,
    /// Bot's facing as-of this waypoint (post-action for Rotate).
    pub orientation: Orientation,
    pub action: WaypointAction,
}

// Custom Debug emits constructor-form (`path_start(...)`, `mv(...)`,
// `rotate(...)`, `wait(...)`, `path_end(...)`) matching the test helpers in
// `sim/dessim/src/pathfinding/fh_sipp/tests/helpers.rs`. Panic output from
// `assert_waypoints!` pastes back into the test source verbatim.
impl fmt::Debug for Waypoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.action {
            WaypointAction::PathStart { time } => {
                write!(f, "path_start({:?}, {:?}, {:?})", self.cell, self.orientation, time)
            }
            WaypointAction::PathEnd { time } => {
                write!(f, "path_end({:?}, {:?}, {:?})", self.cell, self.orientation, time)
            }
            WaypointAction::Move { start, end, .. } => {
                write!(f, "mv({:?}, {:?}, {:?}, {:?})", self.cell, self.orientation, start, end)
            }
            WaypointAction::Rotate { start, end } => {
                write!(f, "rotate({:?}, {:?}, {:?}, {:?})", self.cell, self.orientation, start, end)
            }
            WaypointAction::Wait { start, end } => {
                write!(f, "wait({:?}, {:?}, {:?}, {:?})", self.cell, self.orientation, start, end)
            }
        }
    }
}

/// The action taken at a waypoint. Single source of truth for waypoint timing.
#[derive(Clone, Debug, PartialEq)]
pub enum WaypointAction {
    /// Marker: action chain begins here. Always wp[0].
    PathStart { time: SimTime },
    /// Bot transitions from prior cell to wp.cell. `start` = action-ready at
    /// prior cell (leading edge crosses boundary); `end` = action-ready at
    /// wp.cell (trailing edge fully arrived).
    ///
    /// `entry_velocity_mm_s` and `exit_velocity_mm_s` are the bot's real
    /// velocity (mm/s, not bucketed) at the move's start (= source cell
    /// center) and end (= destination cell center). In `MotionModel::HtmKinematic`
    /// mode they are the trapezoidal-profile endpoints selected by the
    /// search — and may legitimately be `(0.0, 0.0)` for a rest-to-rest
    /// peak profile (e.g. a goal-cell move where `force_decel` clamps the
    /// exit, or the first move out of a Start/Rotate/Wait predecessor whose
    /// exit velocity was zero). In `MotionModel::ConstantSpeed` mode both
    /// fields equal the pathfinder's configured `speed_mm_per_sec` regardless
    /// of any internal NodeMeta value.
    ///
    /// Consumers (e.g. the observer's playback rasterizer) MUST discriminate
    /// kinematic-vs-constant via the carried `MotionModel` (typically threaded
    /// through `MetricsConfig` or equivalent), not by comparing these field
    /// values — under `HtmKinematic` mode `entry == exit` is a real,
    /// physically-meaningful profile shape, not a sentinel.
    Move { start: SimTime, end: SimTime, entry_velocity_mm_s: f64, exit_velocity_mm_s: f64 },
    /// In-place rotation. `start` = prior waypoint's end; `end` = `start +
    /// rotate_duration`. Target orientation in `Waypoint.orientation`.
    Rotate { start: SimTime, end: SimTime },
    /// In-place wait. `start` = prior waypoint's end; `end` = `start +
    /// wait_duration`. Bot stays in cell, facing unchanged.
    Wait { start: SimTime, end: SimTime },
    /// Marker: action chain terminates here, leaves the terminal cell's
    /// reservation open-ended. Marker only — `time` is not a reservation end.
    PathEnd { time: SimTime },
}

impl WaypointAction {
    /// Start timestamp for any variant.
    pub fn start_time(&self) -> SimTime {
        match self {
            Self::PathStart { time } | Self::PathEnd { time } => *time,
            Self::Move { start, .. } | Self::Rotate { start, .. } | Self::Wait { start, .. } => *start,
        }
    }

    /// End timestamp for any variant.
    pub fn end_time(&self) -> SimTime {
        match self {
            Self::PathStart { time } | Self::PathEnd { time } => *time,
            Self::Move { end, .. } | Self::Rotate { end, .. } | Self::Wait { end, .. } => *end,
        }
    }

    /// Duration of the action. PathStart/PathEnd are zero-width markers.
    pub fn duration(&self) -> SimDuration {
        match self {
            Self::PathStart { .. } | Self::PathEnd { .. } => SimDuration::ZERO,
            Self::Move { start, end, .. } => *end - *start,
            Self::Rotate { start, end } | Self::Wait { start, end } => *end - *start,
        }
    }
}

impl Waypoint {
    /// Helpers to read action-variant timing without pattern-matching.
    pub fn start_time(&self) -> SimTime {
        self.action.start_time()
    }

    pub fn end_time(&self) -> SimTime {
        self.action.end_time()
    }

    pub fn duration(&self) -> SimDuration {
        self.action.duration()
    }
}

/// A request to the reservation table to hold a specific cell window for a
/// robot. Composed from a Waypoint chain (or directly constructed by
/// non-A\*-derived holds, e.g. VTM cycle holds).
#[derive(Clone, Debug, PartialEq)]
pub struct ReservationRequest {
    pub cell: CellId,
    /// The hold. Open-ended means a future Move closes it.
    ///
    /// Carrying a [`TimeRange`] rather than a `start` / `Option<SimTime> end`
    /// pair puts the `start <= end` check at construction, so a malformed
    /// request cannot reach the reservation table and be blamed on it.
    ///
    /// It does NOT put the check at the true origin. An inverted hold
    /// originates upstream in [`WaypointAction::Move`]'s unchecked
    /// `start`/`end` pair: `Epoch.start` comes from one Move's start and
    /// `Epoch.end` from a *later* Move's end, and
    /// [`WaypointAction::duration`] saturates an inverted Move to zero
    /// rather than complaining. Pushing `TimeRange` up into `WaypointAction`
    /// is what would make the check origin-local; until then this is the
    /// earliest single point, not the first.
    pub window: TimeRange,
}

impl ReservationRequest {
    pub fn new(cell: CellId, window: TimeRange) -> Self {
        Self { cell, window }
    }

    /// Build from a `start` plus an optional `end`, the shape waypoint-chain
    /// composition produces (`None` = a later Move closes the hold).
    ///
    /// # Panics
    /// If `end` precedes `start`. That is a path-composition bug, and this is
    /// deliberately the one place it surfaces, so the blame lands on the
    /// pathfinder rather than on the reservation table downstream.
    ///
    /// The panic is deliberate rather than an `Err`, and it is NOT the same
    /// call as the close-predecessor sites in `ReservationTable::reserve_path`
    /// (which do return `ReservationConflict`). Two reasons:
    ///
    /// - **No recovery exists to offer.** Both callers (`compose_commit_plan`,
    ///   `commit_plan_requests`) are infallible by signature, and their
    ///   contract is "turn a *valid* waypoint chain into requests". An
    ///   inverted hold means the chain is non-monotonic — `Epoch.end` comes
    ///   from a later Move's end or `PathEnd.time + dwell`, `start` from an
    ///   earlier Move's start. Routing that to `PathfinderError` would
    ///   present a search bug to the planner as an ordinary "no path" and
    ///   have it retry the same broken search.
    /// - **The alternative is silent index corruption, not benign
    ///   continuation.** `ReservationTable::create_reservation` inserts into
    ///   `CellLane` at the position sorted by `start`, then inserts
    ///   `end_key` at that same index. With `end < start` the `ends` array
    ///   goes non-monotonic, breaking documented lane invariant 2 that
    ///   `has_conflict` / `earliest_free` / `robot_has_reservation` all
    ///   binary-search against. Debug builds trip `assert_lane_invariants`;
    ///   release builds — the mandated build mode — silently return wrong
    ///   conflict answers from then on.
    ///
    /// The `match` rather than `unwrap_or_else` is deliberate: `#[track_caller]`
    /// does not propagate into a closure, so a `panic!` there would report
    /// this file instead of the composition site that built the bad hold.
    #[track_caller]
    pub fn spanning(cell: CellId, start: SimTime, end: Option<SimTime>) -> Self {
        match TimeRange::from_parts(start, end) {
            Ok(window) => Self { cell, window },
            Err(e) => panic!("path composition produced an inverted hold for {cell:?}: {e}"),
        }
    }
}

impl MoveOp {
    /// Departure from `source_cell`.
    pub fn depart(&self) -> SimTime {
        self.window.start()
    }

    /// Arrival at `dest_cell`.
    ///
    /// # Panics
    /// If the window is open-ended. A composed Move always has a known end, so
    /// this is unreachable in practice — but `TimeRange` can represent
    /// open-ended, so this is the closest the type gets to enforcing the
    /// "always closed" invariant the field doc states. Prefer comparing
    /// `window.end() == Some(t)` where an `Option` is natural.
    #[track_caller]
    pub fn arrive(&self) -> SimTime {
        match self.window.end() {
            Some(end) => end,
            None => panic!("MoveOp window is open-ended; a composed Move must have a known end"),
        }
    }
}

/// Provenance for a `ReservationRequest`: the producing Move action.
/// Populated only when the request was derived from a Move waypoint
/// (transit cell entry); `None` for fresh-path start-cell opens or
/// direct holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveOp {
    pub source_cell: CellId,
    pub dest_cell: CellId,
    /// The traversal, `[depart, arrive)`. Always closed: a Move that has
    /// begun has a known end.
    pub window: TimeRange,
}

/// Full commit intent for a path. Bundles the new reservations to open AND
/// the predecessor close-time, so the table can apply both atomically.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommitPlan {
    /// New reservations to open, in path order. One per cell advance (one per
    /// Move-into-X plus the start cell if a fresh path opens it).
    pub requests: Vec<ReservationRequest>,
    /// For each request, the producing Move action (None for non-Move-derived
    /// requests like the start-cell open in a fresh path). Same length as
    /// `requests`.
    pub move_ops: Vec<Option<MoveOp>>,
    /// Time at which the predecessor's open hold should be finalized. `None`
    /// if no predecessor (fresh path) OR if the path doesn't move out of the
    /// predecessor cell (degenerate: extension that only rotates/waits).
    pub close_predecessor_at: Option<SimTime>,
}

impl CommitPlan {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Single-request direct hold (no path composition). All `move_ops` are
    /// `None`; `close_predecessor_at` is `None`.
    pub fn single(req: ReservationRequest) -> Self {
        Self { requests: vec![req], move_ops: vec![None], close_predecessor_at: None }
    }

    /// Build a plan from N direct holds with no path composition.
    pub fn direct(requests: Vec<ReservationRequest>) -> Self {
        let move_ops = vec![None; requests.len()];
        Self { requests, move_ops, close_predecessor_at: None }
    }

    pub fn len(&self) -> usize {
        self.requests.len()
    }

    pub fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }
}

/// Output of `reserve_path`: path + committed reservations.
#[derive(Debug)]
pub struct ReservedPath {
    pub path: Path,
    pub stats: SearchStats,
    pub committed_reservations: Vec<CellReservation>,
}
