//! Late-binding order construction from a [`DemandPool`] + scheduled
//! future inventory.
//!
//! Only present in `ScenarioConfig::BulkSource` mode. Pre-formed-order
//! scenarios (Generated/Inline/Csv) populate `OrderReadyQueue` once at sim
//! start and use no builder.
//!
//! ## Algorithm
//!
//! Each `try_build_one` invocation:
//!
//! 1. Walk DCs in their product-sequence order.
//! 2. For the candidate DC, build a desire map from `pool.entries_for_distribution_center`.
//! 3. Call [`pallet_search::find_earliest_pallet`]: walks the union of
//!    every desired SKU's inventory/reservation event times and finds
//!    the smallest `t ≥ after` at which the truncated-by-desire
//!    availability across the chosen SKUs sums to ≥ `min_pallet_units`.
//!    Packs concrete `(sku, qty)` lines greedily up to `max_pallet_units`.
//! 4. If found: call `inventory.reserve_for_order(lines, t)` to lock in
//!    the units. On refusal, search lasting availability for a safe mix or
//!    a later pallet and reserve it. Then `pool.consume()` depletes the
//!    DC's outstanding demand. Return the order and reservation simtime.
//! 5. If not found (no DC has enough future-scheduled inventory yet):
//!    return `None`; caller retries on the next inventory-ledger gen.
//!
//! Reserving immediately matters because subsequent searches (this pass
//! or future passes) consult `available_at(sku, t)` which subtracts
//! cumulative reservations. Without reserving, multiple orders for
//! different DCs sharing a SKU would all see the same availability and
//! over-commit the same physical units.

use std::collections::{BTreeMap, HashSet};

use crate::flows::ttp::demand::pallet_search::{
    PalletSearchScratch, find_earliest_lasting_pallet, find_earliest_pallet,
};
use crate::model::SimTime;
use crate::model::destination::Destination;
use crate::model::ids::{OrderId, SkuId};
use crate::model::inventory::InventoryService;
use crate::model::order::{OrderLine, OrderSpec};
use crate::scenario_gen::DemandPool;

/// A successfully constructed bulk-mode pick order. Its destination is
/// carried through dispatch into the pick event for per-customer xlsx
/// attribution.
#[derive(Debug, Clone)]
pub struct BuiltOrder {
    /// A real order, same type a scenario file produces, so a bulk-mode
    /// order can carry traits and be found in the registry like any other.
    pub order: OrderSpec,
    /// When the ledger already holds this order's reservation. Runtime
    /// state, not a property of the order, so it stays out of `OrderSpec`.
    pub reservation_time: SimTime,
}

/// Stream of orders built from a demand pool as inventory permits.
pub struct OrderBuilder {
    pool: DemandPool,
    next_order_id: u32,
    min_pallet_units: u32,
    max_pallet_units: u32,
    /// DCs in **product-sequence order** — order of first appearance in the
    /// pool's product lines. `try_build_one` walks DCs in this order and the
    /// first whose remaining demand can form a pallet (≥ min available at or
    /// after the embargo) wins, so picking follows the same sequence as
    /// putaway rather than a round-robin or demand rank.
    distribution_center_order: Vec<String>,
    /// Reusable scratch buffers for the pallet search. Owned by the
    /// builder so per-call allocations don't churn the heap — the
    /// builder is called tens of thousands of times during planning.
    scratch: PalletSearchScratch,
    /// Total orders this builder has committed (for observability).
    pub orders_built: u32,
    /// Total units committed across all built orders.
    pub units_built: u64,
}

impl OrderBuilder {
    /// Pallet bounds come from the pool itself (`DemandPool::min/max_pallet_units`),
    /// so callers don't have to plumb them separately.
    pub fn new(pool: DemandPool) -> Self {
        let min_pallet_units = pool.min_pallet_units();
        let max_pallet_units = pool.max_pallet_units();
        // DC priority = first-appearance order in the product sequence.
        let mut seen = HashSet::new();
        let mut distribution_center_order = Vec::new();
        for line in pool.lines() {
            if seen.insert(line.distribution_center.clone()) {
                distribution_center_order.push(line.distribution_center.clone());
            }
        }
        Self {
            pool,
            next_order_id: 0,
            min_pallet_units,
            max_pallet_units,
            distribution_center_order,
            scratch: PalletSearchScratch::new(),
            orders_built: 0,
            units_built: 0,
        }
    }

