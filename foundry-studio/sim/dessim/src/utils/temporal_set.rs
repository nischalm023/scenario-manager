//! Temporal set with frontier caching.
//!
//! Tracks which keys have a property over time via add/remove intervals.
//! Supports efficient point-in-time queries: "which keys are active at T?"
//!
//! # Fast path (forward queries)
//!
//! Maintains a materialized frontier set at `frontier_time`. Queries at
//! T >= frontier_time advance the frontier by processing new adds/removes
//! in [frontier_time, T]. Cost: O(events in the window).
//!
//! # Slow path (backward queries)
//!
//! Queries at T < frontier_time rebuild the frontier from scratch by
//! scanning all intervals. Cost: O(total intervals). The frontier is
//! then valid at T for subsequent forward queries.
//!
//! # Usage
//!
//! ```ignore
//! let mut idx: TemporalSet<ToteId> = TemporalSet::new();
//! idx.add(ToteId(1), SimTime::ZERO);          // tote 1 enters set at T=0
//! idx.remove(&ToteId(1), SimTime::ms(2000));   // tote 1 leaves set at T=2000
//! idx.add(ToteId(1), SimTime::ms(8000));       // tote 1 re-enters at T=8000
//!
//! assert!(idx.active_at(SimTime::ms(1000)).contains(&ToteId(1)));
//! assert!(!idx.active_at(SimTime::ms(3000)).contains(&ToteId(1)));
//! assert!(idx.active_at(SimTime::ms(9000)).contains(&ToteId(1)));
//! ```

use std::collections::BTreeMap;
use std::hash::Hash;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::model::SimTime;

/// A time interval during which a key is active in the set.
#[derive(Clone, Debug)]
pub struct Interval<K> {
    pub key: K,
    pub from: SimTime,
    /// None = open-ended (still active).
    pub until: Option<SimTime>,
}

/// Temporal set index with frontier caching.
pub struct TemporalSet<K: Eq + Hash + Clone> {
    /// All intervals, append-mostly, sorted by `from`.
    intervals: Vec<Interval<K>>,

    /// Key → index of the currently-open interval. At most one open
    /// interval per key at any time.
    open: FxHashMap<K, usize>,

    /// Materialized set of active keys at `frontier_time`.
    frontier: FxHashSet<K>,
    frontier_time: SimTime,

    /// Next unprocessed interval index for frontier advancement.
    /// Intervals before this cursor have already been added to the
    /// frontier (if their `from <= frontier_time`).
    open_cursor: usize,

    /// Pending close events, sorted by time. Processed during frontier
    /// advancement. Rebuilt on backward queries.
    pending_closes: BTreeMap<SimTime, Vec<K>>,
}

impl<K: Eq + Hash + Clone> TemporalSet<K> {
    pub fn new() -> Self {
        Self {
            intervals: Vec::new(),
            open: FxHashMap::default(),
            frontier: FxHashSet::default(),
            frontier_time: SimTime::ZERO,
            open_cursor: 0,
            pending_closes: BTreeMap::new(),
        }
    }

    /// Add a key to the set at time `from`. Opens an interval.
    ///
    /// Panics if the key already has an open interval (must remove first).
    pub fn add(&mut self, key: K, from: SimTime) {
        assert!(!self.open.contains_key(&key), "TemporalSet: key already has an open interval");
        let idx = self.intervals.len();
        self.intervals.push(Interval { key: key.clone(), from, until: None });
        self.open.insert(key.clone(), idx);

        // If this add is at or before the frontier, apply it immediately.
        if from <= self.frontier_time {
            self.frontier.insert(key);
        }
        // Otherwise it will be picked up during frontier advancement.
    }

    /// Remove a key from the set at time `at`. Closes the open interval.
    ///
    /// No-op if the key doesn't have an open interval (idempotent for
    /// cases where the key was never added or already removed).
    pub fn remove(&mut self, key: &K, at: SimTime) {
        let idx = match self.open.remove(key) {
            Some(idx) => idx,
            None => return,
        };
        self.intervals[idx].until = Some(at);

        if at <= self.frontier_time {
            // Close is at or behind the frontier — remove immediately.
            self.frontier.remove(key);
        } else {
            // Close is ahead — queue for frontier advancement.
            self.pending_closes.entry(at).or_default().push(key.clone());
        }
    }

    /// Returns the set of active keys at time `at`.
    ///
    /// Fast path if `at >= frontier_time` (incremental advancement).
    /// Slow path if `at < frontier_time` (full rebuild).
    pub fn active_at(&mut self, at: SimTime) -> &FxHashSet<K> {
        if at >= self.frontier_time {
            self.advance_to(at);
        } else {
            self.rebuild_at(at);
        }
        &self.frontier
    }

    /// Number of currently active keys at the frontier time.
    pub fn frontier_count(&self) -> usize {
        self.frontier.len()
    }

    /// Total intervals tracked (open + closed).
    pub fn interval_count(&self) -> usize {
        self.intervals.len()
    }

