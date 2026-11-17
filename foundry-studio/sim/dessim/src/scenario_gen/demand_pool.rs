//! Bulk demand pool for late-binding order construction.
//!
//! A `DemandPool` represents *unfulfilled* demand: for each forward DC, how
//! many units of each SKU are still owed. The pool is the single source of
//! truth for two consumers:
//!
//! 1. **Putaway sequence** — the ordered product lines drive induction in
//!    source order, each gated no-earlier-than its `putaway_net_time`.
//! 2. **Order builder** — emit orders only when current inventory plus
//!    pool demand can produce a pallet in `[min_pallet, max_pallet]` units.
//!
//! The pool depletes as the builder commits orders. When it's empty AND
//! the ready queue is empty AND nothing is in flight, the pick demand
//! module reports `Done` and the planner halts.
//!
//! Loaded from a CSV with rows `putaway_net_secs, dc_id, sku, quantity`,
//! preserving file order as the product sequence. The upstream converter
//! (`cencora_to_bulk`) models case picks only (one case = one pick) and
//! emits in product-sequence order (numeric `PRD_HR` ascending).

use std::collections::BTreeMap;
use std::path::Path;

use csv::Reader;
use serde::Deserialize;

use crate::model::{SimTime, SkuId};

/// Forward-DC identifier, now defined in the model's destination vocabulary
/// (`crate::model::destination`) so striping and earmarks share it;
/// re-exported here because this module is where existing consumers import
/// it from.
pub use crate::model::destination::DistributionCenterId;

/// One lot of demand in **product-sequence order**: `qty` units of `sku` owed
/// to `distribution_center`, eligible for putaway induction no earlier than `putaway_net_time`
/// (a no-earlier-than gate derived from the source `PRD_HR`). The ordered
/// list of these *is* the product sequence — the single source list both the
/// putaway and pick flows copy and walk in order.
#[derive(Debug, Clone)]
pub struct ProductLine {
    /// No-earlier-than sim-time for putaway induction of this lot. Picking is
    /// unaffected by this — it is gated by `pick.embargo_secs` + per-DC min
    /// pallet quantity.
    pub putaway_net_time: SimTime,
    pub distribution_center: DistributionCenterId,
    pub sku: SkuId,
    pub qty: u32,
}

/// A snapshot of unfulfilled demand keyed by `(dc, sku) → units_remaining`,
/// plus the pallet-size bounds the order builder will use when packing
/// from this pool.
///
/// `total_per_sku` is a derived index: it equals
/// `Σ_dc owed.get((dc, sku))`. The two must stay consistent — use
/// [`DemandPool::consume`] to mutate.
///
/// Pallet bounds live on the pool so the consumer (OrderBuilder) doesn't
/// need a separate `Option<(u32, u32)>` plumbed alongside the pool — the
/// two are always created together from `BulkSourceScenario`.
#[derive(Debug, Clone)]
pub struct DemandPool {
    /// The product sequence: every demand lot in source order (numeric-hour
    /// ascending, source-row order within an hour), each carrying its
    /// `putaway_net_time`. This is the canonical ordered source both the
    /// putaway and pick flows copy and walk. The maps below are derived
    /// flat indices retained for the (soon-to-be-replaced) demand-total
    /// accessors.
    lines: Vec<ProductLine>,
    /// (`DistributionCenterId`, `SkuId`) → units remaining to be picked for that DC.
    /// Entries are removed when they drop to 0 so iteration sees only
    /// non-empty demand.
    owed: BTreeMap<(DistributionCenterId, SkuId), u32>,
    /// Derived: `SkuId → Σ_dc owed[(dc, sku)]`. Drives putaway priority
    /// and order-builder SKU selection.
    total_per_sku: BTreeMap<SkuId, u32>,
    /// Minimum total units in a constructed pallet/order. Builder
    /// returns None for any DC whose simultaneous-availability search
    /// can't reach this.
    min_pallet_units: u32,
    /// Maximum total units in a constructed pallet/order.
    max_pallet_units: u32,
}

impl DemandPool {
    /// Build an empty pool with the given pallet-size bounds.
    pub fn new(min_pallet_units: u32, max_pallet_units: u32) -> Self {
        assert!(min_pallet_units > 0, "min_pallet_units must be > 0");
        assert!(max_pallet_units >= min_pallet_units, "max_pallet_units must be >= min_pallet_units");
        Self {
            lines: Vec::new(),
            owed: BTreeMap::new(),
            total_per_sku: BTreeMap::new(),
            min_pallet_units,
            max_pallet_units,
        }
    }

    /// Consumers (putaway induction, pick builder) take their own working
    /// copy of the sequence and walk it in order.
    pub fn lines(&self) -> &[ProductLine] {
        &self.lines
    }

