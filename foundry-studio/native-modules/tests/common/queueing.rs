//! Completed conveyor visits measured over explicit simulated-time windows.
//! Calendar integrals are checked against the same service's separate visit
//! history, and complete visit custody against InventoryService. The middle
//! window includes carry-in and carry-out, not just completed visits.

use std::env;
use std::fs;
use std::path::PathBuf;

use dessim::model::conveyance::{Conveyance, Conveyances, Occupancy, TotePhase};
use dessim::model::navigation::Navigation;
use dessim::model::{InventoryService, SimDuration, SimTime, ToteLocation};
use serde::Serialize;
use serde_json::json;

#[derive(Debug, Serialize)]
pub struct VisitTiming {
    pub inventory_tote: u32,
    pub admitted_at_ms: u64,
    pub boarded_at_ms: u64,
    pub pick_start_ms: u64,
    pub pick_end_ms: u64,
    pub collected_at_ms: u64,
    pub station_id: u32,
    pub order_tote: u32,
    pub order_robot: u32,
    pub collector_robot: u32,
}

#[derive(Debug, Serialize)]
pub struct WindowMeasurement {
    pub start_ms: u64,
    pub end_ms: u64,
    pub wip_at_start: usize,
    pub wip_at_end: usize,
    /// Transition counts use (start, end], matching the endpoint WIP values.
    pub boardings: usize,
    pub collections: usize,
    pub physical_area_tote_ms: u64,
    pub clipped_visit_tote_ms: u64,
    pub mean_physical_wip: f64,
    pub operator_busy_ms: u64,
    pub operator_utilization: f64,
}

#[derive(Debug, Serialize)]
pub struct ConveyorMeasurement {
    pub visits: Vec<VisitTiming>,
    pub mean_boarded_to_collected_secs: f64,
    pub mean_boarded_to_pick_start_secs: f64,
    pub mean_pick_end_to_collected_secs: f64,
    pub full_cohort_little_wip: f64,
    pub full: WindowMeasurement,
    pub middle: WindowMeasurement,
}

fn clipped(start: u64, end: u64, from: u64, until: u64) -> u64 {
    end.min(until).saturating_sub(start.max(from))
}

pub(super) fn window(machine: &Conveyance, visits: &[VisitTiming], from: u64, until: u64) -> WindowMeasurement {
    assert!(until > from, "measurement window must have positive duration");
    let mut boundaries = vec![from, until];
    boundaries.extend(
        machine.occupancy_change_times().into_iter().map(|at| at.as_ms()).filter(|&at| from < at && at < until),
    );
    boundaries.extend(visits.iter().flat_map(|visit| {
        [visit.boarded_at_ms, visit.pick_start_ms, visit.pick_end_ms, visit.collected_at_ms]
            .into_iter()
            .filter(|&at| from < at && at < until)
    }));
    boundaries.sort_unstable();
    boundaries.dedup();
    let mut area = 0;
    let mut operator_area = 0;
    for span in boundaries.windows(2) {
        let at = span[0];
        let occupancy = machine.occupancy_at(SimTime::ms(at));
        let physical = visits.iter().filter(|v| v.boarded_at_ms <= at && at < v.collected_at_ms).count();
        let picking = visits.iter().filter(|v| v.pick_start_ms <= at && at < v.pick_end_ms).count();
        assert_eq!(occupancy.conveyor, physical, "calendar and tote visit history differ at {at}");
        assert_eq!(occupancy.operator, picking, "calendar and committed operator work differ at {at}");
        assert!(picking <= 1, "both sides must share one operator");
        area += occupancy.conveyor as u64 * (span[1] - at);
        operator_area += occupancy.operator as u64 * (span[1] - at);
    }
    let clipped_visit_tote_ms = visits.iter().map(|v| clipped(v.boarded_at_ms, v.collected_at_ms, from, until)).sum();
    assert_eq!(area, clipped_visit_tote_ms, "finite-window sample-path occupancy identity");
    let clipped_operator_ms: u64 = visits.iter().map(|v| clipped(v.pick_start_ms, v.pick_end_ms, from, until)).sum();
    assert_eq!(operator_area, clipped_operator_ms);
    let wip_at = |at| visits.iter().filter(|v| v.boarded_at_ms <= at && at < v.collected_at_ms).count();
    let wip_at_start = wip_at(from);
    let wip_at_end = wip_at(until);
    let boardings = visits.iter().filter(|v| from < v.boarded_at_ms && v.boarded_at_ms <= until).count();
    let collections = visits.iter().filter(|v| from < v.collected_at_ms && v.collected_at_ms <= until).count();
    assert_eq!(wip_at_start + boardings, wip_at_end + collections, "window flow conservation");
    WindowMeasurement {
        start_ms: from,
        end_ms: until,
        wip_at_start,
        wip_at_end,
        boardings,
        collections,
        physical_area_tote_ms: area,
        clipped_visit_tote_ms,
        mean_physical_wip: area as f64 / (until - from) as f64,
        operator_busy_ms: operator_area,
        operator_utilization: operator_area as f64 / (until - from) as f64,
    }
}

