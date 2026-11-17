//! A *frontier* set is current membership only — what is in the set right
//! now, ordered by a caller-chosen cost, with no interval history behind it.
//!
//! It exists so the relay inventory index can ask "cheapest available tote or
//! slot" in log time while absorbing the offline planner's out-of-order
//! commits: every key carries the time of its last accepted write, and an add
//! or remove stamped earlier than that is dropped rather than allowed to roll
//! the key backwards. Callers that need as-of-time membership want a full
//! interval structure instead.

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::hash::Hash;

use rustc_hash::FxHashMap;

use crate::model::SimTime;

/// Temporal ordering violation.
#[derive(Debug, thiserror::Error)]
#[error("stale operation: {msg}")]
pub struct StaleOperation {
    pub msg: String,
}

/// Frontier-only sorted set with per-key temporal safety.
///
/// - `K`: element identity (e.g., ToteId, RackSlot). Must be Ord + Hash.
/// - `S`: sort sortval (e.g., SimDuration for VTM cycle cost). Must be Ord.
pub struct FrontierSortedSet<T, SortVal>
where
    T: Ord + Hash + Clone + Debug,
    SortVal: Ord + Copy,
{
    /// Active members keyed by `(sortval, key)`, valued by the time of the
    /// write that put them there. Carrying that time here — rather than
    /// reaching back into `members` — is what makes the read paths total:
    /// they never have to ask what happens when the two disagree.
    // ordered: every read walks ascending sortval from the cheapest entry.
    sorted: BTreeMap<(SortVal, T), SimTime>,
    /// Per-key: (last accepted write, current sortval if active). A `None`
    /// sortval is a tombstone, kept so an add that arrives late (planner
    /// commits are not chronological) cannot resurrect a removed key.
    members: FxHashMap<T, (SimTime, Option<SortVal>)>,
}

