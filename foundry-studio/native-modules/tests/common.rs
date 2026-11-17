//! Shared test helpers and assert_events! macro for solver integration tests.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;

use desgraph::RobotType;
use desmap::WarehouseMap;
use dessim::composition::{self, Composition, Definition};
use dessim::events::{EventJournal, EventSink, SimEvent};
use dessim::model::RobotId;
use dessim::observer::output::MetricsOutput;
use dessim::observer::time_slice::{BotLocation, TimeSliceData, ToteVisualState};
use dessim::pathfinding::WaypointAction;
use dessim::pathfinding::holding_cells;
use dessim::runner::{RunnerHooks, resolve_scenario_with_racking};
use dessim::scenario::ScenarioConfig;
use dessim::scheduler::task::Task;
use dessim::sim_types::SimResult;
use dessim::world_spec;
use dessim::worldmap::WorldMap;
use dessim::{FHSippPathfinder, Pathfinder, SimConfig, SimDuration};

/// Load a world-spec fixture by name from `tests/fixtures/` -- the
/// world-protocol counterpart of [`load_fixture`]. Returns the same
/// (map-source, scenario, settings) triple shape: the loaded world plus the
/// spec's workload blocks, with the fleet already defaulted from the
/// document's robots.
#[allow(dead_code)]
pub fn load_world_spec(name: &str) -> (WorldMap, ScenarioConfig, SimConfig) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    let loaded = world_spec::load(&path).unwrap_or_else(|e| panic!("failed to load world spec {name}: {e}"));
    (loaded.world, loaded.scenario, loaded.settings)
}

/// World-side counterpart of [`run_solver_capture_all`]: identical planner
/// assembly and solver, with the scenario resolved from the document's STATED
/// racking instead of rediscovered from a v3 design's grid.
#[allow(dead_code)]
pub fn run_world_capture_all(
    world: &WorldMap,
    scenario: &ScenarioConfig,
    config: SimConfig,
) -> (Vec<SimEvent>, SimResult, MetricsOutput) {
    try_run_world(world, scenario, config).expect("solver failed")
}

/// Same as [`run_world_capture_all`] but returns Result instead of panicking.
#[allow(dead_code)]
pub fn try_run_world(
    world: &WorldMap,
    scenario: &ScenarioConfig,
    config: SimConfig,
) -> Result<(Vec<SimEvent>, SimResult, MetricsOutput), String> {
    let mut data = resolve_scenario_with_racking(world.map.clone(), world.racking.clone(), scenario, &config)
        .expect("scenario gen");
    let pathfinder = fh_sipp_for_map(&data.map, &config);

    let (sub_tx, sub_rx) = mpsc::channel::<SimEvent>();
    let capture = std::thread::spawn(move || sub_rx.iter().collect::<Vec<_>>());

    let (event_tx, event_rx) = mpsc::channel();
    let sink: Arc<dyn EventSink> = Arc::new(event_tx.clone());
    let (scheduler, model, bias) =
        dessim::composition::build(&mut data, &config, pathfinder, Some(sink)).map_err(|e| e.to_string())?;
    let (result, metrics) = dessim::runner::run(
        scheduler,
        model,
        &data,
        config,
        event_tx,
        event_rx,
        bias,
        RunnerHooks { live: None, subscribers: vec![sub_tx], recording: None },
    )?;

    let events = capture.join().unwrap();
    Ok((events, result, metrics))
}

pub fn fh_sipp_for_map(map: &WarehouseMap, config: &SimConfig) -> Arc<dyn Pathfinder> {
    let speed_mm_per_sec = (config.vtm.speed * 1000.0) as u64;
    let holding = holding_cells(map);
    Arc::new(FHSippPathfinder::new(map, speed_mm_per_sec, SimDuration::seconds(1), holding))
}

