//! Scenario configuration: how SKUs, orders, and inventory are sourced.
//!
//! [`ScenarioConfig`] is an enum with four variants:
//! - **Generated** — parametric generation from counts/percentages.
//! - **Inline** — SKU catalog and orders embedded directly in the JSON.
//! - **Csv** — paths to normalized CSV files (items.csv + orders.csv).
//!   Pre-formed orders — the order set is fixed at sim start.
//! - **BulkSource** — items.csv + a *bulk demand pool* CSV
//!   (`putaway_net_secs, dc_id, sku, quantity`). Orders are NOT pre-formed;
//!   the simulator builds pallets
//!   of `[min_pallet_units, max_pallet_units]` on the fly, drawing from
//!   whatever inventory the putaway side has actually delivered. This
//!   makes throughput robust to putaway sequence: the sim can't stall on
//!   "the next pre-committed order needs SKU X but X isn't in rack yet."

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// ScenarioConfig (top-level enum)
// ---------------------------------------------------------------------------

/// How the scenario's SKUs, orders, and inventory are sourced.
///
/// Deserialized with `#[serde(untagged)]` so existing JSON files (with
/// `num_skus`, etc.) still parse as `Generated`, while new files with
/// `skus`/`orders` arrays parse as `Inline`, files with `items_csv`/
/// `orders_csv` strings parse as `Csv`, and files with `items_csv`/
/// `bulk_csv` parse as `BulkSource`.
///
/// Order of variants matters: serde tries each in turn and accepts the
/// first that parses. `BulkSource` is listed before `Csv` because both
/// share `items_csv`; only `BulkSource` has `bulk_csv` (mandatory) and
/// only `Csv` has `orders_csv` (mandatory), so the two are unambiguous
/// when both fields are mandatory — but the ordering pins the tie-break
/// in case someone passes both, in which case bulk wins (intentional).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ScenarioConfig {
    Generated(GeneratedScenario),
    Inline(InlineScenario),
    BulkSource(BulkSourceScenario),
    Csv(CsvScenario),
    SiteExtract(SiteExtractScenario),
}

impl Default for ScenarioConfig {
    fn default() -> Self {
        ScenarioConfig::Generated(GeneratedScenario::default())
    }
}

// ---------------------------------------------------------------------------
// Generated (parametric)
// ---------------------------------------------------------------------------

/// Parameters for programmatic generation of SKUs, orders, and inventory.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GeneratedScenario {
    /// Number of distinct SKUs in the item catalog.
    pub num_skus: usize,
    /// Total inventory units distributed across storage spaces.
    pub total_inventory_units: usize,
    /// Number of customer orders to generate.
    pub num_orders: usize,
    /// Total picks distributed across all orders.
    pub total_order_picks: usize,
    /// Maximum units of a single SKU placed in one tote/storage space.
    pub units_per_tote: u32,
    /// Fraction of SKUs considered "popular" (0.0-1.0).
    pub popular_skus_pct: f64,
    /// Share of total inventory allocated to popular SKUs (0.0-1.0).
    pub inventory_popular_share: f64,
    /// Share of order picks drawn from popular SKUs (0.0-1.0).
    pub order_popular_share: f64,
    /// How many totes are designated to carry ORDERS rather than stock.
    ///
    /// One physical tote type, two pools, and the designation is permanent per
    /// tote — anti-starvation: if either side could draw from the other, one
    /// of them eventually cannot work (Vinay, 2026-08-07; the world protocol
    /// says the same as `carrier_pool`). Real Dallas talk is ~800 inventory /
    /// 200 order of ~1,000 totes.
    ///
    /// Zero means this scenario has no order pool — true of every scenario
    /// that does not run order totes.
    pub order_totes: u32,
}

impl Default for GeneratedScenario {
    fn default() -> Self {
        Self {
            num_skus: 500,
            total_inventory_units: 320_000,
            num_orders: 13_000,
            total_order_picks: 65_000,
            units_per_tote: 1,
            popular_skus_pct: 0.2,
            inventory_popular_share: 0.8,
            order_popular_share: 0.7,
            order_totes: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Inline (embedded in JSON)
// ---------------------------------------------------------------------------

/// SKU catalog and orders embedded directly in the sim spec JSON.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InlineScenario {
    pub skus: Vec<InlineSku>,
    pub orders: Vec<InlineOrder>,
    #[serde(default = "default_units_per_tote")]
    pub units_per_tote: u32,
}

/// A SKU definition in an inline scenario.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InlineSku {
    /// Unique string key (e.g. item number from source data).
    pub sku: String,
    /// Weight in kilograms (used for pick sequencing).
    pub weight_kg: f32,
    /// Total inventory units to place in storage.
    pub inventory_units: u32,
}

/// An order in an inline scenario.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InlineOrder {
    pub lines: Vec<InlineOrderLine>,
}

/// A single line within an inline order.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InlineOrderLine {
    /// References an [`InlineSku::sku`] key.
    pub sku: String,
    pub quantity: u32,
}

