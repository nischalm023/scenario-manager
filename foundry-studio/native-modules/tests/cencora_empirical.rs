//! Temporary empirical diagnostics for the large Cencora fixture.
//!
//! This is intentionally print-heavy and assertion-light while we investigate
//! whether relay-slot growth is caused by stalled order pick, stalled putaway,
//! visual-state drift, or actual relay reservation buildup.

mod common;

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;

use desgraph::{CellId, RelayId, RobotType, StationId, StationType, ZoneId};
use dessim::events::{CycleKind, EventSink, ForkOp, NavEventData, SimEvent, VtmEndpoint, VtmNavKind};
use dessim::model::{OrderSpec, SkuId};
use dessim::observer::time_slice::ToteVisualState;
use dessim::pathfinding::WaypointAction;
use dessim::runner::RunnerHooks;
use native_modules as _;

#[test]
fn cencora_relay_state_empirical_summary() {
    let (world, scenario, mut config) = common::load_world_spec("cencora_phase2_spec.json");
    std::env::set_current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")).expect("chdir workspace root");

    config.wall_timeout_secs = Some(180.0);

    let mut data =
        dessim::runner::resolve_scenario_with_racking(world.map.clone(), world.racking.clone(), &scenario, &config)
            .expect("scenario gen");
    let pathfinder = common::fh_sipp_for_map(&data.map, &config);
    let orders_for_readiness = data.orders.clone();
    // Flat (sku, qty) prefix used for "orders ready after N putaways" diagnostics.
    // Mirrors the spirit of the old build_arrival_queue without depending on the
    // (now station-aware) putaway feeder. Order: walk orders in id order and emit
    // one unit per pick.
    let arrival_prefix: Vec<(SkuId, u32)> = data
        .orders
        .iter()
        .flat_map(|order| order.lines.iter().flat_map(|line| (0..line.quantity).map(move |_| (line.sku, 1u32))))
        .take(2_000)
        .collect();

    let station_cell: BTreeMap<CellId, (StationId, StationType)> =
        data.station_data.iter().map(|s| (s.presentation_cell, (s.station_id, s.station_type))).collect();

    let mut relay_zone: BTreeMap<RelayId, ZoneId> = BTreeMap::new();
    for zone in &data.zone_data {
        for &relay_id in &zone.relay_ids {
            relay_zone.insert(relay_id, zone.zone_id);
        }
    }
    let relay_cell: BTreeMap<CellId, RelayId> = data.relay_data.iter().map(|r| (r.htm_access, r.relay_id)).collect();

    let (sub_tx, sub_rx) = mpsc::channel::<SimEvent>();
    let capture = std::thread::spawn(move || sub_rx.iter().collect::<Vec<_>>());
    let (event_tx, event_rx) = mpsc::channel();
    let sink: Arc<dyn EventSink> = Arc::new(event_tx.clone());
    let result = dessim::composition::build(&mut data, &config, pathfinder, Some(sink))
        .map_err(|e| e.to_string())
        .and_then(|(scheduler, model, bias)| {
            dessim::runner::run(
                scheduler,
                model,
                &data,
                config,
                event_tx,
                event_rx,
                bias,
                RunnerHooks { live: None, subscribers: vec![sub_tx], recording: None },
            )
        });
    let events = capture.join().unwrap();

    eprintln!("\n=== Cencora empirical summary ===");
    let failure_time_ms = match &result {
        Ok((sim, _metrics)) => Some(sim.sim_duration.as_millis() as u64),
        Err(err) => parse_failure_depart_ms(err),
    };
    match &result {
        Ok((sim, _metrics)) => eprintln!(
            "solver_result=Ok sim_time={}ms picks={} orders={}",
            sim.sim_duration.as_millis(),
            sim.completed_order_picks,
            sim.total_orders_completed
        ),
        Err(err) => eprintln!("solver_result=Err {err}"),
    }
    eprintln!("sample_time_ms={failure_time_ms:?}");
    eprintln!("events={}", events.len());

    let mut putaway_cycles = 0usize;
    let mut pick_cycles = 0usize;
    let mut first_putaway = None;
    let mut first_pick = None;
    let mut last_putaway = None;
    let mut last_pick = None;
    let mut putaway_by_station: BTreeMap<u32, usize> = BTreeMap::new();
    let mut pick_by_station: BTreeMap<u32, usize> = BTreeMap::new();
    let mut order_dispatches = 0usize;
    let mut htm_dispatches = 0usize;
    let mut order_completes = 0usize;
    let mut last_order_dispatch = None;
    let mut last_order_complete = None;
    let mut vtm_forward = 0usize;
    let mut vtm_return = 0usize;
    let mut last_vtm_forward = None;
    let mut last_vtm_return = None;

    let mut visually_reserved: BTreeSet<CellId> = BTreeSet::new();
    let mut last_relay_change = None;
    let mut current_tote_at_relay: BTreeMap<u32, CellId> = BTreeMap::new();

    for event in &events {
        match event {
            // CycleKind is authoritative for pick vs putaway classification.
            // This is a diagnostic surface: if a Pick cycle ever emits at a
            // Putaway-typed station (data-integrity bug), it should be
            // visible in the pick/putaway split, not silently smoothed over
            // by a station-type fallback.
            SimEvent::CycleComplete(c) => match c.kind {
                CycleKind::Putaway { .. } => {
                    putaway_cycles += 1;
                    first_putaway = first_putaway.or(Some(c.sim_time_ms));
                    last_putaway = Some(c.sim_time_ms);
                    *putaway_by_station.entry(c.station_id.0).or_default() += 1;
                }
                CycleKind::Pick { .. } => {
                    pick_cycles += 1;
                    first_pick = first_pick.or(Some(c.sim_time_ms));
                    last_pick = Some(c.sim_time_ms);
                    *pick_by_station.entry(c.station_id.0).or_default() += 1;
                }
                // cencora runs no order-tote flow.
                CycleKind::OrderLoad { .. } | CycleKind::OrderExtract { .. } => {}
            },
            SimEvent::OrderDispatch(o) => {
                order_dispatches += 1;
                last_order_dispatch = Some(o.sim_time_ms);
            }
            SimEvent::HtmDispatch(h) => {
                htm_dispatches += 1;
                last_order_dispatch = Some(h.sim_time_ms);
            }
            SimEvent::OrderComplete(o) => {
                order_completes += 1;
                last_order_complete = Some(o.sim_time_ms);
            }
            SimEvent::RelayStateChange(r) => {
                let cell = CellId(r.cell_id);
                if r.reserved {
                    visually_reserved.insert(cell);
                } else {
                    visually_reserved.remove(&cell);
                }
                last_relay_change = Some(r.time.as_ms());
            }
            SimEvent::ToteMoved(t) => match &t.new_state {
                ToteVisualState::AtRelay { cell_id } => {
                    current_tote_at_relay.insert(t.tote_id.0, CellId(*cell_id));
                }
                _ => {
                    current_tote_at_relay.remove(&t.tote_id.0);
                }
            },
            SimEvent::VtmNav(v) => match &v.kind {
                VtmNavKind::Fork { op: ForkOp::Put, at: VtmEndpoint::Relay { .. }, end, .. } => {
                    vtm_forward += 1;
                    last_vtm_forward = Some(end.as_ms());
                }
                VtmNavKind::Fork { op: ForkOp::Get, at: VtmEndpoint::Relay { .. }, end, .. } => {
                    vtm_return += 1;
                    last_vtm_return = Some(end.as_ms());
                }
                _ => {}
            },
            _ => {}
        }
    }

    eprintln!(
        "orders: dispatches={order_dispatches} last_dispatch={last_order_dispatch:?} completes={order_completes} last_complete={last_order_complete:?}"
    );
    eprintln!(
        "cycles: putaway={putaway_cycles} first={first_putaway:?} last={last_putaway:?} pick={pick_cycles} first={first_pick:?} last={last_pick:?}"
    );
    eprintln!("putaway_by_station={putaway_by_station:?}");
    eprintln!("pick_by_station={pick_by_station:?}");
    eprintln!(
        "orders_ready_if_arrival_prefix_applied={} after_putaways={putaway_cycles}",
        orders_ready_after_arrivals(&orders_for_readiness, &arrival_prefix, putaway_cycles)
    );
    eprintln!("htm_dispatches={htm_dispatches}");
    eprintln!(
        "vtm: forward_puts={vtm_forward} last={last_vtm_forward:?} return_gets={vtm_return} last={last_vtm_return:?}"
    );
    print_vtm_motion_summary(&events);
    eprintln!(
        "relay_visual: reserved_cells={} last_change={last_relay_change:?} totes_currently_at_relay={}",
        visually_reserved.len(),
        current_tote_at_relay.len()
    );

    let mut visual_by_zone: BTreeMap<u32, usize> = BTreeMap::new();
    for cell in &visually_reserved {
        if let Some(relay_id) = relay_cell.get(cell)
            && let Some(zone_id) = relay_zone.get(relay_id)
        {
            *visual_by_zone.entry(zone_id.0).or_default() += 1;
        }
    }
    eprintln!("relay_visual_reserved_by_zone={visual_by_zone:?}");

    let mut tote_at_relay_by_zone: BTreeMap<u32, usize> = BTreeMap::new();
    for cell in current_tote_at_relay.values() {
        if let Some(relay_id) = relay_cell.get(cell)
            && let Some(zone_id) = relay_zone.get(relay_id)
        {
            *tote_at_relay_by_zone.entry(zone_id.0).or_default() += 1;
        }
    }
    eprintln!("totes_at_relay_by_zone={tote_at_relay_by_zone:?}");

    if let Some(sample_time_ms) = failure_time_ms {
        let htm_at_relay = htm_relay_access_occupancy_at(&events, sample_time_ms, &relay_cell, &relay_zone);
        let mut htm_by_zone: BTreeMap<u32, usize> = BTreeMap::new();
        for (_, _, _, zone_id) in &htm_at_relay {
            *htm_by_zone.entry(zone_id.0).or_default() += 1;
        }
        eprintln!("htm_at_relay_access_by_zone@{sample_time_ms}={htm_by_zone:?}");
        eprintln!(
            "zone11_htm_at_relay_access@{sample_time_ms}={:?}",
            htm_at_relay.iter().filter(|(_, _, _, z)| z.0 == 11).collect::<Vec<_>>()
        );
    }
    print_terminal_hold_summary(&events, failure_time_ms, &station_cell, &relay_cell, &relay_zone);

    // This test is diagnostic-only for now. Keep it green so it can be used as
    // a repeatable probe while the cencora scenario is still expected to fail.
}

