//! Within `harness`, `run` is THE RUN ITSELF: it builds the scenario, the
//! pathfinder and the composition from a prepared configuration, drives the
//! runner to its stop while the run's recording is written, computes the
//! `SummaryStats` as the run ends and closes the recording with them, and
//! assembles the `PerfProfile` a workbook reads.
//!
//! It is one function because every caller has to get the same numbers: desvis
//! headless and the Crucible server both run a spec through here, so a
//! recording written by the server and one written by desvis differ only in
//! the provenance stamped on them.

// std's `HashMap`, not the repository's `FxHashMap`: every map named below
// belongs to `MetricsOutput`, `observer::duty` or `SummaryStats`, which this
// module receives and hands back unchanged. Switching the spelling here means
// switching those types, which is a change to four modules this round does not
// otherwise touch.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::events::EventSink;
use crate::metrics::profile::{
    IdTables, PerfProfile, ProfileMetadata, ResolvedWorkload, RunDocuments, ScenarioSource, SliceRecord,
    StationGapStats, SummaryStats, VtmFleet, compute_gap_stats, grid_to_dynamic, robot_identities, scalars_to_map,
};
use crate::metrics::reporting::extraction_stream;
use crate::metrics::worldview::{RecordingError, RunRecording, RunScenario};
use crate::model::RobotId;
use crate::observer::duty::{DutyBreakdown, try_partition_by_fleet, try_partition_timelines_by_fleet};
use crate::observer::finality::recording::SharedRecorder;
use crate::observer::output::{Activity, MetricsOutput};
use crate::runner::{RunnerHooks, run as dessim_run};
use crate::scenario::InventoryDistribution;
use crate::scenario_gen::Racking;
use crate::{SimConfig, SimResult};
use chrono::{Local, SecondsFormat};
use desgraph::{RobotType, StationId};
use desmap::WarehouseMap;

use super::RecordingTarget;
use super::identity::RunIdentity;
use serde_json::Value;

