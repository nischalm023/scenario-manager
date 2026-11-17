//! Station gap test: validates that plan_ahead produces near-zero idle gaps
//! between consecutive presentations at each station.
//!
//! Loads the exact minimal_putaway.json fixture (same data as desvis).
//! Measures inter-CycleComplete gaps: gap = CC[i+1] - CC[i] - dwell.
//! A gap > 0 means the station was idle between presentations.
//!
//! Uses the solver path (plan_ahead → events → observer), not the actor DES.

mod common;

use std::collections::BTreeMap;

use dessim::config::SimConfig;
use dessim::events::SimEvent;
use dessim::scenario::{GeneratedScenario, ScenarioConfig};
use dessim::worldmap::WorldMap;
use native_modules as _;

/// Load the minimal_putaway world fixture. Returns (world, scenario, config)
/// — same data a spec-driven desvis launch would use.
fn load_fixture() -> (WorldMap, ScenarioConfig, SimConfig) {
    common::load_world_spec("minimal_putaway_spec.json")
}

/// Extract CycleComplete timestamps grouped by station.
fn cc_times_by_station(events: &[SimEvent]) -> BTreeMap<u32, Vec<u64>> {
    let mut map: BTreeMap<u32, Vec<u64>> = BTreeMap::new();
    for event in events {
        if let SimEvent::CycleComplete(cc) = event {
            map.entry(cc.station_id.0).or_default().push(cc.sim_time_ms);
        }
    }
    for times in map.values_mut() {
        times.sort();
    }
    map
}

/// Compute idle gaps: inter-CC interval minus dwell time.
/// A positive gap means the station was idle between presentations.
fn compute_gaps(cc_times: &[u64], dwell_ms: u64) -> Vec<i64> {
    (1..cc_times.len()).map(|i| (cc_times[i] as i64) - (cc_times[i - 1] as i64) - (dwell_ms as i64)).collect()
}

/// Station gap test on minimal_putaway fixture.
///
/// Map: 6x10, 1 pick station, 1 putaway station, 1 relay zone.
/// Config from fixture: 2 HTMs, 1 VTM, queue_factor=3.0, putaway enabled.
/// Scenario: 10 SKUs, 2000 inventory units, 3 orders / 3 picks.
///
/// This is the EXACT same data that `desvis --fixture minimal_putaway.json` uses.
#[test]
fn station_gap_minimal_putaway() {
    let (world, scenario, config) = load_fixture();

    eprintln!("\n=== Station Gap Test (minimal_putaway) ===");
    eprintln!("  HTMs: {}, QF: {}", config.num_htms, config.queue_factor);
    eprintln!("  Putaway: enabled={}", config.putaway.enabled);
    eprintln!(
        "  Orders: {:?}, Picks: {:?}",
        scenario.num_orders().unwrap_or(0),
        scenario.total_order_picks().unwrap_or(0)
    );

    let presentation_ms = (config.operator.time_per_pick_secs * 1000.0) as u64;
    let putaway_dwell_ms = (config.putaway.dwell_secs * 1000.0) as u64;
    eprintln!("  Presentation: {}ms, Putaway dwell: {}ms", presentation_ms, putaway_dwell_ms);

    let (events, _result, metrics) = common::run_world_capture_all(&world, &scenario, config);

    eprintln!("  Events: {}", events.len());
    eprintln!("  Time slices: {}", metrics.time_slices.len());
    eprintln!("  Picks completed: {}", metrics.cumulative_scalars.picks_completed);

    // Group CycleComplete by station.
    let cc_map = cc_times_by_station(&events);

    for (&station_id, times) in &cc_map {
        let dwell = if station_id == 2 { putaway_dwell_ms } else { presentation_ms };
        let gaps = compute_gaps(times, dwell);

        eprintln!("  Station {}: {} CCs, dwell={}ms", station_id, times.len(), dwell);
        if !gaps.is_empty() {
            let max_gap = *gaps.iter().max().unwrap();
            let min_gap = *gaps.iter().min().unwrap();
            let mean_gap = gaps.iter().sum::<i64>() / gaps.len() as i64;
            eprintln!("    Gaps: min={}ms mean={}ms max={}ms", min_gap, mean_gap, max_gap);

            for (i, &gap) in gaps.iter().enumerate() {
                let cc_time = times[i + 1];
                let label = if gap <= 0 {
                    "ok"
                } else if gap <= 2000 {
                    "minor"
                } else {
                    "GAP"
                };
                eprintln!("    CC[{}] t={}ms  gap={:+}ms  {}", i + 1, cc_time, gap, label);
            }
        }

        // Print raw CC times for debugging.
        for (i, &t) in times.iter().enumerate() {
            eprintln!("    CC[{}] = {}ms", i, t);
        }
    }

    // Assert: at least some CycleComplete events were produced.
    let total_ccs: usize = cc_map.values().map(|v| v.len()).sum();
    assert!(total_ccs > 0, "solver must produce CycleComplete events");

    // Assert: pick station gaps after warm-up should be reasonable.
    // Skip first 2 gaps (pipeline filling). Steady-state gaps should be
    // under MAX_GAP_MS. With perfect scheduling, gap = 0 (back-to-back).
    const MAX_GAP_MS: i64 = 30_000; // 30s tolerance (generous for small map)
    let mut violations = Vec::new();
    for (&station_id, times) in &cc_map {
        let dwell = if station_id == 2 { putaway_dwell_ms } else { presentation_ms };
        let gaps = compute_gaps(times, dwell);
        // Skip warm-up gaps (first 2).
        for (i, &gap) in gaps.iter().enumerate().skip(2) {
            if gap > MAX_GAP_MS {
                violations.push((station_id, i, gap));
            }
        }
    }

    if !violations.is_empty() {
        eprintln!("\n  VIOLATIONS ({}):", violations.len());
        for (sid, idx, gap) in &violations {
            eprintln!("    Station {} gap #{}: {}ms > {}ms", sid, idx, gap, MAX_GAP_MS);
        }
    }
    eprintln!("\n  Violations: {}", violations.len());
    eprintln!("=== Station Gap Test DONE ===\n");

    assert!(
        violations.is_empty(),
        "{} station-gap violations > {}ms (steady-state, skipping first 2 gaps): {:?}",
        violations.len(),
        MAX_GAP_MS,
        violations,
    );
}