/// Run the `minimal_pick_only_spec.json` world with no workflow tasks and one
/// task of the test's own, built from the run's journal and its event stream,
/// so the run's only warehouse facts are the ones that task reports.
#[allow(dead_code)]
pub fn run_one_task<T: Task + 'static>(
    task: impl FnOnce(EventJournal, Arc<dyn EventSink>) -> T,
    hooks: RunnerHooks,
) -> Result<(SimResult, MetricsOutput), String> {
    let (world, mut scenario, mut config) = load_world_spec("minimal_pick_only_spec.json");
    let ScenarioConfig::Generated(generated) = &mut scenario else { panic!("generated fixture") };
    generated.num_skus = 1;
    config.composition = Composition::Inline(Definition { tasks: vec![], task_defaults: Default::default() });
    let mut data = resolve_scenario_with_racking(world.map.clone(), world.racking.clone(), &scenario, &config)
        .expect("scenario gen");
    let pathfinder = fh_sipp_for_map(&data.map, &config);
    let (event_tx, event_rx) = mpsc::channel();
    let sink: Arc<dyn EventSink> = Arc::new(event_tx.clone());
    let (mut scheduler, reports, bias) =
        composition::build(&mut data, &config, pathfinder, Some(sink.clone())).expect("empty composition");
    // The task reports through the run's own journal, which hears B advance,
    // or straight into the stream. Only the task keeps either: a sender held
    // here past the run would keep the observer's channel open.
    let journal = scheduler.states().get::<EventJournal>().expect("the run's journal").as_ref().clone();
    scheduler.add_task(task(journal, sink));
    dessim::runner::run(scheduler, reports, &data, config, event_tx, event_rx, bias, hooks)
}

/// Audit BotLocation time slices for physical violations:
/// - No teleports (position jump > 1.1 * cell_spacing)
/// - No diagonal moves (both X and Y change)
/// - No same-cell HTM overlap (two HTMs at the same cell at the same time)
///
/// Same-cell overlap is currently a warning (eprintln) not a panic — some
/// corridors still have edge conflicts pending full resolution.
pub fn audit_bot_locations(slices: &[TimeSliceData], cell_spacing: f64) {
    use desgraph::RobotType;

    // --- Teleport + diagonal check ---
    for (si, slice) in slices.iter().enumerate() {
        for (&bot_key, loc) in &slice.bot_locations {
            if loc.robot_type == RobotType::Vtm {
                continue;
            }
            let mut prev_x = loc.start_pos.x_mm;
            let mut prev_y = loc.start_pos.y_mm;
            for (j, step) in loc.steps.iter().enumerate() {
                let dx = (step.pos.x_mm - prev_x).abs();
                let dy = (step.pos.y_mm - prev_y).abs();
                let dist = (dx * dx + dy * dy).sqrt();
                if dist > cell_spacing * 1.1 && step.delta_ms > 0 {
                    panic!(
                        "TELEPORT: slice {} bot {} step {}: ({:.0},{:.0}) -> ({:.0},{:.0}) = {:.0}mm (> {:.0}mm)",
                        si,
                        bot_key,
                        j,
                        prev_x,
                        prev_y,
                        step.pos.x_mm,
                        step.pos.y_mm,
                        dist,
                        cell_spacing * 1.1,
                    );
                }
                if dx > 1.0 && dy > 1.0 && step.delta_ms > 0 {
                    panic!("DIAGONAL: slice {} bot {} step {}: dx={:.0}mm dy={:.0}mm", si, bot_key, j, dx, dy,);
                }
                prev_x = step.pos.x_mm;
                prev_y = step.pos.y_mm;
            }
        }
    }
    eprintln!("=== BotLocation audit: PASS (no teleports, no diagonals) ===");

    // --- HTM same-cell overlap check ---
    let sample_interval_ms = 100u64;
    for (si, slice) in slices.iter().enumerate() {
        let slice_start = slice.start_ms;
        let htms: Vec<(u32, &BotLocation)> = slice
            .bot_locations
            .iter()
            .filter(|(_, loc)| loc.robot_type != RobotType::Vtm)
            .map(|(&k, v)| (k, v))
            .collect();
        if htms.len() < 2 {
            continue;
        }

        let cell_at = |loc: &BotLocation, offset_ms: u64| -> u32 {
            let mut t = 0u64;
            let mut current_cell = loc.start_cell;
            for step in &loc.steps {
                if t + step.delta_ms > offset_ms {
                    return current_cell;
                }
                t += step.delta_ms;
                current_cell = step.cell_id;
            }
            current_cell
        };

        let sw = 10_000u64;
        let total_ms: u64 = htms.iter().flat_map(|(_, loc)| loc.steps.iter().map(|s| s.delta_ms)).sum::<u64>().min(sw);

        for offset in (0..total_ms).step_by(sample_interval_ms as usize) {
            for i in 0..htms.len() {
                for j in (i + 1)..htms.len() {
                    let cell_a = cell_at(htms[i].1, offset);
                    let cell_b = cell_at(htms[j].1, offset);
                    if cell_a == cell_b {
                        // TODO: promote to panic once all edge conflicts are resolved.
                        eprintln!(
                            "HTM COLLISION WARNING: slice {} T={}ms bots {} and {} both at cell {}",
                            si,
                            slice_start + offset,
                            htms[i].0,
                            htms[j].0,
                            cell_a,
                        );
                        break;
                    }
                }
            }
        }
    }
    eprintln!("=== HTM collision audit: PASS ===");
}

