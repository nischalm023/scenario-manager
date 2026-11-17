//! Scale benchmarks for RelayService — torture tests for profiling.
//!
//! 20 zones × 20 relays = 400 relays, 100k totes, 100k ops per variant.
//!
//! Run all:   cargo test --release -p dessim --features slow-gate --lib -- scale_ --nocapture
//! With perf: cargo test --release -p dessim --features slow-gate,perf --lib -- scale_ --nocapture

use std::collections::VecDeque;
use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use desgraph::{CellId, ZoneId};

use crate::config::VtmConfig;
use crate::model::ToteDesignation;
use crate::model::ids::{RobotId, ToteId};
use crate::model::inventory::InventoryService;
use crate::model::location::{ToteLocation, ToteType};
use crate::model::relay::*;
use crate::model::reservation::oracle::GhostOracle;
use crate::model::test_model::block;
use crate::model::{RackId, SimTime, SkuId};
#[cfg(feature = "perf")]
use crate::tracing_agg::AggregationLayer;
#[cfg(feature = "perf")]
use tracing::subscriber::DefaultGuard;

const fn t(ms: u64) -> SimTime {
    SimTime::ms(ms)
}

use super::helpers::*;

const ZONES: u32 = 20;
const RELAYS_PER_ZONE: u32 = 20;
const RACKS: u32 = 10_000;
const TOTES: u32 = 100_000;
const TOTES_PER_RACK: u32 = 10;
const OPS: u32 = 100_000;

/// Install tracing subscriber when perf feature is on. Returns a guard
/// that prints the summary on drop.
#[cfg(feature = "perf")]
fn perf_subscriber() -> DefaultGuard {
    use tracing_subscriber::prelude::*;
    let subscriber = tracing_subscriber::registry().with(AggregationLayer::new());
    tracing::subscriber::set_default(subscriber)
}

/// 20 zones × 20 relays, wired to InventoryService via observer.
fn bench_env() -> (RelayService, InventoryService) {
    let mut zone_spec: Vec<(u32, Vec<(u32, u32)>)> = Vec::new();
    let mut relay_id = 1u32;
    for zone in 0..ZONES {
        let base_x = zone * 100;
        let mut relays = Vec::new();
        for r in 0..RELAYS_PER_ZONE {
            relays.push((relay_id, base_x + r * 5));
            relay_id += 1;
        }
        zone_spec.push((zone + 1, relays));
    }

    let zone_refs: Vec<(u32, &[(u32, u32)])> =
        zone_spec.iter().map(|(zid, relays)| (*zid, relays.as_slice())).collect();

    // (rack_id, cell_x, zone_id) — distribute racks across zones.
    let rack_spec: Vec<(u32, u32, u32)> = (0..RACKS)
        .map(|r| {
            let cell_x = r * 2 % 2000;
            let zone = ((cell_x / 100) + 1).min(ZONES);
            (r, cell_x, zone)
        })
        .collect();

    let (data, inventory, pathfinder) = build_test_env(&zone_refs, &rack_spec);
    let registry = super::helpers::empty_htm_registry(&data, &inventory);
    let svc = RelayService::new(
        &data,
        inventory.clone(),
        super::helpers::test_navigation(&data, pathfinder, Arc::new(GhostOracle)),
        &registry,
        &VtmConfig::default(),
    )
    .expect("relay inventory index");
    (svc, inventory)
}

fn register_totes(inv: &InventoryService, range: Range<u32>) {
    for id in range {
        let rack = id / TOTES_PER_RACK;
        inv.register_tote(
            ToteId(id),
            ToteType::Tote,
            ToteDesignation::Inventory,
            ToteLocation::InRack { rack_id: RackId(rack), z_index: (id % TOTES_PER_RACK) as u16 },
        )
        .expect("register_tote");
    }
}

fn seed_skus(inv: &InventoryService, range: Range<u32>, n_skus: u32) {
    for id in range {
        let sku = SkuId(id % n_skus);
        inv.tote(ToteId(id)).expect("seed: tote").add_item(sku, 1, 0).expect("seed: add_item");
    }
}

/// Move totes out of racks to create empty slots for deposit scoring.
fn seed_empty_slots(inv: &InventoryService, count: u32) {
    let base = TOTES + 10_000;
    for i in 0..count {
        let rack = i % RACKS;
        let tid = ToteId(base + i);
        inv.register_tote(
            tid,
            ToteType::Tote,
            ToteDesignation::Inventory,
            ToteLocation::InRack { rack_id: RackId(rack), z_index: 15 },
        )
        .expect("seed: register");
        inv.tote(tid)
            .expect("seed: tote")
            .move_to(ToteLocation::WithRobot { robot_id: RobotId(base + i) }, SimTime::ms(0))
            .expect("seed: move");
    }
}