    /// Append a product line in sequence order, keeping the derived flat
    /// indices (`owed`, `total_per_sku`) consistent. Used by the loader.
    pub fn push_line(
        &mut self,
        putaway_net_time: SimTime,
        distribution_center: DistributionCenterId,
        sku: SkuId,
        qty: u32,
    ) {
        if qty == 0 {
            return;
        }
        self.lines.push(ProductLine { putaway_net_time, distribution_center: distribution_center.clone(), sku, qty });
        self.add(distribution_center, sku, qty);
    }

    pub fn min_pallet_units(&self) -> u32 {
        self.min_pallet_units
    }

    pub fn max_pallet_units(&self) -> u32 {
        self.max_pallet_units
    }

    /// Entries are dropped at zero, so this counts DC/SKU pairs still owed
    /// something — not pairs the pool has ever seen.
    pub fn entry_count(&self) -> usize {
        self.owed.len()
    }

    pub fn total_units(&self) -> u64 {
        self.owed.values().map(|&q| q as u64).sum()
    }

    /// `true` if every DC has been fully served.
    pub fn is_empty(&self) -> bool {
        self.owed.is_empty()
    }

    /// Total units of `sku` owed across all DCs. Returns 0 for SKUs not
    /// in the pool.
    pub fn sku_demand(&self, sku: SkuId) -> u32 {
        self.total_per_sku.get(&sku).copied().unwrap_or(0)
    }

    /// Iterator over `(SkuId, total_units)` for every SKU with non-zero
    /// pool-wide demand. Sorted by `SkuId` (BTreeMap iteration order).
    pub fn iter_sku_totals(&self) -> impl Iterator<Item = (SkuId, u32)> + '_ {
        self.total_per_sku.iter().map(|(&sku, &q)| (sku, q))
    }

    /// Units owed to `distribution_center` for `sku`. Returns 0 if no such entry.
    pub fn owed(&self, distribution_center: &str, sku: SkuId) -> u32 {
        self.owed.get(&(distribution_center.to_string(), sku)).copied().unwrap_or(0)
    }

    /// Iterator over `(&DistributionCenterId, SkuId, units)` for every non-zero entry,
    /// sorted by `(distribution_center, sku)`.
    pub fn iter(&self) -> impl Iterator<Item = (&DistributionCenterId, SkuId, u32)> + '_ {
        self.owed.iter().map(|((distribution_center, sku), &q)| (distribution_center, *sku, q))
    }

    /// Sum of remaining demand per distribution center. The map is rebuilt each
    /// call so callers should not call this on a hot path; the order builder
    /// invokes it at most once per scheduling tick.
    pub fn distribution_center_totals(&self) -> BTreeMap<&DistributionCenterId, u32> {
        let mut totals: BTreeMap<&DistributionCenterId, u32> = BTreeMap::new();
        for ((distribution_center, _sku), &q) in &self.owed {
            *totals.entry(distribution_center).or_insert(0) += q;
        }
        totals
    }

    /// Iterator over `(SkuId, units)` entries owed to `distribution_center`,
    /// sorted by `SkuId` (BTreeMap natural order). Empty if the DC has been
    /// fully served.
    pub fn entries_for_distribution_center<'a>(
        &'a self,
        distribution_center: &'a str,
    ) -> impl Iterator<Item = (SkuId, u32)> + 'a {
        self.owed
            .range(
                (distribution_center.to_string(), SkuId(u32::MIN))..=(distribution_center.to_string(), SkuId(u32::MAX)),
            )
            .map(|((_, sku), &q)| (*sku, q))
    }

    /// Consume `qty` units of `(distribution_center, sku)`. Updates both the
    /// per-DC entry and `total_per_sku`. An overdraw is refused, not clamped:
    /// it means an order was committed against demand the pool never held,
    /// so the run's order stream and its demand ledger have already parted.
    pub fn consume(&mut self, distribution_center: &str, sku: SkuId, qty: u32) -> Result<(), DemandPoolError> {
        if qty == 0 {
            return Ok(());
        }
        let overdraw = |remaining| DemandPoolError::Overdraw {
            distribution_center: distribution_center.to_string(),
            sku,
            qty,
            remaining,
        };
        let key = (distribution_center.to_string(), sku);
        let Some(entry) = self.owed.get_mut(&key) else { return Err(overdraw(0)) };
        if *entry < qty {
            return Err(overdraw(*entry));
        }
        *entry -= qty;
        if *entry == 0 {
            self.owed.remove(&key);
        }
        let Some(sku_total) = self.total_per_sku.get_mut(&sku) else {
            return Err(DemandPoolError::TotalsDesync { distribution_center: distribution_center.to_string(), sku });
        };
        *sku_total -= qty;
        if *sku_total == 0 {
            self.total_per_sku.remove(&sku);
        }
        Ok(())
    }

    /// Add `qty` units to `(dc, sku)`. Used by [`load_demand_pool`] to
    /// aggregate input rows and by tests to build small fixtures; not
    /// intended for runtime mutation outside the loader.
    pub(crate) fn add(&mut self, distribution_center: DistributionCenterId, sku: SkuId, qty: u32) {
        if qty == 0 {
            return;
        }
        *self.owed.entry((distribution_center, sku)).or_insert(0) += qty;
        *self.total_per_sku.entry(sku).or_insert(0) += qty;
    }
}

