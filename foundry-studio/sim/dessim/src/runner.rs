//! Drive a composed scheduler alongside the metrics observer. The runner owns
//! stopping, diagnostic probes, teardown audits and result assembly. It reads
//! managers that tasks have constructed through RunReports; it does not choose
//! workflow tasks or initialize warehouse equipment.

use std::collections::BTreeMap;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use desgraph::{CellId, CellKind, HtmDriveDirection, RelayId, StationId, StationType, WorldPos};
use desmap::WarehouseMap;
use rustc_hash::FxHashMap;

use crate::StopSignal;
use crate::config::SimConfig;
use crate::error::PlanError;
use crate::events::*;
use crate::model::bot::BotService;
use crate::model::bot::plan::BotPlan;
use crate::model::htm::registry::AvailableHtmRegistry;
use crate::model::relay::RelayService;
use crate::model::{RobotId, SimDuration, SimOffset, SimTime};
use crate::observer::SimObserver;
use crate::observer::finality::recording::SharedRecorder;
use crate::observer::output::MetricsOutput;
use crate::observer::time_slice::MetricsConfig;
use crate::reporting::RunReports;
use crate::scenario::ScenarioConfig;
use crate::scenario_gen::{Racking, ScenarioData};
use crate::scheduler::Scheduler;
use crate::sim_spec::SLICE_WIDTH_MS;
use crate::sim_types::{Counters, SimResult, StopReason};

/// Dispatch scenario config variant → ScenarioData.
///
/// The public first stage of a run: callers resolve, then hand the data
/// to [`run`] (building their pathfinder from `data.map` between
/// the two). Reads the storage layout back out of the compiled map's grid
/// geometry, which is the only thing a desmap map offers.
pub fn resolve_scenario(
    map: WarehouseMap,
    scenario: &ScenarioConfig,
    config: &SimConfig,
    vtc: u32,
) -> Result<ScenarioData, String> {
    let racking = crate::scenario_gen::racking_from_map(&map, vtc)?;
    resolve_scenario_with_racking(map, racking, scenario, config)
}

/// The same first stage for a map that states its own storage layout.
///
/// A world-protocol document states which rack each slot belongs to, how high
/// each shelf sits and which face serves it, so its loader builds the
/// [`Racking`] directly and hands it over rather than leaving it to be
/// rediscovered from grid geometry.
pub fn resolve_scenario_with_racking(
    map: WarehouseMap,
    racking: Racking,
    scenario: &ScenarioConfig,
    config: &SimConfig,
) -> Result<ScenarioData, String> {
    use crate::scenario::InventoryDistribution;
    use crate::scenario_gen::resolve;
    use crate::scenario_gen::{build_scenario_data, generate_scenario_from_map_with_racking};
    let data = match scenario {
        ScenarioConfig::Generated(generated) => {
            generate_scenario_from_map_with_racking(map, racking, generated, config)
        }
        ScenarioConfig::Inline(inline) => {
            let (skus, orders) = resolve::resolve_inline(inline)?;
            let upt = inline.units_per_tote.max(1);
            build_scenario_data(map, racking, skus, orders, None, config, upt, InventoryDistribution::SingleSku, 0)
        }
        ScenarioConfig::Csv(csv_cfg) => {
            let (skus, orders) = resolve::resolve_csv(csv_cfg)?;
            let upt = csv_cfg.units_per_tote.max(1);
            build_scenario_data(map, racking, skus, orders, None, config, upt, InventoryDistribution::SingleSku, 0)
        }
        ScenarioConfig::SiteExtract(site) => {
            // The client name appears here and nowhere else in the engine:
            // this is where a site's format adapter is chosen.
            let (skus, orders) = crate::scenario_gen::walmart::resolve(site)?;
            let upt = site.units_per_tote;
            build_scenario_data(
                map,
                racking,
                skus,
                orders,
                None,
                config,
                upt,
                site.inventory_distribution,
                site.order_totes,
            )
        }
        ScenarioConfig::BulkSource(bulk) => {
            let (skus, pool) = resolve::resolve_bulk(bulk, config.putaway.ignore_hour_gate)?;
            let upt = bulk.units_per_tote.max(1);
            build_scenario_data(
                map,
                racking,
                skus,
                Vec::new(),
                Some(pool),
                config,
                upt,
                InventoryDistribution::SingleSku,
                0,
            )
        }
    }?;
    Ok(data)
}