fn print_vtm_motion_summary(events: &[SimEvent]) {
    let mut rail_moves = Vec::new();
    let mut cycle_phases_by_robot: BTreeMap<u32, Vec<(u64, u64)>> = BTreeMap::new();

    for event in events {
        let SimEvent::VtmNav(vtm) = event else { continue };
        let start_ms = vtm.kind.start().as_ms();
        let end_ms = vtm.kind.end().as_ms();
        cycle_phases_by_robot.entry(vtm.robot_id.0).or_default().push((start_ms, end_ms));

        if let VtmNavKind::RailMove { distance_mm, .. } = &vtm.kind {
            let duration_ms = end_ms.saturating_sub(start_ms);
            if duration_ms > 0 {
                rail_moves.push((*distance_mm, duration_ms));
            }
        }
    }

    let mut cycles = Vec::new();
    for phases in cycle_phases_by_robot.values_mut() {
        phases.sort_unstable();
        let mut current_start = None;
        let mut current_end = 0;
        let mut count = 0usize;
        for &(start, end) in phases.iter() {
            let starts_new = current_start.is_none() || start != current_end || count >= 4;
            if starts_new {
                if let Some(cycle_start) = current_start {
                    cycles.push(current_end.saturating_sub(cycle_start));
                }
                current_start = Some(start);
                count = 1;
            } else {
                count += 1;
            }
            current_end = end;
        }
        if let Some(cycle_start) = current_start {
            cycles.push(current_end.saturating_sub(cycle_start));
        }
    }

    if rail_moves.is_empty() {
        eprintln!("vtm_motion: no rail moves");
        return;
    }

    rail_moves.sort_by_key(|&(distance, duration)| (distance, duration));
    cycles.sort_unstable();

    let count = rail_moves.len();
    let total_distance: u64 = rail_moves.iter().map(|(d, _)| *d).sum();
    let total_duration: u64 = rail_moves.iter().map(|(_, d)| *d).sum();
    let speed_mps = if total_duration == 0 { 0.0 } else { total_distance as f64 / total_duration as f64 };
    let p95_idx = ((count as f64 * 0.95).ceil() as usize).saturating_sub(1).min(count - 1);
    let max_idx = count - 1;

    let min = rail_moves[0];
    let p95 = rail_moves[p95_idx];
    let max = rail_moves[max_idx];
    let mean_distance = total_distance as f64 / count as f64;
    let mean_duration = total_duration as f64 / count as f64;

    eprintln!(
        "vtm_motion: rail_moves={} dist_mm min/mean/p95/max={}/{:.0}/{}/{} dur_ms min/mean/p95/max={}/{:.0}/{}/{} implied_mean_mps={:.2}",
        count, min.0, mean_distance, p95.0, max.0, min.1, mean_duration, p95.1, max.1, speed_mps
    );

    if !cycles.is_empty() {
        let cycle_mean = cycles.iter().sum::<u64>() as f64 / cycles.len() as f64;
        let cycle_p95 = cycles[((cycles.len() as f64 * 0.95).ceil() as usize).saturating_sub(1).min(cycles.len() - 1)];
        eprintln!(
            "vtm_motion: cycles={} cycle_ms min/mean/p95/max={}/{:.0}/{}/{}",
            cycles.len(),
            cycles[0],
            cycle_mean,
            cycle_p95,
            cycles[cycles.len() - 1]
        );
    }
}

