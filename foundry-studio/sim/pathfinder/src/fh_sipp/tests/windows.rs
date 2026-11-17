//! Driving the async `reserve_path` from a synchronous test: a sink that
//! records the windows it is handed, and a call polled until it awaits.

use std::cell::{Cell, RefCell};
use std::error::Error;
use std::future::Future;
use std::pin::pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use super::super::FHSippPathfinder;
use crate::reservation::oracle::ReservationOracle;
use crate::{
    CellReservation, CollisionConfig, DriveModel, Orientation, PathfinderError, ReservedPath, SimTime, WindowSink,
};
use desgraph::types::CellId;

/// The windows a journey committed short of its target, in order, with the
/// hold each closed, and the barrier the test sets; clones share them, so a
/// test reads them while the call holds its own clone. `refuses_origin_wait`
/// makes it a sink that refuses the wait on the journey's origin.
#[derive(Clone, Default)]
pub(in crate::fh_sipp) struct RecordedWindows {
    pub(in crate::fh_sipp) barrier: Rc<Cell<SimTime>>,
    pub(in crate::fh_sipp) windows: Rc<RefCell<Vec<ReservedPath>>>,
    pub(in crate::fh_sipp) closed: Rc<RefCell<Vec<Option<CellReservation>>>>,
    pub(in crate::fh_sipp) refuses_origin_wait: bool,
}

impl WindowSink for RecordedWindows {
    fn barrier(&self) -> SimTime {
        self.barrier.get()
    }

    fn committed(
        &mut self,
        window: &ReservedPath,
        closed: Option<&CellReservation>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.closed.borrow_mut().push(closed.cloned());
        self.windows.borrow_mut().push(ReservedPath {
            path: window.path.clone(),
            stats: window.stats.clone(),
            committed_reservations: window.committed_reservations.clone(),
        });
        Ok(())
    }

    fn allows_origin_wait(&self) -> bool {
        !self.refuses_origin_wait
    }
}

/// Poll `future` once: its output, or `None` while it awaits.
pub(in crate::fh_sipp) fn poll_once<F: Future>(future: F) -> Option<F::Output> {
    let mut future = pin!(future);
    match future.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(output) => Some(output),
        Poll::Pending => None,
    }
}

impl FHSippPathfinder {
    /// `reserve_path` for a journey whose way is not held: it panics if the
    /// call commits a window short of `to` or awaits.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::fh_sipp) fn reserve_path_without_waiting(
        &self,
        from: CellId,
        from_orientation: Orientation,
        to: CellId,
        drive: &dyn DriveModel,
        oracle: &dyn ReservationOracle,
        config: &CollisionConfig,
    ) -> Result<ReservedPath, PathfinderError> {
        let mut windows = RecordedWindows::default();
        let result = poll_once(self.reserve_path(from, from_orientation, to, drive, oracle, config, &mut windows))
            .unwrap_or_else(|| panic!("the journey from {from:?} to {to:?} awaits a held cell"));
        assert!(windows.windows.borrow().is_empty(), "the journey from {from:?} to {to:?} stopped short");
        result
    }
}
