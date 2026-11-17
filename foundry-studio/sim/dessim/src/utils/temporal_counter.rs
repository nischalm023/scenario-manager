//! Temporal step-function counter with O(log n) point queries.
//!
//! Tracks overlapping spans [start, end) as cumulative occupancy.
//! Sorted Vec of (SimTime, cumulative_count). Insertions patch the
//! tail forward of the insertion point — cheap when inserts are
//! near the frontier (typical for an offline solver).

use crate::model::SimTime;

/// Temporal occupancy counter. Exact, no bucketing.
///
/// Each entry is (time, cumulative_count_at_that_time).
/// Query at any time T: binary search, read value. O(log n).
/// Insert: binary search + tail patch. O(log n + k) where k = entries after insertion.
#[derive(Clone, Debug, Default)]
pub struct TemporalCounter {
    events: Vec<(SimTime, isize)>,
}

impl TemporalCounter {
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    /// Record a delta at a point in time. Inserts into sorted position
    /// and patches all subsequent cumulative values.
    pub fn record(&mut self, at: SimTime, delta: isize) {
        let idx = self.events.partition_point(|&(t, _)| t < at);
        let prev = if idx > 0 { self.events[idx - 1].1 } else { 0 };
        self.events.insert(idx, (at, prev + delta));
        for i in (idx + 1)..self.events.len() {
            self.events[i].1 += delta;
        }
    }

    /// Cumulative count at time T. O(log n).
    pub fn count_at(&self, t: SimTime) -> isize {
        let idx = self.events.partition_point(|&(time, _)| time <= t);
        if idx == 0 { 0 } else { self.events[idx - 1].1 }
    }

    // TODO remove add_span, add_open, close_open, cancel_open and rename record to add
    // this is just stupid
    /// Record a span [start, end): +1 at start, -1 at end.
    pub fn add_span(&mut self, start: SimTime, end: SimTime) {
        self.record(start, 1);
        self.record(end, -1);
    }

    /// Open-ended span: +1 at start, no end yet.
    pub fn add_open(&mut self, start: SimTime) {
        self.record(start, 1);
    }

    /// Close an open-ended span: -1 at end.
    pub fn close_open(&mut self, end: SimTime) {
        self.record(end, -1);
    }

    /// Cancel an open-ended span: -1 at start (undo add_open).
    pub fn cancel_open(&mut self, start: SimTime) {
        self.record(start, -1);
    }

    /// Frontier: the latest event time and cumulative count there.
    /// This is the state at the leading edge of planned time — the count
    /// persists to infinity since no later events exist.
    pub fn frontier(&self) -> (SimTime, isize) {
        self.events.last().map(|&(t, c)| (t, c)).unwrap_or((SimTime::ZERO, 0))
    }

