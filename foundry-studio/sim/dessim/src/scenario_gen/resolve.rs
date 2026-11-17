//! Resolve inline or CSV scenario data into SkuInfo + Order.
//!
//! Both paths produce the same output: a Vec<SkuInfo> (with weight_kg) and
//! a Vec<Order> with lines sorted by SKU weight descending (heaviest first).

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::Path;

use csv::Reader;

use crate::model::{OrderId, OrderLine, OrderSpec as Order, SkuId};
use crate::scenario::{BulkSourceScenario, CsvScenario, InlineScenario};

use super::demand_pool::{DemandPool, load_demand_pool};
use super::types::SkuInfo;

// ---------------------------------------------------------------------------
// Inline resolution
// ---------------------------------------------------------------------------

/// Resolve an inline scenario: map string SKU keys → SkuId, build Order,
/// sort lines by weight descending.
pub fn resolve_inline(inline: &InlineScenario) -> Result<(Vec<SkuInfo>, Vec<Order>), String> {
    // Build SKU key → SkuId mapping.
    let mut key_to_id: BTreeMap<&str, SkuId> = BTreeMap::new();
    let mut skus = Vec::with_capacity(inline.skus.len());

    for (i, src) in inline.skus.iter().enumerate() {
        let id = SkuId(i as u32);
        key_to_id.insert(&src.sku, id);
        skus.push(SkuInfo { id, is_popular: false, inventory_units: src.inventory_units, weight_kg: src.weight_kg });
    }

    // Build weight lookup for sequencing.
    let weight_by_id: BTreeMap<SkuId, f32> = skus.iter().map(|s| (s.id, s.weight_kg)).collect();

    // Build orders.
    let mut orders = Vec::with_capacity(inline.orders.len());
    for (i, src) in inline.orders.iter().enumerate() {
        let mut lines = Vec::with_capacity(src.lines.len());
        for line in &src.lines {
            let sku = *key_to_id
                .get(line.sku.as_str())
                .ok_or_else(|| format!("inline order {}: unknown SKU key '{}'", i, line.sku))?;
            lines.push(OrderLine { sku, quantity: line.quantity });
        }
        // Sort lines by weight descending (heaviest first).
        lines.sort_by(|a, b| {
            let wa = weight_by_id.get(&a.sku).copied().unwrap_or(0.0);
            let wb = weight_by_id.get(&b.sku).copied().unwrap_or(0.0);
            wb.partial_cmp(&wa).unwrap_or(Ordering::Equal)
        });
        orders.push(Order::unstated(OrderId(i as u32), lines));
    }

    Ok((skus, orders))
}

// ---------------------------------------------------------------------------
// CSV resolution
// ---------------------------------------------------------------------------

/// Row from normalized items.csv: sku,weight_kg,inventory_units
#[derive(Debug, serde::Deserialize)]
struct ItemRow {
    sku: String,
    weight_kg: f32,
    inventory_units: u32,
}

/// Row from normalized orders.csv: order_id,sku,quantity,weight_kg
#[derive(Debug, serde::Deserialize)]
struct OrderRow {
    order_id: String,
    sku: String,
    quantity: u32,
    weight_kg: f32,
}

/// Resolve a CSV scenario: load items + orders CSVs, map to SkuInfo + Order,
/// sort lines by weight descending.
pub fn resolve_csv(csv: &CsvScenario) -> Result<(Vec<SkuInfo>, Vec<Order>), String> {
    let items_path = Path::new(&csv.items_csv);
    let orders_path = Path::new(&csv.orders_csv);

    // Load items.
    let mut key_to_id: BTreeMap<String, SkuId> = BTreeMap::new();
    let mut skus = Vec::new();
    {
        let mut reader =
            Reader::from_path(items_path).map_err(|e| format!("failed to open {}: {}", items_path.display(), e))?;
        for (i, result) in reader.deserialize().enumerate() {
            let row: ItemRow = result.map_err(|e| format!("items.csv row {}: {}", i + 1, e))?;
            let id = SkuId(i as u32);
            key_to_id.insert(row.sku, id);
            skus.push(SkuInfo {
                id,
                is_popular: false,
                inventory_units: row.inventory_units,
                weight_kg: row.weight_kg,
            });
        }
    }

    // Load orders — group rows by order_id.
    let mut order_lines: BTreeMap<String, Vec<(SkuId, u32, f32)>> = BTreeMap::new();
    {
        let mut reader =
            Reader::from_path(orders_path).map_err(|e| format!("failed to open {}: {}", orders_path.display(), e))?;
        for (i, result) in reader.deserialize().enumerate() {
            let row: OrderRow = result.map_err(|e| format!("orders.csv row {}: {}", i + 1, e))?;
            let sku_id = *key_to_id
                .get(&row.sku)
                .ok_or_else(|| format!("orders.csv row {}: unknown SKU '{}'", i + 1, row.sku))?;
            order_lines.entry(row.order_id).or_default().push((sku_id, row.quantity, row.weight_kg));
        }
    }

    // Convert to Order with weight-sorted lines.
    let mut orders = Vec::with_capacity(order_lines.len());
    for (i, (_order_key, mut lines_raw)) in order_lines.into_iter().enumerate() {
        // Sort by weight descending.
        lines_raw.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(Ordering::Equal));
        let lines: Vec<OrderLine> =
            lines_raw.into_iter().map(|(sku, qty, _wt)| OrderLine { sku, quantity: qty }).collect();
        orders.push(Order::unstated(OrderId(i as u32), lines));
    }

    Ok((skus, orders))
}

// ---------------------------------------------------------------------------
// Bulk-source resolution (late-binding orders)
// ---------------------------------------------------------------------------

/// Resolve a [`BulkSourceScenario`]: load items.csv + bulk.csv → SKU
/// catalog + [`DemandPool`].
///
/// Unlike [`resolve_csv`] this returns *no* pre-formed orders — the
/// order builder constructs them during the simulation, drawing from the
/// pool as inventory permits.
pub fn resolve_bulk(bulk: &BulkSourceScenario, ignore_hour_gate: bool) -> Result<(Vec<SkuInfo>, DemandPool), String> {
    let items_path = Path::new(&bulk.items_csv);
    let bulk_path = Path::new(&bulk.bulk_csv);

    // Same items.csv loader as `resolve_csv` — kept inline rather than
    // factored to keep the two paths independently editable while their
    // schemas converge or diverge.
    let mut key_to_id: BTreeMap<String, SkuId> = BTreeMap::new();
    let mut skus = Vec::new();
    let mut reader =
        Reader::from_path(items_path).map_err(|e| format!("failed to open {}: {}", items_path.display(), e))?;
    for (i, result) in reader.deserialize().enumerate() {
        let row: ItemRow = result.map_err(|e| format!("items.csv row {}: {}", i + 1, e))?;
        let id = SkuId(i as u32);
        key_to_id.insert(row.sku, id);
        skus.push(SkuInfo { id, is_popular: false, inventory_units: row.inventory_units, weight_kg: row.weight_kg });
    }

    if bulk.min_pallet_units == 0 || bulk.max_pallet_units < bulk.min_pallet_units {
        return Err(format!(
            "invalid pallet bounds: min={}, max={} (require 1 <= min <= max)",
            bulk.min_pallet_units, bulk.max_pallet_units,
        ));
    }

    let pool = load_demand_pool(bulk_path, &key_to_id, bulk.min_pallet_units, bulk.max_pallet_units, ignore_hour_gate)?;

    Ok((skus, pool))
}