pub fn assert_inventory_custody(machine: &Conveyance, inventory: &InventoryService) {
    for visit in machine.visits() {
        let boarded = visit.boarded_at.expect("completed visit boarded");
        let TotePhase::Collected { at: collected, robot } = visit.phase else { panic!("completed visit collected") };
        assert!(boarded < collected, "a conveyor visit includes physical transport and pick dwell");
        let last_aboard = collected.checked_sub(SimDuration::ms(1)).unwrap();
        let tote = inventory.tote(visit.tote).expect("conveyor visit has authoritative inventory custody");
        if let Some(prior) = boarded.checked_sub(SimDuration::ms(1)) {
            assert!(
                matches!(tote.location_at(prior), ToteLocation::WithRobot { .. }),
                "inventory tote {:?} was not with a robot immediately before boarding at {boarded}",
                visit.tote,
            );
        }
        assert!(
            tote.location_is_through(boarded, last_aboard, ToteLocation::OnConveyance { conveyor_id: machine.id() }),
            "inventory tote {:?} left its conveyor during [{boarded}, {collected})",
            visit.tote,
        );
        assert_eq!(
            tote.location_at(collected),
            ToteLocation::WithRobot { robot_id: robot },
            "inventory tote {:?} was not on its named collector at {collected}",
            visit.tote,
        );
    }
}

pub(super) fn assert_calendar_drained(machine: &Conveyance, horizon_ms: u64) {
    assert_eq!(
        machine.occupancy_at(SimTime::ms(horizon_ms)),
        Occupancy { conveyor: 0, retained_htms: 0, presentations: 0, operator: 0 },
        "completed cohort leaves no calendar occupancy at its final horizon",
    );
}

pub fn measure(machine: &Conveyance, horizon_ms: u64) -> ConveyorMeasurement {
    assert!(horizon_ms >= 4);
    assert_eq!(machine.unfinished(), 0, "measure the full completed cohort");
    assert!(machine.abandoned_custody().is_empty());
    let pairs = machine.transfers();
    let mut matched = vec![false; pairs.len()];
    let mut visits = Vec::new();
    for visit in machine.visits() {
        let boarded = visit.boarded_at.expect("completed visit boarded").as_ms();
        let TotePhase::Collected { at, robot } = visit.phase else { panic!("completed visit collected") };
        let collected = at.as_ms();
        assert!(visit.admitted_at.as_ms() <= boarded && boarded <= collected && collected <= horizon_ms);
        let candidates: Vec<_> = pairs
            .iter()
            .enumerate()
            .filter(|(_, pair)| {
                pair.inventory_tote == visit.tote && boarded <= pair.start.as_ms() && pair.end.as_ms() <= collected
            })
            .collect();
        assert_eq!(candidates.len(), 1, "current workflow has exactly one transfer per inventory visit");
        let (index, pair) = candidates[0];
        assert!(!matched[index], "one transfer matched two visits of the same tote");
        matched[index] = true;
        visits.push(VisitTiming {
            inventory_tote: visit.tote.0,
            admitted_at_ms: visit.admitted_at.as_ms(),
            boarded_at_ms: boarded,
            pick_start_ms: pair.start.as_ms(),
            pick_end_ms: pair.end.as_ms(),
            collected_at_ms: collected,
            station_id: pair.station.0,
            order_tote: pair.order_tote.0,
            order_robot: pair.order_robot.0,
            collector_robot: robot.0,
        });
    }
    assert!(matched.into_iter().all(|value| value), "every committed transfer belongs to a visit");
    visits.sort_by_key(|v| (v.boarded_at_ms, v.inventory_tote, v.collected_at_ms));
    assert_calendar_drained(machine, horizon_ms);
    let full = window(machine, &visits, 0, horizon_ms);
    let middle = window(machine, &visits, horizon_ms / 4, horizon_ms * 3 / 4);
    let residence_ms: u64 = visits.iter().map(|v| v.collected_at_ms - v.boarded_at_ms).sum();
    assert_eq!(full.physical_area_tote_ms, residence_ms, "full-cohort Little identity");
    let count = visits.len() as f64;
    let mean_secs = |total: u64| if count == 0.0 { 0.0 } else { total as f64 / count / 1000.0 };
    ConveyorMeasurement {
        mean_boarded_to_collected_secs: mean_secs(residence_ms),
        mean_boarded_to_pick_start_secs: mean_secs(visits.iter().map(|v| v.pick_start_ms - v.boarded_at_ms).sum()),
        mean_pick_end_to_collected_secs: mean_secs(visits.iter().map(|v| v.collected_at_ms - v.pick_end_ms).sum()),
        full_cohort_little_wip: residence_ms as f64 / horizon_ms as f64,
        visits,
        full,
        middle,
    }
}

