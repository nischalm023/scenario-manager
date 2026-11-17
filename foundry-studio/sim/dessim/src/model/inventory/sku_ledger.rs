//! Unpicked SKU stock and reservation counters.
//!
//! Location-independent temporal step functions tracking what exists in the
//! inventory pool and what is spoken for by in-flight orders. Picked units
//! carried in order totes are excluded. Used by the
//! scheduling loop to determine order readiness without scanning totes.
//!
//! Two counters per SKU:
//! - `inventory`: units received (putaway) minus consumed (picked)
//! - `reserved`: units claimed by in-flight orders, not yet picked
//!
//! Available for new orders at time T = inventory.count_at(T) - reserved.count_at(T)

use std::collections::HashMap;
use std::hash::BuildHasher;

use rustc_hash::FxHashMap;

use crate::model::{SimTime, SkuId};
use crate::utils::temporal_counter::TemporalCounter;

/// Aggregate SKU-level ledger. Location-independent.
pub(crate) struct SkuLedger {
    pub inventory: FxHashMap<SkuId, TemporalCounter>,
    pub reserved: FxHashMap<SkuId, TemporalCounter>,
}

impl SkuLedger {
    pub fn new() -> Self {
        Self { inventory: FxHashMap::default(), reserved: FxHashMap::default() }
    }

    /// Earliest time >= `after` where `inventory - reserved > 0` for `sku`.
    pub fn earliest_available(&self, sku: &SkuId, after: SimTime) -> Option<SimTime> {
        self.earliest_available_qty(sku, after, 1)
    }

    /// Whether `inventory - reserved >= qty` for `sku` at `at` and at every
    /// later simtime the ledger records. This is the test a reservation at
    /// `at` must pass: it stays spoken for until its pick, so a unit that an
    /// event at a later simtime books or consumes cannot be promised at `at`.
    pub fn holds_from(&self, sku: &SkuId, at: SimTime, qty: u32) -> bool {
        self.earliest_lasting_qty(sku, at, qty) == Some(at)
    }

    /// Earliest simtime >= `after` from which `qty` units of `sku` stay
    /// available: `inventory - reserved >= qty` there and at every later
    /// simtime the ledger records, so a reservation placed there passes
    /// `holds_from`. `None` when the last recorded count is short of `qty`.
    ///
    /// The merge walk of `earliest_available_qty`, run to the last event,
    /// because a dip below `qty` at any later simtime moves the answer past
    /// that dip.
    pub fn earliest_lasting_qty(&self, sku: &SkuId, after: SimTime, qty: u32) -> Option<SimTime> {
        let required = qty as isize;
        let inv_counter = self.inventory.get(sku);
        let res_counter = self.reserved.get(sku);
        let mut inv_count = inv_counter.map(|c| c.count_at(after)).unwrap_or(0);
        let mut res_count = res_counter.map(|c| c.count_at(after)).unwrap_or(0);
        let mut lasting_from = (inv_count - res_count >= required).then_some(after);
        let mut inv_events = inv_counter.map(|c| c.events_after(after)).unwrap_or(&[]);
        let mut res_events = res_counter.map(|c| c.events_after(after)).unwrap_or(&[]);
        loop {
            let t = match (inv_events.first().map(|&(t, _)| t), res_events.first().map(|&(t, _)| t)) {
                (None, None) => return lasting_from,
                (Some(t), None) | (None, Some(t)) => t,
                (Some(ti), Some(tr)) => ti.min(tr),
            };
            // Drain every event at `t` before judging, as `count_at(t)` does.
            while inv_events.first().map(|&(ti, _)| ti) == Some(t) {
                inv_count = inv_events[0].1;
                inv_events = &inv_events[1..];
            }
            while res_events.first().map(|&(tr, _)| tr) == Some(t) {
                res_count = res_events[0].1;
                res_events = &res_events[1..];
            }
            if inv_count - res_count < required {
                lasting_from = None;
            } else if lasting_from.is_none() {
                lasting_from = Some(t);
            }
        }
    }