/// Build and run a simulation, writing its recording to `recording` as it
/// goes, and return the profile and summary stats.
///
/// `map` and `racking` come from the world document (`world_doc`, embedded
/// into the profile verbatim) — the caller loads once so it
/// can also read map-derived values (e.g. `WarehouseMap::vtm_count`).
/// `documents` and `ids` come from the same load: the files the run was built
/// from and what the world document calls the ids the run mints, both of
/// which a recording has to carry and neither of which the simulator reads.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_simulation(
    world_doc: Value,
    documents: RunDocuments,
    ids: IdTables,
    map: WarehouseMap,
    racking: Racking,
    scenario_source: &ScenarioSource,
    name: String,
    id: RunIdentity,
    label: &str,
    events: bool,
    hooks: RunnerHooks,
    recording: Option<RecordingTarget>,
    mut config: SimConfig,
) -> Result<(PerfProfile, SummaryStats), RecordingError> {
    config.event_log = events;

    // Both were read when the run was prepared, which is where a spec that
    // states neither is refused; here they cannot fail.
    let bounds = map.grid_bounds.expect("prepare refuses a document with no grid");
    // Read off the map before the scenario builder takes it: the DA50 fleet is
    // the sum of the per-zone counts and nothing in SimConfig states it.
    let vtm_fleet = VtmFleet::try_from(map.vtm_count()).expect("prepare refuses a fleet it cannot size");

    // Build and run the simulation.
    println!("Running simulation: {} ...", label);

    let mut data = match scenario_source {
        ScenarioSource::Loaded { skus, orders, .. } => crate::scenario_gen::build_scenario_data(
            map,
            racking,
            skus.clone(),
            orders.clone(),
            None,
            &config,
            10,
            InventoryDistribution::SingleSku,
            // A replayed profile carries no tote population of its own.
            0,
        )
        .unwrap_or_else(|e| panic!("failed to generate scenario: {}", e)),
        ScenarioSource::Parametric(scenario_config) => {
            crate::runner::resolve_scenario_with_racking(map, racking, scenario_config, &config)
                .unwrap_or_else(|e| panic!("failed to generate scenario: {}", e))
        }
    };

    // The run consumes the resolved workload; the profile keeps a copy, which
    // is where the workbook's Orders and SKU Catalog sheets read it.
    let workload = ResolvedWorkload { skus: data.skus.clone(), orders: data.orders.clone() };
    let metadata = ProfileMetadata {
        branch: id.branch,
        commit: id.commit,
        commit_short: id.commit_short,
        dirty: id.dirty,
        timestamp: id.timestamp,
        hostname: id.hostname,
        grid_width: bounds.width,
        grid_depth: bounds.depth,
        slice_width_ms: config.slice_width_ms,
    };
    let recording = match recording {
        // A run is recorded once: into the file the caller names, or by the
        // recorder its hooks carry.
        Some(_) if hooks.recording.is_some() => return Err(RecordingError::RecorderAndFile),
        Some(target) => {
            let scenario = RunScenario {
                name: name.clone(),
                metadata: metadata.clone(),
                documents: documents.clone(),
                ids: ids.clone(),
                sim_config: config.clone(),
                scenario: scenario_source.clone(),
                workload: workload.clone(),
                vtm_fleet,
                run: target.run,
                attempt: target.attempt,
            };
            Some(Arc::new(Mutex::new(RunRecording::create(&target.path, target.tap, scenario)?)))
        }
        None => None,
    };
    let hooks = match &recording {
        Some(file) => RunnerHooks { recording: Some(file.clone() as SharedRecorder), ..hooks },
        None => hooks,
    };

    let pathfinder = crate::pathfinding::fh_sipp_for_map(&data.map, &config);
    let solver_start = Local::now();
    println!("Sim start:   {}", solver_start.to_rfc3339_opts(SecondsFormat::Millis, true));
    // Drop guard so end time + wall print even if run panics. The
    // the runner panicking on a scheduler invariant is a common failure mode, and
    // the timing readout is most useful exactly in that case.
    struct EndTimingGuard {
        start: Instant,
    }
    impl Drop for EndTimingGuard {
        fn drop(&mut self) {
            let end = Local::now();
            let elapsed = self.start.elapsed();
            println!("Sim end:     {}", end.to_rfc3339_opts(SecondsFormat::Millis, true));
            println!("Solver wall: {:.3}s", elapsed.as_secs_f64());
        }
    }
    let _timing = EndTimingGuard { start: Instant::now() };
    // Assemble the run: the caller owns the event channel and invokes the
    // deployment composition directly; the runner only drives.
    let (event_tx, event_rx) = std::sync::mpsc::channel();
    let sink: Arc<dyn EventSink> = Arc::new(event_tx.clone());
    let (scheduler, model, putaway_direct_bias) = crate::composition::build(&mut data, &config, pathfinder, Some(sink))
        .unwrap_or_else(|e| panic!("composition failed: {}", e));
    let result = dessim_run(scheduler, model, &data, config.clone(), event_tx, event_rx, putaway_direct_bias, hooks);
    let (sim_result, mut metrics_output) = result.unwrap_or_else(|e| panic!("solver failed: {}", e));
    drop(_timing);

    // The summary as the run ends, which the recording's end block states.
    let pick_time_ms = (config.operator.time_per_pick_secs * 1000.0).round() as u64;
    let mut summary = compute_summary(&sim_result, &mut metrics_output, pick_time_ms, config.pick.embargo_secs)
        .unwrap_or_else(|e| panic!("compute_summary: {e}"));
    if let Some(recording) = recording {
        let recording = Arc::into_inner(recording).expect("the observer let go of the recording when it returned");
        let recording = recording.into_inner().expect("the observer never panicked holding the recording");
        recording.finish(&metrics_output, &mut summary, &sim_result)?;
    }

    // Build time slices.
    let time_slices: Vec<SliceRecord> = metrics_output
        .time_slices
        .iter()
        .map(|ts| SliceRecord {
            start_ms: ts.start_ms,
            metrics: scalars_to_map(&ts.scalars),
            grid: grid_to_dynamic(&ts.grid),
            bot_locations: ts.bot_locations.clone(),
            relay_state: ts.relay_state.clone(),
            tote_state: ts.tote_state.clone(),
            station_diag: ts.station_diag.clone(),
        })
        .collect();

    let prof = PerfProfile {
        name,
        metadata,
        world: world_doc,
        documents,
        ids: IdTables { robots: robot_identities(&metrics_output.robot_types), ..ids },
        sim_config: config,
        scenario: scenario_source.clone(),
        workload,
        vtm_fleet,
        summary: summary.clone(),
        sim_result,
        time_slices,
        cumulative_grid: grid_to_dynamic(&metrics_output.cumulative_grid),
        cumulative_scalars: scalars_to_map(&metrics_output.cumulative_scalars),
    };

    Ok((prof, summary))
}