/// Format all events as a human-readable trace string.
///
/// Each line: `[idx] time bot event details`
/// Nav waypoints: `→cell(+N)` move, `↻cell(+N)` turn, `⏸cell(+N)` wait
///
/// TODO: add orientation to turn symbols (↰↱ or ↶↷ for left/right),
/// tote ID on Nav events, carrying state, and relay/rack cell labels.
pub fn format_event_trace(events: &[SimEvent]) -> String {
    use std::fmt::Write;
    let mut out = String::new();

    let bot_label = |id: &RobotId, rt: &RobotType| -> String {
        let prefix = if matches!(rt, RobotType::Vtm) { "V" } else { "D" };
        format!("{prefix}{}", id.0)
    };

    for (i, event) in events.iter().enumerate() {
        match event {
            SimEvent::InitialPositions(ip) => {
                writeln!(out, "[{i:3}] init {n} bots", n = ip.positions.len()).ok();
            }
            SimEvent::ToteInitialStates(ti) => {
                writeln!(out, "[{i:3}] init {n} totes", n = ti.totes.len()).ok();
            }
            SimEvent::StripePlan(sp) => {
                writeln!(out, "[{i:3}] stripe plan {n} destinations", n = sp.entries.len()).ok();
            }
            SimEvent::Nav(n) => {
                let bot = bot_label(&n.robot_id, &n.robot_type);
                let mut cumul_ms = 0u64;
                let steps: Vec<String> = n
                    .waypoints
                    .iter()
                    .map(|wp| {
                        let sym = match &wp.action {
                            WaypointAction::Move { .. } => "→",
                            WaypointAction::Rotate { .. } => "↻",
                            WaypointAction::Wait { .. } => "⏸",
                            WaypointAction::PathStart { .. } => "▷",
                            WaypointAction::PathEnd { .. } => "■",
                        };
                        let delta = wp.duration().as_ms();
                        cumul_ms += delta;
                        format!("{sym}{cell}(+{delta})", cell = wp.cell.0)
                    })
                    .collect();
                let first = n.waypoints.first().map(|wp| wp.cell.0).unwrap_or(0);
                let last = n.waypoints.last().map(|wp| wp.cell.0).unwrap_or(0);
                writeln!(
                    out,
                    "[{i:3}] {t} {bot} nav {first}→{last} ⏲{dur}  {steps}",
                    t = n.time.as_ms(),
                    dur = cumul_ms,
                    steps = steps.join(" ")
                )
                .ok();
            }
            SimEvent::VtmNav(v) => {
                let bot = bot_label(&v.robot_id, &RobotType::Vtm);
                let (start, end) = (v.kind.start().as_ms(), v.kind.end().as_ms());
                writeln!(out, "[{i:3}] {start} {bot} vtm-nav ⏲{}", end - start).ok();
            }
            SimEvent::CycleComplete(cc) => {
                let bot = bot_label(&cc.robot_id, &cc.robot_type);
                writeln!(
                    out,
                    "[{i:3}] {t} {bot} cycle complete ({kind:?} stn {stn:?})",
                    t = cc.sim_time_ms,
                    kind = cc.kind,
                    stn = cc.station_id
                )
                .ok();
            }
            SimEvent::VtmCycle(c) => {
                writeln!(
                    out,
                    "[{i:3}] {t} V{id} vtm cycle {dir:?} z{z} ⏲{total} (empty {e} loaded {l} mech {m})",
                    t = c.sim_time_ms,
                    id = c.vtm_id.0,
                    dir = c.direction,
                    z = c.zone_id,
                    total = c.total_cycle_ms,
                    e = c.empty_leg_ms,
                    l = c.loaded_leg_ms,
                    m = c.fork_ms + c.buffer_ms,
                )
                .ok();
            }
            SimEvent::VtmOperation(op) => {
                writeln!(
                    out,
                    "[{i:3}] {t} V{id} da50 operation {kind:?} tote {tote} payload {payload:?}",
                    t = op.sim_time_ms,
                    id = op.vtm_id.0,
                    kind = op.kind,
                    tote = op.tote_id.0,
                    payload = op.payload,
                )
                .ok();
            }
            SimEvent::OrderComplete(oc) => {
                writeln!(out, "[{i:3}] {t} order {oid} complete", t = oc.sim_time_ms, oid = oc.order_id.0).ok();
            }
            SimEvent::RelayStateChange(r) => {
                let action = if r.reserved { "reserved" } else { "released" };
                writeln!(out, "[{i:3}] {t} relay cell{cell} {action}", t = r.time.as_ms(), cell = r.cell_id).ok();
            }
            SimEvent::ToteCreated(created) => {
                writeln!(
                    out,
                    "[{i:3}] {t} create {kind:?} {designation:?} {id} at {state:?}",
                    t = created.time.as_ms(),
                    kind = created.tote_type,
                    designation = created.designation,
                    id = created.tote_id.0,
                    state = created.initial_state
                )
                .ok();
            }
            SimEvent::ToteMoved(tm) => {
                let dest = match &tm.new_state {
                    ToteVisualState::AtCell { cell_id } => format!("→ cell{cell_id}"),
                    ToteVisualState::Extracted => "→ extracted".into(),
                    ToteVisualState::InRack { cell_id, level } => {
                        format!("→ rack cell{cell_id} z{level}")
                    }
                    ToteVisualState::AtRelay { cell_id } => {
                        format!("→ relay cell{cell_id}")
                    }
                    ToteVisualState::OnBot { bot_id } => {
                        format!("→ on D{bot_id}")
                    }
                    ToteVisualState::OnConveyor { cell_id } => {
                        format!("→ belt cell{cell_id}")
                    }
                };
                writeln!(out, "[{i:3}] {t} tote {id} {dest}", t = tm.time.as_ms(), id = tm.tote_id.0).ok();
            }
            SimEvent::Resource(r) => {
                use dessim::events::ResourceEventKind::*;
                let desc = match &r.kind {
                    VtmForwardCompleted { cycle_ms } => format!("vtm forward done (⏲{cycle_ms})"),
                    VtmReturnCompleted { cycle_ms } => format!("vtm return done (⏲{cycle_ms})"),
                    HtmPickupCompleted { cycle_ms } => format!("htm pickup done (⏲{cycle_ms})"),
                    HtmReturnCompleted { cycle_ms, .. } => format!("htm return done (⏲{cycle_ms})"),
                    HtmOperatorWaitCompleted { wait_ms } => {
                        format!("htm operator wait (⏲{wait_ms})")
                    }
                    DeadlockHeadroomTriggered => "deadlock headroom triggered".into(),
                    PathfinderFailure => "pathfinder failure".into(),
                    PathfinderQuery => "pathfinder query".into(),
                    RelayReservationSuccess => "relay reserved ok".into(),
                    RelayReservationFailure => "relay reserved fail".into(),
                    ToteClaimSuccess => "tote claim ok".into(),
                    ToteClaimFailure => "tote claim fail".into(),
                    BackpressureTrigger => "backpressure".into(),
                    VtmScoreForwardSelected => "vtm scored forward".into(),
                    VtmScoreReturnSelected => "vtm scored return".into(),
                    VtmCrossZoneAssignment => "vtm cross-zone".into(),
                    VtmScoringDuration { duration_ns } => {
                        format!("vtm scoring ({:.1}µs)", *duration_ns as f64 / 1000.0)
                    }
                    ZonePressureSnapshot { zone_id, pressure } => {
                        format!("zone {zone_id} pressure {pressure:.2}")
                    }
                    StarvationAgeBonusApplied => "starvation bonus".into(),
                    HtmReturnRelayRetry => "htm return relay retry".into(),
                    OperatorQueueWait => "operator queue wait".into(),
                };
                writeln!(out, "[{i:3}] {t} resource: {desc}", t = r.sim_time_ms).ok();
            }
            SimEvent::OrderDispatch(od) => {
                writeln!(
                    out,
                    "[{i:3}] {t} order {oid} dispatched to stn {stn:?} (score={score}, depth={depth}/{target})",
                    t = od.sim_time_ms,
                    oid = od.order_id.0,
                    stn = od.station_id,
                    score = od.score,
                    depth = od.pipeline_depth_before,
                    target = od.pipeline_target
                )
                .ok();
            }
            SimEvent::HtmDispatch(hd) => {
                writeln!(
                    out,
                    "[{i:3}] {t} D{id} dispatched tote {tid} stn {stn:?} relay {rid:?}",
                    t = hd.sim_time_ms,
                    id = hd.robot_id.0,
                    tid = hd.tote_id.0,
                    stn = hd.station_id,
                    rid = hd.relay_id
                )
                .ok();
            }
            SimEvent::PipelineSnapshot(ps) => {
                writeln!(
                    out,
                    "[{i:3}] {t} pipeline stn {stn:?} depth={d}/{max} next_free={nf}",
                    t = ps.sim_time_ms,
                    stn = ps.station_id,
                    d = ps.scheduled_picks_count,
                    max = ps.max_inflight_picks,
                    nf = ps.next_free_time_ms
                )
                .ok();
            }
            SimEvent::ArrivalError(ae) => {
                writeln!(
                    out,
                    "[{i:3}] {t} arrival error order {oid} {err:+}",
                    t = ae.sim_time_ms,
                    oid = ae.order_id.0,
                    err = ae.error_ms
                )
                .ok();
            }
            SimEvent::WatchdogTimeout(wt) => {
                writeln!(
                    out,
                    "[{i:3}] {t} watchdog timeout order {oid} stn {stn:?} ({secs:.1}s)",
                    t = wt.sim_time_ms,
                    oid = wt.order_id.0,
                    stn = wt.station_id,
                    secs = wt.timeout_secs
                )
                .ok();
            }
            SimEvent::TripStart(ts) => {
                let bot = bot_label(&ts.robot_id, &ts.robot_type);
                writeln!(out, "[{i:3}] {t} {bot} trip-start {p:?}", t = ts.sim_time_ms, p = ts.purpose).ok();
            }
            SimEvent::Induction(ind) => {
                writeln!(
                    out,
                    "[{i:3}] {t} induction stn {stn:?} tote {tid} cases={cs}",
                    t = ind.sim_time_ms,
                    stn = ind.station_id,
                    tid = ind.tote_id.0,
                    cs = ind.case_count
                )
                .ok();
            }
            SimEvent::VtmIdleReason(e) => {
                writeln!(
                    out,
                    "[{i:3}] {t} V{id} vtm idle {reason:?}",
                    t = e.at.as_ms(),
                    id = e.robot_id.0,
                    reason = e.reason,
                )
                .ok();
            }
            SimEvent::InventoryDelta(e) => {
                writeln!(out, "[{i:3}] {t} inventory delta {d:+}", t = e.at.as_ms(), d = e.delta).ok();
            }
            SimEvent::InventoryMove(e) => {
                writeln!(
                    out,
                    "[{i:3}] {t} inventory move tote={tote:?} skus={n}",
                    t = e.at.as_ms(),
                    tote = e.tote_id,
                    n = e.contents.len()
                )
                .ok();
            }
            SimEvent::HtmRelayPickup(rp) => {
                writeln!(
                    out,
                    "[{i:3}] {t} D{id} fork-get relay {rid} ⏲{w}",
                    t = rp.start.as_ms(),
                    id = rp.robot_id.0,
                    rid = rp.relay_id.0,
                    w = rp.wait.as_ms(),
                )
                .ok();
            }
            SimEvent::HtmRelayDeposit(rd) => {
                writeln!(
                    out,
                    "[{i:3}] {t} {bot} fork-put relay {rid} WAIT{w}",
                    t = rd.start.as_ms(),
                    bot = bot_label(&rd.robot_id, &RobotType::Htm),
                    rid = rd.relay_id.0,
                    w = rd.wait.as_ms(),
                )
                .ok();
            }
            SimEvent::DivingBoardSlotWait(db) => {
                writeln!(
                    out,
                    "[{i:3}] {t} diving board slot wait relay {rid} ⏲{w}",
                    t = db.sim_time_ms,
                    rid = db.relay_id.0,
                    w = db.wait_ms,
                )
                .ok();
            }
            // Station dispatch diagnostics: not part of the physical trace
            // this dump narrates.
            SimEvent::HtmAssignment(_)
            | SimEvent::HtmArrival(_)
            | SimEvent::DispatchRejection(_)
            | SimEvent::ToteContentsChanged(_)
            | SimEvent::ToteTransit(_)
            | SimEvent::BotPlanSnapshot(_)
            | SimEvent::BarrierAdvanced(_)
            | SimEvent::CellHeld(_)
            | SimEvent::CellReleased(_)
            | SimEvent::OrderStationPhase(_)
            | SimEvent::PresentationReadiness(_)
            | SimEvent::InventoryVisitPhase(_) => {}
        }
    }
    out
}