/// Why a pool mutation was refused. Both variants say the same thing about
/// the run: demand was drawn that the ledger never owed, so any downstream
/// count is already wrong and the run must stop.
#[derive(Debug, thiserror::Error)]
pub enum DemandPoolError {
    #[error("consume {qty} of (dc={distribution_center}, sku={sku:?}) but only {remaining} owed")]
    Overdraw { distribution_center: String, sku: SkuId, qty: u32, remaining: u32 },
    #[error("per-SKU totals lost {sku:?} while consuming for dc={distribution_center}")]
    TotalsDesync { distribution_center: String, sku: SkuId },
}

/// Row schema for the normalized bulk demand CSV (product sequence).
#[derive(Debug, Deserialize)]
struct BulkRow {
    /// No-earlier-than sim-time (seconds) for putaway induction of this lot.
    putaway_net_secs: u32,
    #[serde(rename = "dc_id")]
    distribution_center_id: String,
    sku: String,
    quantity: u32,
}

/// Load a `DemandPool` (product sequence) from a normalized bulk CSV
/// (`putaway_net_secs, dc_id, sku, quantity`), preserving row order as the
/// sequence.
///
/// `sku_lookup` resolves source SKU strings to numeric [`SkuId`]s using
/// the same map produced by the items.csv loader. SKUs in the bulk file
/// that are absent from the items catalog return an error — silently
/// dropping them would hide a real configuration mismatch.
pub fn load_demand_pool(
    path: &Path,
    sku_lookup: &BTreeMap<String, SkuId>,
    min_pallet_units: u32,
    max_pallet_units: u32,
    ignore_hour_gate: bool,
) -> Result<DemandPool, String> {
    let mut reader =
        Reader::from_path(path).map_err(|e| format!("failed to open bulk demand CSV {}: {}", path.display(), e))?;
    let mut pool = DemandPool::new(min_pallet_units, max_pallet_units);
    for (i, result) in reader.deserialize().enumerate() {
        let row: BulkRow = result.map_err(|e| format!("bulk CSV row {}: {}", i + 2, e))?;
        let sku_id = sku_lookup
            .get(&row.sku)
            .copied()
            .ok_or_else(|| format!("bulk CSV row {}: unknown SKU '{}'", i + 2, row.sku))?;
        // Gate bypassed → induct in sequence order with no net-time floor.
        let net_time = if ignore_hour_gate { SimTime::ZERO } else { SimTime::seconds(row.putaway_net_secs as u64) };
        pool.push_line(net_time, row.distribution_center_id, sku_id, row.quantity);
    }
    eprintln!(
        "demand_pool: loaded {} product lines from {} → {} (dc,sku) entries, {} total units",
        pool.lines().len(),
        path.display(),
        pool.entry_count(),
        pool.total_units(),
    );
    Ok(pool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_then_consume_zeroes_out_correctly() {
        let mut pool = DemandPool::new(40, 50);
        pool.add("dc1".into(), SkuId(0), 10);
        pool.add("dc1".into(), SkuId(1), 20);
        pool.add("dc2".into(), SkuId(0), 5);
        assert_eq!(pool.entry_count(), 3);
        assert_eq!(pool.total_units(), 35);
        assert_eq!(pool.sku_demand(SkuId(0)), 15);
        assert_eq!(pool.sku_demand(SkuId(1)), 20);

        pool.consume("dc1", SkuId(0), 10).unwrap();
        assert_eq!(pool.entry_count(), 2);
        assert_eq!(pool.sku_demand(SkuId(0)), 5); // only dc2 left for sku 0
        assert_eq!(pool.owed("dc1", SkuId(0)), 0);

        pool.consume("dc1", SkuId(1), 20).unwrap();
        pool.consume("dc2", SkuId(0), 5).unwrap();
        assert!(pool.is_empty());
        assert_eq!(pool.total_per_sku.len(), 0);
    }

    #[test]
    fn add_aggregates_duplicate_keys() {
        let mut pool = DemandPool::new(40, 50);
        pool.add("dc1".into(), SkuId(0), 10);
        pool.add("dc1".into(), SkuId(0), 5);
        assert_eq!(pool.owed("dc1", SkuId(0)), 15);
        assert_eq!(pool.sku_demand(SkuId(0)), 15);
    }

    #[test]
    fn consume_more_than_owed_is_refused_and_leaves_the_pool_intact() {
        let mut pool = DemandPool::new(40, 50);
        pool.add("dc1".into(), SkuId(0), 5);
        let err = pool.consume("dc1", SkuId(0), 10).unwrap_err();
        assert!(matches!(err, DemandPoolError::Overdraw { qty: 10, remaining: 5, .. }), "{err}");
        assert_eq!(pool.owed("dc1", SkuId(0)), 5);
        assert!(pool.consume("dc2", SkuId(0), 1).is_err(), "unknown dc is an overdraw of 0");
    }

    /// Load the real peak_day_12_16 fixture into `(pool, sku_lookup)`, or
    /// `None` if the fixtures aren't checked out. Shared by the load-shape and
    /// net-gate tests; mirrors what `resolve_bulk` does.
    fn load_cencora_pool(ignore_hour_gate: bool) -> Option<(DemandPool, BTreeMap<String, SkuId>)> {
        use std::collections::BTreeMap;
        use std::path::PathBuf;
        let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../desmap/tests/fixtures/cencora_data");
        let items_csv = fixture_dir.join("items.csv");
        let bulk_csv = fixture_dir.join("bulk/peak_day_12_16.csv");
        if !items_csv.exists() || !bulk_csv.exists() {
            eprintln!("skipping: fixtures missing ({}, {})", items_csv.display(), bulk_csv.display());
            return None;
        }
        let mut sku_lookup: BTreeMap<String, SkuId> = BTreeMap::new();
        let mut reader = Reader::from_path(&items_csv).expect("open items.csv");
        for (i, result) in reader.records().enumerate() {
            let record = result.expect("parse items.csv");
            let sku = record.get(0).expect("sku col").to_string();
            sku_lookup.insert(sku, SkuId(i as u32));
        }
        let pool = load_demand_pool(&bulk_csv, &sku_lookup, 35, 50, ignore_hour_gate).expect("load pool");
        Some((pool, sku_lookup))
    }

    /// Sanity-check the real peak_day_12_16 fixture against the converter's
    /// reported counts: 16,903 source rows → 11,247 (hr,dc,sku) product lines,
    /// 10,782 unique (dc,sku) pairs, 66,276 total units (cases only). Exercises
    /// `load_demand_pool` against the actual converted bulk CSV and verifies the
    /// product-sequence load + flat-index aggregation are stable.
    #[test]
    fn load_peak_day_12_16_fixture() {
        let Some((pool, sku_lookup)) = load_cencora_pool(false) else { return };
        assert_eq!(pool.lines().len(), 11247, "product-sequence line count");
        assert_eq!(pool.entry_count(), 10782, "(dc,sku) entry count");
        assert_eq!(pool.total_units(), 66_276, "total units owed (cases only)");
        // Product sequence is putaway_net_time non-decreasing (hour-ascending).
        assert!(
            pool.lines().windows(2).all(|w| w[0].putaway_net_time <= w[1].putaway_net_time),
            "product lines must be ordered by non-decreasing putaway_net_time",
        );
        // Spot-check: 087ABC003 owes 1 case of SKU 2944240.
        let sku_2944240 = *sku_lookup.get("2944240").expect("known sku in fixture");
        assert_eq!(pool.owed("087ABC003", sku_2944240), 1);
    }

    /// `ignore_hour_gate = true` collapses every lot's induction floor to
    /// `SimTime::ZERO` (no hourly gate), leaving demand and sequence untouched.
    /// This is the `--settings putaway.ignore_hour_gate=true` behavior.
    #[test]
    fn gate_off_zeroes_net_time() {
        let Some((gated, _)) = load_cencora_pool(false) else { return };
        let Some((ungated, _)) = load_cencora_pool(true) else { return };
        assert!(
            gated.lines().iter().any(|line| line.putaway_net_time > SimTime::ZERO),
            "gated load should carry hourly net-time floors",
        );
        assert!(
            ungated.lines().iter().all(|line| line.putaway_net_time == SimTime::ZERO),
            "gate off must zero every net-time",
        );
        // Only the induction floor changes — demand and order are identical.
        assert_eq!(gated.total_units(), ungated.total_units());
        assert_eq!(gated.lines().len(), ungated.lines().len());
    }
}