fn report(label: &str, count: u32, elapsed: Duration) {
    eprintln!(
        "  {:<28} {:>7} ops  {:>10.1?}  ({:.1} µs/op)",
        label,
        count,
        elapsed,
        elapsed.as_micros() as f64 / count.max(1) as f64,
    );
}

// ---------------------------------------------------------------------------
// Retrieve-only: pre-populated 100k totes, 100k retrieve+finalize cycles.
// ---------------------------------------------------------------------------

#[test]
#[cfg_attr(not(feature = "slow-gate"), ignore = "scale benchmark; slow gate: scripts/test-slow-gate.sh")]
fn scale_retrieve() {
    #[cfg(feature = "perf")]
    let _guard = perf_subscriber();
    let (svc, inv) = bench_env();

    eprintln!("\n=== scale_retrieve (20×20 relays, 100k totes) ===");

    // Setup: register 100k totes (90k empty, 10k with SKUs).
    let t = Instant::now();
    register_totes(&inv, 0..TOTES);
    report("register 100k totes", TOTES, t.elapsed());

    let t = Instant::now();
    seed_skus(&inv, 0..10_000, 100);
    report("seed 10k SKUs", 10_000, t.elapsed());

    inv.seed_inventory_from_totes();

    // Phase 1: 100k empty retrieves.
    let t = Instant::now();
    let mut ok = 0u32;
    for i in 0..OPS {
        let zone = i % ZONES;
        let cell = CellId(zone * 100 + 50);
        if let Ok(r) = svc.reserve_relay_for_retrieve(
            ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
            cell,
            SimTime::ms(i as u64 * 10),
        ) {
            r.finalize(SimTime::ms(i as u64 * 10 + 5)).unwrap();
            ok += 1;
        }
    }
    report("empty retrieve+finalize", ok, t.elapsed());
    assert!(ok >= OPS * 9 / 10, "expected ≥90% success, got {ok}");

    // Phase 2: 100k SKU retrieves (100 SKUs, cycle through).
    let t = Instant::now();
    let mut ok = 0u32;
    for i in 0..OPS {
        let sku = SkuId(i % 100);
        let zone = i % ZONES;
        let cell = CellId(zone * 100 + 50);
        if let Ok(r) = svc.reserve_relay_for_retrieve(
            ToteSearchCriteria::ContainsSku { sku },
            cell,
            SimTime::ms(1_000_000 + i as u64 * 10),
        ) {
            r.finalize(SimTime::ms(1_000_000 + i as u64 * 10 + 5)).unwrap();
            ok += 1;
        }
    }
    report("SKU retrieve+finalize", ok, t.elapsed());
    assert!(ok >= OPS * 9 / 10, "expected ≥90% SKU success, got {ok}");
}

// ---------------------------------------------------------------------------
// Load: start empty, progressively register 100k totes.
// Measures observer event propagation + BTreeSet insertion throughput.
// ---------------------------------------------------------------------------

#[test]
#[cfg_attr(not(feature = "slow-gate"), ignore = "scale benchmark; slow gate: scripts/test-slow-gate.sh")]
fn scale_load() {
    #[cfg(feature = "perf")]
    let _guard = perf_subscriber();
    let (svc, inv) = bench_env();

    eprintln!("\n=== scale_load (20×20 relays, 100k progressive inserts) ===");

    // Phase 1: register 100k totes one-by-one (fires LocationChanged →
    // BTreeSet insert into 20 relay sets per tote).
    let t = Instant::now();
    register_totes(&inv, 0..TOTES);
    report("register 100k (observer)", TOTES, t.elapsed());

    // Phase 2: add SKUs to 50k totes (fires ContentsChanged →
    // removes from empty sets, inserts into SKU sets).
    let t = Instant::now();
    seed_skus(&inv, 0..50_000, 200);
    report("seed 50k SKUs (observer)", 50_000, t.elapsed());

    // Phase 2.5: Create empty slots for deposit scoring.
    let t = Instant::now();
    seed_empty_slots(&inv, 2000);
    report("seed 2k empty slots", 2000, t.elapsed());

    // Phase 3: 100k deposits into the now-loaded system.
    let t = Instant::now();
    let mut ok = 0u32;
    for i in 0..OPS {
        let zone = i % ZONES;
        let cell = CellId(zone * 100 + 50);
        if let Ok(r) =
            block(svc.reserve_relay_for_deposit(cell, Default::default(), SimTime::ms(i as u64 * 10), RobotId(0), None))
        {
            r.finalize(SimTime::ms(i as u64 * 10 + 5)).unwrap();
            ok += 1;
        }
    }
    report("deposit+finalize", ok, t.elapsed());
    assert!(ok >= OPS * 9 / 10, "expected ≥90% deposit success, got {ok}");
}