    /// Immutable view of the underlying pool — used by the putaway flow
    /// to read `total_per_sku` for arrival prioritization.
    pub fn pool(&self) -> &DemandPool {
        &self.pool
    }

    pub fn pool_is_empty(&self) -> bool {
        self.pool.is_empty()
    }

    /// Try to build one order. Walks DCs in **product-sequence order**,
    /// calling `find_earliest_pallet` per DC; the first DC whose remaining
    /// demand reaches ≥ min available units (at or after `after`) wins —
    /// reserves the inventory at the discovered earliest-sufficient time,
    /// consumes from the pool, and returns
    /// `Some((order_id, lines, reservation_time))`.
    ///
    /// Returns `None` when the pool is empty or no DC can yet form a pallet
    /// (putaway hasn't racked enough of any DC's desired SKUs).
    ///
    /// `after` is the lower bound for the reservation time: the later of the
    /// pick embargo (no pick before it) and the settled frontier (behind it
    /// the ledger may show units free that another order books at a later
    /// simtime). `find_earliest_pallet` finds the earliest time
    /// ≥ `after` at which the DC's lines are simultaneously available. If
    /// later commitments prevent that reservation, the fallback finds the
    /// earliest pallet whose units stay available from its reservation time.
    pub fn try_build_one(&mut self, inventory: &InventoryService, after: SimTime) -> Option<BuiltOrder> {
        if self.pool.is_empty() {
            return None;
        }
        let mut considered_dcs: u32 = 0;

        for i in 0..self.distribution_center_order.len() {
            // Clone the DC name so the `distribution_center_order` borrow is released before we
            // take `&mut self.scratch` / `&mut self.pool` below.
            let target_distribution_center = self.distribution_center_order[i].clone();

            // Build desire map from this DC's remaining demand.
            let desire: BTreeMap<SkuId, u32> =
                self.pool.entries_for_distribution_center(&target_distribution_center).collect();
            if desire.is_empty() {
                continue;
            }
            considered_dcs += 1;

            let mut pallet = match find_earliest_pallet(
                inventory,
                &desire,
                self.min_pallet_units,
                self.max_pallet_units,
                after,
                &mut self.scratch,
            ) {
                Some(p) => p,
                None => continue,
            };

            // Keep the early-exit search for the common case. A refusal must
            // search safe quantities across every desired SKU: a different
            // mix or a later pallet may already be possible without putaway.
            if !inventory.reserve_for_order(&pallet.lines, pallet.reservation_time) {
                let Some(lasting) = find_earliest_lasting_pallet(
                    inventory,
                    &desire,
                    self.min_pallet_units,
                    self.max_pallet_units,
                    after,
                    &mut self.scratch,
                ) else {
                    continue;
                };
                pallet = lasting;
                if !inventory.reserve_for_order(&pallet.lines, pallet.reservation_time) {
                    crate::trace_event!(
                        reject,
                        bulk_reserve_failed,
                        dc = target_distribution_center,
                        reservation_time = pallet.reservation_time,
                        units = pallet.total_units,
                    );
                    continue;
                }
            }

            // Consume from pool.
            for &(sku, qty) in &pallet.lines {
                // TODO(zero-panic): thread when this file's own panic sites convert.
                self.pool.consume(&target_distribution_center, sku, qty).unwrap_or_else(|e| panic!("{e}"));
            }
            let order_id = OrderId(self.next_order_id);
            self.next_order_id = self
                .next_order_id
                .checked_add(1)
                .expect("OrderBuilder: exhausted u32 order-id space (4 billion orders)");
            self.orders_built += 1;
            self.units_built += pallet.total_units as u64;
            let mut order = OrderSpec::unstated(
                order_id,
                pallet.lines.iter().map(|&(sku, quantity)| OrderLine { sku, quantity }).collect(),
            );
            order.destination = Some(Destination::DistributionCenter(target_distribution_center));
            return Some(BuiltOrder { order, reservation_time: pallet.reservation_time });
        }

        // Every DC failed.
        crate::trace_event!(
            metric,
            order_build_skipped,
            dcs_considered = considered_dcs,
            min_pallet_units = self.min_pallet_units,
            pool_remaining = self.pool.total_units(),
        );
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::inventory::InventoryService;
    use std::collections::HashMap;

    /// Helper: empty inventory service.
    fn empty_inv() -> InventoryService {
        InventoryService::new()
    }

    #[test]
    fn empty_pool_returns_none() {
        let mut builder = OrderBuilder::new(DemandPool::new(40, 50));
        let inv = empty_inv();
        assert!(builder.try_build_one(&inv, SimTime::ZERO).is_none());
        assert!(builder.pool_is_empty());
    }

    #[test]
    fn insufficient_inventory_returns_none_and_preserves_pool() {
        let mut pool = DemandPool::new(40, 50);
        pool.push_line(SimTime::ZERO, "dc1".into(), SkuId(0), 100);
        let initial_units = pool.total_units();
        let mut builder = OrderBuilder::new(pool);
        // Empty inventory — even though pool has demand, no order can be built.
        let inv = empty_inv();
        assert!(builder.try_build_one(&inv, SimTime::ZERO).is_none());
        assert_eq!(builder.pool().total_units(), initial_units);
    }

    #[test]
    fn assert_pallet_bounds_panics_on_zero_min() {
        let result = std::panic::catch_unwind(|| {
            OrderBuilder::new(DemandPool::new(0, 50));
        });
        assert!(result.is_err());
    }

    /// Happy path: with one DC owing two SKUs and matching inventory, the
    /// builder packs a single order, reserves the inventory at the
    /// returned reservation time, and depletes the pool.
    #[test]
    fn builds_order_when_inventory_satisfies_pool() {
        let mut pool = DemandPool::new(40, 50);
        pool.push_line(SimTime::ZERO, "dc1".into(), SkuId(0), 30);
        pool.push_line(SimTime::ZERO, "dc1".into(), SkuId(1), 30);
        let inv = empty_inv();
        for _ in 0..30 {
            inv.record_inventory_added(SkuId(0), SimTime::ZERO);
            inv.record_inventory_added(SkuId(1), SimTime::ZERO);
        }
        let mut builder = OrderBuilder::new(pool);
        let built = builder.try_build_one(&inv, SimTime::ZERO).expect("first build");
        assert_eq!(built.order.id, OrderId(0));
        assert_eq!(
            built.order.destination,
            Some(Destination::DistributionCenter("dc1".into())),
            "single-DC pool: built order must attribute to dc1"
        );
        let total: u32 = built.order.unit_count();
        assert!((40..=50).contains(&total), "pallet total {} out of bounds", total);
        // Reservation time should be at or after the search lower bound.
        assert!(built.reservation_time >= SimTime::ZERO);
        assert!(builder.pool().total_units() >= 10 && builder.pool().total_units() <= 20);
        assert_eq!(builder.orders_built, 1);
        assert_eq!(builder.units_built, total as u64);
        // Inventory should reflect reservation: available drops by `total`.
        let available_sum: i64 =
            (0..2).map(|s| inv.sku_available_at(SkuId(s), built.reservation_time).max(0) as i64).sum();
        assert_eq!(available_sum, 60 - total as i64);
    }

    /// Two DCs; product-sequence order serves the first-appearing DC (dc_a)
    /// before moving on — not round-robin. Both pallets are drawn from dc_a
    /// while it can still form one; dc_b is untouched.
    #[test]
    fn picks_first_sequence_dc_until_it_cannot_form_a_pallet() {
        let mut pool = DemandPool::new(40, 50);
        pool.push_line(SimTime::ZERO, "dc_a".into(), SkuId(0), 200);
        pool.push_line(SimTime::ZERO, "dc_b".into(), SkuId(0), 200);
        let inv = empty_inv();
        for _ in 0..400 {
            inv.record_inventory_added(SkuId(0), SimTime::ZERO);
        }
        let mut builder = OrderBuilder::new(pool);
        let built1 = builder.try_build_one(&inv, SimTime::ZERO).expect("build 1");
        let built2 = builder.try_build_one(&inv, SimTime::ZERO).expect("build 2");
        // dc_a is first in the product sequence, so both pallets (≤ 50 each)
        // come from it; dc_b stays untouched until dc_a can't form a pallet.
        assert_eq!(built1.order.destination, Some(Destination::DistributionCenter("dc_a".into())));
        assert_eq!(built2.order.destination, Some(Destination::DistributionCenter("dc_a".into())));
        let dc_a_remaining = builder.pool().owed("dc_a", SkuId(0));
        let dc_b_remaining = builder.pool().owed("dc_b", SkuId(0));
        assert_eq!(dc_a_remaining, 100, "two ≤50 pallets drawn from dc_a");
        assert_eq!(dc_b_remaining, 200, "dc_b untouched while dc_a can still form a pallet");
    }

    /// Inventory below min_pallet returns None without consuming anything.
    #[test]
    fn below_min_pallet_does_not_commit() {
        let mut pool = DemandPool::new(40, 50);
        pool.push_line(SimTime::ZERO, "dc1".into(), SkuId(0), 100);
        let inv = empty_inv();
        for _ in 0..20 {
            inv.record_inventory_added(SkuId(0), SimTime::ZERO);
        }
        let initial_units = pool.total_units();
        let mut builder = OrderBuilder::new(pool);
        assert!(builder.try_build_one(&inv, SimTime::ZERO).is_none());
        assert_eq!(builder.pool().total_units(), initial_units, "pool must not be drained");
        assert_eq!(builder.orders_built, 0);
    }

    #[test]
    fn bulk_builder_does_not_abandon_reservable_remaining_stock() {
        let inventory = InventoryService::new();
        let (a, b, c) = (SkuId(0), SkuId(1), SkuId(2));
        inventory.record_inventory_added(a, SimTime::ZERO);
        inventory.record_inventory_added(a, SimTime::ZERO);
        inventory.record_inventory_added(c, SimTime::ZERO);
        inventory.record_inventory_added(b, SimTime::ms(20_000));
        let mut pool = DemandPool::new(2, 2);
        pool.push_line(SimTime::ZERO, "first".into(), a, 1);
        pool.push_line(SimTime::ZERO, "first".into(), b, 1);
        pool.push_line(SimTime::ZERO, "second".into(), a, 2);
        pool.push_line(SimTime::ZERO, "second".into(), c, 1);
        let mut builder = OrderBuilder::new(pool);
        let first = builder.try_build_one(&inventory, SimTime::ZERO).unwrap();
        assert_eq!(first.reservation_time, SimTime::ms(20_000));
        assert_eq!(inventory.earliest_sku_qty_lasting(a, 1, SimTime::ZERO), Some(SimTime::ZERO));
        assert_eq!(inventory.earliest_sku_qty_lasting(c, 1, SimTime::ZERO), Some(SimTime::ZERO));
        let second = builder.try_build_one(&inventory, SimTime::ZERO).expect("safe A+C pallet");
        assert_eq!(second.reservation_time, SimTime::ZERO);
        assert_eq!(second.order.lines, vec![OrderLine { sku: a, quantity: 1 }, OrderLine { sku: c, quantity: 1 }]);
        assert_eq!(builder.pool().total_units(), 1);
        assert!(builder.try_build_one(&inventory, SimTime::ZERO).is_none());
        assert_eq!(builder.pool().total_units(), 1, "failed fallback must preserve demand");
    }

    #[test]
    fn bulk_builder_searches_past_rejected_early_stock() {
        let inventory = InventoryService::new();
        let sku = SkuId(0);
        inventory.record_inventory_added(sku, SimTime::ms(1_000));
        assert!(inventory.reserve_for_order(&[(sku, 1)], SimTime::ms(2_000)));
        inventory.record_pick_consumed(sku, SimTime::ms(3_000));
        inventory.record_inventory_added(sku, SimTime::ms(4_000));
        let mut pool = DemandPool::new(1, 1);
        pool.push_line(SimTime::ZERO, "dc".into(), sku, 1);
        let mut builder = OrderBuilder::new(pool);
        assert_eq!(inventory.earliest_sku_qty_lasting(sku, 1, SimTime::ms(1_500)), Some(SimTime::ms(4_000)));
        let order = builder.try_build_one(&inventory, SimTime::ms(1_500)).expect("already recorded replenishment");
        assert_eq!(order.reservation_time, SimTime::ms(4_000));
        assert!(builder.pool_is_empty());
        assert_eq!(inventory.sku_reserved_at(sku, SimTime::ms(4_000)), 1);
    }
}