fn orders_ready_after_arrivals(orders: &[OrderSpec], arrival_prefix: &[(SkuId, u32)], count: usize) -> usize {
    let mut available: BTreeMap<SkuId, u32> = BTreeMap::new();
    for &(sku, qty) in arrival_prefix.iter().take(count) {
        *available.entry(sku).or_default() += qty;
    }

    orders
        .iter()
        .filter(|order| {
            let mut demand: BTreeMap<SkuId, u32> = BTreeMap::new();
            for line in &order.lines {
                *demand.entry(line.sku).or_default() += line.quantity;
            }
            demand.iter().all(|(sku, qty)| available.get(sku).copied().unwrap_or(0) >= *qty)
        })
        .count()
}

fn parse_failure_depart_ms(err: &str) -> Option<u64> {
    let (_, tail) = err.split_once(" at ")?;
    let (secs, _) = tail.split_once("s:")?;
    let secs = secs.parse::<f64>().ok()?;
    Some((secs * 1000.0).round() as u64)
}

fn htm_relay_access_occupancy_at(
    events: &[SimEvent],
    sample_time_ms: u64,
    relay_cell: &BTreeMap<CellId, RelayId>,
    relay_zone: &BTreeMap<RelayId, ZoneId>,
) -> Vec<(u32, CellId, RelayId, ZoneId)> {
    let mut changes_by_robot: BTreeMap<u32, Vec<(u64, usize, CellId)>> = BTreeMap::new();
    let mut seq = 0usize;

    for event in events {
        match event {
            SimEvent::InitialPositions(init) => {
                for (robot, cell, robot_type, _) in &init.positions {
                    if matches!(robot_type, RobotType::Htm) {
                        changes_by_robot.entry(robot.0).or_default().push((0, seq, *cell));
                        seq += 1;
                    }
                }
            }
            SimEvent::Nav(nav) if matches!(nav.robot_type, RobotType::Htm) => {
                for wp in &nav.waypoints {
                    let time = match wp.action {
                        WaypointAction::PathStart { time } => Some(time.as_ms()),
                        WaypointAction::Move { end, .. } => Some(end.as_ms()),
                        _ => None,
                    };
                    if let Some(time) = time {
                        changes_by_robot.entry(nav.robot_id.0).or_default().push((time, seq, wp.cell));
                        seq += 1;
                    }
                }
            }
            _ => {}
        }
    }

    let mut out = Vec::new();
    for (robot, mut changes) in changes_by_robot {
        changes.sort_by_key(|&(time, seq, _)| (time, seq));
        let cell_at_sample =
            changes.iter().take_while(|(time, _, _)| *time <= sample_time_ms).last().map(|(_, _, cell)| *cell);
        if let Some(cell) = cell_at_sample
            && let Some(&relay_id) = relay_cell.get(&cell)
            && let Some(&zone_id) = relay_zone.get(&relay_id)
        {
            out.push((robot, cell, relay_id, zone_id));
        }
    }
    out.sort_by_key(|(_, cell, relay, zone)| (zone.0, relay.0, cell.0));
    out
}

