//! Pre-simulation inventory completability validation.

use std::collections::BTreeMap;

use crate::model::{OrderSpec, SkuId, StorageSpaceId};

/// Validate that the scenario's inventory can fulfill all orders.
///
/// For each SKU, sums total demand across all order lines and total supply
/// across all inventory entries. Returns `Err` if any SKU has demand > supply.
pub(super) fn validate_inventory_completability(
    orders: &[OrderSpec],
    inventory: &BTreeMap<(SkuId, StorageSpaceId), u32>,
) -> Result<(), String> {
    let mut demand: BTreeMap<SkuId, u64> = BTreeMap::new();
    for order in orders {
        for line in &order.lines {
            *demand.entry(line.sku).or_insert(0) += line.quantity as u64;
        }
    }

    let mut supply: BTreeMap<SkuId, u64> = BTreeMap::new();
    for (&(sku, _), &qty) in inventory {
        *supply.entry(sku).or_insert(0) += qty as u64;
    }

    let mut deficits: Vec<(SkuId, u64, u64)> = demand
        .iter()
        .filter_map(|(&sku, &sku_demand)| {
            let sku_supply = supply.get(&sku).copied().unwrap_or(0);
            (sku_demand > sku_supply).then_some((sku, sku_demand, sku_supply))
        })
        .collect();

    if deficits.is_empty() {
        return Ok(());
    }

    deficits.sort_by_key(|(sku, _, _)| sku.0);

    let mut msg = format!(
        "Inventory cannot fulfill all orders. {} SKU{} {} insufficient stock:\n",
        deficits.len(),
        if deficits.len() == 1 { "" } else { "s" },
        if deficits.len() == 1 { "has" } else { "have" },
    );
    for (sku, sku_demand, sku_supply) in &deficits {
        msg.push_str(&format!(
            "  SKU {}: demand={}, supply={} (deficit={})\n",
            sku.0,
            sku_demand,
            sku_supply,
            sku_demand - sku_supply,
        ));
    }
    msg.push_str("Set waive_completability_check=true in SimConfig to run anyway.");

    Err(msg)
}
