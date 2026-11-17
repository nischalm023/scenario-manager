//! A *pallet* is one order's worth of units drawn from a single DC's demand;
//! searching for one answers the question the bulk order builder cannot ask
//! the ledger directly: **when** does enough of what this DC wants exist at
//! the same moment to be worth reserving?
//!
//! The answer is the earliest SimTime at which the DC's desired SKUs yield
//! `[min_pallet, max_pallet]` units all available simultaneously, plus the
//! concrete lines to reserve at that time.
//!
//! `available(sku, t) = inv(sku, t) − reserved(sku, t)` on the inventory
//! ledger. The search finds the smallest `t ≥ after` at which:
//!
//! ```text
//! Σ_sku ∈ desire   min( desire[sku], available(sku, t) )  ≥ min_pallet
//! ```
//!
//! and then packs concrete `(sku, qty)` lines summing to at most
//! `max_pallet`. Each line's qty is bounded by both DC-owed and physical
//! (forward-scheduled) on-hand at that moment. A later reservation can
//! still make that candidate unsafe: the builder then uses
//! `find_earliest_lasting_pallet` to search quantities that stay available.
//!
//! ## Algorithm
//!
//! Heap-based event sweep with early exit, **not** a quadratic scan
//! over candidate times.
//!
//! 1. **Setup** — for every SKU in `desire`, take `inv` and `reserved`
//!    counters' slices of events strictly after `after`. Compute
//!    `available(sku, after)` (point-in-time) and a per-SKU
//!    `truncated = min(desire, available)`. The running `sum =
//!    Σ truncated`.
//! 2. **Initial check** — if `sum ≥ min_pallet` at `after`, pack & return.
//! 3. **Walk** — push every SKU's next event time into a min-heap. Pop
//!    in time order; for each time `t`, *drain* all heap entries at `t`
//!    (multiple SKUs can share an event time), apply each event to its
//!    SKU's `available`, recompute that SKU's `truncated`, and update
//!    `sum`. After draining a time, check `sum`; if `≥ min_pallet`, pack
//!    & return with `reservation_time = t`. Otherwise continue.
//! 4. Returns `None` once the heap empties without crossing the threshold.
//!
//! Complexity: `O(D log E_per_sku)` for the setup binary searches
//! (`count_at`), then `O(E_walked · log D)` for the walk, where
//! `E_walked` is the number of events processed before early exit.
//! Early exit dominates in practice — typical builds find a pallet
//! within a handful of events. No `O(D²)`, no copying of all events
//! into intermediate buffers.

use rustc_hash::FxHashMap;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};

use crate::model::ids::SkuId;
use crate::model::inventory::InventoryService;
use crate::model::inventory::sku_ledger::{CounterLookup, CountersByMap};
use crate::simtime::SimTime;
use crate::utils::temporal_counter::TemporalCounter;

/// Output of a successful pallet search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PalletFound {
    /// Earliest SimTime where every line's quantity is physically
    /// available simultaneously. Used as the `reservation_time` for the
    /// subsequent `reserve_for_order` call.
    pub reservation_time: SimTime,
    /// `(sku, qty)` lines summing to at most `max_pallet`. Non-empty,
    /// sum ≥ `min_pallet`, sorted by `SkuId` ascending for deterministic
    /// output.
    pub lines: Vec<(SkuId, u32)>,
    /// Sum of `qty` across lines.
    pub total_units: u32,
}

/// Reusable buffers — pass a single instance per OrderBuilder across
/// many `find_earliest_pallet` calls to avoid re-allocating per call.
/// The struct is opaque; clear-and-reuse happens automatically on entry.
///
/// `inv_cum` and `res_cum` track the **cumulative** counts (not deltas)
/// at the current sweep frontier. `TemporalCounter::events[i].1` stores
/// cumulative counts, so advancing past an event just sets the cum
/// fields to the event's stored value (we don't add a delta).
#[derive(Default)]
pub struct PalletSearchScratch {
    skus: Vec<SkuId>,
    desires: Vec<u32>,
    inv_cum: Vec<isize>,
    res_cum: Vec<isize>,
    truncated: Vec<u32>,
    inv_idx: Vec<usize>,
    res_idx: Vec<usize>,
    heap: BinaryHeap<Reverse<(SimTime, usize)>>,
}

impl PalletSearchScratch {
    pub fn new() -> Self {
        Self::default()
    }