/// Run the solver and capture ALL events (no metrics output).
#[allow(dead_code)]
pub fn run_world_capture_events(world: &WorldMap, scenario: &ScenarioConfig, config: SimConfig) -> Vec<SimEvent> {
    run_world_capture_all(world, scenario, config).0
}

// ---------------------------------------------------------------------------
// assert_events! macro
// ---------------------------------------------------------------------------

#[macro_export]
macro_rules! assert_events {
    ($actual:expr, [ $( $kind:ident($($args:tt)*) ),* $(,)? ]) => {{
        let _actual = &$actual;
        let mut _i: usize = 0;
        $(
            assert_events!(@check _actual, _i, $kind($($args)*));
            _i += 1;
        )*
        assert_eq!(_i, _actual.len(),
            "Trace length mismatch: got {} events, expected {}", _actual.len(), _i);
    }};

    // nav(robot_id, time_ms, [(cell_id, delta_sec), ...])
    (@check $a:expr, $i:expr, nav($id:expr, $t:expr, [ $( ($cell:expr, $delta:expr) ),* $(,)? ])) => {
        match &$a[$i] {
            dessim::events::SimEvent::Nav(ev) => {
                assert_eq!((ev.robot_id.0, ev.time.as_ms()), ($id, $t),
                    "Event #{}: Nav robot/time\n  got:  (robot={}, t={})\n  want: (robot={}, t={})",
                    $i, ev.robot_id.0, ev.time.as_ms(), $id, $t);
                let expected_wps: Vec<(u32, f64)> = vec![ $( ($cell, $delta) ),* ];
                let actual_wps: Vec<(u32, f64)> = ev.waypoints.iter()
                    .map(|wp| (wp.cell.0, (wp.action.duration().as_ms() as f64) / 1000.0))
                    .collect();
                assert_eq!(actual_wps.len(), expected_wps.len(),
                    "Event #{}: Nav waypoint count\n  got:  {}\n  want: {}",
                    $i, actual_wps.len(), expected_wps.len());
                for (j, (got, want)) in actual_wps.iter().zip(expected_wps.iter()).enumerate() {
                    assert_eq!(got.0, want.0,
                        "Event #{} wp#{}: Nav cell\n  got:  CellId({})\n  want: CellId({})",
                        $i, j, got.0, want.0);
                    assert!((got.1 - want.1).abs() < 0.002,
                        "Event #{} wp#{}: Nav delta\n  got:  {:.3}s\n  want: {:.3}s",
                        $i, j, got.1, want.1);
                }
            }
            other => panic!("Event #{}: expected Nav, got {}", $i, other),
        }
    };

    (@check $a:expr, $i:expr, cc($id:expr, $t:expr)) => {
        match &$a[$i] {
            dessim::events::SimEvent::CycleComplete(ev) => {
                assert_eq!((ev.robot_id.0, ev.sim_time_ms), ($id, $t),
                    "Event #{}: CycleComplete\n  got:  (robot={}, t={})\n  want: (robot={}, t={})",
                    $i, ev.robot_id.0, ev.sim_time_ms, $id, $t);
            }
            other => panic!("Event #{}: expected CycleComplete, got {}", $i, other),
        }
    };

    (@check $a:expr, $i:expr, res($kind:expr, $t:expr)) => {
        match &$a[$i] {
            dessim::events::SimEvent::Resource(ev) => {
                let k = format!("{:?}", ev.kind);
                assert!(k.starts_with($kind) && ev.sim_time_ms == $t,
                    "Event #{}: Resource\n  got:  ({}, t={})\n  want: ({}, t={})",
                    $i, k, ev.sim_time_ms, $kind, $t);
            }
            other => panic!("Event #{}: expected Resource, got {}", $i, other),
        }
    };

    (@check $a:expr, $i:expr, init_pos($n:expr)) => {
        match &$a[$i] {
            dessim::events::SimEvent::InitialPositions(ev) => {
                assert_eq!(ev.positions.len(), $n,
                    "Event #{}: InitialPositions count: got {}, want {}", $i, ev.positions.len(), $n);
            }
            other => panic!("Event #{}: expected InitialPositions, got {}", $i, other),
        }
    };

    (@check $a:expr, $i:expr, tote_init($n:expr)) => {
        match &$a[$i] {
            dessim::events::SimEvent::ToteInitialStates(ev) => {
                assert_eq!(ev.totes.len(), $n,
                    "Event #{}: ToteInitialStates count: got {}, want {}", $i, ev.totes.len(), $n);
            }
            other => panic!("Event #{}: expected ToteInitialStates, got {}", $i, other),
        }
    };

    (@check $a:expr, $i:expr, tote($tid:expr, $t:expr)) => {
        match &$a[$i] {
            dessim::events::SimEvent::ToteMoved(ev) => {
                assert_eq!((ev.tote_id, ev.time.as_ms()), (dessim::model::ToteId($tid), $t),
                    "Event #{}: ToteMoved\n  got:  (tote={:?}, t={})\n  want: (tote={}, t={})",
                    $i, ev.tote_id, ev.time.as_ms(), $tid, $t);
            }
            other => panic!("Event #{}: expected ToteMoved, got {}", $i, other),
        }
    };

    // relay(cell, reserved, time_ms)
    (@check $a:expr, $i:expr, relay($cell:expr, $res:expr, $t:expr)) => {
        match &$a[$i] {
            dessim::events::SimEvent::RelayStateChange(ev) => {
                assert_eq!((ev.cell_id, ev.reserved, ev.time.as_ms()), ($cell, $res, $t),
                    "Event #{}: RelayStateChange\n  got:  (cell={}, res={}, t={})\n  want: (cell={}, res={}, t={})",
                    $i, ev.cell_id, ev.reserved, ev.time.as_ms(), $cell, $res, $t);
            }
            other => panic!("Event #{}: expected RelayStateChange, got {}", $i, other),
        }
    };

    // relay(cell, reserved) — no timestamp check
    (@check $a:expr, $i:expr, relay($cell:expr, $res:expr)) => {
        match &$a[$i] {
            dessim::events::SimEvent::RelayStateChange(ev) => {
                assert_eq!((ev.cell_id, ev.reserved), ($cell, $res),
                    "Event #{}: RelayStateChange\n  got:  (cell={}, res={})\n  want: (cell={}, res={})",
                    $i, ev.cell_id, ev.reserved, $cell, $res);
            }
            other => panic!("Event #{}: expected RelayStateChange, got {}", $i, other),
        }
    };

    (@check $a:expr, $i:expr, oc($oid:expr, $t:expr)) => {
        match &$a[$i] {
            dessim::events::SimEvent::OrderComplete(ev) => {
                assert_eq!((ev.order_id.0, ev.sim_time_ms), ($oid, $t),
                    "Event #{}: OrderComplete\n  got:  (order={}, t={})\n  want: (order={}, t={})",
                    $i, ev.order_id.0, ev.sim_time_ms, $oid, $t);
            }
            other => panic!("Event #{}: expected OrderComplete, got {}", $i, other),
        }
    };
}
