//! SKU catalog generation and order generation.

use crate::model::{OrderId, OrderLine, OrderSpec, SkuId};
use crate::scenario::GeneratedScenario;

use super::types::SkuInfo;

/// Result of inventory-aware order generation.
pub(super) struct OrderGenResult {
    pub orders: Vec<OrderSpec>,
    pub picks_dropped: usize,
    pub popular_exhausted_early: bool,
}

use crate::utils::rng::Rng;

pub(super) fn generate_skus(scenario: &GeneratedScenario) -> Vec<SkuInfo> {
    let num_popular = ((scenario.num_skus as f64) * scenario.popular_skus_pct).round() as usize;
    let num_unpopular = scenario.num_skus.saturating_sub(num_popular);

    let popular_inventory = ((scenario.total_inventory_units as f64) * scenario.inventory_popular_share).round() as u32;
    let unpopular_inventory = (scenario.total_inventory_units as u32).saturating_sub(popular_inventory);

    let per_popular = if num_popular > 0 { popular_inventory / num_popular as u32 } else { 0 };
    let per_unpopular = if num_unpopular > 0 { unpopular_inventory / num_unpopular as u32 } else { 0 };

    let mut skus = Vec::with_capacity(scenario.num_skus);

    for i in 0..num_popular {
        let extra = if (i as u32) < popular_inventory % (num_popular as u32).max(1) { 1 } else { 0 };
        skus.push(SkuInfo {
            id: SkuId(i as u32),
            is_popular: true,
            inventory_units: per_popular + extra,
            weight_kg: 0.0,
        });
    }

    for i in 0..num_unpopular {
        let extra = if (i as u32) < unpopular_inventory % (num_unpopular as u32).max(1) { 1 } else { 0 };
        skus.push(SkuInfo {
            id: SkuId((num_popular + i) as u32),
            is_popular: false,
            inventory_units: per_unpopular + extra,
            weight_kg: 0.0,
        });
    }

    skus
}

pub(super) fn generate_orders(scenario: &GeneratedScenario, skus: &[SkuInfo]) -> OrderGenResult {
    if scenario.num_orders == 0 || scenario.total_order_picks == 0 {
        return OrderGenResult { orders: Vec::new(), picks_dropped: 0, popular_exhausted_early: false };
    }

    // Flat pool of individual inventory units — popular SKUs naturally dominate.
    let mut pool: Vec<SkuId> = Vec::with_capacity(scenario.total_inventory_units as usize);
    for sku in skus {
        for _ in 0..sku.inventory_units {
            pool.push(sku.id);
        }
    }

    let mut rng = Rng::new(0xDE5_CAFE);
    let base_picks = scenario.total_order_picks / scenario.num_orders;
    let remainder_picks = scenario.total_order_picks % scenario.num_orders;

    let mut picks_dropped = 0usize;
    let mut orders = Vec::with_capacity(scenario.num_orders);

    for i in 0..scenario.num_orders {
        let order_picks = base_picks + if i < remainder_picks { 1 } else { 0 };
        if order_picks == 0 {
            continue;
        }

        let mut lines = Vec::new();
        for _ in 0..order_picks {
            if pool.is_empty() {
                picks_dropped += 1;
                continue;
            }
            let idx = rng.index(pool.len());
            let sku = pool.swap_remove(idx);
            lines.push(OrderLine { sku, quantity: 1 });
        }

        if !lines.is_empty() {
            orders.push(OrderSpec::unstated(OrderId(i as u32), lines));
        }
    }

    OrderGenResult { orders, picks_dropped, popular_exhausted_early: false }
}
