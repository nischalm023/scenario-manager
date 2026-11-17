//! Diagnostic: does DFW6 spread work across all eight pick stations?
//!
//! Temporary, in the manner of `dfw6_reachability`. Vinay's 2026-08-31 read of
//! a run was "only 2 stations are getting the tasks". This counts, rather than
//! infers, presentations per station off the real Walmart extract.

use std::collections::BTreeMap;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use std::path::{Path, PathBuf};

use desgraph::StationType;
use dessim::events::{CycleKind, EventSink, SimEvent};
use dessim::pathfinding::holding_cells;
use dessim::runner::RunnerHooks;
use dessim::{FHSippPathfinder, FidelityMode, SimDuration};
use native_modules as _;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

#[test]
#[cfg_attr(not(feature = "slow-gate"), ignore = "whole-fixture simulation; slow gate: scripts/test-slow-gate.sh")]
fn dfw6_presentations_per_pick_station() {
    let loaded = dessim::world_spec::load(&fixture("dfw6_real_spec.json")).expect("spec loads");
    let mut config = loaded.settings;
    config.fidelity = FidelityMode::CollisionDes;
    config.order.close_out = std::env::var("CLOSE_OUT").as_deref() != Ok("0");
    // Long enough to be past warm-up, short enough to iterate on.
    config.sim_duration = Duration::from_secs(1800);

    let mut data = dessim::runner::resolve_scenario_with_racking(
        loaded.world.map.clone(),
        loaded.world.racking.clone(),
        &loaded.scenario,
        &config,
    )
    .expect("scenario generation");

    let picks: Vec<_> = data
        .map
        .stations
        .iter()
        .filter(|s| s.station_type == StationType::Pick)
        .map(|s| (s.id, s.presentation_cells().collect::<Vec<_>>()))
        .collect();
    eprintln!("pick stations: {}", picks.len());
    for (id, cells) in &picks {
        eprintln!("  {id:?} presentation={cells:?}");
    }

    // Match desvis: explicit indefinite holds require relay access or charging
    // destinations. Omitting this set prevents idle relocation.
    let holding = holding_cells(&data.map);
    {
        use dessim::scenario_gen::RackUse;
        let depot: Vec<_> = loaded.world.racking.columns.iter().filter(|c| c.uses == RackUse::OrderDepot).collect();
        eprintln!("OrderDepot rack columns: {}", depot.len());
        if let Some(c) = depot.first() {
            eprintln!("  first at {:?} cell {:?}", c.world_pos, c.cell);
        }
        let ys: Vec<i64> = depot.iter().filter_map(|c| c.world_pos.map(|p| p.y_mm as i64)).collect();
        if !ys.is_empty() {
            eprintln!("  y range {}..{}", ys.iter().min().unwrap(), ys.iter().max().unwrap());
        }
        // Relays whose access cell is near the depot row.
        let near: Vec<_> = data
            .map
            .relays
            .iter()
            .filter(|r| {
                data.map
                    .graph
                    .cell(r.htm_access)
                    .and_then(|c| c.world_pos)
                    .is_some_and(|p| ys.iter().any(|&y| (p.y_mm as i64 - y).abs() < 2000))
            })
            .collect();
        eprintln!("  relays with an access cell within 2m of the row: {}", near.len());
        eprintln!("  total relays: {}", data.map.relays.len());
    }

    let pathfinder = Arc::new(FHSippPathfinder::new(
        &data.map,
        (config.vtm.speed * 1000.0) as u64,
        SimDuration::seconds(1),
        holding,
    ));
    let (event_tx, event_rx) = mpsc::channel();
    let (collect_tx, collect_rx) = mpsc::channel();
    let sink: Arc<dyn EventSink> = Arc::new(collect_tx);
    let (planner, model, bias) =
        dessim::composition::build(&mut data, &config, pathfinder, Some(sink)).expect("planner assembly");
    let (result, _metrics) = dessim::runner::run(
        planner,
        model,
        &data,
        config,
        event_tx,
        event_rx,
        bias,
        RunnerHooks { live: None, subscribers: vec![], recording: None },
    )
    .expect("the run completes");

    let mut per_station: BTreeMap<u32, BTreeMap<&'static str, u64>> = BTreeMap::new();
    let mut dispatched: BTreeMap<u32, u64> = BTreeMap::new();
    let mut rejections: BTreeMap<u32, BTreeMap<&'static str, u64>> = BTreeMap::new();
    // Chain lifecycle per station: bound -> arrived -> presented.
    let mut assigned: BTreeMap<u32, u64> = BTreeMap::new();
    let mut arrived: BTreeMap<u32, u64> = BTreeMap::new();
    let mut last_present_ms: BTreeMap<u32, u64> = BTreeMap::new();
    let mut first_present_ms: BTreeMap<u32, u64> = BTreeMap::new();
    let mut watchdogs: u64 = 0;
    for event in collect_rx.try_iter() {
        match event {
            SimEvent::CycleComplete(e) => {
                let kind = match e.kind {
                    CycleKind::Pick { .. } => "pick",
                    CycleKind::Putaway { .. } => "putaway",
                    CycleKind::OrderLoad { .. } => "order_load",
                    CycleKind::OrderExtract { .. } => "order_extract",
                };
                *per_station.entry(e.station_id.0).or_default().entry(kind).or_default() += 1;
                first_present_ms.entry(e.station_id.0).or_insert(e.sim_time_ms);
                last_present_ms.insert(e.station_id.0, e.sim_time_ms);
            }
            SimEvent::OrderDispatch(e) => {
                *dispatched.entry(e.station_id.0).or_default() += 1;
            }
            SimEvent::HtmAssignment(e) => {
                *assigned.entry(e.station_id.0).or_default() += 1;
            }
            SimEvent::HtmArrival(e) => {
                *arrived.entry(e.station_id.0).or_default() += 1;
            }
            SimEvent::WatchdogTimeout(_) => {
                watchdogs += 1;
            }
            SimEvent::DispatchRejection(e) => {
                *rejections.entry(e.station_id.0).or_default().entry(e.reason.label()).or_default() += e.count as u64;
            }
            _ => {}
        }
    }

    eprintln!("--- presentations per station ---");
    for (station, kinds) in &per_station {
        eprintln!("  StationId({station}) {kinds:?}");
    }
    eprintln!("--- orders dispatched per station ---");
    for (station, n) in &dispatched {
        eprintln!("  StationId({station}) {n}");
    }
    eprintln!("--- chain lifecycle per station (bound -> arrived -> presented) ---");
    eprintln!("  {:>4} {:>8} {:>8} {:>10} {:>12} {:>12}", "stn", "bound", "arrived", "presented", "first_s", "last_s");
    for station in 1..=8u32 {
        let presented: u64 = per_station.get(&station).map(|m| m.values().sum()).unwrap_or(0);
        eprintln!(
            "  {:>4} {:>8} {:>8} {:>10} {:>12} {:>12}",
            station,
            assigned.get(&station).copied().unwrap_or(0),
            arrived.get(&station).copied().unwrap_or(0),
            presented,
            first_present_ms.get(&station).map(|v| v / 1000).unwrap_or(0),
            last_present_ms.get(&station).map(|v| v / 1000).unwrap_or(0),
        );
    }
    eprintln!("--- watchdog timeouts: {watchdogs} ---");
    eprintln!("--- dispatch rejections per station ---");
    for (station, reasons) in &rejections {
        eprintln!("  StationId({station}) {reasons:?}");
    }
    eprintln!("total picks={} orders={}", result.total_picks, result.total_orders_completed);
}
