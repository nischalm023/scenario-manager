//! Per-relay occupancy timeline.
//!
//! Tracks whether a relay is occupied or free, when it becomes free,
//! and which direction (inbound/outbound) holds the reservation.
//! Append-only: reservations are created open-ended, then finalized.

use crate::model::SimTime;

/// Direction of a relay reservation.
///
/// - Inbound: VTM deposits tote at relay for HTM pickup (retrieve).
/// - Outbound: HTM deposits tote at relay for VTM return (deposit).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReservationDirection {
    Inbound,
    Outbound,
}

/// Per-relay reservation state.
pub(super) struct RelayTimeline {
    available_at: SimTime,
    unfinalized: bool,
    direction: Option<ReservationDirection>,
    /// Start of the currently-open reservation (`reserve` → `finalize`/
    /// `cancel`), for recording the closed slot-hold interval. `None` when the
    /// slot is free.
    pending_start: Option<SimTime>,
    /// Finalized slot-hold intervals `[reserve, finalize)` in ms — the diving
    /// board's reservation-hold history (the slot's serialization window, an
    /// upper bound on physical tote dwell). Append-only; drives the desvis
    /// slot-occupancy heat lanes. A still-open reservation isn't here (it's
    /// `pending_start`); the timeline builder clamps that to the report horizon.
    occupancy_intervals: Vec<(u64, u64)>,
}

impl RelayTimeline {
    pub fn new() -> Self {
        Self {
            available_at: SimTime::ZERO,
            unfinalized: false,
            direction: None,
            pending_start: None,
            occupancy_intervals: Vec::new(),
        }
    }

    pub fn available_at(&self) -> SimTime {
        self.available_at
    }

    /// Whether this relay has a reservation that hasn't been finalized yet.
    pub fn is_unfinalized(&self) -> bool {
        self.unfinalized
    }

    pub fn direction(&self) -> Option<ReservationDirection> {
        self.direction
    }

    /// Finalized slot-occupancy intervals `[start_ms, end_ms)`.
    pub fn occupancy_intervals(&self) -> &[(u64, u64)] {
        &self.occupancy_intervals
    }

    /// Start of a still-open reservation, if any (unfinalized at read time).
    pub fn pending_start(&self) -> Option<SimTime> {
        self.pending_start
    }

    pub fn reserve(&mut self, direction: ReservationDirection, start: SimTime) {
        self.unfinalized = true;
        self.available_at = SimTime::EOT;
        self.direction = Some(direction);
        self.pending_start = Some(start);
    }

    pub fn finalize(&mut self, end: SimTime) {
        self.unfinalized = false;
        self.available_at = end;
        self.direction = None;
        // Record the closed slot-occupancy interval. Guard start <= end so a
        // degenerate finalize can't emit an inverted interval.
        if let Some(start) = self.pending_start.take() {
            if start.as_ms() <= end.as_ms() {
                self.occupancy_intervals.push((start.as_ms(), end.as_ms()));
            }
        }
    }

    /// Undo a `reserve` — relay becomes available at `start` again. The
    /// aborted reservation never occupied the slot, so nothing is recorded.
    pub fn cancel(&mut self, start: SimTime) {
        self.unfinalized = false;
        self.available_at = start;
        self.direction = None;
        self.pending_start = None;
    }
}

#[cfg(test)]
mod tests {
    use super::{RelayTimeline, ReservationDirection};
    use crate::model::SimTime;

    #[test]
    fn finalize_records_the_occupancy_interval() {
        let mut tl = RelayTimeline::new();
        tl.reserve(ReservationDirection::Inbound, SimTime::ms(100));
        assert!(tl.pending_start().is_some());
        tl.finalize(SimTime::ms(450));
        assert_eq!(tl.occupancy_intervals(), &[(100, 450)]);
        assert!(tl.pending_start().is_none());
    }

    #[test]
    fn cancel_records_nothing_and_clears_pending() {
        let mut tl = RelayTimeline::new();
        tl.reserve(ReservationDirection::Outbound, SimTime::ms(200));
        tl.cancel(SimTime::ms(200));
        assert!(tl.occupancy_intervals().is_empty());
        assert!(tl.pending_start().is_none());
    }

    #[test]
    fn open_reservation_is_left_pending_for_horizon_clamp() {
        let mut tl = RelayTimeline::new();
        tl.reserve(ReservationDirection::Inbound, SimTime::ms(900));
        // Never finalized: no closed interval, but the builder can read the
        // pending start and clamp it to the report horizon.
        assert!(tl.occupancy_intervals().is_empty());
        assert_eq!(tl.pending_start(), Some(SimTime::ms(900)));
    }
}