/// What a caller holds on a run while it goes: the figures it polls the run's
/// progress from, and the signal that stops the run, each when it wants one.
pub struct RunnerLiveHandles {
    pub counters: Option<Counters>,
    pub stop_signal: Option<Arc<StopSignal>>,
}

/// Optional observation taps for a run. `Default` = a plain headless run.
///
/// `live` attaches a caller's handles on the running run; `subscribers` each
/// receive a clone of every [`SimEvent`] (the test harness's capture
/// channel); `recording` takes each slice as the observer makes it final,
/// which is how anything reads a live run's slices.
#[derive(Default)]
pub struct RunnerHooks {
    pub live: Option<RunnerLiveHandles>,
    pub subscribers: Vec<mpsc::Sender<SimEvent>>,
    pub recording: Option<SharedRecorder>,
}

// ---------------------------------------------------------------------------
// Solver inner
// ---------------------------------------------------------------------------

/// Drive a caller-built [`Scheduler`] to completion and return the results.
///
/// The scheduler arrives with root tasks registered; their managers may not
/// exist yet. `reports` reads only the managers they construct. This harness
/// owns the sim-horizon probe, stall census, leak audit and reporting while
/// the scheduler remains independent of warehouse concerns. `event_tx` is
/// the same sender the world emits through, passed in so this function
/// can drop every sender at teardown and the observer's channel closes.
/// Dropping is load-bearing: a sender clone retained by the caller past
/// this call keeps the channel open and the observer join below never
/// returns. `data` is borrowed only for observer configuration and
/// result assembly.
pub fn run(
    scheduler: Scheduler,
    reports: RunReports,
    data: &ScenarioData,
    config: SimConfig,
    event_tx: mpsc::Sender<SimEvent>,
    event_rx: mpsc::Receiver<SimEvent>,
    putaway_direct_bias: SimOffset,
    hooks: RunnerHooks,
) -> Result<(SimResult, MetricsOutput), String> {
    data.robot_models.validate_counts(data.htm_initial_positions.len(), data.vtm_zone_assignments.len())?;
    let RunnerHooks { live, subscribers, recording } = hooks;
    #[cfg(feature = "trace")]
    let _tracing_guard = crate::trace_output::install_subscriber(&config.trace)?;
    #[cfg(not(feature = "trace"))]
    if config.trace.enabled {
        eprintln!("[trace] trace output requested, but this binary was not built with the `trace` feature");
    }

    let start = Instant::now();
    let stop_signal = live.as_ref().and_then(|l| l.stop_signal.clone()).unwrap_or_else(StopSignal::new);
    if let Some(timeout_secs) = config.wall_timeout_secs {
        let signal = stop_signal.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs_f64(timeout_secs));
            if !signal.is_stopped() {
                eprintln!("[runner] wall-timeout ({timeout_secs:.1}s) reached, stopping the run");
                signal.stop(StopReason::WallTimeout);
            }
        });
    }

    let initial_inventory: u64 = data.inventory.values().map(|&v| v as u64).sum();
    let cell_spacing_mm = data.map.grid_bounds.as_ref().map(|b| b.cell_size_mm as u64).unwrap_or(1000);
    let speed_mm_per_sec = (config.vtm.speed * 1000.0) as u64;
    let graph = &data.map.graph;

    // Observer thread.
    // `MetricsConfig::station_types` is FxHashMap — collect directly to match
    // the field type (StationId is a small enum-friendly key).
    let station_types: FxHashMap<StationId, StationType> =
        data.station_data.iter().map(|s| (s.station_id, s.station_type)).collect();
    let station_cells: FxHashMap<StationId, CellId> =
        data.station_data.iter().map(|s| (s.station_id, s.presentation_cell)).collect();
    // Relay → zone lookup for stamping journey-leg journal rows with the zone of
    // each relay endpoint (mirrors the relay→zone index built in relay config).
    let relay_zones: FxHashMap<RelayId, u32> =
        data.zone_data.iter().flat_map(|zd| zd.relay_ids.iter().map(move |&rid| (rid, zd.zone_id.0))).collect();
    let mut relay_cell_zone: FxHashMap<u32, u32> = FxHashMap::default();
    let mut relay_zone_capacity: BTreeMap<u32, u32> = BTreeMap::new();
    for relay in &data.relay_data {
        if let Some(&zone_id) = relay_zones.get(&relay.relay_id) {
            relay_cell_zone.insert(relay.htm_access.0, zone_id);
            *relay_zone_capacity.entry(zone_id).or_default() += 1;
        }
    }
    // Build cell_positions table indexed by CellId.0 from the map graph.
    // Empty entries (cell_id slot with no cell) get WorldPos::default().
    let max_cell_id = graph.cells().map(|c| c.id.0).max().unwrap_or(0);
    let mut cell_positions = vec![WorldPos::default(); (max_cell_id as usize) + 1];
    // Cell KIND table: same index space, same pass. The observer holds no map
    // handle, so this is how #272's waiting attribution learns whether a hold
    // left a D30 on a parking cell, a presentation cell, a station queue cell,
    // a relay cell, or plain floor.
    let mut cell_kinds: Vec<Option<CellKind>> = vec![None; (max_cell_id as usize) + 1];
    for c in graph.cells() {
        if let Some(pos) = c.world_pos {
            cell_positions[c.id.0 as usize] = pos;
        }
        cell_kinds[c.id.0 as usize] = Some(c.kind);
    }
    // Events and execution use the same physical identity: VTMs precede HTMs
    // in the scenario's RobotId space. Playback keeps the exact model Arcs.
    let htm_models = Arc::new(
        data.robot_models
            .htm
            .iter()
            .enumerate()
            .map(|(index, model)| (RobotId((data.vtm_zone_assignments.len() + index) as u32), model.clone()))
            .collect(),
    );
    // Rack capacity + aisle attribution, all derived once from the scenario's
    // `zone_storage` (aisle = relay zone → its rack cells). Single definition
    // of rack capacity; the observer emits it per slice as `rack_capacity`
    // (global) and mirrors per-aisle capacity to MetricsOutput.
    let vtc = data.vertical_tote_capacity as u32;
    let mut rack_cell_zone: FxHashMap<u32, u32> = FxHashMap::default();
    for (zone_id, cells) in &data.zone_storage {
        for cell in cells {
            rack_cell_zone.insert(cell.0, zone_id.0);
        }
    }
    // Rack (RackId) → aisle, composed through rack_cells (rack → cell).
    let rack_id_zone: FxHashMap<u32, u32> = data
        .rack_cells
        .iter()
        .filter_map(|(&rack_id, &cell)| rack_cell_zone.get(&cell.0).map(|&zone| (rack_id.0, zone)))
        .collect();
    // Capacity counts racks, not cells: zones with `slots_per_row` anchor
    // several storage columns to one cell. Relay maps always have racks;
    // RTP maps have none, so fall back to cell counts there.
    let mut zone_capacity: BTreeMap<u32, u32> = BTreeMap::new();
    if data.rack_cells.is_empty() {
        for (zone_id, cells) in &data.zone_storage {
            zone_capacity.insert(zone_id.0, cells.len() as u32 * vtc);
        }
    } else {
        for &zone in rack_id_zone.values() {
            *zone_capacity.entry(zone).or_insert(0) += vtc;
        }
    }
    let total_rack_slots: u32 = zone_capacity.values().sum();
    let total_relays = relay_zones.len() as u32;
    // RelayId.0 → HTM-access cell for the resource-diagnostics handler (keys
    // VTM-idle / relay-pickup-wait records by the relay's clicked cell).
    let relay_htm_access_cells: FxHashMap<u32, u32> =
        data.map.relays.iter().map(|relay| (relay.id.0, relay.htm_access.0)).collect();
    let metrics_config = MetricsConfig {
        slice_width_ms: crate::sim_spec::SLICE_WIDTH_MS,
        sim_time_limit: Some(SimTime::ZERO + SimDuration::from(config.sim_duration)),
        speed_mm_per_sec: speed_mm_per_sec as f64,
        cell_spacing_mm: cell_spacing_mm as f64,
        nav_factor: config.nav_factor,
        station_types,
        station_cells,
        relay_zones,
        relay_cell_zone: Arc::new(relay_cell_zone),
        relay_zone_capacity: Arc::new(relay_zone_capacity),
        pick_embargo_ms: (config.pick.embargo_secs * 1000.0) as u64,
        cell_positions: Arc::new(cell_positions),
        link_curves: Arc::new(
            data.map.graph.edges().filter_map(|(from, to, edge)| edge.curve.map(|arc| ((from, to), arc))).collect(),
        ),
        htm_reverse_links: Arc::new(
            data.map
                .graph
                .edges()
                .filter_map(|(from, to, edge)| (edge.htm_drive == HtmDriveDirection::Reverse).then_some((from, to)))
                .collect(),
        ),
        htm_models,
        motion_step_ms: 100,
        record_robot_timelines: config.record_robot_timelines,
        total_rack_slots,
        rack_cell_zone: Arc::new(rack_cell_zone),
        rack_id_zone: Arc::new(rack_id_zone),
        zone_capacity: Arc::new(zone_capacity),
        total_relays,
        relay_htm_access_cells,
        cell_kinds: Arc::new(cell_kinds),
    };
    // The journal the world emits through, fetched before the observer starts:
    // a run that cannot build it returns with no observer thread left to join.
    let journal = scheduler.states().get::<EventJournal>().map_err(|error| error.to_string())?;
    let event_log = config.event_log;
    let observer_stop = stop_signal.clone();
    let observer_handle = std::thread::spawn(move || {
        let mut observer =
            SimObserver::new(metrics_config, event_rx).with_print_events(event_log).with_failure_stop(observer_stop);
        for sub in subscribers {
            observer = observer.subscribe_events(sub);
        }
        if let Some(recorder) = recording {
            observer = observer.with_recorder(recorder);
        }
        if let Some(counters) = live.and_then(|live| live.counters) {
            observer = observer.with_counters(counters);
        }
        observer.run()
    });

    // Wire the caller-built scheduler to this run's harness. The scheduler
    // is domain-blind: the sim bound reads the planned frontier through an
    // injected probe (`sim_duration` is a day length; the bound is
    // end-of-day = epoch + length, and the sim epoch is ZERO), and the
    // stall alarm's census is injected the same way.
    let mut scheduler = scheduler
        .with_stop_signal(stop_signal.clone())
        .with_sim_time_limit(SimTime::ZERO + SimDuration::from(config.sim_duration))
        .with_sim_horizon({
            let reports = reports.clone();
            Box::new(move || reports.planned_horizon())
        })
        .with_stall_report({
            let reports = reports.clone();
            Box::new(move || reports.stall_census())
        });
    // The observer hears each advance of B through the same channel as the
    // facts, so it reads every fact sent before an advance ahead of it. The
    // journal the world emits through hears it first, and refuses a fact
    // stamped behind it.
    let markers = event_tx.clone();
    scheduler = scheduler.with_advance_listener(Box::new(move |at| {
        journal.barrier_advanced(at);
        EventSink::send(&markers, SimEvent::BarrierAdvanced(at));
    }));

    let mut scheduler_result = scheduler.run();

    // Domain epilogue — this harness owns the world; the scheduler does
    // not. The task futures dropped inside run(), so every leaked
    // reservation is recorded by now: after a NATURAL completion a leak is
    // a real bug; a stopped or failed run's teardown leaks its dropped
    // chains' reservations by design.
    let leaked = reports.existing::<RelayService>().map_or_else(Vec::new, |relays| relays.drain_leaked());
    if !leaked.is_empty() {
        if scheduler_result.is_ok() && !scheduler.stopped() {
            scheduler_result = Err(PlanError::TaskGraphInvariant {
                context: "runner",
                detail: format!(
                    "leak audit: {} relay reservations dropped unfinalized after natural completion (first: relay={:?} zone={:?} start={:?})",
                    leaked.len(),
                    leaked[0].0,
                    leaked[0].1,
                    leaked[0].2,
                ),
            });
        } else {
            eprintln!("[runner] teardown dropped {} unfinalized relay reservations", leaked.len());
        }
    }
    if scheduler_result.is_ok() && !scheduler.stopped() {
        scheduler_result = reports.check_quiescence_invariants();
    }
    if let Some(report) = reports.status_report() {
        eprintln!("{report}");
    }
    if let Some(report) = reports.existing::<RelayService>().and_then(|relays| relays.acceptance_report()) {
        eprintln!("{report}");
    }
    if let Some(report) = reports.existing::<AvailableHtmRegistry>().and_then(|available| available.selection_report())
    {
        eprintln!("{report}");
    }
    if let Some(report) = reports.existing::<RelayService>().and_then(|relays| relays.deposit_quote_report()) {
        eprintln!("{report}");
    }
    if let Some(report) = reports.existing::<RelayService>().and_then(|relays| relays.vtm_service.acceptance_report()) {
        eprintln!("{report}");
    }
    if let Some(line) = scheduler.barrier().report() {
        eprintln!("{line}");
    }
    if scheduler_result.is_ok() {
        emit_bot_plan_snapshots(&reports);
    }

    // Oracle-based relay telemetry (same numbers as the headless
    // relay_utilization_pct line), read before the world drops. The
    // observer builds MetricsOutput from events and can't see the oracle,
    // so these are injected below after join.
    let relay_utilization = reports.relay_utilization_summary();
    let relay_slot_occupancy_timeline = reports.relay_slot_occupancy_timeline(SLICE_WIDTH_MS);

    // Drop the entire run registry, including raw input senders and any lazy
    // managers. The observer joins only after every sender has been released.
    drop(event_tx);
    drop(scheduler);
    drop(reports);

    // The observer joins on BOTH exits. A planner error used to return
    // before this join, discarding MetricsOutput — which made a failing
    // fixture ungateable. The task-model-v2 program gates transformation
    // PRs on run identity (specs/task-model-v2/PLAN.md G3/G4), and on this
    // baseline some fixtures terminate in a planner error (the known
    // quiescence-recognition family), so the identity block below must
    // survive the error path: identical behavior includes the identical
    // terminal signature.
    let observed = observer_handle.join().unwrap_or_else(|e| {
        let msg = if let Some(s) = e.downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = e.downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown panic".to_string()
        };
        panic!("observer thread panicked: {}", msg);
    });
    // The facts broke a rule of the event stream and the observer stopped the
    // run on it: that is the run's failure, whatever the scheduler says.
    let mut metrics_output = match observed {
        Ok(output) => output,
        Err(violation) => {
            eprintln!("[gate] result=error error=observer: {violation}");
            return Err(format!("observer stopped the run: {violation}"));
        }
    };
    metrics_output.relay_utilization = relay_utilization;
    metrics_output.relay_slot_occupancy_timeline = relay_slot_occupancy_timeline;

    if let Err(ref e) = scheduler_result {
        let cum = &metrics_output.cumulative_scalars;
        eprintln!(
            "[gate] result=error sim_ms={} picks={} putaways={} orders={} events={}",
            metrics_output.sim_elapsed_ms,
            cum.picks_completed,
            cum.putaways_completed,
            cum.orders_completed,
            metrics_output.total_events,
        );
        eprintln!("[gate] error={e}");
        eprintln!("[gate] digest {}", metrics_output.event_digest);
        eprintln!("[gate] wall_secs={:.3} (informational; not identity)", start.elapsed().as_secs_f64(),);
    }
    scheduler_result.map_err(|e| {
        eprintln!("[runner] scheduler failed: {}", e);
        format!("scheduler failed: {}", e)
    })?;

    let wall_clock = start.elapsed();
    let stop_reason = stop_signal.take_reason().unwrap_or(StopReason::Complete);
    let sim_duration = match stop_reason {
        StopReason::SimTimeLimit => config.sim_duration,
        _ => Duration::from_millis(metrics_output.sim_elapsed_ms),
    };
    let cum = &metrics_output.cumulative_scalars;
    let completions = metrics_output.completions_through(sim_duration.as_millis() as u64);
    let total_picks = completions.picked_units as usize;
    let total_orders_completed = completions.completed_orders as usize;
    let completed_order_picks = total_picks;
    let sim_hours = sim_duration.as_secs_f64() / 3600.0;
    let per_hour = |n: u64| {
        if sim_hours > 0.0 { n as f64 / sim_hours } else { 0.0 }
    };
    // Throughput divides post-embargo picks by the post-embargo window so the
    // numerator and denominator cover the same span. `pre_embargo_picks` is 0
    // in a correct run (the dispatch path floors admission at the embargo);
    // subtracting it keeps a stray violation from double-inflating picks/hr.
    let post_embargo_picks = completions.picked_units.saturating_sub(cum.pre_embargo_picks);
    let throughput_picks_per_hour =
        crate::sim_types::picks_per_hour(post_embargo_picks, sim_duration, config.pick.embargo_secs);
    let throughput_putaways_per_hour = per_hour(completions.inducted_totes);
    let throughput_putaway_items_per_hour = per_hour(completions.inducted_units);

    // The task-model-v2 identity block (specs/task-model-v2/PLAN.md G3/G4):
    // the first two lines are compared verbatim between baseline and
    // candidate runs; wall time is explicitly excluded from identity. The
    // digest hash tier requires DES_DIGEST=1 (see `crate::observer::digest`).
    // The error-path twin above prints `result=error` + the terminal
    // signature instead of `stop=`.
    eprintln!(
        "[gate] result=complete stop={:?} sim_ms={} picks={} putaways={} orders={} events={}",
        stop_reason,
        sim_duration.as_millis(),
        completions.picked_units,
        completions.inducted_units,
        completions.completed_orders,
        metrics_output.total_events,
    );
    eprintln!("[gate] digest {}", metrics_output.event_digest);
    eprintln!("[gate] wall_secs={:.3} (informational; not identity)", wall_clock.as_secs_f64(),);
    Ok((
        SimResult {
            wall_clock_duration: wall_clock,
            sim_duration,
            total_orders_completed,
            total_picks,
            throughput_picks_per_hour,
            throughput_putaways_per_hour,
            throughput_putaway_items_per_hour,
            completed_order_picks,
            // Not tracked by the observer; leave 0 until a scenario-side
            // expected-picks counter exists (callers compute their own
            // `expected` from the order list when they need completion %).
            expected_order_picks: 0,
            event_count: metrics_output.total_events as u64,
            metrics: Default::default(),
            initial_inventory,
            putaway_direct_bias,
            stop_reason,
        },
        metrics_output,
    ))
}