    fn reset(&mut self, n: usize) {
        self.skus.clear();
        self.desires.clear();
        self.inv_cum.clear();
        self.res_cum.clear();
        self.truncated.clear();
        self.inv_idx.clear();
        self.res_idx.clear();
        self.heap.clear();
        self.skus.reserve(n);
        self.desires.reserve(n);
        self.inv_cum.reserve(n);
        self.res_cum.reserve(n);
        self.truncated.reserve(n);
        self.inv_idx.reserve(n);
        self.res_idx.reserve(n);
    }
}

/// Find the earliest viable pallet over `inventory`. See module docs.
///
/// This is the production entry point. It acquires a single read lock
/// on `inventory`'s ledger for the whole search — important because
/// many small lock cycles would otherwise dominate the wall-clock cost
/// when building thousands of orders.
pub fn find_earliest_pallet(
    inventory: &InventoryService,
    desire: &BTreeMap<SkuId, u32>,
    min_pallet: u32,
    max_pallet: u32,
    after: SimTime,
    scratch: &mut PalletSearchScratch,
) -> Option<PalletFound> {
    assert!(min_pallet > 0, "min_pallet must be > 0");
    assert!(max_pallet >= min_pallet, "max_pallet must be >= min_pallet");
    if desire.is_empty() {
        return None;
    }

    inventory.with_ledger_for_pallet_search(|inv_counters, res_counters| {
        search_inner(inv_counters, res_counters, desire, min_pallet, max_pallet, after, scratch)
    })
}

/// Fallback after a point-in-time candidate fails to reserve. For every
/// desired SKU, take the suffix minimum of its available count, then run
/// the same pallet sweep over these nondecreasing, safe quantities. This
/// scans all later events, so the builder uses it only after a refusal.
pub fn find_earliest_lasting_pallet(
    inventory: &InventoryService,
    desire: &BTreeMap<SkuId, u32>,
    min_pallet: u32,
    max_pallet: u32,
    after: SimTime,
    scratch: &mut PalletSearchScratch,
) -> Option<PalletFound> {
    inventory.with_ledger_for_pallet_search(|inv, res| {
        lasting_search_inner(inv, res, desire, min_pallet, max_pallet, after, scratch)
    })
}

fn lasting_search_inner<I: CounterLookup, R: CounterLookup>(
    inv: &I,
    res: &R,
    desire: &BTreeMap<SkuId, u32>,
    min_pallet: u32,
    max_pallet: u32,
    after: SimTime,
    scratch: &mut PalletSearchScratch,
) -> Option<PalletFound> {
    let mut lasting = FxHashMap::default();
    for &sku in desire.keys() {
        let mut inv_count = inv.count_at(sku, after);
        let mut res_count = res.count_at(sku, after);
        let mut inv_events = inv.events_after(sku, after).iter().peekable();
        let mut res_events = res.events_after(sku, after).iter().peekable();
        let mut available = vec![(after, (inv_count - res_count).max(0))];
        while let Some(at) = inv_events.peek().map(|e| e.0).into_iter().chain(res_events.peek().map(|e| e.0)).min() {
            // Drain ties before measuring availability, including a pick's
            // simultaneous inventory debit and reservation release.
            while let Some(&(_, count)) = inv_events.next_if(|e| e.0 == at) {
                inv_count = count;
            }
            while let Some(&(_, count)) = res_events.next_if(|e| e.0 == at) {
                res_count = count;
            }
            available.push((at, (inv_count - res_count).max(0)));
        }
        let mut minimum = isize::MAX;
        for (_, count) in available.iter_mut().rev() {
            minimum = minimum.min(*count);
            *count = minimum;
        }
        let mut counter = TemporalCounter::new();
        let mut previous = 0;
        for (at, count) in available {
            if count != previous {
                counter.record(at, count - previous);
                previous = count;
            }
        }
        lasting.insert(sku, counter);
    }
    let empty = FxHashMap::default();
    search_inner(
        &CountersByMap { map: &lasting },
        &CountersByMap { map: &empty },
        desire,
        min_pallet,
        max_pallet,
        after,
        scratch,
    )
}

