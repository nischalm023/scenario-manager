//! Load pregenerated scenarios from Java-format CSV files.
//!
//! Parses `item_catalog.csv` and `orders.csv` produced by the Java DES scenario
//! generator. The loaded data feeds into the existing `generate_scenario_from_map`
//! pipeline at steps 3/4, replacing procedural generation.

use std::collections::BTreeMap;
use std::path::Path;

use csv::Reader;
use serde::Deserialize;

use crate::model::{OrderId, OrderLine, OrderSpec, SkuId};
use crate::scenario_gen::SkuInfo;

// ---------------------------------------------------------------------------
// CSV row structs (Java format)
//
// Fields marked #[allow(dead_code)] are populated by serde during CSV
// deserialization but not read by the loader. They must remain to match the
// CSV column layout; removing them would break positional deserialization.
// ---------------------------------------------------------------------------

/// Row from `item_catalog.csv`:
/// `sku,name,category,weight_kg,is_popular,max_inventory_quantity,ordered_quantity`
#[derive(Debug, Deserialize)]
struct ItemCatalogRow {
    sku: String,
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    category: String,
    #[allow(dead_code)]
    weight_kg: f64,
    is_popular: String, // "True" or "False"
    max_inventory_quantity: u32,
    #[allow(dead_code)]
    ordered_quantity: u32,
}

/// Row from `orders.csv`:
/// `order_id,customer_type,items,picks,item_list`
#[derive(Debug, Deserialize)]
struct OrderRow {
    order_id: String,
    #[allow(dead_code)]
    customer_type: String,
    #[allow(dead_code)]
    items: u32,
    #[allow(dead_code)]
    picks: u32,
    item_list: String,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse a Java `item_catalog.csv` file into `SkuInfo` entries.
///
/// SKU names like "SKU001" are mapped to `SkuId(0)`, "SKU002" to `SkuId(1)`, etc.
/// Returns the SKU list and a name-to-id lookup map.
pub fn load_item_catalog(path: &Path) -> Result<(Vec<SkuInfo>, BTreeMap<String, SkuId>), String> {
    let mut reader = Reader::from_path(path).map_err(|e| format!("failed to open {}: {}", path.display(), e))?;

    let mut skus = Vec::new();
    let mut name_to_id = BTreeMap::new();

    for (i, result) in reader.deserialize().enumerate() {
        let row: ItemCatalogRow =
            result.map_err(|e| format!("failed to parse row {} in {}: {}", i + 1, path.display(), e))?;

        let sku_id = SkuId(i as u32);
        name_to_id.insert(row.sku.clone(), sku_id);

        skus.push(SkuInfo {
            id: sku_id,
            is_popular: row.is_popular.eq_ignore_ascii_case("true"),
            inventory_units: row.max_inventory_quantity,
            weight_kg: row.weight_kg as f32,
        });
    }

    Ok((skus, name_to_id))
}

/// Parse a Java `orders.csv` file into `Order` entries.
///
/// Resolves SKU names (e.g. "SKU038") to `SkuId` via the provided lookup map.
/// Item list format: `SKU038:1;SKU034:2` (semicolon-separated, colon qty).
pub fn load_orders(path: &Path, sku_lookup: &BTreeMap<String, SkuId>) -> Result<Vec<OrderSpec>, String> {
    let mut reader = Reader::from_path(path).map_err(|e| format!("failed to open {}: {}", path.display(), e))?;

    let mut orders = Vec::new();

    for (i, result) in reader.deserialize().enumerate() {
        let row: OrderRow =
            result.map_err(|e| format!("failed to parse row {} in {}: {}", i + 1, path.display(), e))?;

        let mut lines = Vec::new();
        for item_spec in row.item_list.split(';') {
            let item_spec = item_spec.trim();
            if item_spec.is_empty() {
                continue;
            }
            let parts: Vec<&str> = item_spec.split(':').collect();
            if parts.len() != 2 {
                return Err(format!("invalid item spec '{}' in order {} (row {})", item_spec, row.order_id, i + 1,));
            }
            let sku_name = parts[0];
            let qty: u32 = parts[1].parse().map_err(|e| {
                format!("invalid quantity '{}' in order {} (row {}): {}", parts[1], row.order_id, i + 1, e,)
            })?;
            let sku_id = sku_lookup
                .get(sku_name)
                .ok_or_else(|| format!("unknown SKU '{}' in order {} (row {})", sku_name, row.order_id, i + 1,))?;
            lines.push(OrderLine { sku: *sku_id, quantity: qty });
        }

        orders.push(OrderSpec::unstated(OrderId(i as u32), lines));
    }

    Ok(orders)
}

/// Convenience: load both item catalog and orders from a scenario directory.
///
/// Expects `item_catalog.csv` and `orders.csv` in the given directory.
pub fn load_scenario_from_dir(dir: &Path) -> Result<(Vec<SkuInfo>, Vec<OrderSpec>), String> {
    let catalog_path = dir.join("item_catalog.csv");
    let orders_path = dir.join("orders.csv");

    if !catalog_path.exists() {
        return Err(format!("item_catalog.csv not found in {}", dir.display()));
    }
    if !orders_path.exists() {
        return Err(format!("orders.csv not found in {}", dir.display()));
    }

    let (skus, sku_lookup) = load_item_catalog(&catalog_path)?;
    let orders = load_orders(&orders_path, &sku_lookup)?;

    Ok((skus, orders))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scenario_a_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scenario_a")
    }

    #[test]
    fn load_scenario_a_catalog() {
        let dir = scenario_a_dir();
        let (skus, lookup) = load_item_catalog(&dir.join("item_catalog.csv")).unwrap();
        assert_eq!(skus.len(), 500);
        assert!(lookup.contains_key("SKU001"));
        assert!(lookup.contains_key("SKU500"));
        let popular_count = skus.iter().filter(|s| s.is_popular).count();
        assert_eq!(popular_count, 100);
    }

    #[test]
    fn load_scenario_a_orders() {
        let dir = scenario_a_dir();
        let (_, lookup) = load_item_catalog(&dir.join("item_catalog.csv")).unwrap();
        let orders = load_orders(&dir.join("orders.csv"), &lookup).unwrap();
        assert_eq!(orders.len(), 1185);
        let total_picks: u32 = orders.iter().flat_map(|o| &o.lines).map(|l| l.quantity).sum();
        assert_eq!(total_picks, 3920);
    }

    #[test]
    fn load_scenario_a_from_dir() {
        let dir = scenario_a_dir();
        let (skus, orders) = load_scenario_from_dir(&dir).unwrap();
        assert_eq!(skus.len(), 500);
        assert_eq!(orders.len(), 1185);
    }
}