// After a completed run, snapshot each bot's remaining plan legs for the
// journal.
fn emit_bot_plan_snapshots(reports: &RunReports) {
    let (Some(bots), Some(events)) = (reports.existing::<BotService>(), reports.existing::<EventJournal>()) else {
        return;
    };
    bots.for_each_bot(|robot_id, _kind, plan| {
        let legs = plan_leg_infos(plan);
        if !legs.is_empty() {
            events.emit(BotPlanSnapshotEvent { robot_id, legs });
        }
    });
}

fn plan_leg_infos(plan: &BotPlan) -> Vec<PlanLegInfo> {
    use crate::events::{PLAN_SNAPSHOT_MAX_LEGS, PlanLegInfo, PlanLegKind};
    use crate::model::bot::plan::Leg;
    plan.legs()
        .iter()
        .filter_map(|leg| {
            let dest_cell = match leg.kind {
                Leg::StationWork { cell } => cell.0,
                _ => plan.waypoints_for_leg(leg.id).last()?.cell.0,
            };
            let kind = match leg.kind {
                Leg::Travel => PlanLegKind::Travel,
                Leg::RelayApproach => PlanLegKind::RelayApproach,
                Leg::StationApproach => PlanLegKind::StationApproach,
                Leg::StationWork { .. } => PlanLegKind::StationWork,
                Leg::RelayDeposit => PlanLegKind::RelayDeposit,
                Leg::Parking { .. } => PlanLegKind::Parking,
                Leg::VtmTravel => PlanLegKind::VtmTravel,
                Leg::VtmForkOp => PlanLegKind::VtmForkOp,
                Leg::ConveyorDeposit { .. } => PlanLegKind::ConveyorDeposit,
                Leg::ConveyorCollect { .. } => PlanLegKind::ConveyorCollect,
            };
            Some(PlanLegInfo { kind, dest_cell, enter: leg.action_ready_arrival })
        })
        .take(PLAN_SNAPSHOT_MAX_LEGS)
        .collect()
}
