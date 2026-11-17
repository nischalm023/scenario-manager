//! A\* decision tracing for diagnostics and test assertions.
//!
//! Gated behind `pathfinder-trace` feature OR test mode. When enabled,
//! the A\* search calls a hook for each decision event. Tests set the
//! hook to collect events; the hook can also pause or inspect oracle
//! state synchronously.
//!
//! Each event variant has a corresponding struct (with `From<X> for
//! TraceEvent`) so test assertion lists can construct events without the
//! `TraceEvent::` qualifier — the [`prelude`] module bundles them all
//! for one-line `use crate::pathfinder_trace::prelude::*;` imports.
//!
//! TODO: once oracle is a field on Pathfinder, remove the oracle arg
//! from the hook — the hook can just query self.oracle directly.

use std::fmt;

use super::Orientation;
use crate::reservation::oracle::ReservationOracle;
use crate::{RobotId, SimDuration, SimTime};
use desgraph::CellId;

// ----------------------------------------------------------------------------
// Event payload structs
// ----------------------------------------------------------------------------

/// A\* expanded a node (popped from open set).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expand {
    pub cell: CellId,
    pub orientation: Orientation,
    pub g: SimDuration,
    pub step: u32,
}

/// Rotate-in-place successor pushed to the open set.
/// Tuple shape: `(cell, from_orient, to_orient, g, wait)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rotate(pub CellId, pub Orientation, pub Orientation, pub SimDuration, pub SimDuration);

/// Move successor pushed to the open set.
/// Tuple shape: `(from, to, g, wait)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Move(pub CellId, pub CellId, pub SimDuration, pub SimDuration);

/// Rotate-in-place blocked by the source-hold check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RotateBlocked {
    pub cell: CellId,
    pub from_orient: Orientation,
    pub to_orient: Orientation,
    pub hold_start: SimTime,
    pub hold_needed: SimDuration,
}

/// Reservation query result for a move to a neighbor cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveReservationCheck {
    pub from: CellId,
    pub to: CellId,
    pub check_from: SimTime,
    pub check_dur: SimDuration,
    pub result: ReservationOutcome,
}

/// Source-hold check for a delayed move (waiting at the current cell).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceHoldCheck {
    pub cell: CellId,
    pub hold_start: SimTime,
    pub hold_needed: SimDuration,
    pub passed: bool,
}

/// Cell window committed to the reservation table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub robot: RobotId,
    pub cell: CellId,
    pub enter: SimTime,
    pub exit: SimTime,
}

/// Edge reservation committed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitEdge {
    pub robot: RobotId,
    pub from: CellId,
    pub to: CellId,
    pub start: SimTime,
    pub end: SimTime,
}

/// Search exhausted (no path found).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exhausted {
    pub from: CellId,
    pub to: CellId,
    pub iterations: u64,
    pub expanded: u64,
    pub turns_skipped: u64,
    pub src_hold_blocked: u64,
    pub delayed_count: u64,
    pub permanent_blocked: u64,
}

/// Extension from frontier (iterative deepening, legacy windowed search).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extension {
    pub frontier_cell: CellId,
    pub frontier_time: SimTime,
    pub dest: CellId,
    pub committed_cells: usize,
}

/// Canonical safe intervals queried for a neighbor cell.
#[derive(Clone, PartialEq, Eq)]
pub struct EntryIntervals {
    pub cell: CellId,
    pub intervals: Vec<(SimTime, Option<SimTime>)>,
}

// Custom Debug suffixes the `intervals` Vec with `.into()` so the panic
// output pastes back into a `Vec<...>`-typed field via `[T; N] → Vec<T>`.
impl fmt::Debug for EntryIntervals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EntryIntervals {{ cell: {:?}, intervals: {:?}.into() }}", self.cell, self.intervals)
    }
}

/// What the reservation oracle returned for a cell query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReservationOutcome {
    Free,
    Permanent,
    Delayed { free_at: SimTime },
}

// ----------------------------------------------------------------------------
// Tagged-union event type
// ----------------------------------------------------------------------------