impl<T, SortVal> FrontierSortedSet<T, SortVal>
where
    T: Ord + Hash + Clone + Debug,
    SortVal: Ord + Copy,
{
    pub fn new() -> Self {
        Self { sorted: BTreeMap::new(), members: FxHashMap::default() }
    }

    /// Add or update a key with `sortval` at time `at`, dropping the key's
    /// old entry if its sortval changed. A write older than the key's last
    /// accepted one is ignored.
    pub fn add(&mut self, key: T, sortval: SortVal, at: SimTime) -> Result<(), StaleOperation> {
        if let Some(&(last_t, old_sortval)) = self.members.get(&key) {
            // FIXME(putaway): this ignore is only sound while every caller
            // wants frontier-only candidates. Anyone needing as-of-time
            // availability needs a temporal index, not this set.
            if at < last_t {
                return Ok(());
            }
            if let Some(old_sortval) = old_sortval {
                self.sorted.remove(&(old_sortval, key.clone()));
            }
        }
        self.sorted.insert((sortval, key.clone()), at);
        self.members.insert(key, (at, Some(sortval)));
        Ok(())
    }

    /// Remove a key at time `at`, tombstoning it even when it was absent so a
    /// late add cannot resurrect it. A write older than the key's last
    /// accepted one is ignored (same caveat as [`Self::add`]).
    pub fn remove(&mut self, key: &T, at: SimTime) -> Result<(), StaleOperation> {
        if let Some(&(last_t, sortval)) = self.members.get(key) {
            if at < last_t {
                return Ok(());
            }
            if let Some(sortval) = sortval {
                self.sorted.remove(&(sortval, key.clone()));
            }
        }
        self.members.insert(key.clone(), (at, None));
        Ok(())
    }

    /// Cheapest active member: `(key, sortval)`.
    pub fn first(&self) -> Option<(&T, SortVal)> {
        self.sorted.first_key_value().map(|((sortval, key), _)| (key, *sortval))
    }

    /// Cheapest active member with the time it was written:
    /// `(key, sortval, last_write_at)`.
    pub fn first_with_time(&self) -> Option<(&T, SortVal, SimTime)> {
        self.sorted.first_key_value().map(|((sortval, key), at)| (key, *sortval, *at))
    }

    /// Active members with their sort value and last accepted write time.
    /// Consumers may rank the same membership by another cost without
    /// copying its temporal state into a separate frontier.
    pub fn iter(&self) -> impl Iterator<Item = (&T, SortVal, SimTime)> {
        self.sorted.iter().map(|((sortval, key), at)| (key, *sortval, *at))
    }

    /// Cheapest active member satisfying `pred(key, sortval, last_write_at)`,
    /// scanning upward from the minimum sortval.
    pub fn find<P>(&self, mut pred: P) -> Option<(&T, SortVal)>
    where
        P: FnMut(&T, SortVal, SimTime) -> bool,
    {
        self.sorted.iter().find_map(|((sortval, key), at)| pred(key, *sortval, *at).then_some((key, *sortval)))
    }

    /// Number of active members.
    pub fn len(&self) -> usize {
        self.sorted.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sorted.is_empty()
    }

    /// One named member: `(sortval, last_write_at)`, or `None` if it is not
    /// currently in the set.
    ///
    /// The frontier queries ask "cheapest"; this asks "this one". A search
    /// that names its item — releasing a specific order from the buffer —
    /// still needs the same sortval and timestamp the frontier answers carry.
    pub fn get(&self, key: &T) -> Option<(SortVal, SimTime)> {
        let &(last_t, active) = self.members.get(key)?;
        Some((active?, last_t))
    }

    /// Whether a key is currently a member.
    pub fn contains(&self, key: &T) -> bool {
        self.members.get(key).is_some_and(|(_, active)| active.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(ms: u64) -> SimTime {
        SimTime::ms(ms)
    }

    #[test]
    fn empty_set() {
        let set: FrontierSortedSet<u32, u32> = FrontierSortedSet::new();
        assert!(set.is_empty());
        assert!(set.first().is_none());
    }

    #[test]
    fn add_and_first() {
        let mut set = FrontierSortedSet::new();
        set.add(1u32, 100u32, t(0)).unwrap();
        set.add(2, 50, t(0)).unwrap();
        set.add(3, 200, t(0)).unwrap();

        let (k, s) = set.first().expect("first should exist");
        assert_eq!(*k, 2);
        assert_eq!(s, 50);
        assert_eq!(set.len(), 3);
    }

    #[test]
    fn remove_first_promotes_next() {
        let mut set = FrontierSortedSet::new();
        set.add(1u32, 100u32, t(0)).unwrap();
        set.add(2, 50, t(0)).unwrap();

        set.remove(&2, t(1)).unwrap();
        let (k, _) = set.first().expect("first should exist");
        assert_eq!(*k, 1);
    }

    #[test]
    fn re_add_with_different_sortval() {
        let mut set = FrontierSortedSet::new();
        set.add(1u32, 100u32, t(0)).unwrap();
        set.add(2, 50, t(0)).unwrap();

        // Re-add key 2 with worse sortval.
        set.add(2, 200, t(1)).unwrap();
        let (k, _) = set.first().expect("first should exist");
        assert_eq!(*k, 1); // key 1 is now cheapest
    }

    #[test]
    fn stale_add_is_ignored() {
        let mut set = FrontierSortedSet::new();
        set.add(1u32, 100u32, t(10)).unwrap();

        let result = set.add(1, 50, t(5)); // at=5 < last=10
        assert!(result.is_ok());

        let (k, s, last_write_at) = set.first_with_time().expect("key should remain active");
        assert_eq!(*k, 1);
        assert_eq!(s, 100);
        assert_eq!(last_write_at, t(10));
    }

    #[test]
    fn stale_remove_is_ignored() {
        let mut set = FrontierSortedSet::new();
        set.add(1u32, 100u32, t(10)).unwrap();

        let result = set.remove(&1, t(5)); // at=5 < last=10
        assert!(result.is_ok());
        assert!(set.contains(&1)); // still present
    }

    #[test]
    fn newer_remove_tombstone_blocks_older_add() {
        let mut set = FrontierSortedSet::new();

        set.remove(&1u32, t(10)).unwrap();
        set.add(1, 50, t(5)).unwrap();

        assert!(!set.contains(&1));
        assert!(set.first().is_none());
    }

    #[test]
    fn remove_nonexistent_is_noop() {
        let mut set: FrontierSortedSet<u32, u32> = FrontierSortedSet::new();
        set.remove(&99, t(0)).unwrap(); // no panic, no error
    }

    #[test]
    fn same_sortval_tiebreaks_by_key() {
        let mut set = FrontierSortedSet::new();
        set.add(3u32, 100u32, t(0)).unwrap();
        set.add(1, 100, t(0)).unwrap();
        set.add(2, 100, t(0)).unwrap();

        let (k, _) = set.first().expect("first should exist");
        assert_eq!(*k, 1); // lowest key wins tie
    }

    #[test]
    fn add_remove_add_lifecycle() {
        let mut set = FrontierSortedSet::new();
        set.add(1u32, 100u32, t(0)).unwrap();
        set.remove(&1, t(5)).unwrap();
        assert!(set.is_empty());

        set.add(1, 200, t(10)).unwrap();
        assert_eq!(set.len(), 1);
        let (k, s) = set.first().expect("first should exist");
        assert_eq!(*k, 1);
        assert_eq!(s, 200);
    }

    #[test]
    fn find_returns_first_sorted_match() {
        let mut set = FrontierSortedSet::new();
        set.add(1u32, 100u32, t(0)).unwrap();
        set.add(2, 50, t(0)).unwrap();
        set.add(3, 200, t(0)).unwrap();

        let (k, s) = set
            .find(|_key, sort_value, _last_write_at| sort_value >= 100)
            .expect("find should return first matching item");
        assert_eq!(*k, 1);
        assert_eq!(s, 100);
    }

    #[test]
    fn find_can_filter_by_last_write_time() {
        let mut set = FrontierSortedSet::new();
        set.add(1u32, 100u32, t(0)).unwrap();
        set.add(2, 50, t(10)).unwrap(); // better sort value but written later

        let (k, s) = set
            .find(|_key, _sort_value, last_write_at| last_write_at <= t(0))
            .expect("key written at t=0 should be visible");
        assert_eq!(*k, 1);
        assert_eq!(s, 100);
    }
}