pub fn report(label: &str, machines: &Conveyances, navigation: &Navigation) {
    let horizon_ms = machines
        .machines()
        .iter()
        .flat_map(|(_, machine)| machine.visits())
        .map(|visit| match visit.phase {
            TotePhase::Collected { at, .. } => at.as_ms(),
            _ => panic!("queue measurement requires completed conveyor custody"),
        })
        .max()
        .expect("throughput workload has conveyor visits");
    eprintln!(
        "[queueing] {label} horizon=[0,{horizon_ms}]ms (last conveyor collection; reported batch duration is shown separately); middle=25%-75%, descriptive, not steady state"
    );
    let mut output = Vec::new();
    for (layout, machine) in machines.machines() {
        let measurement = measure(machine, horizon_ms);
        eprintln!(
            "[queueing] machine={} visits={} mean_board_collect_s={:.3} mean_board_pick_start_s={:.3} mean_pick_end_collect_s={:.3} full_wip={:.3} little_wip={:.3} full_operator_pct={:.2} middle_wip={:.3} middle_operator_pct={:.2} middle_carry_in={} middle_carry_out={}",
            layout.conveyor_group,
            measurement.visits.len(),
            measurement.mean_boarded_to_collected_secs,
            measurement.mean_boarded_to_pick_start_secs,
            measurement.mean_pick_end_to_collected_secs,
            measurement.full.mean_physical_wip,
            measurement.full_cohort_little_wip,
            100.0 * measurement.full.operator_utilization,
            measurement.middle.mean_physical_wip,
            100.0 * measurement.middle.operator_utilization,
            measurement.middle.wip_at_start,
            measurement.middle.wip_at_end,
        );
        let presentation_holds: Vec<_> = if navigation.oracle.tracks_reservations() {
            machine
                .transfers()
                .iter()
                .map(|pair| {
                    let place = layout.presentations.iter().find(|place| place.station_id == pair.station).unwrap();
                    let holds = navigation.oracle.reservations_for_cell(place.cell);
                    let matching: Vec<_> = holds
                        .iter()
                        .filter(|hold| {
                            hold.robot() == pair.order_robot
                                && hold.window().contains(pair.start)
                                && hold.window().contains(pair.end)
                        })
                        .collect();
                    assert_eq!(matching.len(), 1, "paired pick must have one physical presentation hold");
                    let hold = matching[0].window();
                    let end = hold.end().expect("completed outbound work vacates the order presentation");
                    json!({
                        "station_id": pair.station.0, "cell_id": place.cell.0,
                        "order_robot": pair.order_robot.0, "order_tote": pair.order_tote.0,
                        "inventory_tote": pair.inventory_tote.0,
                        "pick_start_ms": pair.start.as_ms(), "pick_end_ms": pair.end.as_ms(),
                        "hold_start_ms": hold.start().as_ms(), "hold_end_ms": end.as_ms(),
                        "pre_pick_occupied_ms": (pair.start - hold.start()).as_ms(),
                        "post_pick_occupied_ms": (end - pair.end).as_ms(),
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        if !presentation_holds.is_empty() {
            let mean = |field: &str| {
                presentation_holds.iter().map(|row| row[field].as_u64().unwrap()).sum::<u64>() as f64
                    / presentation_holds.len() as f64
                    / 1000.0
            };
            eprintln!(
                "[queueing] machine={} mean_order_presentation_pre_pick_s={:.3} mean_order_presentation_post_pick_s={:.3}",
                layout.conveyor_group,
                mean("pre_pick_occupied_ms"),
                mean("post_pick_occupied_ms")
            );
        }
        output.push(json!({"layout": layout, "measurement": measurement, "presentation_holds": presentation_holds}));
    }
    if let Some(directory) = env::var_os("WALMART_QUEUE_TRACE_DIR") {
        let directory = PathBuf::from(directory);
        fs::create_dir_all(&directory).expect("create queue measurement directory");
        let path = directory.join(format!("{label}.json"));
        fs::write(
            &path,
            serde_json::to_vec_pretty(&json!({
                "label": label, "horizon_ms": horizon_ms,
                "horizon_definition": "[0,last conveyor collection]; excludes later outbound cleanup",
                "middle_window": "middle half of same horizon; not a steady-state estimator",
                "machines": output,
            }))
            .unwrap(),
        )
        .expect("write queue measurements");
        eprintln!("[queueing] raw timings: {}", path.display());
    }
}