    /// Earliest time >= `after` where `inventory - reserved >= qty` for `sku`.
    ///
    /// Merge-walks the two counters' already-sorted event slices, carrying
    /// cumulative inv/res counts forward — no allocation, no sort, no
    /// per-step binary search.
    #[cfg_attr(feature = "perf", tracing::instrument(skip_all))]
    pub fn earliest_available_qty(&self, sku: &SkuId, after: SimTime, qty: u32) -> Option<SimTime> {
        let required = qty as isize;
        if required <= 0 {
            return Some(after);
        }

        let inv_counter = self.inventory.get(sku);
        let res_counter = self.reserved.get(sku);

        let mut inv_count = inv_counter.map(|c| c.count_at(after)).unwrap_or(0);
        let mut res_count = res_counter.map(|c| c.count_at(after)).unwrap_or(0);
        if inv_count - res_count >= required {
            return Some(after);
        }

        let mut inv_events = inv_counter.map(|c| c.events_after(after)).unwrap_or(&[]);
        let mut res_events = res_counter.map(|c| c.events_after(after)).unwrap_or(&[]);

        loop {
            let next_inv = inv_events.first().map(|&(t, _)| t);
            let next_res = res_events.first().map(|&(t, _)| t);

            let t = match (next_inv, next_res) {
                (None, None) => return None,
                (Some(t), None) | (None, Some(t)) => t,
                (Some(ti), Some(tr)) => ti.min(tr),
            };

            // Drain ALL coincident events at `t` from each counter before
            // checking — matches `count_at(t)` semantics (post-all-events at
            // time t). Stopping mid-batch would observe non-physical
            // intermediate cumulative values that are artifacts of
            // `record()`'s insertion order.
            while inv_events.first().map(|&(ti, _)| ti) == Some(t) {
                inv_count = inv_events[0].1;
                inv_events = &inv_events[1..];
            }
            while res_events.first().map(|&(tr, _)| tr) == Some(t) {
                res_count = res_events[0].1;
                res_events = &res_events[1..];
            }

            if inv_count - res_count >= required {
                return Some(t);
            }
        }
    }
}

/// Lookup helper passed by the InventoryService closure: per-SKU `count_at`
/// + per-SKU sorted event slice references. Lets `search_inner` stay
/// independent of `InventoryService`'s lock plumbing.
pub trait CounterLookup {
    fn count_at(&self, sku: SkuId, at: SimTime) -> isize;
    fn events_after(&self, sku: SkuId, after: SimTime) -> &[(SimTime, isize)];
}

/// View of one half of a SkuLedger (inv or reserved) — `HashMap<SkuId,
/// TemporalCounter>`. InventoryService passes this from inside its lock;
/// tests construct it directly.
pub struct CountersByMap<'a, S> {
    pub map: &'a HashMap<SkuId, TemporalCounter, S>,
}