#[derive(Clone, Debug)]
struct TerminalHold {
    robot: u32,
    cell: CellId,
    start_ms: u64,
    end_ms: Option<u64>,
}

fn print_terminal_hold_summary(
    events: &[SimEvent],
    sample_time_ms: Option<u64>,
    station_cell: &BTreeMap<CellId, (StationId, StationType)>,
    relay_cell: &BTreeMap<CellId, RelayId>,
    relay_zone: &BTreeMap<RelayId, ZoneId>,
) {
    let holds = htm_terminal_holds(events);
    if holds.is_empty() {
        eprintln!("terminal_holds: none");
        return;
    }

    let sample_ms = sample_time_ms.unwrap_or_else(|| {
        holds.iter().filter_map(|h| h.end_ms).chain(holds.iter().map(|h| h.start_ms)).max().unwrap_or(0)
    });

    let mut station_at_sample: BTreeMap<u32, usize> = BTreeMap::new();
    let mut relay_at_sample: BTreeMap<u32, usize> = BTreeMap::new();
    let mut relay_hold_by_zone: BTreeMap<u32, Vec<&TerminalHold>> = BTreeMap::new();
    let mut station_hold_by_id: BTreeMap<u32, Vec<&TerminalHold>> = BTreeMap::new();
    let mut relay_hold_by_cell: BTreeMap<CellId, Vec<&TerminalHold>> = BTreeMap::new();
    let mut station_hold_by_cell: BTreeMap<CellId, Vec<&TerminalHold>> = BTreeMap::new();

    for hold in &holds {
        if let Some((station_id, _)) = station_cell.get(&hold.cell) {
            station_hold_by_id.entry(station_id.0).or_default().push(hold);
            station_hold_by_cell.entry(hold.cell).or_default().push(hold);
            if hold_overlaps(hold, sample_ms) {
                *station_at_sample.entry(station_id.0).or_default() += 1;
            }
        }
        if let Some(relay_id) = relay_cell.get(&hold.cell)
            && let Some(zone_id) = relay_zone.get(relay_id)
        {
            relay_hold_by_zone.entry(zone_id.0).or_default().push(hold);
            relay_hold_by_cell.entry(hold.cell).or_default().push(hold);
            if hold_overlaps(hold, sample_ms) {
                *relay_at_sample.entry(zone_id.0).or_default() += 1;
            }
        }
    }

    eprintln!("\n=== Terminal HTM Hold Summary (offline planned intervals) ===");
    eprintln!("sample_ms={sample_ms}");
    eprintln!("station_terminal_holds_at_sample={station_at_sample:?}");
    eprintln!("relay_terminal_holds_at_sample_by_zone={relay_at_sample:?}");

    eprintln!("top_station_cells_by_terminal_hold_depth:");
    for (cell, holds) in top_cells_by_depth(&station_hold_by_cell, 10) {
        let (station_id, station_type) = station_cell.get(&cell).copied().unwrap();
        eprintln!(
            "  stn={:?} type={:?} cell={:?} at_sample={} max_depth={} holds={} longest={}ms",
            station_id,
            station_type,
            cell,
            holds.iter().filter(|h| hold_overlaps(h, sample_ms)).count(),
            max_overlap(holds.iter().copied()),
            holds.len(),
            longest_hold_ms(holds.iter().copied(), sample_ms),
        );
    }

    eprintln!("top_relay_cells_by_terminal_hold_depth:");
    for (cell, holds) in top_cells_by_depth(&relay_hold_by_cell, 15) {
        let relay_id = relay_cell.get(&cell).copied().unwrap();
        let zone_id = relay_zone.get(&relay_id).copied().unwrap();
        eprintln!(
            "  zone={:?} relay={:?} cell={:?} at_sample={} max_depth={} holds={} longest={}ms",
            zone_id,
            relay_id,
            cell,
            holds.iter().filter(|h| hold_overlaps(h, sample_ms)).count(),
            max_overlap(holds.iter().copied()),
            holds.len(),
            longest_hold_ms(holds.iter().copied(), sample_ms),
        );
    }

    eprintln!("top_relay_zones_by_terminal_holds_at_sample:");
    let mut relay_zones: Vec<_> = relay_hold_by_zone
        .iter()
        .map(|(zone, holds)| {
            let at_sample = holds.iter().filter(|h| hold_overlaps(h, sample_ms)).count();
            let max_cell_depth = holds
                .iter()
                .fold(BTreeMap::<CellId, Vec<&TerminalHold>>::new(), |mut by_cell, h| {
                    by_cell.entry(h.cell).or_default().push(*h);
                    by_cell
                })
                .values()
                .map(|cell_holds| max_overlap(cell_holds.iter().copied()))
                .max()
                .unwrap_or(0);
            (*zone, at_sample, holds.len(), max_cell_depth)
        })
        .collect();
    relay_zones.sort_by_key(|&(zone, at_sample, holds, max_cell_depth)| {
        (Reverse(at_sample), Reverse(max_cell_depth), Reverse(holds), zone)
    });
    for (zone, at_sample, holds, max_cell_depth) in relay_zones.into_iter().take(12) {
        eprintln!("  zone={zone} at_sample={at_sample} holds={holds} max_cell_depth={max_cell_depth}");
    }

    eprintln!("top_long_relay_terminal_holds_crossing_sample:");
    let mut long_crossing: Vec<_> = holds
        .iter()
        .filter(|h| hold_overlaps(h, sample_ms))
        .filter_map(|h| {
            let relay_id = relay_cell.get(&h.cell)?;
            let zone_id = relay_zone.get(relay_id)?;
            Some((hold_duration_ms(h, sample_ms), h, *relay_id, *zone_id))
        })
        .collect();
    long_crossing.sort_by_key(|(dur, h, relay, zone)| (Reverse(*dur), zone.0, relay.0, h.cell.0, h.robot));
    for (duration, hold, relay_id, zone_id) in long_crossing.into_iter().take(15) {
        eprintln!(
            "  zone={:?} relay={:?} cell={:?} robot=r({}) start={} end={:?} duration={}ms",
            zone_id, relay_id, hold.cell, hold.robot, hold.start_ms, hold.end_ms, duration
        );
    }
}