/// A single decision event from the A\* search. Each variant wraps a
/// dedicated struct; `From<X> for TraceEvent` is implemented for every
/// payload so callers and tests can construct events without the
/// `TraceEvent::` qualifier and let `.into()` lift to this enum.
#[derive(Clone, PartialEq, Eq)]
pub enum TraceEvent {
    Expand(Expand),
    Rotate(Rotate),
    Move(Move),
    RotateBlocked(RotateBlocked),
    MoveReservationCheck(MoveReservationCheck),
    SourceHoldCheck(SourceHoldCheck),
    Commit(Commit),
    CommitEdge(CommitEdge),
    Exhausted(Exhausted),
    Extension(Extension),
    EntryIntervals(EntryIntervals),
}

// Custom Debug forwards to the inner payload's Debug — drops the redundant
// `Expand(...)` / `Rotate(...)` variant wrapper so panic output from
// `assert_trace!` is paste-ready (each entry is just the payload struct,
// matching the form used in test source).
impl fmt::Debug for TraceEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TraceEvent::Expand(p) => p.fmt(f),
            TraceEvent::Rotate(p) => p.fmt(f),
            TraceEvent::Move(p) => p.fmt(f),
            TraceEvent::RotateBlocked(p) => p.fmt(f),
            TraceEvent::MoveReservationCheck(p) => p.fmt(f),
            TraceEvent::SourceHoldCheck(p) => p.fmt(f),
            TraceEvent::Commit(p) => p.fmt(f),
            TraceEvent::CommitEdge(p) => p.fmt(f),
            TraceEvent::Exhausted(p) => p.fmt(f),
            TraceEvent::Extension(p) => p.fmt(f),
            TraceEvent::EntryIntervals(p) => p.fmt(f),
        }
    }
}

macro_rules! impl_from_payload {
    ($($variant:ident),* $(,)?) => {
        $(
            impl From<$variant> for TraceEvent {
                fn from(payload: $variant) -> Self { TraceEvent::$variant(payload) }
            }
        )*
    };
}

impl_from_payload!(
    Expand,
    Rotate,
    Move,
    RotateBlocked,
    MoveReservationCheck,
    SourceHoldCheck,
    Commit,
    CommitEdge,
    Exhausted,
    Extension,
    EntryIntervals,
);

// ----------------------------------------------------------------------------
// Prelude — bring all event types into scope at once.
// ----------------------------------------------------------------------------

/// Single import for production trace_emit! sites and test assertions.
///
/// ```ignore
/// use crate::pathfinder_trace::prelude::*;
/// ```
pub mod prelude {
    pub use super::{
        Commit, CommitEdge, EntryIntervals, Exhausted, Expand, Extension, Move, MoveReservationCheck,
        ReservationOutcome, Rotate, RotateBlocked, SourceHoldCheck, TraceEvent,
    };
}

// ----------------------------------------------------------------------------
// Hook + emit machinery
// ----------------------------------------------------------------------------

/// Hook type: called synchronously for each trace event during A\* search.
/// Receives the event and an optional oracle reference for state
/// introspection.
/// TODO: remove oracle arg once oracle is a field on Pathfinder.
pub type TraceHook = Box<dyn Fn(&TraceEvent, Option<&dyn ReservationOracle>) + Send + Sync>;

/// Emit a trace event. No-op when neither `pathfinder-trace` nor `test`
/// is set. Two forms:
///   `trace_emit!(self, event)`           — no oracle available
///   `trace_emit!(self, event, oracle)`   — passes oracle to hook
#[cfg(any(feature = "pathfinder-trace", test))]
macro_rules! trace_emit {
    ($self:expr, $event:expr) => {
        if let Some(ref hook) = $self.trace_hook {
            let event: TraceEvent = $event.into();
            hook(&event, None);
        }
    };
    ($self:expr, $event:expr, $oracle:expr) => {
        if let Some(ref hook) = $self.trace_hook {
            let event: TraceEvent = $event.into();
            hook(&event, Some($oracle));
        }
    };
}

#[cfg(not(any(feature = "pathfinder-trace", test)))]
macro_rules! trace_emit {
    ($self:expr, $event:expr) => {};
    ($self:expr, $event:expr, $oracle:expr) => {};
}

pub(crate) use trace_emit;

/// Trace hook that prints events to stderr.
pub fn stderr_trace_hook(event: &TraceEvent, _oracle: Option<&dyn ReservationOracle>) {
    eprintln!("[A*] {event:?}");
}
