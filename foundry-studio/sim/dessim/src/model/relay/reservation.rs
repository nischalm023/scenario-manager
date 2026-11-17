//! RAII relay reservation handle.
//!
//! `RelayReservation<R>` is returned by `RelayService::reserve_relay_for_*`.
//! It must be consumed via `finalize(self, end)` or `cancel*()` — a leaked
//! reservation permanently degrades zone headroom, so dropping without
//! either is recorded for the planner's end-of-run leak audit.
//!
//! For type-erased storage (e.g. ResourceHandle), box as
//! `Box<dyn Any + Send + Sync>` and downcast at the consumer.

use desgraph::{RelayId, ZoneId};

use crate::error::ReservationError;
use crate::model::reservation::cell_reservation::CellReservation;
use crate::model::{SimDuration, SimTime, ToteId};

use super::RelayService;
use super::inventory_index::RackSlotReservation;

/// Which floor pinned a retrieve reservation's `start` time, where
/// `start = max(relay_slot_avail, after, tote_available_at)`. The VTM that
/// services the reservation idles until `start − cycle`, so recording the
/// binding floor lets a forward VTM gap be attributed to its real upstream
/// cause rather than a generic "awaiting relay." `Slot` mirrors
/// `slot_wait > 0` exactly. Not meaningful for deposit reservations (their
/// VTM-return gap is attributed to HTM deposit timing instead), which carry
/// `start_cause: None`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelayStartCause {
    /// `relay_slot_avail` bound — a prior reservation still held the slot.
    Slot,
    /// `after` (station demand signal) bound — the work is not due yet.
    Demand,
    /// Tote frontier bound — the tote is not yet physically back in the rack.
    Tote,
}

/// RAII relay reservation handle.
///
/// `R` is `ToteId` (retrieve) or `RackSlot` (deposit). Deposits also hold
/// their destination rack slot until inventory records physical storage.
/// Retrieval's tote claim is handled separately by its accepting task.
pub struct RelayReservation<R: Send + Sync + 'static> {
    pub recommendation: R,
    pub relay_id: RelayId,
    pub zone_id: ZoneId,
    pub start: SimTime,
    /// Pure diving-board slot wait for this reservation: how much the relay
    /// slot's prior occupancy pushed `start` beyond all non-slot constraints.
    /// Zero when the slot was not the binding constraint. The committing
    /// demand module emits this as the empty-slot wait metric.
    pub slot_wait: SimDuration,
    /// Which floor pinned `start`. `Some` for retrieve reservations (read by
    /// `VtmForward` to classify the forward idle gap); `None` for deposit.
    pub start_cause: Option<RelayStartCause>,
    pub(super) service: RelayService,
    pub(super) finalized: bool,
    pub(super) rack_slot_reservation: Option<RackSlotReservation>,
}

impl<R: Send + Sync + 'static> RelayReservation<R> {
    /// Close the open-ended reservation with an actual end time.
    /// Consumes self — double-finalize is a compile error.
    pub fn finalize(mut self, end: SimTime) -> Result<(), ReservationError> {
        self.rack_slot_reservation.take();
        self.finalized = true;
        self.service.finalize_inner(self.relay_id, self.zone_id, end, None)
    }

    /// Cancel the reservation without finalizing. Releases the relay slot
    /// so another task can use it. Use when the task cannot proceed (e.g.,
    /// destination cell blocked) and needs to return NotReady.
    pub fn cancel(mut self) {
        self.rack_slot_reservation.take();
        self.finalized = true; // suppress the leak record
        self.service.cancel_inner(self.relay_id, self.zone_id, self.start);
    }

    /// Cancel, but never release the slot before `floor`. Use when a deposit
    /// has already been scheduled onto this relay (tote inbound or landed):
    /// freeing the slot at `start` would let another VTM deposit on top of
    /// the pending tote. Releases at `start.max(floor)`.
    pub fn cancel_after(mut self, floor: SimTime) {
        self.rack_slot_reservation.take();
        self.finalized = true; // suppress the leak record
        self.service.cancel_inner(self.relay_id, self.zone_id, self.start.max(floor));
    }
}

impl RelayReservation<ToteId> {
    /// End tote occupancy at pickup while retaining inbound admission through
    /// the HTM's unresolved source hold. A real path commit bounds that exact
    /// hold and automatically makes the credit available again.
    pub fn finalize_pickup(mut self, end: SimTime, source: &CellReservation) -> Result<(), ReservationError> {
        self.finalized = true;
        self.service.finalize_inner(self.relay_id, self.zone_id, end, Some(source))
    }
}

impl<R: Send + Sync + 'static + std::fmt::Debug> std::fmt::Debug for RelayReservation<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayReservation")
            .field("recommendation", &self.recommendation)
            .field("relay_id", &self.relay_id)
            .field("zone_id", &self.zone_id)
            .field("start", &self.start)
            .finish()
    }
}

// Dropping unfinalized is a leak: recorded for the planner's end-of-run
// audit rather than panicking — a stopped run's teardown legitimately
// drops mid-chain reservations, and Drop never panics.
impl<R: Send + Sync + 'static> Drop for RelayReservation<R> {
    fn drop(&mut self) {
        if !self.finalized {
            self.service.record_leak(self.relay_id, self.zone_id, self.start);
        }
    }
}