impl<'a, S: BuildHasher> CounterLookup for CountersByMap<'a, S> {
    fn count_at(&self, sku: SkuId, at: SimTime) -> isize {
        // A missing SKU is not a missing entry hidden by a default: no events
        // means the cumulative count genuinely is 0. Every DC in bulk mode
        // desires SKUs putaway has not racked yet, constantly, at sim start.
        self.map.get(&sku).map(|c| c.count_at(at)).unwrap_or(0)
    }
    fn events_after(&self, sku: SkuId, after: SimTime) -> &[(SimTime, isize)] {
        self.map.get(&sku).map(|c| c.events_after(after)).unwrap_or(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(ms: u64) -> SimTime {
        SimTime::ms(ms)
    }

    /// Coincident events with mixed signs on a single counter must not
    /// expose an intermediate cumulative value. `count_at(t)` returns the
    /// post-all-events state — the merge walk must match.
    #[test]
    fn coincident_events_match_count_at_semantics() {
        let sku = SkuId(1);
        let mut ledger = SkuLedger::new();
        let inv = ledger.inventory.entry(sku).or_default();
        // A pick consuming at t=30, then a putaway adding at t=30. Net is 0,
        // but the data structure stores intermediate cumulative +1 because
        // the +1 was inserted at the front of same-time events.
        inv.record(t(30), -1);
        inv.record(t(30), 1);
        assert_eq!(inv.count_at(t(30)), 0, "post-all-events cumulative at t=30 is 0");

        // Net inv at t=30 is 0 — required=1 must not be satisfied.
        assert_eq!(ledger.earliest_available_qty(&sku, t(0), 1), None);
    }

    /// Symmetric case: coincident events on the reserved counter must not
    /// expose intermediates either.
    #[test]
    fn coincident_reserved_events_match_count_at_semantics() {
        let sku = SkuId(1);
        let mut ledger = SkuLedger::new();
        // Inventory: 1 unit available from t=0.
        ledger.inventory.entry(sku).or_default().record(t(0), 1);
        // Reservations at t=30: a release (-1) recorded before a new
        // reservation (+1). Net reserved at t=30 is 0; inventory - reserved
        // at t=30 is 1; the merge walk must report Some(0) (already
        // satisfied at the initial check) — but if the walk ever observed
        // the intermediate +1 reserved, it would miss the true state.
        let res = ledger.reserved.entry(sku).or_default();
        res.record(t(30), -1);
        res.record(t(30), 1);
        // Initial check at after=0 already satisfies required=1 (inv=1, res=0).
        assert_eq!(ledger.earliest_available_qty(&sku, t(0), 1), Some(t(0)));
        // Querying at exactly t=30: post-all-events res = 0, inv = 1, satisfied.
        assert_eq!(ledger.earliest_available_qty(&sku, t(30), 1), Some(t(30)));
    }

    /// Many coincident +1s on inventory must satisfy the threshold at the
    /// final cumulative — earlier intermediates don't matter when same-sign.
    #[test]
    fn coincident_same_sign_inventory_events() {
        let sku = SkuId(1);
        let mut ledger = SkuLedger::new();
        let inv = ledger.inventory.entry(sku).or_default();
        // Three putaways at t=50; final cumulative = 3.
        inv.record(t(50), 1);
        inv.record(t(50), 1);
        inv.record(t(50), 1);
        assert_eq!(inv.count_at(t(50)), 3);

        // required=3 must be satisfied at t=50.
        assert_eq!(ledger.earliest_available_qty(&sku, t(0), 3), Some(t(50)));
        // required=4 must NOT be satisfied.
        assert_eq!(ledger.earliest_available_qty(&sku, t(0), 4), None);
    }

    /// Multiple events at the same time across BOTH counters — typical when
    /// a pick fires (inv -1 AND res -1 at the same `at`). Must match
    /// count_at on both.
    #[test]
    fn coincident_events_on_both_counters() {
        let sku = SkuId(1);
        let mut ledger = SkuLedger::new();
        // Initial state at t=0: inv=2, res=2 → available=0.
        ledger.inventory.entry(sku).or_default().record(t(0), 2);
        ledger.reserved.entry(sku).or_default().record(t(0), 2);

        // Two picks complete at t=40: each does inv -=1, res -= 1.
        let inv = ledger.inventory.entry(sku).or_default();
        inv.record(t(40), -1);
        inv.record(t(40), -1);
        let res = ledger.reserved.entry(sku).or_default();
        res.record(t(40), -1);
        res.record(t(40), -1);

        // post-all-events at t=40: inv=0, res=0, available=0 — still not satisfied.
        assert_eq!(ledger.earliest_available_qty(&sku, t(0), 1), None);
    }

    /// The lasting walk drains coincident events too: a release and a new
    /// reservation at the same simtime leave no dip, so a unit free from t=0
    /// stays free from t=0.
    #[test]
    fn lasting_qty_matches_count_at_semantics() {
        let sku = SkuId(1);
        let mut ledger = SkuLedger::new();
        ledger.inventory.entry(sku).or_default().record(t(0), 1);
        // Recorded in this order, the reserved counter stores an intermediate
        // cumulative of 1 at t=30 before settling at 0.
        let res = ledger.reserved.entry(sku).or_default();
        res.record(t(30), -1);
        res.record(t(30), 1);
        assert_eq!(ledger.earliest_lasting_qty(&sku, t(0), 1), Some(t(0)));
        assert!(ledger.holds_from(&sku, t(0), 1));
    }
}