    /// Number of currently open (un-removed) intervals.
    pub fn open_count(&self) -> usize {
        self.open.len()
    }

    // -- Internal -------------------------------------------------------------

    /// Advance the frontier from `frontier_time` to `at`.
    fn advance_to(&mut self, at: SimTime) {
        // Process new intervals that start in (frontier_time, at].
        while self.open_cursor < self.intervals.len() {
            let interval = &self.intervals[self.open_cursor];
            if interval.from > at {
                break;
            }
            self.open_cursor += 1;
            // Add to frontier if the interval is still active at `at`.
            if interval.until.map_or(true, |u| u > at) {
                self.frontier.insert(interval.key.clone());
            }
        }

        // Process close events up to `at`.
        let to_process: Vec<SimTime> = self.pending_closes.range(..=at).map(|(&t, _)| t).collect();
        for t in to_process {
            if let Some(keys) = self.pending_closes.remove(&t) {
                for key in keys {
                    self.frontier.remove(&key);
                }
            }
        }

        self.frontier_time = at;
    }

    /// Rebuild the frontier from scratch at time `at`.
    fn rebuild_at(&mut self, at: SimTime) {
        self.frontier.clear();
        self.pending_closes.clear();

        for (i, interval) in self.intervals.iter().enumerate() {
            if interval.from <= at {
                if interval.until.map_or(true, |u| u > at) {
                    self.frontier.insert(interval.key.clone());
                }
            }
            // Queue future close events for intervals active at `at`.
            if let Some(until) = interval.until {
                if until > at && interval.from <= at {
                    self.pending_closes.entry(until).or_default().push(interval.key.clone());
                }
            }
            // Track the cursor: first interval with from > at.
            if interval.from > at && i < self.open_cursor {
                self.open_cursor = i;
            }
        }

        // Set cursor to the first interval not yet processed.
        self.open_cursor = self.intervals.partition_point(|i| i.from <= at);
        self.frontier_time = at;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    fn t(ms: u64) -> SimTime {
        SimTime::ms(ms)
    }

    #[test]
    fn empty_index() {
        let mut idx: TemporalSet<u32> = TemporalSet::new();
        assert!(idx.active_at(t(0)).is_empty());
        assert!(idx.active_at(t(1000)).is_empty());
    }

    #[test]
    fn add_and_query() {
        let mut idx = TemporalSet::new();
        idx.add(1u32, t(100));
        assert!(!idx.active_at(t(50)).contains(&1));
        assert!(idx.active_at(t(100)).contains(&1));
        assert!(idx.active_at(t(500)).contains(&1));
    }

    #[test]
    fn add_remove_lifecycle() {
        let mut idx = TemporalSet::new();
        idx.add(1u32, t(100));
        idx.remove(&1, t(500));

        assert!(!idx.active_at(t(50)).contains(&1));
        assert!(idx.active_at(t(100)).contains(&1));
        assert!(idx.active_at(t(300)).contains(&1));
        assert!(!idx.active_at(t(500)).contains(&1));
        assert!(!idx.active_at(t(1000)).contains(&1));
    }

    #[test]
    fn re_add_after_remove() {
        let mut idx = TemporalSet::new();
        idx.add(1u32, t(0));
        idx.remove(&1, t(2000));
        idx.add(1, t(8000));

        assert!(idx.active_at(t(1000)).contains(&1));
        assert!(!idx.active_at(t(3000)).contains(&1));
        assert!(idx.active_at(t(9000)).contains(&1));
    }

    #[test]
    fn multiple_keys() {
        let mut idx = TemporalSet::new();
        idx.add(1u32, t(0));
        idx.add(2, t(100));
        idx.add(3, t(200));
        idx.remove(&1, t(150));

        let active = idx.active_at(t(200));
        assert!(!active.contains(&1));
        assert!(active.contains(&2));
        assert!(active.contains(&3));
    }

    #[test]
    fn backward_query_rebuilds() {
        let mut idx = TemporalSet::new();
        idx.add(1u32, t(0));
        idx.remove(&1, t(500));
        idx.add(2, t(300));

        // Forward to T=1000
        let active = idx.active_at(t(1000));
        assert!(!active.contains(&1));
        assert!(active.contains(&2));

        // Backward to T=200 — rebuild
        let active = idx.active_at(t(200));
        assert!(active.contains(&1));
        assert!(!active.contains(&2));

        // Forward again to T=400
        let active = idx.active_at(t(400));
        assert!(active.contains(&1));
        assert!(active.contains(&2));
    }

    #[test]
    fn forward_advancement_processes_closes() {
        let mut idx = TemporalSet::new();
        idx.add(1u32, t(0));
        idx.add(2, t(0));
        idx.remove(&1, t(500));

        // At T=300: both active
        assert_eq!(idx.active_at(t(300)).len(), 2);

        // At T=600: only key 2 (key 1 closed at 500)
        let active = idx.active_at(t(600));
        assert_eq!(active.len(), 1);
        assert!(active.contains(&2));
    }

    #[test]
    fn remove_before_frontier_applies_immediately() {
        let mut idx = TemporalSet::new();
        idx.add(1u32, t(0));

        // Advance frontier to T=1000
        assert!(idx.active_at(t(1000)).contains(&1));

        // Remove at T=500 (behind frontier)
        idx.remove(&1, t(500));
        // Frontier should be updated immediately
        assert!(!idx.frontier.contains(&1));
    }

    #[test]
    fn add_behind_frontier() {
        let mut idx = TemporalSet::new();
        // Advance to T=1000 (empty)
        idx.active_at(t(1000));

        // Add at T=500 (behind frontier) — should appear in frontier
        idx.add(1u32, t(500));
        assert!(idx.frontier.contains(&1));
        assert!(idx.active_at(t(1000)).contains(&1));
    }

    #[test]
    fn remove_idempotent() {
        let mut idx: TemporalSet<u32> = TemporalSet::new();
        // Remove a key that was never added — should not panic
        idx.remove(&1, t(100));
        assert!(idx.active_at(t(100)).is_empty());
    }

    #[test]
    #[should_panic(expected = "already has an open interval")]
    fn double_add_panics() {
        let mut idx = TemporalSet::new();
        idx.add(1u32, t(100));
        idx.add(1, t(200)); // should panic — 1 still open
    }

    #[test]
    fn remove_at_from_time_creates_empty_interval() {
        // Interval [100, 100) = empty. Key is never active.
        let mut idx = TemporalSet::new();
        idx.add(1u32, t(100));
        idx.remove(&1, t(100));
        assert!(!idx.active_at(t(100)).contains(&1));
        assert!(!idx.active_at(t(99)).contains(&1));
        assert!(!idx.active_at(t(101)).contains(&1));
    }

    #[test]
    fn accessors() {
        let mut idx = TemporalSet::new();
        idx.add(1u32, t(0));
        idx.add(2, t(100));
        idx.remove(&1, t(500));

        assert_eq!(idx.interval_count(), 2);
        assert_eq!(idx.open_count(), 1); // key 2 still open
        idx.active_at(t(600));
        assert_eq!(idx.frontier_count(), 1); // only key 2 at T=600
    }

    #[test]
    fn interleaved_adds_removes_many_keys() {
        let mut idx = TemporalSet::new();
        // 100 keys, each active for [i*10, i*10 + 500)
        for i in 0..100u32 {
            idx.add(i, SimTime::ms(i as u64 * 10));
        }
        for i in 0..100u32 {
            idx.remove(&i, SimTime::ms(i as u64 * 10 + 500));
        }
        // At T=250: keys 0..=25 are active (from <= 250 and until > 250)
        // Key i is active at T if i*10 <= T < i*10+500
        // i*10 <= 250 → i <= 25
        // i*10+500 > 250 → always true for i <= 25
        let active = idx.active_at(t(250));
        assert_eq!(active.len(), 26); // keys 0..=25

        // At T=600: keys 11..=60 active
        // i*10 <= 600 → i <= 60
        // i*10+500 > 600 → i > 10
        let active = idx.active_at(t(600));
        assert_eq!(active.len(), 50); // keys 11..=60

        // At T=1500: only keys 100+ would be active, but max key is 99
        // key 99 active in [990, 1490). At T=1500: not active.
        let active = idx.active_at(t(1500));
        assert_eq!(active.len(), 0);
    }

    #[test]
    fn backward_after_many_advances() {
        let mut idx = TemporalSet::new();
        for i in 0..50u32 {
            idx.add(i, SimTime::ms(i as u64 * 100));
            idx.remove(&i, SimTime::ms(i as u64 * 100 + 2000));
        }

        // Advance through several points
        assert_eq!(idx.active_at(SimTime::ms(500)).len(), 6); // keys 0..=5
        assert_eq!(idx.active_at(SimTime::ms(1000)).len(), 11); // keys 0..=10
        assert_eq!(idx.active_at(SimTime::ms(3000)).len(), 20); // keys 11..=30

        // Backward to T=200 — must rebuild correctly
        let active = idx.active_at(SimTime::ms(200));
        assert_eq!(active.len(), 3); // keys 0,1,2
        assert!(active.contains(&0));
        assert!(active.contains(&1));
        assert!(active.contains(&2));
        assert!(!active.contains(&3));

        // Forward again — must work from rebuilt state
        let active = idx.active_at(SimTime::ms(500));
        assert_eq!(active.len(), 6);
    }

    #[test]
    fn scale_10k_adds() {
        let mut idx = TemporalSet::new();
        for i in 0..10_000u32 {
            idx.add(i, SimTime::ms(i as u64 * 10));
        }
        // Remove half
        for i in 0..5_000u32 {
            idx.remove(&i, SimTime::ms(i as u64 * 10 + 50_000));
        }

        let t0 = Instant::now();
        let active = idx.active_at(SimTime::ms(60_000));
        let elapsed = t0.elapsed();
        assert_eq!(active.len(), 5_000);
        eprintln!("[temporal_index scale] 10k intervals, query: {:?}", elapsed);
    }
}
