//! Within `studio-protocol`, `workload` is what spec 07's generator is asked for and what it answers (section 3.8):
//! the generator endpoint's body, the NDJSON lines of the workload it streams, each order's arrival in simtime, and
//! the readiness a scenario version reads at the heads of its site's `main`, with the cut it read and what the
//! generator made. No workload data is stored anywhere (ruling D96): these travel and are made again from the spec.

use std::collections::BTreeMap;

use desunits::{SimDuration, SimTime};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::clock::WallClock;
use crate::grammar::{BranchName, Pin, ScopePath, Seq};
use crate::problem::ProblemItem;
use crate::scenario::GeneratorSpec;

/// The site's inventory tote slots: rack positions whose use is inventory, times the tote positions per rack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export_to = "workload.ts")]
pub struct SlotCount(pub u64);

/// A count of what a workload holds: orders, lines, SKUs, or units of stock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export_to = "workload.ts")]
pub struct Tally(pub u64);

/// The share of each SKU's units FillInventory places on the racks at simtime 0, rounded up, 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize, TS)]
#[ts(export_to = "workload.ts")]
pub struct PreloadFraction(pub f64);

/// The generator endpoint's body, version 1: the version's generator spec with the run's variant applied, the run's
/// effective duration, and the two site facts the caller read from the world it loaded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "workload.ts")]
pub struct GenerateRequest {
    pub v: u32,
    pub generator: GeneratorSpec,
    #[ts(type = "number")]
    pub duration_sim_ms: SimDuration,
    pub slots: SlotCount,
    pub brings_stock_in: bool,
}

/// The stream's first line: the generator's crate version and what follows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "workload.ts")]
pub struct WorkloadHeader {
    pub v: u32,
    pub generator: String,
    pub orders: Tally,
    pub lines: Tally,
    pub skus: Tally,
    pub preload_fraction: PreloadFraction,
}

/// One SKU holding stock, and how many units of it the run starts with or brings in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "workload.ts")]
pub struct WorkloadItem {
    pub sku: String,
    pub units: Tally,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "workload.ts")]
pub struct OrderLine {
    pub sku: String,
    pub quantity: Tally,
}

/// One order, its arrival in simtime and its lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "workload.ts")]
pub struct WorkloadOrder {
    pub id: String,
    #[ts(type = "number")]
    pub arrival_ms: SimTime,
    pub lines: Vec<OrderLine>,
}

/// One NDJSON line: the header, then an item per SKU holding stock, then an order per order in arrival order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "workload.ts")]
pub enum WorkloadLine {
    Header(WorkloadHeader),
    Item(WorkloadItem),
    Order(WorkloadOrder),
}

/// Spec 04's `UnitCut` as the API answers it: each scope of the site at one changeset, read at one pin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "workload.ts")]
pub struct UnitCut {
    pub unit: ScopePath,
    pub requested: Pin,
    pub branch: BranchName,
    // ordered: a cut lists its scopes in path order.
    pub seqs: BTreeMap<ScopePath, Seq>,
    pub exact: bool,
    pub exact_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "workload.ts")]
pub struct WorkloadSummary {
    pub orders: Tally,
    pub lines: Tally,
    pub skus: Tally,
    pub preload_fraction: PreloadFraction,
}

/// `GET /api/scenarios/{s}/readiness`'s answer, read at the wall-clock `checked_at`: `ready` when no problem is an
/// error; `cut` null when the site could not be read, `workload` null when nothing was generated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "workload.ts")]
pub struct Readiness {
    pub checked_at: WallClock,
    pub cut: Option<UnitCut>,
    pub ready: bool,
    pub workload: Option<WorkloadSummary>,
    pub problems: Vec<ProblemItem>,
}
