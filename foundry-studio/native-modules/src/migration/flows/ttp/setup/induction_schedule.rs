//! Synthetic inbound lots in static order-release priority. Existing stock and
//! surplus from earlier lots cover demand before another lot is created. This
//! sequences a finite stock file; it does not invent an inbound receiving plan.

use crate::model::order::OrderSpec;
use crate::prelude::{SimDuration, SimTime, SkuId};
use crate::scenario_gen::SkuInfo;
use rustc_hash::FxHashMap;
use std::collections::VecDeque;

pub(super) fn order_aligned(
    remaining_stock: &[SkuInfo],
    mut credit: FxHashMap<SkuId, u64>,
    orders: &[OrderSpec],
    units_per_tote: u32,
    release_all_at_start: bool,
    lead: Option<SimDuration>,
) -> VecDeque<(SkuId, u32, SimTime)> {
    assert!(units_per_tote > 0);
    let mut stock: FxHashMap<_, _> = remaining_stock.iter().map(|sku| (sku.id, sku.inventory_units)).collect();
    let mut ordered: Vec<_> = orders.iter().collect();
    let release = |order: &OrderSpec| {
        if release_all_at_start { SimTime::ZERO } else { order.arrival.unwrap_or(SimTime::ZERO) }
    };
    ordered.sort_by_key(|order| (release(order), order.id));
    let mut queue = VecDeque::new();
    for order in ordered {
        let at = lead.map_or(SimTime::ZERO, |lead| SimTime::ms(release(order).as_ms().saturating_sub(lead.as_ms())));
        for line in &order.lines {
            let available = credit.entry(line.sku).or_default();
            let used = (*available).min(u64::from(line.quantity));
            *available -= used;
            let mut needed = u64::from(line.quantity) - used;
            let remaining = stock.entry(line.sku).or_default();
            while needed > 0 && *remaining > 0 {
                let qty = (*remaining).min(units_per_tote);
                *remaining -= qty;
                queue.push_back((line.sku, qty, at));
                let used = needed.min(u64::from(qty));
                needed -= used;
                *available += u64::from(qty) - used;
            }
        }
    }
    let mut leftover: Vec<_> = stock.into_iter().collect();
    leftover.sort_by_key(|&(sku, _)| sku);
    for (sku, mut remaining) in leftover {
        while remaining > 0 {
            let qty = remaining.min(units_per_tote);
            queue.push_back((sku, qty, SimTime::ZERO));
            remaining -= qty;
        }
    }
    queue
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::order::OrderLine;
    use crate::prelude::OrderId;

    fn sku(id: u32, inventory_units: u32) -> SkuInfo {
        SkuInfo { id: SkuId(id), inventory_units, is_popular: false, weight_kg: 0.0 }
    }

    fn order(id: u32, sku: u32, quantity: u32, seconds: u64) -> OrderSpec {
        let mut order = OrderSpec::unstated(OrderId(id), vec![OrderLine { sku: SkuId(sku), quantity }]);
        order.arrival = Some(SimTime::seconds(seconds));
        order
    }

    #[test]
    fn timing_follows_source_release_or_explicit_t0_override() {
        let skus = [sku(1, 6), sku(2, 6)];
        let orders = [order(1, 1, 1, 600), order(2, 2, 1, 60)];
        let timed = order_aligned(&skus, FxHashMap::default(), &orders, 6, false, Some(SimDuration::seconds(120)));
        assert_eq!(timed, [(SkuId(2), 6, SimTime::ZERO), (SkuId(1), 6, SimTime::seconds(480))]);
        let immediate = order_aligned(&skus, FxHashMap::default(), &orders, 6, true, Some(SimDuration::seconds(120)));
        assert_eq!(immediate, [(SkuId(1), 6, SimTime::ZERO), (SkuId(2), 6, SimTime::ZERO)]);
        assert_eq!(orders[0].arrival, Some(SimTime::seconds(600)));
    }

    #[test]
    fn preload_and_surplus_cover_later_demand_without_extra_priority_lots() {
        let skus = [sku(1, 12), sku(2, 6)];
        let credit = [(SkuId(1), 2)].into_iter().collect();
        let orders = [order(1, 1, 2, 0), order(2, 2, 1, 1), order(3, 2, 5, 2), order(4, 1, 1, 3)];
        let queue = order_aligned(&skus, credit, &orders, 6, false, None);
        assert_eq!(queue, [(SkuId(2), 6, SimTime::ZERO), (SkuId(1), 6, SimTime::ZERO), (SkuId(1), 6, SimTime::ZERO)]);
        assert_eq!(queue.iter().map(|&(_, qty, _)| qty).sum::<u32>(), 18);
    }

    #[test]
    fn short_stock_is_not_fabricated_and_partial_lots_are_preserved() {
        let queue = order_aligned(&[sku(1, 7)], FxHashMap::default(), &[order(1, 1, 9, 0)], 6, false, None);
        assert_eq!(queue, [(SkuId(1), 6, SimTime::ZERO), (SkuId(1), 1, SimTime::ZERO)]);
    }
}