fn htm_terminal_holds(events: &[SimEvent]) -> Vec<TerminalHold> {
    let mut navs_by_robot: BTreeMap<u32, Vec<(u64, usize, CellId, u64, Option<u64>)>> = BTreeMap::new();
    let mut seq = 0usize;

    for event in events {
        let SimEvent::Nav(nav) = event else { continue };
        if !matches!(nav.robot_type, RobotType::Htm) {
            continue;
        }
        let Some((terminal_cell, terminal_start)) = terminal_cell_and_start(nav) else {
            continue;
        };
        let first_move_end = nav.waypoints.iter().find_map(|wp| match wp.action {
            WaypointAction::Move { end, .. } => Some(end.as_ms()),
            _ => None,
        });
        navs_by_robot.entry(nav.robot_id.0).or_default().push((
            nav.time.as_ms(),
            seq,
            terminal_cell,
            terminal_start,
            first_move_end,
        ));
        seq += 1;
    }

    let mut holds = Vec::new();
    for (robot, mut navs) in navs_by_robot {
        navs.sort_by_key(|&(time, seq, _, _, _)| (time, seq));
        for idx in 0..navs.len() {
            let (_, _, terminal_cell, terminal_start, _) = navs[idx];
            let end_ms = navs
                .iter()
                .skip(idx + 1)
                .find_map(|(_, _, _, _, first_move_end)| *first_move_end)
                .filter(|end| *end >= terminal_start);
            holds.push(TerminalHold { robot, cell: terminal_cell, start_ms: terminal_start, end_ms });
        }
    }
    holds
}