// ---------------------------------------------------------------------------
// Csv (external file paths)
// ---------------------------------------------------------------------------

/// Paths to normalized CSV files containing SKU catalog and orders.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CsvScenario {
    /// Path to items CSV: `sku,weight_kg,inventory_units`
    pub items_csv: String,
    /// Path to orders CSV: `order_id,sku,quantity,weight_kg`
    pub orders_csv: String,
    #[serde(default = "default_units_per_tote")]
    pub units_per_tote: u32,
}

fn default_units_per_tote() -> u32 {
    1
}

// ---------------------------------------------------------------------------
// SiteExtract (a site's own delivered files)
// ---------------------------------------------------------------------------

/// Paths to a site's delivered extracts, which carry order trait columns a
/// normalized [`CsvScenario`] has no room for. The column vocabulary is the
/// reader's business, not this struct's.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SiteExtractScenario {
    /// `order_id,order_line_id,sku_id,...,destination_type,...,shipping_protection,...`
    pub outbound_csv: String,
    /// `item_id,channel,Velocity_Band,...,on_hand_qty`
    pub onhand_csv: String,
    /// Optional explicit additions (`item_id,additional_units`) applied to the
    /// snapshot before stock selection. The source extract is never modified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stock_top_up_csv: Option<String>,
    /// Required. A site may name a container class we have no model for; a
    /// run states its own assumption rather than inheriting one.
    pub unknown_container: UnknownContainerPolicy,
    /// Which quantities from the on-hand extract to preload.
    pub stock: StockPolicy,
    /// How that stock is distributed among inventory totes. This is a
    /// placement policy, not a restriction on what a tote may contain.
    #[serde(default)]
    pub inventory_distribution: InventoryDistribution,
    /// Totes designated to carry orders rather than stock. See
    /// [`GeneratedScenario::order_totes`]; the extract states demand, not the
    /// tote population, so this is the run's.
    #[serde(default)]
    pub order_totes: u32,
    #[serde(default = "default_units_per_tote")]
    pub units_per_tote: u32,
}

/// Automatic preload placement, independent of the selected stock quantities.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryDistribution {
    /// Pack each SKU separately, using `units_per_tote` as the batch size.
    #[default]
    SingleSku,
    /// Spread all units over inventory totes with totals differing by at
    /// most one. Totes may contain multiple SKUs; `units_per_tote` is a cap.
    Even,
}

/// How much of a site's on-hand extract to place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StockPolicy {
    /// Every SKU at its extracted quantity. Even distribution refuses a
    /// preload that exceeds the available inventory tote capacity.
    AsExtracted,
    /// Only the SKUs the orders touch, each stocked to the lesser of its
    /// on-hand quantity and what the orders demand. The pilot holds the stock
    /// its workload needs, without inventory depth or ABC stratification.
    CoverDemand,
}

/// What to do with a container class the extract names and the model has no
/// definition for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownContainerPolicy {
    AsTote,
    AsCarton,
    /// Provisional Walmart SIOC handling: use a reusable order tray.
    AsTray,
    /// Refuse to load. Correct while the question is open and the run cannot
    /// tolerate a guess.
    Refuse,
}

// ---------------------------------------------------------------------------
// BulkSource (late-binding orders from a demand pool)
// ---------------------------------------------------------------------------

/// Paths to an items CSV (same schema as [`CsvScenario::items_csv`]) and a
/// **bulk demand** CSV with rows `putaway_net_secs, dc_id, sku, quantity`.
///
/// Multiple rows sharing the same `(dc_id, sku)` are summed. Orders are
/// not pre-formed; see the module docs for `OrderBuilder` semantics.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BulkSourceScenario {
    /// Path to items CSV: `sku,weight_kg,inventory_units` (same as Csv).
    pub items_csv: String,
    /// Path to bulk CSV: `putaway_net_secs,dc_id,sku,quantity` (the product
    /// sequence — one lot per row, in induction order).
    pub bulk_csv: String,
    /// Minimum units in a constructed pallet/order. Builder won't emit an
    /// order with fewer units than this — even at end of pool, it just
    /// writes off the remainder.
    #[serde(default = "default_min_pallet_units")]
    pub min_pallet_units: u32,
    /// Maximum units in a constructed pallet/order. Builder stops adding
    /// lines once the running total hits or exceeds this.
    #[serde(default = "default_max_pallet_units")]
    pub max_pallet_units: u32,
    #[serde(default = "default_units_per_tote")]
    pub units_per_tote: u32,
}

fn default_min_pallet_units() -> u32 {
    35
}

fn default_max_pallet_units() -> u32 {
    50
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

impl ScenarioConfig {
    /// Total number of orders, if known statically from the config.
    pub fn num_orders(&self) -> Option<usize> {
        match self {
            ScenarioConfig::Generated(g) => Some(g.num_orders),
            _ => None,
        }
    }

    /// Total order picks, if known statically from the config.
    pub fn total_order_picks(&self) -> Option<usize> {
        match self {
            ScenarioConfig::Generated(g) => Some(g.total_order_picks),
            _ => None,
        }
    }
}