/// Core sweep. Generic over the inventory and reservation lookup so
/// `InventoryService` can pass `SkuLedger`-backed lookups under its lock
/// and tests can pass `HashMap<SkuId, TemporalCounter>`-backed lookups
/// without any locking machinery.
fn search_inner<I: CounterLookup, R: CounterLookup>(
    inv: &I,
    res: &R,
    desire: &BTreeMap<SkuId, u32>,
    min_pallet: u32,
    max_pallet: u32,
    after: SimTime,
    scratch: &mut PalletSearchScratch,
) -> Option<PalletFound> {
    let n = desire.len();
    scratch.reset(n);

    // Setup: compute initial state at `after` and seed the heap.
    let mut sum: u64 = 0;
    for (&sku, &desire_qty) in desire {
        let inv_cnt = inv.count_at(sku, after);
        let res_cnt = res.count_at(sku, after);
        let avail = (inv_cnt - res_cnt).max(0);
        let trunc = (avail as u64).min(desire_qty as u64) as u32;
        sum += trunc as u64;
        scratch.skus.push(sku);
        scratch.desires.push(desire_qty);
        scratch.inv_cum.push(inv_cnt);
        scratch.res_cum.push(res_cnt);
        scratch.truncated.push(trunc);
        scratch.inv_idx.push(0);
        scratch.res_idx.push(0);
    }

    if sum >= min_pallet as u64 {
        return Some(pack(&scratch.skus, &scratch.truncated, sum, max_pallet, after));
    }

    // `after` is constant for the call, so each SKU's slice is computed once
    // here and never again. Calling `events_after` per heap pop instead costs
    // a fresh O(log E) partition_point for an identical answer — profiling
    // pinned that at 1.1% of total wall time.
    let inv_slices: Vec<&[(SimTime, isize)]> = scratch.skus.iter().map(|&sku| inv.events_after(sku, after)).collect();
    let res_slices: Vec<&[(SimTime, isize)]> = scratch.skus.iter().map(|&sku| res.events_after(sku, after)).collect();

    // Seed heap: each SKU's earliest event time. Saturation pruning
    // mirrors the in-walk logic — a SKU already at `truncated == desire`
    // at `after` only needs to track future *res* events (which can
    // break saturation); future inv events can't grow truncated above
    // desire so they're skipped.
    for i in 0..scratch.skus.len() {
        let saturated = scratch.truncated[i] == scratch.desires[i];
        let inv_evs = inv_slices[i];
        let res_evs = res_slices[i];
        let next_t = if saturated {
            res_evs.first().map(|&(t, _)| t)
        } else {
            match (inv_evs.first(), res_evs.first()) {
                (Some(&(a, _)), Some(&(b, _))) => Some(a.min(b)),
                (Some(&(a, _)), None) => Some(a),
                (None, Some(&(b, _))) => Some(b),
                (None, None) => None,
            }
        };
        if let Some(t) = next_t {
            scratch.heap.push(Reverse((t, i)));
        }
    }

    // Walk: pop in time order, drain ties, check sum after each time-batch.
    while let Some(&Reverse((t, _))) = scratch.heap.peek() {
        // Several SKUs can carry an event at the same instant, so drain by
        // popping until an entry belongs to a later time and putting that one
        // back — the pop is unconditional, which is what keeps this loop free
        // of a "peek said yes so pop must succeed" claim.
        while let Some(Reverse((event_t, i))) = scratch.heap.pop() {
            if event_t != t {
                scratch.heap.push(Reverse((event_t, i)));
                break;
            }
            let desire_qty = scratch.desires[i];

            // Advance the inv cursor through every event with time `≤ t`, not
            // just `== t`: the saturation skip below may have left earlier inv
            // events unpushed, and they still have to be absorbed. Each stored
            // value IS the new cumulative count (TemporalCounter keeps
            // cumulatives, not deltas), so the cursor sets rather than adds.
            let inv_evs = inv_slices[i];
            while scratch.inv_idx[i] < inv_evs.len() && inv_evs[scratch.inv_idx[i]].0 <= t {
                scratch.inv_cum[i] = inv_evs[scratch.inv_idx[i]].1;
                scratch.inv_idx[i] += 1;
            }
            let res_evs = res_slices[i];
            while scratch.res_idx[i] < res_evs.len() && res_evs[scratch.res_idx[i]].0 <= t {
                scratch.res_cum[i] = res_evs[scratch.res_idx[i]].1;
                scratch.res_idx[i] += 1;
            }

            // Recompute avail and truncated for this SKU.
            let avail = (scratch.inv_cum[i] - scratch.res_cum[i]).max(0) as u64;
            let new_trunc = avail.min(desire_qty as u64) as u32;
            sum = sum + new_trunc as u64 - scratch.truncated[i] as u64;
            scratch.truncated[i] = new_trunc;

            // Saturation pruning: once this SKU contributes its full
            // `desire_qty`, further inv events can only raise avail above
            // desire and so cannot move `sum` — skip them. Res events still
            // matter: a reservation drops avail back under desire.
            let saturated = scratch.truncated[i] == desire_qty;
            let next_inv = inv_evs.get(scratch.inv_idx[i]).map(|&(et, _)| et);
            let next_res = res_evs.get(scratch.res_idx[i]).map(|&(et, _)| et);
            let next_t = if saturated {
                next_res
            } else {
                match (next_inv, next_res) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (Some(a), None) => Some(a),
                    (None, Some(b)) => Some(b),
                    (None, None) => None,
                }
            };
            if let Some(nt) = next_t {
                scratch.heap.push(Reverse((nt, i)));
            }
        }

        if sum >= min_pallet as u64 {
            return Some(pack(&scratch.skus, &scratch.truncated, sum, max_pallet, t));
        }
    }

    None
}

