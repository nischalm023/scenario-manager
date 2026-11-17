//! Monotonic-time counter — adds must be at or after the frontier.
//!
//! A regular counter that also tracks the time of its latest write.
//! Adds at a time strictly before the latest write return an error
//! instead of silently mis-counting. Use this when the schedule is
//! gap-filled strictly forward in time (e.g. station presentation
//! occupancy planned by an offline solver). No history is kept — if
//! you need historical queries, use a [`super::temporal_counter::TemporalCounter`].
//!
//! Mirrors [`super::frontier_sorted_set::FrontierSortedSet`]'s
//! safety pattern but for cumulative counts. Unlike that struct's
//! silent no-op on stale writes, this one returns an error so the
//! bug surfaces instead of hiding.

use crate::model::SimTime;

/// Returned when [`FrontierCounter::add`] is called with `at` strictly
/// before the latest recorded event time.
#[derive(Debug, thiserror::Error)]
#[error("FrontierCounter add at {at} is before frontier at {frontier}")]
pub struct StaleAdd {
    pub at: SimTime,
    pub frontier: SimTime,
}

#[derive(Clone, Debug, Default)]
pub struct FrontierCounter {
    frontier_time: SimTime,
    count: isize,
}

impl FrontierCounter {
    pub fn new() -> Self {
        Self { frontier_time: SimTime::ZERO, count: 0 }
    }

    /// Apply `delta` at `at`, advancing the frontier time. Errors if
    /// `at < frontier_time` and leaves state unchanged.
    pub fn add(&mut self, at: SimTime, delta: isize) -> Result<(), StaleAdd> {
        if at < self.frontier_time {
            return Err(StaleAdd { at, frontier: self.frontier_time });
        }
        self.frontier_time = at;
        self.count += delta;
        Ok(())
    }

    pub fn count(&self) -> isize {
        self.count
    }

    pub fn frontier_time(&self) -> SimTime {
        self.frontier_time
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(ms: u64) -> SimTime {
        SimTime::ms(ms)
    }

    #[test]
    fn empty() {
        let c = FrontierCounter::new();
        assert_eq!(c.count(), 0);
        assert_eq!(c.frontier_time(), SimTime::ZERO);
    }

    #[test]
    fn forward_adds_accumulate() {
        let mut c = FrontierCounter::new();
        c.add(t(100), 1).unwrap();
        assert_eq!(c.count(), 1);
        c.add(t(200), 1).unwrap();
        assert_eq!(c.count(), 2);
        c.add(t(300), -1).unwrap();
        assert_eq!(c.count(), 1);
        assert_eq!(c.frontier_time(), t(300));
    }

    #[test]
    fn ties_at_same_time_ok() {
        let mut c = FrontierCounter::new();
        c.add(t(100), 1).unwrap();
        c.add(t(100), 1).unwrap();
        c.add(t(100), -1).unwrap();
        assert_eq!(c.count(), 1);
        assert_eq!(c.frontier_time(), t(100));
    }

    #[test]
    fn stale_add_errors_and_does_not_mutate() {
        let mut c = FrontierCounter::new();
        c.add(t(100), 1).unwrap();
        let err = c.add(t(50), -1).unwrap_err();
        assert_eq!(err.at, t(50));
        assert_eq!(err.frontier, t(100));
        // State unchanged.
        assert_eq!(c.count(), 1);
        assert_eq!(c.frontier_time(), t(100));
    }

    #[test]
    fn negative_count_allowed() {
        // The counter does not assert non-negativity; callers that need
        // it must check themselves.
        let mut c = FrontierCounter::new();
        c.add(t(100), -1).unwrap();
        assert_eq!(c.count(), -1);
    }
}