pub(super) fn compute_summary(
    result: &SimResult,
    metrics: &mut MetricsOutput,
    pick_time_ms: u64,
    embargo_secs: f64,
) -> Result<SummaryStats, String> {
    let sim_elapsed_ms = result.sim_duration.as_millis() as u64;
    let completions = metrics.completions_through(sim_elapsed_ms);

    // Partition duty totals and timelines by robot type over the FULL fleet:
    // both detail maps carry every VTM/HTM in the InitialPositions roster,
    // defaulting never-moved robots to all-idle so serialized per-bot data,
    // histograms, and the Gantt show the whole fleet. Map cardinality is not
    // the Summary idle denominator; the configured counts below remain the
    // explicit metric definition from #342. The recorder is type-agnostic and
    // robot_types is seeded by the observer's handle_initial_positions (from
    // the InitialPositions event built in solver::run). A robot with
    // recorded activity but no roster entry means the seeding contract
    // upstream is broken — the `try_` variants surface it as an Err naming
    // the robot, which `run_simulation` stops the run on.
    let duty_maps = try_partition_by_fleet(&metrics.robot_duty_totals, &metrics.robot_types)
        .map_err(|orphan| format!("robot {orphan:?} has duty totals but is missing from MetricsOutput::robot_types"))?;
    let (vtm_duty_totals, htm_duty_totals) = (duty_maps.vtm, duty_maps.htm);

    // Idle denominators use the configured fleet from `robot_types` (the
    // full InitialPositions roster), never from duty-map length —
    // greymatter-foundry#254's fleet-state rule (eligible count × included
    // duration), as implemented by `da50_state_report`. A robot with a
    // default, zero-busy duty row counts as fully idle, the same way the state
    // family backfills configured-but-unobserved vehicles as Waiting/NoTask.
    let vtm_fleet = fleet_count(&metrics.robot_types, RobotType::Vtm);
    let htm_fleet = fleet_count(&metrics.robot_types, RobotType::Htm);
    let vtm_idle_pct = fleet_idle_pct(&vtm_duty_totals, vtm_fleet, sim_elapsed_ms);
    let htm_idle_pct = fleet_idle_pct(&htm_duty_totals, htm_fleet, sim_elapsed_ms);

    let vtm_idle_reason_counts = metrics.vtm_idle_reason_counts.clone();
    let timeline_maps = try_partition_timelines_by_fleet(&metrics.robot_timelines, &metrics.robot_types)
        .map_err(|orphan| format!("robot {orphan:?} has a timeline but is missing from MetricsOutput::robot_types"))?;
    let (vtm_robot_timelines, htm_robot_timelines) = (timeline_maps.vtm, timeline_maps.htm);

    // Per-station picks/hr uses the SAME post-embargo window as the top-line
    // `throughput_picks_per_hour` (via the shared helper) so the per-station and
    // aggregate rates can't disagree (greymatter-foundry#208).
    let mut presentations_per_station: Vec<(StationId, f64)> = completions
        .picks_per_station
        .iter()
        .map(|(&station_id, &picks)| {
            (station_id, crate::sim_types::picks_per_hour(picks, result.sim_duration, embargo_secs))
        })
        .collect();
    presentations_per_station.sort_by_key(|(station, _)| *station);

    let avg_pres = if presentations_per_station.is_empty() {
        0.0
    } else {
        let total: f64 = presentations_per_station.iter().map(|(_, v)| v).sum();
        total / presentations_per_station.len() as f64
    };

    // Station utilization: operator-busy / active-window, per station and
    // across-station mean. Active window = [first_seen, sim_end] so a
    // station that bootstrapped late isn't penalized for the warmup it
    // had no work for. Stations are the throughput-binding resource;
    // VTM/HTM duty is diagnostic but secondary. Sorted by StationId for
    // stable output ordering.
    let mut station_utilization_pct: Vec<(StationId, f64)> = if sim_elapsed_ms == 0 {
        Vec::new()
    } else {
        metrics
            .station_busy_ms
            .iter()
            .map(|(&station_id, &busy_ms)| {
                let first_seen_ms = metrics.station_first_seen.get(&station_id).map(|t| t.as_ms()).unwrap_or(0);
                let window_ms = sim_elapsed_ms.saturating_sub(first_seen_ms).max(1) as f64;
                (station_id, (busy_ms as f64 / window_ms).min(1.0))
            })
            .collect()
    };
    station_utilization_pct.sort_by_key(|(id, _)| id.0);
    let avg_station_utilization_pct = if station_utilization_pct.is_empty() {
        0.0
    } else {
        let total: f64 = station_utilization_pct.iter().map(|(_, v)| v).sum();
        total / station_utilization_pct.len() as f64
    };

    let pf_queries = metrics.cumulative_scalars.pathfinder_queries;
    let pf_failures = metrics.cumulative_scalars.pathfinder_failures;
    let pf_failure_rate = if pf_queries > 0 { pf_failures as f64 / pf_queries as f64 } else { 0.0 };

    let mut station_gap_stats: Vec<StationGapStats> = Vec::new();
    let mut total_gaps = 0usize;
    let mut smooth_gaps = 0usize;

    let mut station_ids: Vec<StationId> = metrics.station_gaps_ms.keys().copied().collect();
    station_ids.sort();
    for station_id in station_ids {
        let gaps = metrics.station_gaps_ms.get_mut(&station_id).unwrap();
        if gaps.is_empty() {
            continue;
        }
        let (min_ms, mean_ms, p95_ms, max_ms) = compute_gap_stats(gaps);
        smooth_gaps += gaps.iter().filter(|&&g| g <= pick_time_ms).count();
        total_gaps += gaps.len();
        station_gap_stats.push(StationGapStats { station_id, count: gaps.len(), min_ms, mean_ms, p95_ms, max_ms });
    }

    let flow_score = if total_gaps > 0 { smooth_gaps as f64 / total_gaps as f64 } else { 0.0 };

    // Effective HTM cruise speed: distance-weighted mean translation speed over
    // every `Travel` leg, split by load state. A leg's `duration` spans its
    // whole contiguous move/rotation run (see `HtmLegRecord`), so
    // `Σ distance_mm / Σ duration` (mm/ms = m/s) is the flat speed that
    // reproduces this run's translation-plus-rotation time; it is the value to
    // plug into `htm.{loaded,empty}_constant_speed`.
    // `Wait`/`Mechanical` rows carry duration but zero distance, so filtering to
    // `Travel` is required or the speed would be deflated by idle time.
    let (mut loaded_dist_mm, mut loaded_dur_ms) = (0u64, 0u64);
    let (mut empty_dist_mm, mut empty_dur_ms) = (0u64, 0u64);
    for leg in &metrics.htm_leg_log {
        if !matches!(leg.activity, Activity::Travel) {
            continue;
        }
        if leg.carrying_cargo {
            loaded_dist_mm += leg.distance_mm;
            loaded_dur_ms += leg.duration.as_ms();
        } else {
            empty_dist_mm += leg.distance_mm;
            empty_dur_ms += leg.duration.as_ms();
        }
    }
    let speed_mps = |dist_mm: u64, dur_ms: u64| {
        if dur_ms > 0 { dist_mm as f64 / dur_ms as f64 } else { 0.0 }
    };
    let htm_effective_speed_loaded_mps = speed_mps(loaded_dist_mm, loaded_dur_ms);
    let htm_effective_speed_empty_mps = speed_mps(empty_dist_mm, empty_dur_ms);
    let htm_effective_speed_combined_mps = speed_mps(loaded_dist_mm + empty_dist_mm, loaded_dur_ms + empty_dur_ms);

    let log_streams = vec![extraction_stream(&metrics.extraction_log).map_err(|error| error.to_string())?];
    metrics.extraction_log.clear();

    Ok(SummaryStats {
        log_streams,
        wall_clock_secs: result.wall_clock_duration.as_secs_f64(),
        sim_time_secs: result.sim_duration.as_secs_f64(),
        total_picks: result.total_picks as u64,
        total_orders_completed: result.total_orders_completed as u64,
        throughput_picks_per_hour: result.throughput_picks_per_hour,
        total_putaways: completions.inducted_units,
        throughput_putaway_items_per_hour: result.throughput_putaway_items_per_hour,
        htm_effective_speed_loaded_mps,
        htm_effective_speed_empty_mps,
        htm_effective_speed_combined_mps,
        vtm_idle_pct,
        htm_idle_pct,
        vtm_fleet_size: vtm_fleet,
        htm_fleet_size: htm_fleet,
        presentations_per_station_per_hour: presentations_per_station,
        avg_presentations_per_station_per_hour: avg_pres,
        station_utilization_pct,
        avg_station_utilization_pct,
        station_timelines: metrics.station_timelines.clone(),
        station_types: metrics.station_types.clone(),
        event_count: result.event_count,
        pathfinder_failure_rate: pf_failure_rate,
        station_gap_stats,
        flow_score,
        vtm_duty_totals,
        htm_duty_totals,
        vtm_idle_reason_counts,
        relay_utilization: metrics.relay_utilization.clone(),
        relay_slot_occupancy_timeline: metrics.relay_slot_occupancy_timeline.clone(),
        vtm_robot_timelines,
        htm_robot_timelines,
        stripe_plan_log: std::mem::take(&mut metrics.stripe_plan_log),
        pick_log: std::mem::take(&mut metrics.pick_log),
        completed_order_log: std::mem::take(&mut metrics.completed_order_log),
        putaway_log: std::mem::take(&mut metrics.putaway_log),
        htm_leg_log: std::mem::take(&mut metrics.htm_leg_log),
        vtm_cycle_log: std::mem::take(&mut metrics.vtm_cycle_log),
        vtm_operation_log: std::mem::take(&mut metrics.vtm_operation_log),
        aisle_state_log: std::mem::take(&mut metrics.aisle_state_log),
        vtm_leg_log: std::mem::take(&mut metrics.vtm_leg_log),
        aisle_flux_log: std::mem::take(&mut metrics.aisle_flux_log),
        rack_zone_capacity: metrics.rack_zone_capacity.iter().map(|(&z, &c)| (z, c)).collect(),
        putaway_direct_bias: result.putaway_direct_bias,
    })
}

