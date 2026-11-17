//! Convert raw cencora "Order Data" CSV to the simulator's normalized bulk
//! pick-demand format (the "product sequence").
//!
//! The cencora source format encodes demand per (PRD_DTE, PRD_HR, FWD_DC,
//! SKU) split into eaches (`EACH_PICKS`) and cases (`CASES` × `CASE_QTY`).
//! We model **case picks only** — `quantity = CASES` (one case = one pick).
//! The each-pick stream is handled by a separate system outside our scope.
//!
//! The production hour `PRD_HR` is preserved as a putaway eligibility gate:
//! the output carries `putaway_net_secs = PRD_HR * 3600` — a no-earlier-than
//! ("net") sim-time at which that lot may be inducted into putaway. Rows are
//! emitted in **product-sequence order**: numeric `PRD_HR` ascending, then
//! source-row order within an hour (no DC/SKU sortation). Same
//! `(PRD_HR, FWD_DC, SKU)` rows are summed at their first occurrence.
//!
//! `PRD_DTE` is discarded — single-day input is assumed and the earliest
//! (only) date normalizes to sim-time 0. Multi-day input is rejected rather
//! than silently mis-ordered (see the distinct-date guard below).
//!
//! Usage:
//!   cencora_to_bulk <input.csv> <output.csv>

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::process::ExitCode;

use csv::{Reader, Writer};

#[derive(Debug, serde::Deserialize)]
struct SourceRow {
    #[serde(rename = "PRD_DTE")]
    production_date: String,
    #[serde(rename = "PRD_HR")]
    production_hour: u32,
    #[serde(rename = "FWD_DC")]
    forward_distribution_center: String,
    #[serde(rename = "SKU")]
    sku: String,
    #[serde(rename = "ALL_UNITS")]
    _all_units: u32,
    #[serde(rename = "CASES")]
    cases: u32,
    #[serde(rename = "EACH_PICKS")]
    _each_picks: u32,
    #[serde(rename = "AREA")]
    _area: String,
    #[serde(rename = "SHIPWRAP_FLAG")]
    _shipwrap_flag: String,
    #[serde(rename = "NONCON")]
    _noncon: String,
    #[serde(rename = "AMBIENT_ONLY")]
    _ambient_only: String,
    #[serde(rename = "CASE_QTY")]
    _case_qty: u32,
    #[serde(rename = "UNIT_L")]
    _unit_l: String,
    #[serde(rename = "UNIT_W")]
    _unit_w: String,
    #[serde(rename = "UNIT_H")]
    _unit_h: String,
    #[serde(rename = "CASE_L")]
    _case_l: String,
    #[serde(rename = "CASE_W")]
    _case_w: String,
    #[serde(rename = "CASE_H")]
    _case_h: String,
    #[serde(rename = "CASE_WT")]
    _case_wt: String,
}

#[derive(Debug, serde::Serialize)]
struct BulkRow<'a> {
    /// No-earlier-than sim-time (seconds) for putaway induction of this lot.
    putaway_net_secs: u32,
    #[serde(rename = "dc_id")]
    distribution_center_id: &'a str,
    sku: &'a str,
    quantity: u32,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: {} <input.csv> <output.csv>", args[0]);
        return ExitCode::from(2);
    }
    let input = PathBuf::from(&args[1]);
    let output = PathBuf::from(&args[2]);

    let mut reader = match Reader::from_path(&input) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("failed to open {}: {}", input.display(), e);
            return ExitCode::FAILURE;
        }
    };

    // Aggregate (PRD_HR, FWD_DC, SKU) → cases, while preserving first-occurrence
    // order. `order` records each key the first time it's seen; a stable sort by
    // hour then yields product-sequence order (hour ascending, source order
    // within an hour). HashMap holds the running sum.
    type Key = (u32, String, String);
    let mut order: Vec<Key> = Vec::new();
    let mut units: HashMap<Key, u32> = HashMap::new();
    let mut dates: BTreeSet<String> = BTreeSet::new();
    let mut row_count = 0u32;
    let mut total_units = 0u64;
    let mut zero_quantity_rows = 0u32;

    for (i, result) in reader.deserialize().enumerate() {
        let row: SourceRow = match result {
            Ok(r) => r,
            Err(e) => {
                eprintln!("failed to parse row {} in {}: {}", i + 2, input.display(), e);
                return ExitCode::FAILURE;
            }
        };
        row_count += 1;
        dates.insert(row.production_date);
        // Case picks only — eaches and CASE_QTY are out of scope.
        let quantity = row.cases;
        if quantity == 0 {
            zero_quantity_rows += 1;
            continue;
        }
        total_units += quantity as u64;
        let key = (row.production_hour, row.forward_distribution_center, row.sku);
        if !units.contains_key(&key) {
            order.push(key.clone());
        }
        *units.entry(key).or_insert(0) += quantity;
    }

    // Reject multi-day input rather than silently flattening it to a single
    // hour ladder (which would mis-order lots across days). Single-day input
    // normalizes its one date to sim-time 0, so `putaway_net_secs = hr * 3600`.
    if dates.len() > 1 {
        eprintln!(
            "multi-day input not supported: found {} distinct PRD_DTE values ({:?}). \
             putaway_net_secs would mis-order across days. Split by date or extend \
             the converter to compute (date - day0) * 24h + hr.",
            dates.len(),
            dates,
        );
        return ExitCode::FAILURE;
    }

    // Stable sort by hour: numeric-hour ascending, first-occurrence order
    // preserved within an hour (slice::sort_by_key is stable).
    order.sort_by_key(|(production_hour, _, _)| *production_hour);

    let mut writer = match Writer::from_path(&output) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("failed to open output {}: {}", output.display(), e);
            return ExitCode::FAILURE;
        }
    };
    for key in &order {
        let (production_hour, distribution_center_id, sku) = key;
        let quantity = *units.get(key).expect("every key in `order` was inserted into `units`");
        if let Err(e) = writer.serialize(BulkRow {
            putaway_net_secs: production_hour * 3600,
            distribution_center_id,
            sku,
            quantity,
        }) {
            eprintln!("write error: {}", e);
            return ExitCode::FAILURE;
        }
    }
    if let Err(e) = writer.flush() {
        eprintln!("flush error: {}", e);
        return ExitCode::FAILURE;
    }

    eprintln!(
        "{} → {}: {} source rows, {} zero-case skipped, {} (hr,dc,sku) lots, {} total units (cases)",
        input.display(),
        output.display(),
        row_count,
        zero_quantity_rows,
        order.len(),
        total_units,
    );
    ExitCode::SUCCESS
}
