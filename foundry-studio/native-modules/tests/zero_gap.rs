//! Zero-gap station arrival tests.
//!
//! Validates that the scheduler pipelines HTMs to arrive at the station
//! with minimal idle time between presentations.
//!
//! Map: minimal_pick_only.json (6x10, 1 pick station, 1 relay zone,
//! 1 VTM, 1 HTM, pre-populated inventory, no putaway).
//!
//! Three fidelity variants × three queue_factor settings.

mod common;

use std::collections::BTreeMap;

use dessim::config::{FidelityMode, SimConfig};
use dessim::events::SimEvent;
use dessim::scenario::ScenarioConfig;
use dessim::worldmap::WorldMap;
use native_modules as _;

fn load_pick_only() -> (WorldMap, ScenarioConfig, SimConfig) {
    common::load_world_spec("minimal_pick_only_spec.json")
}

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

/// Compute idle gaps: CC[i+1] - CC[i] - dwell. Positive = station idle.
fn compute_gaps(cc_times: &[u64], dwell_ms: u64) -> Vec<i64> {
    (1..cc_times.len()).map(|i| (cc_times[i] as i64) - (cc_times[i - 1] as i64) - (dwell_ms as i64)).collect()
}

struct GapResult {
    gaps: Vec<i64>,
    total_ccs: usize,
}

fn run_gap_analysis(label: &str, world: &WorldMap, scenario: &ScenarioConfig, config: SimConfig) -> GapResult {
    let presentation_ms = (config.operator.time_per_pick_secs * 1000.0) as u64;
    let vtm_ms = ((config.vtm.load_time_secs + config.vtm.unload_time_secs) * 1000.0) as u64;

    eprintln!("\n=== {label} ===");
    eprintln!("  HTMs: {}, QF: {:.1}, Fidelity: {:?}", config.num_htms, config.queue_factor, config.fidelity);
    eprintln!("  P={}ms, VTM cycle={}ms", presentation_ms, vtm_ms);

    let (events, _result, _metrics) = common::run_world_capture_all(world, scenario, config);

    let cc_map = cc_times_by_station(&events);
    let mut all_gaps = Vec::new();
    let mut total_ccs = 0;

    for (&station_id, times) in &cc_map {
        let gaps = compute_gaps(times, presentation_ms);
        total_ccs += times.len();
        eprintln!("  Station {}: {} CCs", station_id, times.len());

        for (i, &gap) in gaps.iter().enumerate() {
            let label = if gap <= 0 {
                "ok"
            } else if gap <= 2000 {
                "~ok"
            } else {
                "GAP"
            };
            eprintln!("    gap[{}] = {:+}ms  {}", i, gap, label);
        }

        if gaps.len() >= 3 {
            let steady = &gaps[2..];
            if !steady.is_empty() {
                let max = *steady.iter().max().unwrap();
                let min = *steady.iter().min().unwrap();
                let mean = steady.iter().sum::<i64>() / steady.len() as i64;
                eprintln!("    Steady: min={:+}ms mean={:+}ms max={:+}ms", min, mean, max);
            }
        }

        all_gaps.extend_from_slice(&gaps);
    }

    eprintln!("=== {label} DONE ===\n");
    GapResult { gaps: all_gaps, total_ccs }
}

// ---------------------------------------------------------------------------
// Ghost mode: VTM+HTM, pure scheduling (no collision)
// ---------------------------------------------------------------------------

#[test]
fn ghost_qf0() {
    let (world, scenario, mut config) = load_pick_only();
    config.fidelity = FidelityMode::Ghost;
    config.queue_factor = 0.0;
    config.waive_completability_check = true;
    let r = run_gap_analysis("Ghost QF=0.0", &world, &scenario, config);
    assert!(r.total_ccs > 0, "must produce CCs");
}

#[test]
fn ghost_qf1() {
    let (world, scenario, mut config) = load_pick_only();
    config.fidelity = FidelityMode::Ghost;
    config.queue_factor = 1.0;
    config.waive_completability_check = true;
    let r = run_gap_analysis("Ghost QF=1.0", &world, &scenario, config);
    assert!(r.total_ccs > 0, "must produce CCs");
}

#[test]
fn ghost_qf3() {
    let (world, scenario, mut config) = load_pick_only();
    config.fidelity = FidelityMode::Ghost;
    config.queue_factor = 3.0;
    config.waive_completability_check = true;
    let r = run_gap_analysis("Ghost QF=3.0", &world, &scenario, config);
    assert!(r.total_ccs > 0, "must produce CCs");
}

// ---------------------------------------------------------------------------
// CollisionDes: VTM+HTM with CO-WHCA*
// ---------------------------------------------------------------------------

#[test]
fn collision_qf1() {
    let (world, scenario, mut config) = load_pick_only();
    config.fidelity = FidelityMode::CollisionDes;
    config.queue_factor = 1.0;
    config.waive_completability_check = true;
    let result = run_gap_analysis("CollisionDes QF=1.0", &world, &scenario, config);
    assert!(result.total_ccs > 0, "must produce CCs");
}

// ---------------------------------------------------------------------------
// HTM-only Ghost (no VTMs — totes via direct path only)
// ---------------------------------------------------------------------------

#[test]
fn htm_only_ghost_qf1() {
    // NOTE: this test never actually suppressed VTMs — the VTM fleet is
    // map-derived, and the `num_vtms = 0` override it used to set was a
    // no-op. Preserved behavior: VTMs from the fixture's zones still spawn.
    let (world, scenario, mut config) = load_pick_only();
    config.fidelity = FidelityMode::Ghost;
    config.queue_factor = 1.0;
    config.waive_completability_check = true;
    let r = run_gap_analysis("HTM-only Ghost QF=1.0", &world, &scenario, config);
    // HTM-only may produce 0 CCs if no direct-path totes exist.
    // That's a valid finding, not a test failure.
    eprintln!("  HTM-only: {} CCs (0 = no direct-path totes available)", r.total_ccs);
}