/// Pack a pallet from the per-SKU `truncated` snapshot. `total` is the
/// already-known sum (≥ min_pallet); we just distribute up to
/// `max_pallet` across the contributing SKUs.
///
/// Greedy-by-largest-contributor is provably optimal here: each
/// `truncated[i]` is already capped at `min(desire[i], available[i])`,
/// so no SKU can supply more than the value we see. Any feasible
/// distribution summing to `target` selects ≤ `truncated[i]` per SKU;
/// taking the largest available contributors first reaches `target`
/// with the fewest line items without leaving capacity on the table.
fn pack(skus: &[SkuId], truncated: &[u32], sum: u64, max_pallet: u32, t: SimTime) -> PalletFound {
    let target = (max_pallet as u64).min(sum) as u32;
    let mut indexed: Vec<(usize, u32)> =
        truncated.iter().enumerate().filter_map(|(i, &q)| if q > 0 { Some((i, q)) } else { None }).collect();
    // Greedy: highest contributors first (ties broken by SkuId ascending).
    indexed.sort_by(|a, b| b.1.cmp(&a.1).then(skus[a.0].0.cmp(&skus[b.0].0)));

    let mut lines: Vec<(SkuId, u32)> = Vec::new();
    let mut remaining = target;
    for (i, q) in indexed {
        if remaining == 0 {
            break;
        }
        let take = q.min(remaining);
        if take > 0 {
            lines.push((skus[i], take));
            remaining -= take;
        }
    }
    // Sort by SkuId for deterministic output.
    lines.sort_by_key(|(s, _)| s.0);
    PalletFound { reservation_time: t, lines, total_units: target }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Build hand-crafted inv + res counter maps for the test, then run
    /// `search_inner` directly (skipping the InventoryService lock).
    struct Harness {
        inv: HashMap<SkuId, TemporalCounter>,
        res: HashMap<SkuId, TemporalCounter>,
    }
    impl Harness {
        fn new() -> Self {
            Self { inv: HashMap::new(), res: HashMap::new() }
        }
        fn deliver(&mut self, sku: SkuId, t: SimTime, qty: i32) {
            self.inv.entry(sku).or_insert_with(TemporalCounter::new).record(t, qty as isize);
        }
        fn reserve(&mut self, sku: SkuId, t: SimTime, qty: i32) {
            self.res.entry(sku).or_insert_with(TemporalCounter::new).record(t, qty as isize);
        }
        fn search(&self, desire: &BTreeMap<SkuId, u32>, min: u32, max: u32, after: SimTime) -> Option<PalletFound> {
            let inv_lookup = CountersByMap { map: &self.inv };
            let res_lookup = CountersByMap { map: &self.res };
            let mut scratch = PalletSearchScratch::new();
            search_inner(&inv_lookup, &res_lookup, desire, min, max, after, &mut scratch)
        }
    }

    fn ms(s: u64) -> SimTime {
        SimTime::ms(s * 1000)
    }

    #[test]
    fn lasting_search_matches_exhaustive_future_minima() {
        // Exercise rises, dips, ties, missing SKUs, and bounds between events.
        for seed in 0..40 {
            let mut h = Harness::new();
            for sku in 0..3 {
                let (mut previous_inv, mut previous_res) = (0, 0);
                for t in 0..8 {
                    let inv = (seed * 7 + sku * 11 + t * 13) % 9;
                    let res = (seed * 3 + sku * 7 + t * 5) % 6;
                    // Two changes at one simtime must be observed together.
                    h.deliver(SkuId(sku as u32), ms(t as u64 * 2), -10);
                    h.deliver(SkuId(sku as u32), ms(t as u64 * 2), inv - previous_inv + 10);
                    h.reserve(SkuId(sku as u32), ms(t as u64 * 2), res - previous_res);
                    (previous_inv, previous_res) = (inv, res);
                }
            }
            let desire: BTreeMap<SkuId, u32> =
                [(SkuId(0), 2), (SkuId(1), 4), (SkuId(2), 3), (SkuId(3), 1)].into_iter().collect();
            let inv = CountersByMap { map: &h.inv };
            let res = CountersByMap { map: &h.res };
            let mut scratch = PalletSearchScratch::new();
            for after in 0..17 {
                for (min, max) in [(1, 1), (2, 4), (5, 7)] {
                    let safe_at = |sku, at: u64| {
                        (at..=at.max(16))
                            .map(|t| (inv.count_at(sku, ms(t)) - res.count_at(sku, ms(t))).max(0) as u32)
                            .min()
                            .unwrap()
                    };
                    let expected = (after..=after.max(16)).find_map(|at| {
                        let sum: u32 = desire.iter().map(|(&sku, &qty)| safe_at(sku, at).min(qty)).sum();
                        (sum >= min).then_some((ms(at), sum.min(max)))
                    });
                    let actual = lasting_search_inner(&inv, &res, &desire, min, max, ms(after), &mut scratch);
                    assert_eq!(
                        actual.as_ref().map(|p| (p.reservation_time, p.total_units)),
                        expected,
                        "seed={seed}, after={after}, min={min}, max={max}"
                    );
                    if let Some(pallet) = actual {
                        assert_eq!(pallet.lines.iter().map(|&(_, qty)| qty).sum::<u32>(), pallet.total_units);
                        for &(sku, qty) in &pallet.lines {
                            assert!(qty > 0 && qty <= desire[&sku]);
                            for at in (after..=after.max(16)).map(ms).filter(|&at| at >= pallet.reservation_time) {
                                assert!(qty as isize <= (inv.count_at(sku, at) - res.count_at(sku, at)).max(0));
                            }
                        }
                    }
                }
            }
        }
    }

    /// 5 SKUs, 3-unit pallet — the example the user described. Earliest
    /// simultaneous availability of 3 total units is t=15s when SKUs
    /// 1, 2, 3 each have 1 unit on hand.
    #[test]
    fn earliest_three_of_five_simultaneous() {
        let mut h = Harness::new();
        h.deliver(SkuId(1), ms(10), 1);
        h.deliver(SkuId(1), ms(20), 1);
        h.deliver(SkuId(1), ms(30), 1);
        h.deliver(SkuId(2), ms(15), 1);
        h.deliver(SkuId(2), ms(25), 1);
        h.deliver(SkuId(3), ms(5), 1);
        h.deliver(SkuId(5), ms(40), 1);
        let desire: BTreeMap<SkuId, u32> = (1..=5).map(|i| (SkuId(i), 5)).collect();

        let result = h.search(&desire, 3, 10, SimTime::ZERO).expect("pallet");
        assert_eq!(result.reservation_time, ms(15));
        assert_eq!(result.total_units, 3);
        let lines_map: BTreeMap<SkuId, u32> = result.lines.into_iter().collect();
        assert_eq!(lines_map.get(&SkuId(1)), Some(&1));
        assert_eq!(lines_map.get(&SkuId(2)), Some(&1));
        assert_eq!(lines_map.get(&SkuId(3)), Some(&1));
    }

    #[test]
    fn single_sku_pallet() {
        let mut h = Harness::new();
        for i in 1..=10 {
            h.deliver(SkuId(1), ms(i), 1);
        }
        let desire: BTreeMap<SkuId, u32> = [(SkuId(1), 100)].into_iter().collect();
        let r = h.search(&desire, 5, 10, SimTime::ZERO).expect("pallet");
        assert_eq!(r.reservation_time, ms(5));
        assert_eq!(r.lines, vec![(SkuId(1), 5)]);
        assert_eq!(r.total_units, 5);
    }

    #[test]
    fn desire_caps_contribution() {
        let mut h = Harness::new();
        h.deliver(SkuId(1), ms(1), 100);
        h.deliver(SkuId(2), ms(1), 100);
        let desire: BTreeMap<SkuId, u32> = [(SkuId(1), 2), (SkuId(2), 1)].into_iter().collect();
        let r = h.search(&desire, 3, 5, SimTime::ZERO).expect("pallet");
        assert_eq!(r.reservation_time, ms(1));
        assert_eq!(r.total_units, 3);
    }

    #[test]
    fn max_pallet_caps_total() {
        let mut h = Harness::new();
        h.deliver(SkuId(1), ms(1), 100);
        let desire: BTreeMap<SkuId, u32> = [(SkuId(1), 100)].into_iter().collect();
        let r = h.search(&desire, 5, 10, SimTime::ZERO).expect("pallet");
        assert_eq!(r.total_units, 10);
        assert_eq!(r.lines, vec![(SkuId(1), 10)]);
    }

    #[test]
    fn reservations_push_earliest_time_back() {
        let mut h = Harness::new();
        h.deliver(SkuId(1), ms(1), 5);
        h.deliver(SkuId(1), ms(10), 5);
        // 5 already reserved at t=1, so we need the t=10 delivery.
        h.reserve(SkuId(1), ms(1), 5);
        let desire: BTreeMap<SkuId, u32> = [(SkuId(1), 10)].into_iter().collect();
        let r = h.search(&desire, 3, 5, SimTime::ZERO).expect("pallet");
        assert_eq!(r.reservation_time, ms(10));
        assert_eq!(r.total_units, 5);
    }

    #[test]
    fn empty_desire_returns_none() {
        let h = Harness::new();
        let desire: BTreeMap<SkuId, u32> = BTreeMap::new();
        assert!(h.search(&desire, 1, 10, SimTime::ZERO).is_none());
    }

    #[test]
    fn after_excludes_earlier_times() {
        let mut h = Harness::new();
        h.deliver(SkuId(1), ms(1), 10);
        h.deliver(SkuId(1), ms(100), 10);
        let desire: BTreeMap<SkuId, u32> = [(SkuId(1), 50)].into_iter().collect();
        let r = h.search(&desire, 5, 10, ms(50)).expect("pallet");
        assert_eq!(r.reservation_time, ms(50));
        assert_eq!(r.total_units, 10);
    }

    #[test]
    fn insufficient_total_returns_none() {
        let mut h = Harness::new();
        h.deliver(SkuId(1), ms(1), 2);
        let desire: BTreeMap<SkuId, u32> = [(SkuId(1), 100)].into_iter().collect();
        assert!(h.search(&desire, 10, 20, SimTime::ZERO).is_none());
    }

    /// Two SKUs both delivering at different rates; the earliest joint
    /// 5-unit availability requires waiting for the slower one. Exercises
    /// the "multi-SKU coordination" path.
    #[test]
    fn two_skus_coordinated_arrivals() {
        let mut h = Harness::new();
        // SKU1: 5 at t=1.
        h.deliver(SkuId(1), ms(1), 5);
        // SKU2: 1 unit at t=1, 1 at t=2, ..., 1 at t=5. Cumulative 1..5.
        for i in 1..=5 {
            h.deliver(SkuId(2), ms(i), 1);
        }
        // Each SKU is desired 5 units.
        let desire: BTreeMap<SkuId, u32> = [(SkuId(1), 5), (SkuId(2), 5)].into_iter().collect();
        // We want min_pallet=8. At t=1: SKU1=5, SKU2=1 → sum=6 < 8.
        // At t=2: SKU2=2 → sum=7. At t=3: 8 ✓.
        let r = h.search(&desire, 8, 10, SimTime::ZERO).expect("pallet");
        assert_eq!(r.reservation_time, ms(3));
        assert!(r.total_units >= 8 && r.total_units <= 10);
    }

    /// Coincident events on inv and res at the same time for the same
    /// SKU — they should both be applied before checking sum.
    #[test]
    fn coincident_inv_and_res_at_same_time() {
        let mut h = Harness::new();
        // At t=1, inv+10 and res+8 simultaneously → net +2 available.
        h.deliver(SkuId(1), ms(1), 10);
        h.reserve(SkuId(1), ms(1), 8);
        let desire: BTreeMap<SkuId, u32> = [(SkuId(1), 100)].into_iter().collect();
        let r = h.search(&desire, 1, 5, SimTime::ZERO).expect("pallet");
        assert_eq!(r.reservation_time, ms(1));
        assert_eq!(r.total_units, 2);
    }

    /// Delivery scheduled at `SimTime::ZERO` with `after = SimTime::ZERO`.
    /// The event is folded into the initial `count_at(after)` (its time
    /// satisfies `t <= after`), not picked up by `events_after`. Earliest
    /// pallet should land at `after = ZERO` directly without any walk.
    #[test]
    fn delivery_at_simtime_zero() {
        let mut h = Harness::new();
        h.deliver(SkuId(1), SimTime::ZERO, 5);
        let desire: BTreeMap<SkuId, u32> = [(SkuId(1), 100)].into_iter().collect();
        let r = h.search(&desire, 3, 10, SimTime::ZERO).expect("pallet");
        assert_eq!(r.reservation_time, SimTime::ZERO);
        assert_eq!(r.total_units, 5);
    }

    /// Saturation pruning: a SKU already at `truncated == desire` at
    /// `after` shouldn't push its future inv events to the heap. The
    /// behaviour we can observe in a test is that adding many future
    /// inv events to a saturated SKU does not change the search result
    /// when no res events break saturation.
    #[test]
    fn saturated_sku_ignores_future_inv_events() {
        let mut h = Harness::new();
        // SKU1 delivers 100 at t=1 (saturates against desire=5 immediately).
        h.deliver(SkuId(1), ms(1), 100);
        // Lots of extra inv events on SKU1 — would inflate event-walk cost
        // if we processed them, but their effect is capped by desire.
        for i in 2..200 {
            h.deliver(SkuId(1), ms(i), 1);
        }
        // SKU2 alone reaches min_pallet=10 only at t=100 (1 unit / tick).
        for i in 1..=200 {
            h.deliver(SkuId(2), ms(i), 1);
        }
        let desire: BTreeMap<SkuId, u32> = [(SkuId(1), 5), (SkuId(2), 100)].into_iter().collect();
        // At t=1: SKU1=5 (saturated), SKU2=1 → sum=6 < 10. Need SKU2 to
        // reach 5. Cumulative on SKU2 = 5 at t=5 → sum=10 ✓.
        let r = h.search(&desire, 10, 20, SimTime::ZERO).expect("pallet");
        assert_eq!(r.reservation_time, ms(5));
        // SKU1 contributes its full 5 (desire cap); SKU2 contributes 5.
        let lines_map: BTreeMap<SkuId, u32> = r.lines.into_iter().collect();
        assert_eq!(lines_map.get(&SkuId(1)), Some(&5));
        assert_eq!(lines_map.get(&SkuId(2)), Some(&5));
    }

    /// Saturation can BREAK if a reservation event drops avail below
    /// desire — the SKU must re-enter the event walk so its future inv
    /// can re-saturate it. Constructed so the pallet only becomes
    /// viable AFTER a desaturation+re-saturation cycle.
    #[test]
    fn saturation_breaks_on_reservation_then_resaturates() {
        let mut h = Harness::new();
        // SKU1: 5 at t=1 (saturates desire=5), then big reservation at
        // t=2 drops avail to 0; new inv at t=3 brings it back to 5.
        h.deliver(SkuId(1), ms(1), 5);
        h.reserve(SkuId(1), ms(2), 5);
        h.deliver(SkuId(1), ms(3), 5);
        // SKU2 simple: 5 at t=3.
        h.deliver(SkuId(2), ms(3), 5);
        let desire: BTreeMap<SkuId, u32> = [(SkuId(1), 5), (SkuId(2), 5)].into_iter().collect();
        // At t=1: SKU1=5, SKU2=0 → 5 < 10.
        // At t=2: SKU1=0 (reserved), SKU2=0 → 0 < 10.
        // At t=3: SKU1=5 (re-saturated), SKU2=5 → 10 ✓.
        let r = h.search(&desire, 10, 10, SimTime::ZERO).expect("pallet");
        assert_eq!(r.reservation_time, ms(3));
        assert_eq!(r.total_units, 10);
    }
}
