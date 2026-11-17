//! The caller's side of a foreshortened `reserve_path`. A journey whose
//! target, or a queue cell on its way, another robot holds open-ended is
//! committed in windows of its path: each ends on a queue cell the robot holds
//! open-ended, and the next starts there once the hold ahead is bounded. The
//! caller hears of each window as it is committed (scheduler time), so the
//! robot's plan and the facts of that window go out then; the call returns
//! only the window that ends on the target. When no window can be committed
//! at the journey's start, the robot would wait (simtime) on its origin while
//! the call awaits (scheduler time); the caller is asked first, since the
//! stay on its origin is the caller's to answer for.

use std::error::Error;

use crate::{CellReservation, ReservedPath, SimTime};

/// Where a foreshortened `reserve_path` hands each window that ends short of
/// the target, how it learns the earliest simtime a later window may start,
/// and whether the robot may wait on its origin.
pub trait WindowSink {
    /// The caller's barrier (simtime): nothing may be committed behind it, so
    /// a window committed after an await starts no earlier than this.
    fn barrier(&self) -> SimTime;

    /// A window that ends short of the target, just committed (scheduler
    /// time), and the robot's hold it closed (the journey's predecessor, or
    /// the previous window's last hold). An error stops the journey:
    /// `reserve_path` returns it as `PathfinderError::WindowRefused`.
    fn committed(
        &mut self,
        window: &ReservedPath,
        closed: Option<&CellReservation>,
    ) -> Result<(), Box<dyn Error + Send + Sync>>;

    /// Whether the caller allows the robot to wait (simtime) on the cell its
    /// journey starts from while the call awaits (scheduler time), nothing of
    /// the journey committed. Asked before each such await; `false` makes
    /// `reserve_path` return `PathfinderError::OriginWaitRefused` having
    /// committed nothing, so the robot's stay on its origin stays the
    /// caller's to end. Yes by default, as for a robot whose stay on its
    /// origin nothing else claims.
    fn allows_origin_wait(&self) -> bool {
        true
    }
}