    /// Event timestamps strictly after `after`.
    pub fn event_times_after(&self, after: SimTime) -> impl Iterator<Item = SimTime> + '_ {
        self.events_after(after).iter().map(|&(t, _)| t)
    }

    /// Events `(time, cumulative_count)` strictly after `after`. Returned
    /// directly as a slice so callers can walk in lockstep with another
    /// counter's events without intermediate allocation.
    pub fn events_after(&self, after: SimTime) -> &[(SimTime, isize)] {
        let start = self.events.partition_point(|&(t, _)| t <= after);
        &self.events[start..]
    }

    /// Latest cumulative count at or after `at`.
    pub fn max_count_from(&self, at: SimTime) -> isize {
        let start = self.events.partition_point(|&(t, _)| t <= at);
        self.events[start..].iter().map(|&(_, count)| count).fold(self.count_at(at), isize::max)
    }

    /// Earliest sim-time at or after `not_before` where a *prospective*
    /// new `+1` would not push count over `cap` at any future event in the
    /// currently-recorded state. Returns `None` if the latest recorded
    /// event already has count ≥ cap.
    ///
    /// Conservative: assumes the new span is open-ended (no matching `-1`
    /// recorded yet). Stricter than `count_at(not_before) < cap` —
    /// future already-planned events at-or-above cap push the answer
    /// further forward.
    pub fn earliest_new_inflight_start(&self, cap: isize, not_before: SimTime) -> Option<SimTime> {
        if cap <= 0 {
            return None;
        }

        let mut candidate = not_before;
        let start = self.events.partition_point(|&(t, _)| t <= not_before);

        if self.count_at(not_before) >= cap {
            let Some(&(next_time, _)) = self.events.get(start) else {
                return None;
            };
            candidate = candidate.max(next_time);
        }

        for i in start..self.events.len() {
            let (_, count) = self.events[i];
            // If this event starts a capped interval, the new inflight start
            // must be after the next event that changes the count. If it is
            // the final event, capacity never re-opens in the current plan.
            if count >= cap {
                let Some(&(next_time, _)) = self.events.get(i + 1) else {
                    return None;
                };
                candidate = candidate.max(next_time);
            }
        }

        if self.max_count_from(candidate) < cap { Some(candidate) } else { None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SimDuration;

    fn t(ms: u64) -> SimTime {
        SimTime::ms(ms)
    }

    #[test]
    fn empty() {
        let c = TemporalCounter::new();
        assert_eq!(c.count_at(t(0)), 0);
        assert_eq!(c.count_at(t(99999)), 0);
        assert_eq!(c.frontier(), (SimTime::ZERO, 0));
    }

    #[test]
    fn single_span() {
        let mut c = TemporalCounter::new();
        c.add_span(t(2000), t(5000));
        assert_eq!(c.count_at(t(1999)), 0);
        assert_eq!(c.count_at(t(2000)), 1);
        assert_eq!(c.count_at(t(4999)), 1);
        assert_eq!(c.count_at(t(5000)), 0);
        assert_eq!(c.frontier(), (t(5000), 0));
    }

    #[test]
    fn overlapping_spans() {
        let mut c = TemporalCounter::new();
        c.add_span(t(1000), t(5000));
        c.add_span(t(3000), t(8000));
        assert_eq!(c.count_at(t(0)), 0);
        assert_eq!(c.count_at(t(2000)), 1);
        assert_eq!(c.count_at(t(3000)), 2);
        assert_eq!(c.count_at(t(4999)), 2);
        assert_eq!(c.count_at(t(5000)), 1);
        assert_eq!(c.count_at(t(8000)), 0);
    }

    #[test]
    fn open_then_close() {
        let mut c = TemporalCounter::new();
        c.add_open(t(2000));
        assert_eq!(c.count_at(t(5000)), 1);
        assert_eq!(c.count_at(t(99999)), 1);
        assert_eq!(c.frontier(), (t(2000), 1));

        c.close_open(t(6000));
        assert_eq!(c.count_at(t(5000)), 1);
        assert_eq!(c.count_at(t(6000)), 0);
        assert_eq!(c.frontier(), (t(6000), 0));
    }

    #[test]
    fn multiple_open_close() {
        let mut c = TemporalCounter::new();
        c.add_open(t(1000));
        c.add_open(t(3000));
        assert_eq!(c.count_at(t(4000)), 2);

        c.close_open(t(5000));
        assert_eq!(c.count_at(t(4000)), 2);
        assert_eq!(c.count_at(t(5000)), 1);

        c.close_open(t(8000));
        assert_eq!(c.count_at(t(7000)), 1);
        assert_eq!(c.count_at(t(8000)), 0);
    }

    #[test]
    fn insert_in_past() {
        let mut c = TemporalCounter::new();
        c.add_span(t(5000), t(10000));
        assert_eq!(c.count_at(t(7000)), 1);

        c.add_span(t(2000), t(7000));
        assert_eq!(c.count_at(t(1000)), 0);
        assert_eq!(c.count_at(t(3000)), 1);
        assert_eq!(c.count_at(t(6000)), 2);
        assert_eq!(c.count_at(t(7000)), 1);
        assert_eq!(c.count_at(t(10000)), 0);
    }

    #[test]
    fn frontier_with_opens() {
        let mut c = TemporalCounter::new();
        c.add_open(t(100));
        c.add_open(t(200));
        assert_eq!(c.frontier(), (t(200), 2));
    }

    #[test]
    fn frontier_after_mixed() {
        let mut c = TemporalCounter::new();
        c.add_open(t(100));
        c.add_open(t(200));
        c.add_open(t(300));
        assert_eq!(c.frontier(), (t(300), 3));
        c.close_open(t(400));
        c.close_open(t(500));
        assert_eq!(c.frontier(), (t(500), 1));
        c.close_open(t(600));
        assert_eq!(c.frontier(), (t(600), 0));
    }

    #[test]
    fn record_at_zero() {
        let mut c = TemporalCounter::new();
        c.record(SimTime::ZERO, 5);
        assert_eq!(c.count_at(SimTime::ZERO), 5);
        assert_eq!(c.frontier(), (SimTime::ZERO, 5));
    }

    #[test]
    fn event_times_after() {
        let mut c = TemporalCounter::new();
        c.add_span(t(1000), t(5000));
        c.add_span(t(3000), t(8000));
        let times: Vec<SimTime> = c.event_times_after(t(2000)).collect();
        assert_eq!(times, vec![t(3000), t(5000), t(8000)]);
    }

    #[test]
    fn max_count_from_includes_current_and_future() {
        let mut c = TemporalCounter::new();
        c.add_span(t(1000), t(5000));
        c.add_span(t(3000), t(8000));
        assert_eq!(c.max_count_from(t(0)), 2);
        assert_eq!(c.max_count_from(t(4000)), 2);
        assert_eq!(c.max_count_from(t(5000)), 1);
        assert_eq!(c.max_count_from(t(8000)), 0);
    }

    #[test]
    fn earliest_new_inflight_start_skips_capped_future_intervals() {
        let mut c = TemporalCounter::new();
        c.add_span(t(1000), t(5000));
        c.add_span(t(3000), t(8000));

        assert_eq!(c.earliest_new_inflight_start(2, t(0)), Some(t(5000)));
        assert_eq!(c.earliest_new_inflight_start(2, t(5000)), Some(t(5000)));
        assert_eq!(c.earliest_new_inflight_start(3, t(0)), Some(t(0)));
    }

    #[test]
    fn earliest_new_inflight_start_returns_none_when_final_interval_is_capped() {
        let mut c = TemporalCounter::new();
        c.add_open(t(1000));
        assert_eq!(c.earliest_new_inflight_start(1, t(0)), None);

        c.close_open(t(5000));
        assert_eq!(c.earliest_new_inflight_start(1, t(0)), Some(t(5000)));
    }

    // ------------------------------------------------------------------
    // Empirical correctness: `earliest_new_inflight_start` as a CONWIP cap.
    //
    // Bears out the user's specific worries: brief below-cap windows,
    // overshoot magnitude, sequential releases. Each test simulates
    // releasing one or more chains at the returned time, then asserts
    // count stays at-or-below cap throughout.
    // ------------------------------------------------------------------

    /// Max observable cumulative count, checked at each distinct event
    /// time via `count_at` (post-all-events-at-T). This excludes
    /// representation-artifact "spikes" at coincident events: when a +1
    /// and -1 record at the same time T, the data structure stores
    /// intermediate cumulative values, but the count over any real
    /// interval (between distinct event times) is determined by the
    /// post-all-events value, which is what `count_at` returns.
    fn peak_observable(c: &TemporalCounter) -> isize {
        use std::collections::BTreeSet;
        let times: BTreeSet<SimTime> = c.events.iter().map(|&(t, _)| t).collect();
        times.iter().map(|&t| c.count_at(t)).max().unwrap_or(0)
    }

    #[test]
    fn release_at_returned_time_never_overshoots() {
        // A:[10,50), B:[20,60), C:[30,70). cap=3 means count peaks at 3.
        let mut c = TemporalCounter::new();
        c.add_span(t(10), t(50));
        c.add_span(t(20), t(60));
        c.add_span(t(30), t(70));
        assert_eq!(peak_observable(&c), 3);

        // Release a 4th chain D under cap=3. Should NOT be allowed
        // anywhere [30,50) (count=3 there), but allowed at 50 onward.
        let release_at = c.earliest_new_inflight_start(3, t(15)).expect("cap=3 should permit a release");
        assert_eq!(release_at, t(50), "earliest valid release is when count first drops below 3");

        // Simulate D's release: open-ended +1 at release_at.
        c.add_open(release_at);
        assert!(peak_observable(&c) <= 3, "post-release peak must stay ≤ cap; was {}", peak_observable(&c));
    }

    #[test]
    fn brief_below_window_does_not_trigger_early_release() {
        // count goes 0→1 at t=10, drops to 0 at t=20, jumps to 1 at t=30
        // and stays. Cap=1: brief [20,30) below window must be skipped
        // because adding an open-ended +1 there would overshoot at t=30.
        let mut c = TemporalCounter::new();
        c.add_span(t(10), t(20));
        c.add_open(t(30)); // open-ended (e.g. an in-flight chain not yet closed)

        let release_at = c.earliest_new_inflight_start(1, t(0));
        assert_eq!(release_at, None, "open-ended span at frontier means no future release possible");

        // Now close the open-ended span at t=100 to allow a future release.
        c.close_open(t(100));
        let release_at = c.earliest_new_inflight_start(1, t(0)).expect("after close, room exists");
        assert_eq!(release_at, t(100), "must wait past the second span; the brief [20,30) gap is not enough");

        c.add_open(release_at);
        assert!(peak_observable(&c) <= 1, "post-release peak must stay ≤ cap=1; was {}", peak_observable(&c));
    }

    #[test]
    fn overshoot_magnitude_is_bounded_by_one_per_release() {
        // Even at the most aggressive admission (cap=3 with count peaking at 3
        // somewhere), a single release adds exactly one. Verify the post-release
        // peak is at most cap (= old peak when limited by earliest_new_inflight_start).
        let mut c = TemporalCounter::new();
        c.add_span(t(10), t(50));
        c.add_span(t(20), t(60));
        c.add_span(t(30), t(70));
        let pre_peak = peak_observable(&c);
        assert_eq!(pre_peak, 3);

        let release_at = c.earliest_new_inflight_start(3, t(0)).expect("cap=3 permits release");
        c.add_open(release_at);

        let post_peak = peak_observable(&c);
        // The release added +1 going forward from release_at, but only to a
        // region where count was strictly below cap. Peak does NOT exceed cap.
        assert!(post_peak <= 3, "single release can never push peak above cap; peak was {}", post_peak);
        // And at most +1 anywhere — never +2.
        assert!(post_peak <= pre_peak + 1, "single release adds at most 1 to peak");
    }

    #[test]
    fn sequential_releases_via_earliest_new_inflight_start_never_overshoot() {
        // Simulate the demand-module loop: release N chains, each at the
        // earliest_new_inflight_start-recommended time, capped at K. Assert
        // count never exceeds K anywhere across all events.
        const K: isize = 3;
        let mut c = TemporalCounter::new();
        // Pre-existing committed work: three overlapping chains forming a
        // count peak of 3 in [30,50). After 70 the system is empty.
        c.add_span(t(10), t(50));
        c.add_span(t(20), t(60));
        c.add_span(t(30), t(70));

        // Release 5 more chains, each a 100ms-wide span (we close each one).
        let mut release_times = Vec::new();
        for i in 0..5 {
            let not_before = t(0); // for this test, no floor constraint
            let r = c
                .earliest_new_inflight_start(K, not_before)
                .unwrap_or_else(|| panic!("iteration {i}: should always find a slot under cap=3"));
            release_times.push(r);
            // Bracket as a real chain: +1 at release, -1 at release+100ms.
            c.add_span(r, r + SimDuration::ms(100));
        }

        let post_peak = peak_observable(&c);
        assert!(post_peak <= K, "sequential releases under cap={} must never overshoot; peak={}", K, post_peak);
        // Releases must be monotonically non-decreasing (each next release
        // happens at-or-after the previous, since cap stays at K).
        for w in release_times.windows(2) {
            assert!(w[1] >= w[0], "release times non-decreasing: {:?} → {:?}", w[0], w[1]);
        }
    }

    #[test]
    fn floor_clamps_returned_release_time() {
        // count is 0 throughout. earliest_new_inflight_start(cap, t) should
        // return t itself (no events at all means no constraint).
        let c = TemporalCounter::new();
        assert_eq!(c.earliest_new_inflight_start(1, t(500)), Some(t(500)));
        assert_eq!(c.earliest_new_inflight_start(1, t(0)), Some(t(0)));
    }

    #[test]
    fn not_before_after_last_capped_interval_returns_not_before() {
        // A:[10,50). cap=1 means [10,50) is at-or-above cap, [50,∞) is below.
        // not_before in the below region should be returned as-is.
        let mut c = TemporalCounter::new();
        c.add_span(t(10), t(50));
        assert_eq!(c.earliest_new_inflight_start(1, t(60)), Some(t(60)));
        assert_eq!(c.earliest_new_inflight_start(1, t(50)), Some(t(50)));
    }

    #[test]
    fn deep_past_insert_patches_tail() {
        let mut c = TemporalCounter::new();
        for i in 1..=5 {
            c.record(t(i * 1000), 1);
        }
        assert_eq!(c.count_at(t(3000)), 3);
        assert_eq!(c.count_at(t(5000)), 5);

        c.record(t(500), 10);
        assert_eq!(c.count_at(t(499)), 0);
        assert_eq!(c.count_at(t(500)), 10);
        assert_eq!(c.count_at(t(1000)), 11);
        assert_eq!(c.count_at(t(3000)), 13);
        assert_eq!(c.count_at(t(5000)), 15);
        assert_eq!(c.frontier(), (t(5000), 15));
    }
}
