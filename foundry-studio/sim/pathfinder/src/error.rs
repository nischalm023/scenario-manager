//! Pathfinding error types.

use std::error::Error;

use desgraph::{CellId, CellKind};

use crate::SimDuration;

/// Errors from `Pathfinder::reserve_path`.
#[derive(Debug, thiserror::Error)]
pub enum PathfinderError {
    /// Generic "no path" — retained for the spatial-only `find_path`. The
    /// reservation-aware search prefers the more specific `NoStaticRoute` /
    /// `UnreachableWithinHorizon` / `DestinationExceedsMaxWait` variants.
    #[error("no path exists from {from:?} to {to:?}")]
    Unreachable { from: CellId, to: CellId },
    /// The static graph has no route from `from` to `to` for the requested
    /// robot type, cargo state, and zone constraints. No amount of waiting,
    /// rerouting, or horizon extension would change the outcome.
    ///
    /// Detected by running a spatial-only reachability fallback after the
    /// reservation-aware search exhausts.
    #[error("no static route from {from:?} to {to:?} for the configured robot/cargo/zones")]
    NoStaticRoute { from: CellId, to: CellId },
    /// A feasible plan may exist but it requires a reservation event or
    /// action-ready time beyond `sim_start + max_wait`. A tripwire, not a
    /// deferral: `reserve_path` itself awaits an open-ended hold on the
    /// target or on a queue cell feeding it, so a caller should not retry
    /// this. An open-ended hold elsewhere on the only route can still return
    /// it.
    ///
    /// Preferred over `DestinationExceedsMaxWait` whenever uncertainty remains
    /// about whether a longer horizon would expose a valid plan.
    #[error(
        "no plan within max_wait horizon ({max_wait}) from {from:?} to {to:?} - this probably means gaps are being created, or there's some imbalance in how things are being scheduled"
    )]
    UnreachableWithinHorizon { max_wait: SimDuration, from: CellId, to: CellId },
    /// Destination cell cannot accept the requested terminal reservation within
    /// this task's `max_wait` policy.
    ///
    /// For open-ended terminal holds (`dest_dwell = None`), this means the
    /// destination cell's trailing free interval begins after the configured
    /// horizon. Another robot's open-ended hold on it is awaited by
    /// `reserve_path` while the robot's empty-floor route reaches it; with no
    /// such route the call can still return this. For a finite
    /// `dest_dwell` the interval may also not exist. Returned only when
    /// destination-cell interval enumeration proves this before route timing
    /// is considered.
    #[error("destination {to:?} exceeds max_wait for terminal reservation from {from:?}")]
    DestinationExceedsMaxWait { from: CellId, to: CellId },
    #[error("commit conflict not resolved after retries from {from:?} to {to:?}")]
    CommitConflict { from: CellId, to: CellId },
    #[error("zero-length path: from == to at {cell:?}")]
    ZeroLength { cell: CellId },
    #[error("iteration limit exceeded from {from:?} to {to:?}")]
    IterationLimit { from: CellId, to: CellId },
    #[error("dest_dwell=MAX requires an eligible holding cell; {cell:?} is not eligible")]
    MaxDwellNonHolding { cell: CellId },
    /// The target's kind is not one a journey may end on (see
    /// `is_journey_end`). The rule (2026-10-05): a task never targets a
    /// non-destination cell with `reserve_path` to stage its way to its
    /// destination; the queue cells a journey stops on are `reserve_path`'s
    /// own windows. An invariant
    /// breach, not a deferral: nothing retries it, so the run aborts.
    #[error(
        "{cell:?} is a {kind:?} cell, not a journey end: reserve_path targets the task's destination, and an arrival \
         is scheduled by reserve_path itself, never by a task targeting cells on the way"
    )]
    NotAJourneyEnd { cell: CellId, kind: CellKind },
    /// The caller could not take a window that `reserve_path` had committed
    /// short of the target (`WindowSink::committed`); the journey stops.
    #[error("the caller refused a committed window: {0}")]
    WindowRefused(#[source] Box<dyn Error + Send + Sync>),
    /// No window could be committed at the journey's start (`held`, the
    /// cells on the robot's route held open-ended, left no queue cell to
    /// reach), and the caller would not let the robot wait on `origin`
    /// meanwhile (`WindowSink::allows_origin_wait`). Nothing of the journey
    /// was committed: the robot's hold on `origin` is as the caller left it.
    #[error(
        "no window from {origin:?} can be committed at the journey's start while {held:?} are held open-ended, and \
         the caller refused to let the robot wait on {origin:?}"
    )]
    OriginWaitRefused { origin: CellId, held: Vec<CellId> },
}