/// Fleet idle fraction over configured fleet-time: `1 − Σ busy /
/// (fleet × sim_elapsed)`, with `fleet = configured.max(observed)` — the
/// same eligible-count clamp `da50_state_report` applies, so a roster
/// undercount widens the denominator instead of pushing idle negative.
/// From `compute_summary` the clamp is inert: the duty partition above
/// hard-errors on any source entry missing from the roster and returns one
/// row per configured robot, so observed never exceeds configured there; the
/// clamp keeps the helper safe standalone. A never-used robot contributes a
/// default, zero-busy row. Returns `0.0` for an empty fleet or zero elapsed
/// time (avoids divide-by-zero on degenerate runs).
fn fleet_idle_pct(duty: &HashMap<RobotId, DutyBreakdown>, configured_fleet: usize, sim_elapsed_ms: u64) -> f64 {
    let fleet = configured_fleet.max(duty.len());
    if fleet == 0 || sim_elapsed_ms == 0 {
        return 0.0;
    }
    let fleet_time_ms = fleet as f64 * sim_elapsed_ms as f64;
    let busy_ms: f64 = duty.values().map(|d| d.busy().as_ms() as f64).sum();
    (1.0 - busy_ms / fleet_time_ms).max(0.0)
}

/// Number of configured robots of `robot_type` in the observer's roster.
/// `MetricsOutput::robot_types` covers the full fleet of both types whether
/// or not a robot ever worked — `emit_initial_state` (flows/ttp/composition)
/// announces every robot once assembly completes.
fn fleet_count(robot_types: &HashMap<RobotId, RobotType>, robot_type: RobotType) -> usize {
    robot_types.values().filter(|&&t| t == robot_type).count()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::model::SimDuration;
    use crate::model::SkuId;
    use crate::observer::output::PutawayRecord;

    use super::*;

    fn busy_for(ms: u64) -> DutyBreakdown {
        DutyBreakdown { traveling: SimDuration::ms(ms), ..Default::default() }
    }

    #[test]
    fn never_dispatched_robots_count_as_fully_idle() {
        // 2 of 4 configured robots worked the whole run: fleet idle is 50%,
        // not the 0% a dispatched-only mean would report.
        let duty: HashMap<RobotId, DutyBreakdown> =
            [(RobotId(0), busy_for(1_000)), (RobotId(1), busy_for(1_000))].into();
        assert_eq!(fleet_idle_pct(&duty, 4, 1_000), 0.5);
    }

    #[test]
    fn zero_dispatch_run_is_fully_idle_not_zero() {
        // No robot ever received work: the configured fleet idled the whole
        // run. (The dispatched-only formula reported 0% idle here.)
        assert_eq!(fleet_idle_pct(&HashMap::new(), 3, 1_000), 1.0);
    }

    #[test]
    fn partial_busy_prorates_over_configured_fleet_time() {
        let duty: HashMap<RobotId, DutyBreakdown> = [(RobotId(0), busy_for(500))].into();
        assert_eq!(fleet_idle_pct(&duty, 2, 1_000), 0.75);
    }

    #[test]
    fn observed_robots_beyond_configured_widen_the_denominator() {
        // Same max(configured, observed) clamp as `da50_state_report`.
        // This input cannot arise via `compute_summary` (its duty partition
        // hard-errors on roster desyncs first); the test pins the helper's
        // standalone contract: an undercount widens the denominator rather
        // than pushing idle below zero.
        let duty: HashMap<RobotId, DutyBreakdown> =
            [(RobotId(0), busy_for(1_000)), (RobotId(1), busy_for(1_000)), (RobotId(2), busy_for(1_000))].into();
        assert_eq!(fleet_idle_pct(&duty, 2, 1_000), 0.0);
    }

    #[test]
    fn empty_fleet_and_zero_elapsed_return_zero() {
        assert_eq!(fleet_idle_pct(&HashMap::new(), 0, 1_000), 0.0);
        let duty: HashMap<RobotId, DutyBreakdown> = [(RobotId(0), busy_for(0))].into();
        assert_eq!(fleet_idle_pct(&duty, 1, 0), 0.0);
    }

    #[test]
    fn compute_summary_counts_and_serializes_the_full_roster() {
        // The idle denominators come from the InitialPositions roster
        // (`robot_types`), not from duty-map cardinality: one VTM + one HTM +
        // one RTP, none ever dispatched → both fleets are fully idle and the
        // RTP inflates neither count. The serialized duty maps nevertheless
        // carry the two configured robots as zero-busy rows, so histograms and
        // Gantt consumers see the full roster too.
        let mut metrics = MetricsOutput::default();
        metrics.robot_types =
            [(RobotId(0), RobotType::Vtm), (RobotId(1), RobotType::Htm), (RobotId(2), RobotType::Rtp)].into();
        let result = SimResult { sim_duration: Duration::from_secs(1), ..Default::default() };
        let summary = compute_summary(&result, &mut metrics, 1_000, 0.0).expect("compute_summary on an idle roster");
        assert_eq!(summary.vtm_fleet_size, 1);
        assert_eq!(summary.htm_fleet_size, 1);
        assert_eq!(summary.vtm_idle_pct, 1.0);
        assert_eq!(summary.htm_idle_pct, 1.0);
        assert_eq!(summary.vtm_duty_totals.len(), 1);
        assert_eq!(summary.htm_duty_totals.len(), 1);
        assert_eq!(summary.vtm_duty_totals[&RobotId(0)], DutyBreakdown::default());
        assert_eq!(summary.htm_duty_totals[&RobotId(1)], DutyBreakdown::default());
        assert!(!summary.vtm_duty_totals.contains_key(&RobotId(2)));
        assert!(!summary.htm_duty_totals.contains_key(&RobotId(2)));
    }

    #[test]
    fn summary_induction_total_excludes_work_after_run_cutoff() {
        let mut metrics = MetricsOutput::default();
        metrics.putaway_log = [1000, 1001]
            .map(|at| PutawayRecord {
                sim_time_ms: at,
                station_id: StationId(1),
                sku: SkuId(1),
                quantity: 6,
                presentation_id: None,
            })
            .to_vec();
        metrics.cumulative_scalars.putaways_completed = 12;
        let result = SimResult { sim_duration: Duration::from_secs(1), ..Default::default() };
        let summary = compute_summary(&result, &mut metrics, 1000, 0.0).unwrap();
        assert_eq!(summary.total_putaways, 6);
        assert_eq!(summary.putaway_log.len(), 2, "retain future facts for diagnostics");
    }
}