fn terminal_cell_and_start(nav: &NavEventData) -> Option<(CellId, u64)> {
    nav.waypoints.iter().rev().find_map(|wp| match wp.action {
        WaypointAction::PathEnd { time } => Some((wp.cell, time.as_ms())),
        _ => None,
    })
}

fn hold_overlaps(hold: &TerminalHold, t: u64) -> bool {
    hold.start_ms <= t && hold.end_ms.map(|end| t < end).unwrap_or(true)
}

fn hold_duration_ms(hold: &TerminalHold, fallback_end: u64) -> u64 {
    hold.end_ms.unwrap_or(fallback_end).saturating_sub(hold.start_ms)
}

fn longest_hold_ms<'a>(holds: impl Iterator<Item = &'a TerminalHold>, fallback_end: u64) -> u64 {
    holds.map(|h| hold_duration_ms(h, fallback_end)).max().unwrap_or(0)
}

fn max_overlap<'a>(holds: impl Iterator<Item = &'a TerminalHold>) -> usize {
    let mut events = Vec::new();
    for hold in holds {
        events.push((hold.start_ms, 1i32));
        if let Some(end) = hold.end_ms {
            events.push((end, -1i32));
        }
    }
    events.sort_by_key(|&(t, delta)| (t, delta));
    let mut depth = 0i32;
    let mut max_depth = 0i32;
    for (_, delta) in events {
        depth += delta;
        max_depth = max_depth.max(depth);
    }
    max_depth.max(0) as usize
}

fn top_cells_by_depth<'a>(
    by_cell: &'a BTreeMap<CellId, Vec<&'a TerminalHold>>,
    limit: usize,
) -> Vec<(CellId, Vec<&'a TerminalHold>)> {
    let mut rows: Vec<_> = by_cell.iter().map(|(&cell, holds)| (cell, holds.clone())).collect();
    rows.sort_by_key(|(cell, holds)| (Reverse(max_overlap(holds.iter().copied())), Reverse(holds.len()), cell.0));
    rows.truncate(limit);
    rows
}
