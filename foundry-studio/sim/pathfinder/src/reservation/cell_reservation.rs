//! CellReservation — identity-bearing reservation value for cell occupancy.
//!
//! Every reservation in the table carries a unique ReservationId allocated
//! by the table on insert. External code holds a `CellReservation` as
//! identity for release / extend operations.

use std::fmt;

use desgraph::CellId;

use crate::RobotId;

use crate::TimeRange;

/// Opaque unique identifier for a cell reservation.
///
/// Allocated by the table via a monotonic counter. Callers never construct
/// one directly — they receive `CellReservation` values from the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ReservationId(u64);

impl ReservationId {
    pub(super) fn new(n: u64) -> Self {
        Self(n)
    }
}

impl fmt::Display for ReservationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "R#{}", self.0)
    }
}

/// A reservation on a single cell for a time window, owned by a single robot.
///
/// External code uses `&CellReservation` as identity for release / extend
/// operations. Fields are private — use the accessors.
#[derive(Clone, Debug)]
pub struct CellReservation {
    id: ReservationId,
    cell: CellId,
    robot: RobotId,
    window: TimeRange,
}

impl fmt::Display for CellReservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} @ cell{} bot{} {}", self.id, self.cell.0, self.robot.0, self.window)
    }
}

impl CellReservation {
    pub(super) fn new(id: ReservationId, cell: CellId, robot: RobotId, window: TimeRange) -> Self {
        Self { id, cell, robot, window }
    }

    pub fn id(&self) -> ReservationId {
        self.id
    }
    pub fn cell(&self) -> CellId {
        self.cell
    }
    pub fn robot(&self) -> RobotId {
        self.robot
    }
    pub fn window(&self) -> TimeRange {
        self.window
    }
    pub fn is_open_ended(&self) -> bool {
        self.window.end().is_none()
    }

    /// Update the window end in place — used by commit_windows when finalizing
    /// an extended open-ended reservation. Crate-private.
    pub(super) fn set_window(&mut self, window: TimeRange) {
        self.window = window;
    }
}