// ---------------------------------------------------------------------------
// Mixed: steady-state churn. Retrieve totes out, add SKUs, return them.
// Exercises the full observer add/remove cycle under load.
// ---------------------------------------------------------------------------

#[test]
#[cfg_attr(not(feature = "slow-gate"), ignore = "scale benchmark; slow gate: scripts/test-slow-gate.sh")]
fn scale_mixed() {
    #[cfg(feature = "perf")]
    let _guard = perf_subscriber();
    let (svc, inv) = bench_env();

    eprintln!("\n=== scale_mixed (20×20 relays, 100k churn ops) ===");

    // Pre-populate 100k empty totes.
    let t = Instant::now();
    register_totes(&inv, 0..TOTES);
    report("register 100k totes", TOTES, t.elapsed());
    inv.seed_inventory_from_totes();

    // Churn loop: retrieve → move out → add SKU → return → deposit.
    // Pool of dispatched totes waiting to be returned.
    let mut dispatched: VecDeque<ToteId> = VecDeque::new();
    let mut retrieves = 0u32;
    let mut returns = 0u32;
    let mut deposits = 0u32;

    // Seed some empty slots for the deposit leg of the churn loop.
    seed_empty_slots(&inv, 2000);

    let t = Instant::now();
    for i in 0..OPS {
        let sim_ms = i as u64 * 20;
        let zone = i % ZONES;
        let cell = CellId(zone * 100 + 50);

        // Return oldest dispatched tote every other iteration.
        if i % 2 == 1 {
            if let Some(tote_id) = dispatched.pop_front() {
                // Return tote to a rack (fires LocationChanged → re-adds to
                // BTreeSets, including SKU sets since we added a SKU below).
                let rack = (i / TOTES_PER_RACK) % RACKS;
                let z = (i % TOTES_PER_RACK) as u16;
                inv.tote(tote_id)
                    .expect("churn: tote")
                    .move_to(ToteLocation::InRack { rack_id: RackId(rack), z_index: z }, SimTime::ms(sim_ms))
                    .expect("churn: move_to");
                // Deposit to find relay for the return leg.
                if let Ok(r) = block(svc.reserve_relay_for_deposit(
                    cell,
                    Default::default(),
                    SimTime::ms(sim_ms),
                    RobotId(0),
                    None,
                )) {
                    r.finalize(SimTime::ms(sim_ms + 5)).unwrap();
                    deposits += 1;
                }
                returns += 1;
            }
        }

        // Retrieve an empty tote.
        if let Ok(r) = svc.reserve_relay_for_retrieve(
            ToteSearchCriteria::Empty { designation: ToteDesignation::Inventory },
            cell,
            SimTime::ms(sim_ms),
        ) {
            let tote_id = r.recommendation;
            // Move tote out of rack (fires LocationChanged → removes from
            // BTreeSets).
            inv.tote(tote_id)
                .expect("churn: tote")
                .move_to(ToteLocation::WithRobot { robot_id: RobotId(i) }, SimTime::ms(sim_ms + 1))
                .expect("churn: move_to");
            // Add a SKU while tote is out (fires ContentsChanged, but tote
            // is WithRobot so sku_by_relay is not updated until return).
            inv.tote(tote_id).expect("churn: tote").add_item(SkuId(i % 200), 1, sim_ms + 2).expect("churn: add_item");
            r.finalize(SimTime::ms(sim_ms + 3)).unwrap();
            dispatched.push_back(tote_id);
            retrieves += 1;
        }
    }
    let elapsed = t.elapsed();

    report("total churn ops", retrieves + returns, elapsed);
    eprintln!("    retrieves={retrieves}  returns={returns}  deposits={deposits}",);
    eprintln!("    dispatched pool peak: {}", dispatched.len());
    assert!(retrieves >= OPS / 2, "expected ≥50k retrieves, got {retrieves}");
}