/// Same fixture but with more orders to see steady-state behavior.
/// Overrides the scenario to 30 orders / 30 picks.
#[test]
fn station_gap_30_orders() {
    let (world, _scenario, mut config) = load_fixture();

    // Override scenario for more orders (steady-state analysis).
    let scenario = ScenarioConfig::Generated(GeneratedScenario {
        num_skus: 10,
        total_inventory_units: 2000,
        num_orders: 30,
        total_order_picks: 30,
        units_per_tote: 1,
        popular_skus_pct: 0.0,
        inventory_popular_share: 0.0,
        order_popular_share: 0.0,
        order_totes: 0,
    });
    config.waive_completability_check = true;

    eprintln!("\n=== Station Gap Test (30 orders) ===");
    eprintln!("  HTMs: {}, QF: {}", config.num_htms, config.queue_factor);

    let presentation_ms = (config.operator.time_per_pick_secs * 1000.0) as u64;
    let putaway_dwell_ms = (config.putaway.dwell_secs * 1000.0) as u64;

    let (events, _result, _metrics) = common::run_world_capture_all(&world, &scenario, config);

    let cc_map = cc_times_by_station(&events);

    for (&station_id, times) in &cc_map {
        let dwell = if station_id == 2 { putaway_dwell_ms } else { presentation_ms };
        let gaps = compute_gaps(times, dwell);

        eprintln!("  Station {}: {} CCs", station_id, times.len());
        if gaps.len() >= 3 {
            // Steady state: skip first 3 gaps (pipeline fill).
            let steady = &gaps[3..];
            if !steady.is_empty() {
                let max_gap = *steady.iter().max().unwrap();
                let mean_gap = steady.iter().sum::<i64>() / steady.len() as i64;
                eprintln!("    Steady-state ({} gaps): mean={}ms max={}ms", steady.len(), mean_gap, max_gap);
            }
        }

        for (i, &gap) in gaps.iter().enumerate() {
            eprintln!("    gap[{}] = {:+}ms", i, gap);
        }
    }

    let total_ccs: usize = cc_map.values().map(|v| v.len()).sum();
    assert!(total_ccs > 0, "must produce CycleComplete events");

    // Steady-state gaps after pipeline fill (skip first 3) should be
    // bounded. Same 30s tolerance as the 3-order test.
    const MAX_GAP_MS: i64 = 30_000;
    let mut violations = Vec::new();
    for (&station_id, times) in &cc_map {
        let dwell = if station_id == 2 { putaway_dwell_ms } else { presentation_ms };
        let gaps = compute_gaps(times, dwell);
        for (i, &gap) in gaps.iter().enumerate().skip(3) {
            if gap > MAX_GAP_MS {
                violations.push((station_id, i, gap));
            }
        }
    }
    eprintln!("  Violations: {}", violations.len());
    eprintln!("=== Station Gap 30 Orders DONE ===\n");

    assert!(
        violations.is_empty(),
        "{} steady-state station-gap violations > {}ms: {:?}",
        violations.len(),
        MAX_GAP_MS,
        violations,
    );
}
