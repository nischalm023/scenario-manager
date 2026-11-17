//! Collision workload for the minimal EasyLoop and its current lane discipline.
//! Inventory HTMs continue from dropoff through eight holding locations to
//! collection; each order block serves its own fixed presentation.
//! The workload assertions remain active while the old task flow is replaced.

mod common;

use std::path::{Path, PathBuf};

use dessim::events::{CycleKind, ResourceEventKind, SimEvent};
use dessim::observer::output::MetricsOutput;
use dessim::sim_types::SimResult;
use dessim::worldmap;
use native_modules as _;
use rustc_hash::{FxHashMap, FxHashSet};

const SPEC: &str = "minimal_easyloop_collision_spec.json";

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn run() -> (Vec<SimEvent>, SimResult, MetricsOutput) {
    let (world, scenario, config) = common::load_world_spec(SPEC);
    common::run_world_capture_all(&world, &scenario, config)
}

/// The workload must complete under contention, without headroom alarms or
/// order watchdogs. Completion alone does not prove correct tote pairing.
#[test]
fn every_order_is_picked_and_extracted_with_collisions_on() {
    let (events, result, metrics) = run();
    let s = &metrics.cumulative_scalars;

    assert_eq!(result.total_picks, 24, "all twenty-four of the scenario's order picks");
    assert_eq!(s.orders_completed, 12, "all twelve orders");
    assert_eq!(s.order_extract_presentations, 12, "one extraction per completed order");

    let mut order_sides = FxHashMap::default();
    let mut used_sides = FxHashSet::default();
    for event in &events {
        if let SimEvent::CycleComplete(cycle) = event
            && matches!(cycle.kind, CycleKind::Pick { .. })
        {
            let order = cycle.tote_presentation.unwrap().order_id;
            if let Some(previous) = order_sides.insert(order, cycle.station_id) {
                assert_eq!(previous, cycle.station_id, "all picks for an order use its assigned presentation");
            }
            used_sides.insert(cycle.station_id);
        }
    }
    assert_eq!(order_sides.len(), 12);
    assert_eq!(used_sides.len(), 2, "a small fleet must still feed both waiting blocks");

    let deadlocks = events
        .iter()
        .filter(|e| matches!(e, SimEvent::Resource(r) if r.kind == ResourceEventKind::DeadlockHeadroomTriggered))
        .count();
    assert_eq!(deadlocks, 0, "the deadlock headroom alarm never fired");
    let watchdogs = events.iter().filter(|e| matches!(e, SimEvent::WatchdogTimeout(_))).count();
    assert_eq!(watchdogs, 0, "no order sat at a station past its watchdog");
}

/// The lane itself enforces the no-early-exit rule; physical fleet ownership
/// requires separate task-level checks. Every empty HTM must pass all eight queue cells
/// before it can reach collection, with no lateral shortcut or reverse edge.
#[test]
fn inventory_htms_have_one_way_through_eight_holding_cells() {
    let w = worldmap::load(&fixture("minimal_easyloop.json")).unwrap();
    let machine = &w.map.conveyor_machines[0];
    let queue = machine.inventory_queue.as_ref().unwrap();
    assert_eq!(machine.inventory_holding_capacity(), 8);
    assert_eq!(queue.entry_from, machine.dropoff_cell);
    assert_eq!(Some(queue.into), machine.collection_cell);
    let steps: Vec<_> =
        machine.dropoff_cell.into_iter().chain(queue.path.iter().copied()).chain(machine.collection_cell).collect();
    for pair in steps.windows(2) {
        let onward = w.map.graph.neighbors(pair[0]);
        assert_eq!(onward.len(), 1, "{:?} has no lateral or early exit", pair[0]);
        assert_eq!(onward[0].0, pair[1]);
        assert!(onward[0].1.dest_only.is_none_or(|gate| gate.admits(queue.into)));
        assert!(!w.map.graph.neighbors(pair[1]).iter().any(|(to, _)| *to == pair[0]));
    }
    assert!(!w.map.graph.neighbors(queue.into).is_empty(), "a collected tote can leave");
}
