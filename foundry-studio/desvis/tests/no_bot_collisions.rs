//! Invariant: no two bots ever occupy the same cell at the same time.
//!
//! Runs `minimal_putaway.json --orders 0` (a putaway-only fixture), walks
//! the observer's emitted time slices, samples each slice at fine intervals,
//! and computes every tracked bot's interpolated cell using the same
//! step-walk logic as desvis playback. If two bots are computed to be in
//! the same cell at the same time, the test panics with a precise location.
//!
//! # Why this lives here, not in dessim
//!
//! The check is "would the desvis renderer ever paint two bots into the
//! same cell?" It uses the *same* step-walk that `desvis::app::playback`
//! uses to compute bot position at a given time. So if it ever fires,
//! either:
//!   - the upstream emit is wrong (pathfinder collision violation, ParkIdleHtms-
//!     style Nav-emit gap, oracle/registry divergence), or
//!   - the observer's slice splitting drops a bot's coverage during a long
//!     stretch (e.g., wait waypoints spanning many slices — see
//!     `record_boundary_split` which only writes into slice_idx and
//!     slice_idx+1).
//!
//! Either way, defense-in-depth catches what visual inspection would
//! eventually catch — but here, with a precise location and zero ambiguity.
//!
//! # Limitations
//!
//! - Bots that never have a Nav event aren't tracked (no `InitialPositions`
//!   exposure in `MetricsOutput` today). For minimal_putaway --orders 0
//!   every bot eventually moves, so this is fine for the current scope.
//!   Future enhancement: thread initial-positions through MetricsOutput so
//!   stationary bots are also covered.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use desgraph::RobotType;
use desmap::WarehouseMap;
use dessim::events::EventSink;
use dessim::observer::time_slice::BotLocation;
use dessim::pathfinding::holding_cells;
use dessim::runner::RunnerHooks;
use dessim::{FHSippPathfinder, Pathfinder, ScenarioConfig, SimConfig, SimDuration};
use native_modules as _;

const N_SAMPLES_PER_SLICE: usize = 20;

#[test]
fn no_bot_cell_collisions_minimal_putaway_orders_0() {
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sim/dessim/tests/fixtures/minimal_putaway_spec.json");
    let loaded = dessim::world_spec::load(&fixture).expect("load world spec");
    let mut scenario_config = loaded.scenario;
    let mut config = loaded.settings;

    // --orders 0 equivalent: zero out any pick demand.
    config.max_orders = Some(0);
    if let ScenarioConfig::Generated(ref mut g) = scenario_config {
        g.num_orders = 0;
        g.total_order_picks = 0;
    }

    let mut data = dessim::runner::resolve_scenario_with_racking(
        loaded.world.map.clone(),
        loaded.world.racking.clone(),
        &scenario_config,
        &config,
    )
    .expect("scenario gen");

    let pathfinder = fh_sipp_for_map(&data.map, &config);
    let (event_tx, event_rx) = std::sync::mpsc::channel();
    let sink: Arc<dyn EventSink> = Arc::new(event_tx.clone());
    let (scheduler, model, bias) =
        dessim::composition::build(&mut data, &config, pathfinder, Some(sink)).expect("composition");
    let (_sim_result, metrics_output) =
        dessim::runner::run(scheduler, model, &data, config.clone(), event_tx, event_rx, bias, RunnerHooks::default())
            .expect("runner");

    let slice_width_ms = config.slice_width_ms;

    // Walk slices; carry forward each bot's last-known cell so bots absent
    // from a slice's bot_locations (e.g., during a multi-slice wait) still
    // count toward collision checks at their last position.
    let mut last_known: HashMap<u32, u32> = HashMap::new();

    for (slice_idx, slice) in metrics_output.time_slices.iter().enumerate() {
        for sample in 0..N_SAMPLES_PER_SLICE {
            let offset = (sample as u64) * slice_width_ms / (N_SAMPLES_PER_SLICE as u64);
            let t_sample = slice.start_ms + offset;

            // Snapshot floor-bot positions at this sample time. Start from
            // last known, then overlay each floor bot present in the current
            // slice (computed via the playback step-walk logic). VTMs are
            // rail-mounted (different physical plane than the floor) so a
            // VTM appearing at the same 2D cell as an HTM is not a real
            // collision — exclude them from the check.
            let mut current: HashMap<u32, u32> = last_known.clone();
            for (bot_id, loc) in &slice.bot_locations {
                if matches!(loc.robot_type, RobotType::Vtm) {
                    continue;
                }
                current.insert(*bot_id, interpolated_cell(loc, offset));
            }

            // Group by cell; any cell with ≥2 occupants is a collision.
            let mut occupants: HashMap<u32, Vec<u32>> = HashMap::new();
            for (bot_id, cell) in &current {
                occupants.entry(*cell).or_default().push(*bot_id);
            }
            for (cell, bots) in &occupants {
                assert!(
                    bots.len() <= 1,
                    "Bot collision at t={}ms (slice {}, sample {}/{}): cell c{} occupied by bots {:?}",
                    t_sample,
                    slice_idx,
                    sample,
                    N_SAMPLES_PER_SLICE,
                    cell,
                    {
                        let mut sorted = bots.clone();
                        sorted.sort();
                        sorted
                    }
                );
            }
        }

        // Update last_known to the cell each floor bot ends this slice at.
        // VTMs are excluded — see the per-sample comment above.
        for (bot_id, loc) in &slice.bot_locations {
            if matches!(loc.robot_type, RobotType::Vtm) {
                continue;
            }
            let last_cell = loc.steps.last().map(|s| s.cell_id).unwrap_or(loc.start_cell);
            last_known.insert(*bot_id, last_cell);
        }
    }
}

fn fh_sipp_for_map(map: &WarehouseMap, config: &SimConfig) -> Arc<dyn Pathfinder> {
    let speed_mm_per_sec = (config.vtm.speed * 1000.0) as u64;
    let holding = holding_cells(map);
    Arc::new(FHSippPathfinder::new(map, speed_mm_per_sec, SimDuration::seconds(1), holding))
}

/// Compute the cell a bot occupies at `offset_ms` within its slice.
///
/// Mirrors the step-walk in `desvis::app::playback` — when the sample
/// lands mid-step, the bot is reported at the *previous* cell (the source
/// of the in-progress move), only snapping to the destination once the
/// move's `delta_ms` has fully elapsed. For Wait waypoints the source and
/// destination are the same, so the bot stays put for the full delta.
fn interpolated_cell(loc: &BotLocation, offset_ms: u64) -> u32 {
    let mut elapsed: u64 = 0;
    let mut prev_cell = loc.start_cell;
    let mut found = loc.start_cell;
    for step in &loc.steps {
        let step_end = elapsed + step.delta_ms;
        if step_end > offset_ms && step.delta_ms > 0 {
            // Mid-step: bot is at the source cell during transit.
            return prev_cell;
        }
        elapsed = step_end;
        prev_cell = step.cell_id;
        found = step.cell_id;
    }
    found
}
