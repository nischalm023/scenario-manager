//! Workload loading, fleet admission and outbound timing govern the actual
//! fulfillment tree rather than merely changing configuration descriptions.

mod common;

use std::sync::{Arc, Mutex, mpsc};

use desgraph::{CellId, CellKind, StationId, StationType};
use desmap::{OutboundRole, PresentationPoint};
use dessim::composition::{self, Composition};
use dessim::events::{
    CycleKind, InventoryVisitPhase, OrderStationPhase, SimEvent, TotePayloadState, TripEndpoint, TripPurpose,
    VtmCycleDirection, VtmOperationKind,
};
use dessim::model::ParkingRegistry;
use dessim::model::area_admission::AreaAdmission;
use dessim::model::conveyance::{Conveyances, TotePhase};
use dessim::model::htm::registry::AvailableHtmRegistry;
use dessim::model::inventory::events::{InventoryEvent, InventoryObserver, InventoryObserverError};
use dessim::model::navigation::Navigation;
use dessim::model::order::{ExitPath, OrderContainer};
use dessim::model::{
    BotKind, BotService, InventoryService, OrderId, OrderLine, RobotId, SkuId, ToteId, ToteLocation, ToteType,
};
use dessim::observer::output::{Activity, HtmJournalEndpoint};
use dessim::observer::time_slice::ToteVisualState;
use dessim::pathfinding::{CommitPlan, ReservationRequest, WaypointAction};
use dessim::prelude::{Display, EmptyProposal, PlanError, ProposalId, SchedCtx, Task, async_trait};
use dessim::reporting::RunReports;
use dessim::runner::resolve_scenario_with_racking;
use dessim::scenario_gen::{RackUse, ScenarioData};
use dessim::task;
use dessim::task::TaskContext;
use dessim::{FidelityMode, SimConfig, SimDuration, SimTime};
use native_modules::migration::flows::fulfillment::fetch_bound::FetchBounds;
use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::json;

fn fixture() -> (ScenarioData, SimConfig) {
    let (world, scenario, mut config) = common::load_world_spec("minimal_easyloop_flow_spec.json");
    config.fidelity = FidelityMode::CollisionDes;
    let data = resolve_scenario_with_racking(world.map, world.racking, &scenario, &config).unwrap();
    (data, config)
}

fn run(mut data: ScenarioData, config: &SimConfig) -> (RunReports, Vec<SimEvent>) {
    let (tx, rx) = mpsc::channel();
    let pathfinder = common::fh_sipp_for_map(&data.map, config);
    let (mut scheduler, reports, _) = composition::build(&mut data, config, pathfinder, Some(Arc::new(tx))).unwrap();
    if let Err(error) = scheduler.run() {
        let events: Vec<_> = rx.try_iter().collect();
        eprintln!("{}", common::format_event_trace(&events));
        for event in &events {
            if matches!(event, SimEvent::OrderStationPhase(_) | SimEvent::InventoryVisitPhase(_)) {
                eprintln!("{event:?}");
            }
        }
        panic!("{error}");
    }
    reports.check_quiescence_invariants().unwrap();
    (reports, rx.try_iter().collect())
}

#[test]
fn fulfillment_consumes_only_work_loaded_by_load_orders() {
    let (data, mut config) = fixture();
    assert!(!data.orders.is_empty());
    let mut definition = config.composition.resolve().unwrap();
    definition.tasks.retain(|task| task.name() != "LoadOrders");
    definition.tasks.iter_mut().find(|task| task.name() == "FulfillOrders").unwrap().after =
        vec!["FillInventory".into()];
    config.composition = Composition::Inline(definition);
    let (reports, events) = run(data, &config);
    assert!(!events.iter().any(|event| matches!(event, SimEvent::CycleComplete(_))));
    assert!(
        reports.existing::<Conveyances>().unwrap().machines().iter().all(|(_, conveyor)| conveyor.visits().is_empty())
    );
}

#[test]
fn fulfillment_refuses_a_fleet_that_cannot_supply_both_totes() {
    let (mut data, config) = fixture();
    data.htm_initial_positions.truncate(1);
    let pathfinder = common::fh_sipp_for_map(&data.map, &config);
    let (mut scheduler, _, _) = composition::build(&mut data, &config, pathfinder, None).unwrap();
    let error = scheduler.run().unwrap_err();
    assert!(error.to_string().contains("needs at least two HTMs"), "{error}");
}

#[test]
fn carton_orders_require_a_declared_induction_presentation() {
    let (mut data, config) = fixture();
    data.orders.truncate(1);
    data.orders[0].container = OrderContainer::Carton;
    data.map.stations.retain(|station| station.station_type != StationType::BoxInduction);
    let pathfinder = common::fh_sipp_for_map(&data.map, &config);
    let (mut scheduler, _, _) = composition::build(&mut data, &config, pathfinder, None).unwrap();
    let error = scheduler.run().unwrap_err();
    assert!(error.to_string().contains("require a declared box-induction presentation"), "{error}");
}

#[test]
fn typed_outbound_work_ignores_an_ineligible_handoff() {
    for (container, exit_path, role) in [
        (OrderContainer::Carton, ExitPath::Dock, OutboundRole::SealerDropoff),
        (OrderContainer::Tray, ExitPath::Sorter, OutboundRole::Flex),
        (OrderContainer::Tote, ExitPath::Dock, OutboundRole::Bagging),
        (OrderContainer::Tote, ExitPath::Buffer, OutboundRole::RouteDispense),
    ] {
        let (mut data, mut config) = fixture();
        data.orders.truncate(1);
        data.orders[0].container = container;
        data.orders[0].exit_path = exit_path;
        config.order.direct_extraction = true;
        let mut definition = config.composition.resolve().unwrap();
        definition.tasks.iter_mut().find(|task| task.name() == "FillInventory").unwrap().config["order_trays"] =
            json!(1);
        config.composition = Composition::Inline(definition);
        let correct = data.map.stations.iter_mut().find(|s| s.station_type == StationType::Extraction).unwrap();
        correct.outbound_role = Some(role);
        let expected = correct.id;
        let mut decoy = correct.clone();
        let cell =
            data.map.graph.cells().find(|cell| cell.kind == CellKind::Open && cell.station_id.is_none()).unwrap().id;
        decoy.id = StationId(10_000);
        decoy.presentation_points = vec![PresentationPoint { presentation_cell: cell, queue_path: vec![] }];
        decoy.position = cell;
        decoy.handoff_queues.clear();
        decoy.outbound_role = Some(if role == OutboundRole::Flex { OutboundRole::Bagging } else { OutboundRole::Flex });
        data.map.stations.push(decoy);
        let (_, events) = run(data, &config);
        let extracted: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                SimEvent::CycleComplete(cycle) if matches!(cycle.kind, CycleKind::OrderExtract { .. }) => {
                    Some(cycle.station_id)
                }
                _ => None,
            })
            .collect();
        assert_eq!(extracted, [expected], "{container:?}/{exit_path:?}");
    }
}

#[test]
fn sealed_box_pickup_cannot_supply_a_new_empty_carton() {
    let (mut data, config) = fixture();
    data.orders.truncate(1);
    data.orders[0].container = OrderContainer::Carton;
    for station in &mut data.map.stations {
        if station.station_type == StationType::BoxInduction {
            station.station_type = StationType::ContainerPickup;
            station.outbound_role = Some(OutboundRole::SealedBoxPickup);
        }
    }
    let pathfinder = common::fh_sipp_for_map(&data.map, &config);
    let (mut scheduler, _, _) = composition::build(&mut data, &config, pathfinder, None).unwrap();
    let error = scheduler.run().unwrap_err();
    assert!(error.to_string().contains("require a declared box-induction presentation"), "{error}");
}

fn retain_idle_relays(data: &mut ScenarioData, keep: Option<CellId>) {
    data.map.relays.retain(|relay| Some(relay.htm_access) == keep);
}

#[test]
fn carton_workload_without_parking_is_refused_before_induction() {
    let (mut data, config) = fixture();
    data.orders.truncate(2);
    for order in &mut data.orders {
        order.container = OrderContainer::Carton;
    }
    retain_idle_relays(&mut data, None);
    let (tx, rx) = mpsc::channel();
    let pathfinder = common::fh_sipp_for_map(&data.map, &config);
    let (mut scheduler, _, _) = composition::build(&mut data, &config, pathfinder, Some(Arc::new(tx))).unwrap();
    let error = scheduler.run().unwrap_err();
    assert!(error.to_string().contains("carton extraction requires relay egress"), "{error}");
    assert!(
        !rx.try_iter()
            .any(|event| matches!(event, SimEvent::ToteCreated(created) if created.tote_type == ToteType::Carton))
    );
}

#[test]
fn full_parking_reports_extraction_clearance_failure_and_retains_the_empty_htm_hold() {
    let (mut data, mut config) = fixture();
    config.fidelity = FidelityMode::CollisionDes;
    data.htm_initial_positions.truncate(4);
    data.orders.truncate(2);
    for order in &mut data.orders {
        order.container = OrderContainer::Carton;
        order.exit_path = ExitPath::Dock;
        order.lines.truncate(1);
    }
    let parking = ParkingRegistry::from_map(&data.map)
        .candidate_cells()
        .find(|cell| !data.htm_initial_positions.contains(cell))
        .unwrap();
    retain_idle_relays(&mut data, Some(parking));
    let extraction = data
        .map
        .stations
        .iter()
        .find(|station| station.station_type == StationType::Extraction)
        .unwrap()
        .delivery_cell();
    let mut definition = config.composition.resolve().unwrap();
    definition.tasks.iter_mut().find(|task| task.name() == "FulfillOrders").unwrap().config["extraction_secs"] =
        json!(1000);
    config.composition = Composition::Inline(definition);
    let (tx, rx) = mpsc::channel();
    let pathfinder = common::fh_sipp_for_map(&data.map, &config);
    let (mut scheduler, reports, _) = composition::build(&mut data, &config, pathfinder, Some(Arc::new(tx))).unwrap();
    // A parked HTM owned by another task occupies the only bay. Four HTMs
    // remain available for both orders and their inventory transport.
    let navigation = scheduler.states().get::<Navigation>().unwrap();
    let occupied_by = RobotId(1000);
    let _held_parking = navigation
        .oracle
        .commit_path(occupied_by, &CommitPlan::single(ReservationRequest::spanning(parking, SimTime::ZERO, None)), None)
        .unwrap();
    scheduler.states().get::<ParkingRegistry>().unwrap().reserve(parking, occupied_by, SimTime::ZERO);
    scheduler.states().get::<BotService>().unwrap().register(occupied_by, BotKind::Htm, parking);
    let error = scheduler.run().unwrap_err();
    assert!(error.to_string().contains("cannot clear extraction cell"), "{error}");
    assert!(error.to_string().contains("no free reachable relay egress"), "{error}");
    let events: Vec<_> = rx.try_iter().collect();
    assert_eq!(events.iter().filter(|event| matches!(event, SimEvent::OrderComplete(_))).count(), 2);
    let extraction_end = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::CycleComplete(cycle) if matches!(cycle.kind, CycleKind::OrderExtract { .. }) => {
                Some(SimTime::ms(cycle.sim_time_ms + cycle.dwell_ms))
            }
            _ => None,
        })
        .min()
        .unwrap();
    assert!(
        scheduler.barrier().floor() >= extraction_end,
        "unavailable early parking retains the extraction hold and retries at service completion"
    );
    assert!(
        reports.existing::<Conveyances>().unwrap().machines().iter().all(|(_, conveyor)| conveyor.unfinished() == 0)
    );
    let available = reports.existing::<AvailableHtmRegistry>().unwrap();
    let held = available.peek_matching(|htm| htm.cell() == extraction);
    assert_eq!(held.len(), 1);
    assert!(held[0].tote.is_none());
    assert!(held[0].cell_reservation.is_open_ended());
    let navigation = reports.existing::<Navigation>().unwrap();
    assert!(
        navigation
            .oracle
            .reservations_for_cell(extraction)
            .iter()
            .any(|hold| hold.robot() == held[0].robot() && hold.is_open_ended())
    );
    assert!(
        navigation
            .oracle
            .reservations_for_cell(parking)
            .iter()
            .any(|hold| hold.robot() != held[0].robot() && hold.is_open_ended())
    );
}

#[test]
fn consecutive_reusable_extractions_plan_the_known_return_before_service_ends() {
    let (mut data, mut config) = fixture();
    data.htm_initial_positions.truncate(4);
    data.orders.truncate(2);
    for order in &mut data.orders {
        order.container = OrderContainer::Tote;
        order.exit_path = ExitPath::Dock;
        order.lines.truncate(1);
        order.lines[0].quantity = 1;
    }
    let extraction = data
        .map
        .stations
        .iter()
        .find(|station| station.station_type == StationType::Extraction)
        .unwrap()
        .delivery_cell();
    let mut definition = config.composition.resolve().unwrap();
    definition.tasks.iter_mut().find(|task| task.name() == "FulfillOrders").unwrap().config["extraction_secs"] =
        json!(1000);
    config.composition = Composition::Inline(definition);
    let (reports, events) = run(data, &config);
    let mut extractions: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::CycleComplete(cycle) if matches!(cycle.kind, CycleKind::OrderExtract { .. }) => Some(cycle),
            _ => None,
        })
        .collect();
    extractions.sort_by_key(|cycle| cycle.sim_time_ms);
    assert_eq!(extractions.len(), 2);
    let previous = extractions[0];
    let incoming = extractions[1];
    assert_ne!(previous.robot_id, incoming.robot_id, "both orders must be in flight");
    let previous_end = SimTime::ms(previous.sim_time_ms + previous.dwell_ms);
    let approach = events
        .iter()
        .find_map(|event| match event {
            SimEvent::Nav(nav)
                if nav.robot_id == incoming.robot_id
                    && nav.waypoints.last().is_some_and(|end| end.cell == extraction) =>
            {
                Some(nav)
            }
            _ => None,
        })
        .expect("the second order physically approaches extraction");
    let first_move = approach
        .waypoints
        .windows(2)
        .find_map(|pair| match pair[1].action {
            WaypointAction::Move { start, .. } if pair[0].cell != pair[1].cell => Some(start),
            _ => None,
        })
        .expect("the incoming approach contains actual movement");
    assert!(first_move < previous_end, "incoming movement at {first_move} waited for service end {previous_end}");
    let departure = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::Nav(nav)
                if nav.robot_id == previous.robot_id
                    && nav.waypoints.first().is_some_and(|start| start.cell == extraction) =>
            {
                Some(nav)
            }
            _ => None,
        })
        .min_by_key(|nav| nav.time)
        .expect("the preceding reusable container returns to storage");
    assert!(departure.time >= previous_end, "planning early must not shorten extraction dwell");
    assert!(departure.carrying_cargo, "the empty reusable container stays on its assigned HTM");
    let navigation = reports.existing::<Navigation>().unwrap();
    let holds = navigation.oracle.reservations_for_cell(extraction);
    let previous_hold = holds
        .iter()
        .find(|hold| hold.robot() == previous.robot_id && hold.window().contains(SimTime::ms(previous.sim_time_ms)))
        .expect("the preceding extraction has a real cell reservation");
    let incoming_hold = holds
        .iter()
        .find(|hold| hold.robot() == incoming.robot_id && hold.window().contains(SimTime::ms(incoming.sim_time_ms)))
        .expect("the next extraction has a real cell reservation");
    let cleared_at = previous_hold.window().end().expect("only the committed storage route closes the hold");
    assert!(cleared_at >= previous_end);
    assert!(incoming_hold.window().start() >= cleared_at, "approach must respect physical departure clearance");
    assert!(approach.waypoints.last().unwrap().action.end_time() >= cleared_at);
    let inventory = reports.existing::<InventoryService>().unwrap();
    for extraction in extractions {
        let start = SimTime::ms(extraction.sim_time_ms);
        let end = start + SimDuration::ms(extraction.dwell_ms);
        let (id, _) = inventory.tote_on_robot(extraction.robot_id, start).unwrap();
        let tote = inventory.tote(id).unwrap();
        assert!(tote.location_is_through(start, end, ToteLocation::WithRobot { robot_id: extraction.robot_id }));
        assert!(!tote.is_empty_at(end - SimDuration::ms(1)));
        assert!(tote.is_empty_at(end));
    }
}

#[test]
fn extracted_cartons_release_htms_for_subsequent_induction() {
    let (mut data, mut config) = fixture();
    config.fidelity = FidelityMode::CollisionDes;
    data.htm_initial_positions.truncate(2);
    data.orders.truncate(3);
    for order in &mut data.orders {
        order.container = OrderContainer::Carton;
        order.exit_path = ExitPath::Dock;
        order.lines.truncate(1);
    }
    let (reports, events) = run(data, &config);
    let inventory = reports.existing::<InventoryService>().unwrap();
    let cartons: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::ToteCreated(created) if created.tote_type == ToteType::Carton => Some(created.tote_id),
            _ => None,
        })
        .collect();
    assert_eq!(cartons.len(), 3);
    let mut pickup_robots = FxHashMap::default();
    for tote in cartons {
        let robot = events
            .iter()
            .find_map(|event| match event {
                SimEvent::ToteMoved(moved) if moved.tote_id == tote => match moved.new_state {
                    ToteVisualState::OnBot { bot_id } => Some(bot_id),
                    _ => None,
                },
                _ => None,
            })
            .unwrap();
        *pickup_robots.entry(robot).or_insert(0_usize) += 1;
        assert_eq!(inventory.tote(tote).unwrap().location_at(reports.planned_horizon()), ToteLocation::Extracted);
    }
    assert!(pickup_robots.values().any(|&count| count > 1), "two HTMs must serve three newly inducted cartons");
}

#[test]
fn extracted_empty_htm_takes_pending_work_without_a_parking_detour() {
    let (mut data, mut config) = fixture();
    data.htm_initial_positions.truncate(2);
    data.orders.truncate(2);
    for order in &mut data.orders {
        order.container = OrderContainer::Carton;
        order.exit_path = ExitPath::Dock;
        order.arrival = None;
        order.lines = vec![OrderLine { sku: data.skus[0].id, quantity: 1 }];
    }
    let mut definition = config.composition.resolve().unwrap();
    definition.tasks.iter_mut().find(|task| task.name() == "FulfillOrders").unwrap().config =
        json!({"max_active_orders_per_machine": 1});
    config.composition = Composition::Inline(definition);
    let extraction_cells: Vec<_> = data
        .map
        .stations
        .iter()
        .filter(|station| station.station_type == StationType::Extraction)
        .map(|station| (station.id, station.delivery_cell()))
        .collect();
    let (_, events) = run(data, &config);
    let first_extraction = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::CycleComplete(cycle) if matches!(cycle.kind, CycleKind::OrderExtract { .. }) => Some(cycle),
            _ => None,
        })
        .min_by_key(|cycle| cycle.sim_time_ms)
        .expect("first carton is extracted");
    let released_at = first_extraction.sim_time_ms + first_extraction.dwell_ms;
    let departure = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::Nav(nav)
                if nav.robot_id == first_extraction.robot_id
                    && nav.time.as_ms() >= released_at
                    && nav.waypoints.windows(2).any(|pair| pair[0].cell != pair[1].cell) =>
            {
                Some(nav)
            }
            _ => None,
        })
        .min_by_key(|nav| nav.time)
        .expect("the released HTM physically leaves extraction");
    let extraction_cell =
        extraction_cells.iter().find(|(station, _)| *station == first_extraction.station_id).unwrap().1;
    assert_eq!(departure.waypoints.first().unwrap().cell, extraction_cell);
    assert!(!departure.carrying_cargo, "the extracted carton is no longer carried");
    let next_assignment = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::TripStart(trip)
                if trip.robot_id == first_extraction.robot_id && trip.sim_time_ms >= released_at =>
            {
                Some(trip)
            }
            _ => None,
        })
        .min_by_key(|trip| trip.sim_time_ms)
        .expect("the empty HTM receives another assignment");
    assert_eq!(next_assignment.sim_time_ms, departure.time.as_ms());
    assert!(
        matches!(next_assignment.purpose, TripPurpose::ToStation | TripPurpose::ToRelay),
        "pending productive work owns departure: {:?}",
        next_assignment.purpose
    );
    assert!(!events.iter().any(|event| matches!(event,
        SimEvent::TripStart(trip) if trip.robot_id == first_extraction.robot_id
            && trip.sim_time_ms >= released_at && trip.sim_time_ms <= departure.time.as_ms()
            && trip.purpose == TripPurpose::ToParking
    )));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event,
                SimEvent::CycleComplete(cycle) if matches!(cycle.kind, CycleKind::OrderExtract { .. })
            ))
            .count(),
        2
    );
    assert_eq!(events.iter().filter(|event| matches!(event, SimEvent::OrderComplete(_))).count(), 2);
}

#[test]
fn inventory_retrieval_overlaps_order_container_acquisition() {
    let (mut data, mut config) = fixture();
    data.htm_initial_positions.truncate(2);
    data.orders.truncate(1);
    data.orders[0].container = OrderContainer::Carton;
    data.orders[0].exit_path = ExitPath::Dock;
    data.orders[0].lines.truncate(1);
    config.htm.pickup_secs = 120.0;
    let (reports, events) = run(data, &config);
    let conveyances = reports.existing::<Conveyances>().unwrap();
    let transfers = conveyances.machines()[0].1.transfers();
    assert_eq!(transfers.len(), 1);
    let pair = transfers[0];
    let bots = reports.existing::<BotService>().unwrap();
    let loaded_at = |tote| {
        events
            .iter()
            .filter_map(|event| match event {
                SimEvent::ToteMoved(moved) if moved.tote_id == tote => match moved.new_state {
                    ToteVisualState::OnBot { bot_id } if bots.bot_kind(RobotId(bot_id)) == Some(BotKind::Htm) => {
                        Some(moved.time)
                    }
                    _ => None,
                },
                _ => None,
            })
            .min()
            .expect("the tote was physically picked up by an HTM")
    };
    let order_loaded = loaded_at(pair.order_tote);
    let inventory_pickup_start = loaded_at(pair.inventory_tote) - config.htm.pickup_time();
    assert!(
        inventory_pickup_start < order_loaded,
        "inventory pickup started at {inventory_pickup_start}, after order pickup finished at {order_loaded}"
    );
    assert!(pair.start >= loaded_at(pair.inventory_tote).max(order_loaded));
    assert_eq!(events.iter().filter(|event| matches!(event, SimEvent::OrderComplete(_))).count(), 1);
}

#[test]
fn inventory_supply_does_not_wait_for_order_container_induction() {
    let (mut data, config) = fixture();
    data.htm_initial_positions.truncate(2);
    data.orders.truncate(1);
    data.orders[0].container = OrderContainer::Carton;
    data.orders[0].exit_path = ExitPath::Dock;
    data.orders[0].lines.truncate(1);
    data.orders[0].lines[0].quantity = 1;
    let pathfinder = common::fh_sipp_for_map(&data.map, &config);
    let (tx, rx) = mpsc::channel();
    let (mut scheduler, reports, _) = composition::build(&mut data, &config, pathfinder, Some(Arc::new(tx))).unwrap();
    let navigation = scheduler.states().get::<Navigation>().unwrap();
    let induction_free_at = SimTime::seconds(300);
    let _traffic: Vec<_> = data
        .map
        .stations
        .iter()
        .filter(|station| station.station_type == StationType::BoxInduction)
        .enumerate()
        .map(|(index, station)| {
            navigation
                .oracle
                .commit_path(
                    RobotId(1000 + index as u32),
                    &CommitPlan::single(ReservationRequest::spanning(
                        station.delivery_cell(),
                        SimTime::ZERO,
                        Some(induction_free_at),
                    )),
                    None,
                )
                .unwrap()
        })
        .collect();
    scheduler.run().unwrap();
    reports.check_quiescence_invariants().unwrap();
    let events: Vec<_> = rx.try_iter().collect();
    let conveyances = reports.existing::<Conveyances>().unwrap();
    let (machine, conveyor) = &conveyances.machines()[0];
    let pair = conveyor.transfers()[0];
    let bots = reports.existing::<BotService>().unwrap();
    let inventory_robot = events
        .iter()
        .find_map(|event| match event {
            SimEvent::ToteMoved(moved) if moved.tote_id == pair.inventory_tote => match moved.new_state {
                ToteVisualState::OnBot { bot_id } if bots.bot_kind(RobotId(bot_id)) == Some(BotKind::Htm) => {
                    Some(RobotId(bot_id))
                }
                _ => None,
            },
            _ => None,
        })
        .unwrap();
    let delivery = events
        .iter()
        .find_map(|event| match event {
            SimEvent::Nav(nav)
                if nav.robot_id == inventory_robot
                    && nav.carrying_cargo
                    && nav.waypoints.last().is_some_and(|waypoint| Some(waypoint.cell) == machine.dropoff_cell) =>
            {
                Some(nav)
            }
            _ => None,
        })
        .unwrap();
    let first_move = delivery
        .waypoints
        .windows(2)
        .find_map(|waypoints| match waypoints[1].action {
            WaypointAction::Move { start, .. } if waypoints[0].cell != waypoints[1].cell => Some(start),
            _ => None,
        })
        .unwrap();
    assert!(
        first_move < induction_free_at,
        "inventory delivery starts at {first_move}, although only order induction is blocked until {induction_free_at}"
    );
    let delivered_at = delivery.waypoints.last().unwrap().end_time();
    assert!(delivered_at < induction_free_at, "the inventory reaches dropoff before induction clears");
    let source = delivery.waypoints.first().unwrap().cell;
    let source_hold = navigation
        .oracle
        .reservations_for_cell(source)
        .into_iter()
        .find(|hold| hold.robot() == inventory_robot && hold.window().contains(first_move))
        .expect("the delivering HTM retained its pickup cell until physical departure");
    assert!(source_hold.window().end().is_some_and(|end| end < induction_free_at));
    let inventory = reports.existing::<InventoryService>().unwrap();
    let tote = inventory.tote(pair.inventory_tote).unwrap();
    let boarded = conveyor.visits()[0].boarded_at.unwrap();
    assert!(tote.location_is_through(
        first_move,
        boarded - SimDuration::ms(1),
        ToteLocation::WithRobot { robot_id: inventory_robot }
    ));
    assert!(boarded < induction_free_at, "inventory unload waits until {boarded} for unrelated order induction");
    assert!(tote.location_is_through(
        boarded,
        induction_free_at,
        ToteLocation::OnConveyance { conveyor_id: conveyor.id() }
    ));
    assert_eq!(
        bots.position_at(inventory_robot, induction_free_at - SimDuration::ms(1)),
        machine.collection_cell,
        "the emptied HTM waits for its return on the collection cell"
    );
    assert!(pair.start >= induction_free_at + config.htm.conveyor_pickup_time());
    assert_eq!(pair.end - pair.start, SimDuration::from_secs(config.operator.time_per_pick_secs));
    assert_eq!(conveyor.visits().len(), 1);
    assert_eq!(conveyor.transfers().len(), 1);
    assert_eq!(events.iter().filter(|event| matches!(event, SimEvent::OrderComplete(_))).count(), 1);
}

#[test]
fn next_inventory_retrieval_overlaps_order_return_to_waiting() {
    let (mut data, config) = fixture();
    data.orders.truncate(1);
    let [first, second] = [data.skus[0].id, data.skus[1].id];
    data.orders[0].container = OrderContainer::Carton;
    data.orders[0].exit_path = ExitPath::Dock;
    data.orders[0].lines = vec![OrderLine { sku: first, quantity: 1 }, OrderLine { sku: second, quantity: 1 }];
    let (reports, events) = run(data, &config);
    let conveyances = reports.existing::<Conveyances>().unwrap();
    let (machine, conveyance) = &conveyances.machines()[0];
    let mut pairs = conveyance.transfers();
    pairs.sort_by_key(|pair| pair.start);
    assert_eq!(pairs.len(), 2);
    let returned_at = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::Nav(nav)
                if nav.robot_id == pairs[0].order_robot
                    && nav.time >= pairs[0].end
                    && nav.waypoints.last().is_some_and(|end| machine.waiting_cells.contains(&end.cell)) =>
            {
                Some(nav.waypoints.last().unwrap().end_time())
            }
            _ => None,
        })
        .min()
        .expect("the incomplete order returns to its waiting berth between picks");
    let next_retrieval = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::VtmCycle(cycle)
                if cycle.tote_id == pairs[1].inventory_tote && cycle.direction == VtmCycleDirection::Forward =>
            {
                Some(SimTime::ms(cycle.sim_time_ms))
            }
            _ => None,
        })
        .min()
        .expect("the second inventory tote is retrieved through its VTM");
    assert!(next_retrieval >= pairs[0].end, "the next inventory visit starts after the current pick");
    assert!(
        next_retrieval < returned_at,
        "next retrieval started at {next_retrieval}, after the order reached waiting at {returned_at}"
    );
    let forward_count = events
        .iter()
        .filter(|event| matches!(event, SimEvent::VtmCycle(cycle) if cycle.direction == VtmCycleDirection::Forward))
        .count();
    assert_eq!(forward_count, 2, "the final pick must not prepare another inventory visit");
    assert_eq!(events.iter().filter(|event| matches!(event, SimEvent::OrderComplete(_))).count(), 1);
}

#[test]
fn prepared_inventory_visits_follow_split_and_repeated_lines() {
    let (mut data, config) = fixture();
    data.orders.truncate(1);
    let [first, second] = [data.skus[0].id, data.skus[1].id];
    let initial =
        [first, second].map(|sku| data.inventory.iter().filter(|((s, _), _)| *s == sku).map(|(_, q)| *q).sum::<u32>());
    data.orders[0].container = OrderContainer::Carton;
    data.orders[0].exit_path = ExitPath::Dock;
    // One-unit source totes force the first line to span three visits. A
    // repeated SKU after another line must not be mistaken for fulfilled work.
    data.orders[0].lines = vec![
        OrderLine { sku: second, quantity: 0 },
        OrderLine { sku: first, quantity: 3 },
        OrderLine { sku: first, quantity: 0 },
        OrderLine { sku: second, quantity: 1 },
        OrderLine { sku: first, quantity: 1 },
        OrderLine { sku: second, quantity: 0 },
    ];
    let (reports, events) = run(data, &config);
    let mut picks: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::CycleComplete(cycle) => match cycle.kind {
                CycleKind::Pick { sku, qty } => Some((cycle.sim_time_ms, sku, qty)),
                _ => None,
            },
            _ => None,
        })
        .collect();
    picks.sort_unstable();
    assert_eq!(
        picks.iter().map(|&(_, sku, qty)| (sku, qty)).collect::<Vec<_>>(),
        [(first, 1), (first, 1), (first, 1), (second, 1), (first, 1)]
    );
    let inventory = reports.existing::<InventoryService>().unwrap();
    let end = reports.planned_horizon();
    for ((sku, initial), picked) in [first, second].into_iter().zip(initial).zip([4, 1]) {
        assert_eq!(inventory.sku_inventory_at(sku, end), (initial - picked) as isize);
        assert_eq!(inventory.sku_reserved_at(sku, end), 0);
    }
    let conveyances = reports.existing::<Conveyances>().unwrap();
    let (machine, conveyance) = &conveyances.machines()[0];
    let pairs = conveyance.transfers();
    assert_eq!(pairs.len(), 5);
    assert_eq!(conveyance.visits().len(), 5, "zero-quantity lines and the completed order need no retrieval");
    let final_pick = pairs.iter().max_by_key(|pair| pair.end).unwrap();
    let departure = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::Nav(nav) if nav.robot_id == final_pick.order_robot && nav.time >= final_pick.end => Some(nav),
            _ => None,
        })
        .min_by_key(|nav| nav.time)
        .expect("the complete order leaves its final presentation");
    assert!(
        departure.waypoints.iter().all(|waypoint| !machine.waiting_cells.contains(&waypoint.cell)),
        "after the final unit, the outbound route must not visit a waiting cell"
    );
    assert!(!machine.cells.contains(&departure.waypoints.last().unwrap().cell), "the next leg exits the EasyLoop");
    assert_eq!(events.iter().filter(|event| matches!(event, SimEvent::OrderComplete(_))).count(), 1);
}

#[test]
fn mixed_containers_share_picking_but_have_distinct_buffer_and_extraction_custody() {
    let (mut data, mut config) = fixture();
    config.fidelity = FidelityMode::CollisionDes;
    let cases = [
        (OrderContainer::Tote, ExitPath::Dock, ToteType::Tote),
        (OrderContainer::Carton, ExitPath::Dock, ToteType::Carton),
        (OrderContainer::Tray, ExitPath::Sorter, ToteType::Tray),
        (OrderContainer::Tote, ExitPath::Buffer, ToteType::Tote),
        (OrderContainer::Carton, ExitPath::Buffer, ToteType::Carton),
        (OrderContainer::Tray, ExitPath::Buffer, ToteType::Tray),
    ];
    data.orders.truncate(cases.len());
    for (order, &(container, exit_path, _)) in data.orders.iter_mut().zip(&cases) {
        order.container = container;
        order.exit_path = exit_path;
        order.lines.truncate(1);
        order.arrival = None;
        order.deadline = Some(SimTime::seconds(36_000));
    }
    let ids: Vec<_> = data.orders.iter().map(|order| order.id).collect();
    let rack_use = data.rack_use.clone();
    let induction_cells: Vec<_> = data
        .map
        .stations
        .iter()
        .filter(|station| station.station_type == StationType::BoxInduction)
        .flat_map(|station| station.presentation_cells().map(|cell| cell.0))
        .collect();
    let mut definition = config.composition.resolve().unwrap();
    definition.tasks.iter_mut().find(|task| task.name() == "FillInventory").unwrap().config["order_trays"] = json!(2);
    config.composition = Composition::Inline(definition);
    let (reports, events) = run(data, &config);
    let inventory = reports.existing::<InventoryService>().unwrap();
    let bots = reports.existing::<BotService>().unwrap();
    let waiting_cells: Vec<_> = reports
        .existing::<Conveyances>()
        .unwrap()
        .machines()
        .iter()
        .flat_map(|(machine, _)| machine.waiting_cells.iter().copied())
        .collect();
    assert!(reports.planned_horizon() < SimTime::seconds(36_000), "cutoffs do not delay buffered release");
    let extractions: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::CycleComplete(cycle) if matches!(cycle.kind, CycleKind::OrderExtract { .. }) => Some(cycle),
            _ => None,
        })
        .collect();
    assert_eq!(extractions.len(), cases.len());
    for (order_id, &(_, exit_path, physical_type)) in ids.iter().zip(&cases) {
        let phases: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                SimEvent::OrderStationPhase(phase) if phase.order_id == *order_id => Some(phase),
                _ => None,
            })
            .collect();
        let one = |wanted| {
            let matching: Vec<_> = phases.iter().filter(|phase| phase.phase == wanted).collect();
            assert_eq!(matching.len(), 1, "one {wanted:?} for {order_id:?}");
            *matching[0]
        };
        let assigned = one(OrderStationPhase::Assigned);
        let entered = one(OrderStationPhase::Admitted);
        let completed = one(OrderStationPhase::FinalPickCompleted);
        let exited = one(OrderStationPhase::AreaExited);
        assert!(assigned.at <= entered.at && entered.at <= completed.at && completed.at < exited.at);
        let admission = reports.existing::<AreaAdmission<StationId, OrderId>>().unwrap();
        let membership = admission.membership(order_id).unwrap();
        assert_eq!(membership.entry, entered.at);
        assert_eq!(membership.exit, Some(exited.at));
        assert!(
            admission.contains(&entered.presentation_id, order_id, completed.at),
            "logical completion must retain physical residency until outbound clearance"
        );
        assert!(!admission.contains(&entered.presentation_id, order_id, exited.at));
        assert!(
            phases.iter().all(|phase| phase.phase != OrderStationPhase::Waiting || phase.at < completed.at),
            "final-pick completion must never send a container back to waiting"
        );
        let final_pick = events
            .iter()
            .filter_map(|event| match event {
                SimEvent::CycleComplete(cycle)
                    if cycle.tote_presentation.is_some_and(|pick| pick.order_id == *order_id) =>
                {
                    Some(cycle)
                }
                _ => None,
            })
            .max_by_key(|cycle| cycle.sim_time_ms + cycle.dwell_ms)
            .unwrap();
        let departure = events
            .iter()
            .filter_map(|event| match event {
                SimEvent::Nav(nav)
                    if nav.robot_id == final_pick.robot_id
                        && nav.carrying_cargo
                        && nav.time.as_ms() >= final_pick.sim_time_ms + final_pick.dwell_ms =>
                {
                    Some(nav)
                }
                _ => None,
            })
            .min_by_key(|nav| nav.time)
            .expect("completed order container departs its final presentation");
        assert!(
            departure.waypoints.iter().all(|waypoint| !waiting_cells.contains(&waypoint.cell)),
            "a completed {physical_type:?} bound for {exit_path:?} must not visit a sequencing berth"
        );
        assert!(
            reports
                .existing::<Conveyances>()
                .unwrap()
                .machines()
                .iter()
                .all(|(machine, _)| !machine.cells.contains(&departure.waypoints.last().unwrap().cell)),
            "the first outbound leg must leave the EasyLoop"
        );
        let machines = reports.existing::<Conveyances>().unwrap();
        let machine = &machines.machines()[0].0;
        let area = machine
            .presentations
            .iter()
            .find(|p| p.station_id == exited.presentation_id)
            .unwrap()
            .order_area
            .as_ref()
            .unwrap();
        let clear = departure
            .waypoints
            .windows(2)
            .find_map(|pair| {
                if area.resident_cells.contains(&pair[0].cell) && pair[1].cell == area.egress {
                    if let WaypointAction::Move { end, .. } = pair[1].action {
                        return Some(end);
                    }
                }
                None
            })
            .expect("the committed outbound route crosses the block's egress boundary");
        assert_eq!(exited.at, clear, "residency ends at committed physical clearance, not extraction");
        let pick = events
            .iter()
            .find_map(|event| match event {
                SimEvent::CycleComplete(cycle)
                    if cycle.tote_presentation.is_some_and(|presentation| presentation.order_id == *order_id) =>
                {
                    Some(cycle)
                }
                _ => None,
            })
            .expect("every container form receives its actual item transfer");
        let (tote_id, _) = inventory.tote_on_robot(pick.robot_id, SimTime::ms(pick.sim_time_ms)).unwrap();
        let tote = inventory.tote(tote_id).unwrap();
        assert_eq!(tote.tote_type(), physical_type);
        let pick_end = SimTime::ms(pick.sim_time_ms + pick.dwell_ms);
        let extraction = extractions
            .iter()
            .filter(|cycle| SimTime::ms(cycle.sim_time_ms) >= pick_end)
            .find(|cycle| {
                inventory
                    .tote_on_robot(cycle.robot_id, SimTime::ms(cycle.sim_time_ms))
                    .is_some_and(|(id, _)| id == tote_id)
            })
            .expect("the same physical container reaches extraction");
        let extracted_at = SimTime::ms(extraction.sim_time_ms + extraction.dwell_ms);
        let buffered = events.iter().any(|event| {
            matches!(event, SimEvent::InventoryMove(moved)
            if moved.tote_id == tote_id && moved.at >= pick_end && moved.at < extracted_at
            && matches!(moved.to, ToteLocation::InRack { rack_id, .. }
                if rack_use.get(&rack_id) == Some(&RackUse::OrderBuffer)))
        });
        assert_eq!(buffered, exit_path == ExitPath::Buffer);
        assert!(tote.is_empty_at(extracted_at));
        if physical_type == ToteType::Carton {
            assert_eq!(tote.location_at(extracted_at), ToteLocation::Extracted);
            assert!(inventory.tote_on_robot(extraction.robot_id, extracted_at).is_none());
            assert!(!events.iter().any(|event| matches!(event, SimEvent::InventoryMove(moved)
                if moved.tote_id == tote_id && moved.at > extracted_at)));
        } else {
            assert!(events.iter().any(|event| matches!(event, SimEvent::InventoryMove(moved)
                if moved.tote_id == tote_id && moved.at > extracted_at
                && matches!(moved.to, ToteLocation::InRack { rack_id, .. }
                    if rack_use.get(&rack_id) == Some(&RackUse::OrderDepot)))));
        }
    }
    let cartons: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::ToteCreated(created) if created.tote_type == ToteType::Carton => Some(created.tote_id),
            _ => None,
        })
        .collect();
    assert_eq!(cartons.len(), 2, "only carton orders consume newly inducted boxes");
    for created in events.iter().filter_map(|event| match event {
        SimEvent::ToteCreated(created) if created.tote_type == ToteType::Carton => Some(created),
        _ => None,
    }) {
        let ToteVisualState::AtCell { cell_id } = created.initial_state else { panic!("box not created at induction") };
        assert!(induction_cells.contains(&cell_id));
        let loaded_at = events
            .iter()
            .find_map(|event| match event {
                SimEvent::ToteMoved(moved)
                    if moved.tote_id == created.tote_id && matches!(moved.new_state, ToteVisualState::OnBot { .. }) =>
                {
                    Some(moved.time)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(loaded_at, created.time + config.htm.conveyor_pickup_time());
    }
    bots.for_each(BotKind::Htm, |_, plan| assert!(plan.carrying_tote().is_none(), "all HTMs are released empty"));
}

#[test]
fn extraction_bypass_clears_the_area_and_preserves_reusable_containers_without_any_port() {
    for bypass_induction in [false, true] {
        let (mut data, mut config) = fixture();
        let cases = [OrderContainer::Tote, OrderContainer::Carton, OrderContainer::Tray];
        data.orders.truncate(cases.len());
        for (order, container) in data.orders.iter_mut().zip(cases) {
            order.container = container;
            order.exit_path = ExitPath::Buffer;
            order.lines.truncate(1);
            order.arrival = None;
        }
        let ids: Vec<_> = data.orders.iter().map(|order| order.id).collect();
        let rack_use = data.rack_use.clone();
        let ports: Vec<_> = data
            .map
            .stations
            .iter()
            .filter(|station| station.station_type == StationType::Extraction)
            .flat_map(|station| station.presentation_cells())
            .collect();
        data.map.stations.retain(|station| station.station_type != StationType::Extraction);
        let mut definition = config.composition.resolve().unwrap();
        definition.tasks.iter_mut().find(|task| task.name() == "FulfillOrders").unwrap().config["bypass_extraction"] =
            json!(true);
        definition.tasks.iter_mut().find(|task| task.name() == "FillInventory").unwrap().config["order_trays"] =
            json!(2);
        if bypass_induction {
            data.map.stations.retain(|station| station.station_type != StationType::BoxInduction);
            definition.tasks.iter_mut().find(|task| task.name() == "FulfillOrders").unwrap().config["bypass_box_induction"] =
                json!(true);
        }
        config.composition = Composition::Inline(definition);
        let (reports, events) = run(data, &config);
        assert_eq!(events.iter().filter(|event| matches!(event, SimEvent::OrderComplete(_))).count(), cases.len());
        assert!(!events.iter().any(|event| matches!(event, SimEvent::CycleComplete(cycle)
        if matches!(cycle.kind, CycleKind::OrderExtract { .. }))));
        assert!(!events.iter().any(|event| matches!(event, SimEvent::Nav(nav)
        if nav.waypoints.last().is_some_and(|waypoint| ports.contains(&waypoint.cell)))));
        let inventory = reports.existing::<InventoryService>().unwrap();
        for (id, container) in ids.iter().zip(cases) {
            let pick = events
                .iter()
                .find_map(|event| match event {
                    SimEvent::CycleComplete(cycle) if cycle.tote_presentation.is_some_and(|p| p.order_id == *id) => {
                        Some(cycle)
                    }
                    _ => None,
                })
                .unwrap();
            let (tote_id, _) = inventory.tote_on_robot(pick.robot_id, SimTime::ms(pick.sim_time_ms)).unwrap();
            let tote = inventory.tote(tote_id).unwrap();
            let exited = events
                .iter()
                .find_map(|event| match event {
                    SimEvent::OrderStationPhase(phase)
                        if phase.order_id == *id && phase.phase == OrderStationPhase::AreaExited =>
                    {
                        Some(phase)
                    }
                    _ => None,
                })
                .unwrap();
            let pick_end = SimTime::ms(pick.sim_time_ms + pick.dwell_ms);
            assert!(exited.at > pick_end, "completed containers physically leave the presentation before draining");
            assert!(!tote.is_empty_at(exited.at - SimDuration::ms(1)), "contents remain aboard until physical exit");
            assert!(tote.is_empty_at(exited.at), "the diagnostic adds no service dwell after exit");
            let admission = reports.existing::<AreaAdmission<StationId, OrderId>>().unwrap();
            assert_eq!(admission.membership(id).unwrap().exit, Some(exited.at));
            assert!(!events.iter().any(|event| matches!(event, SimEvent::OrderStationPhase(phase)
            if phase.order_id == *id && phase.phase == OrderStationPhase::Waiting && phase.at >= pick_end)));
            assert!(!events.iter().any(|event| matches!(event, SimEvent::InventoryMove(moved)
            if moved.tote_id == tote_id && matches!(moved.to, ToteLocation::InRack { rack_id, .. }
                if rack_use.get(&rack_id) == Some(&RackUse::OrderBuffer)))));
            if container == OrderContainer::Carton {
                assert_eq!(tote.location_at(exited.at), ToteLocation::Extracted);
                assert!(inventory.tote_on_robot(pick.robot_id, exited.at).is_none());
            } else {
                assert!(events.iter().any(|event| matches!(event, SimEvent::InventoryMove(moved)
                if moved.tote_id == tote_id && moved.at > exited.at
                    && matches!(moved.to, ToteLocation::InRack { rack_id, .. }
                        if rack_use.get(&rack_id) == Some(&RackUse::OrderDepot)))));
            }
        }
        reports.existing::<BotService>().unwrap().for_each(BotKind::Htm, |_, plan| {
            assert!(plan.carrying_tote().is_none(), "all HTMs finish available and empty");
        });
    }
}

#[test]
fn box_induction_bypass_preserves_order_travel_and_normal_extraction_with_two_htms() {
    let (mut data, mut config) = fixture();
    data.htm_initial_positions.truncate(2);
    data.orders.truncate(3);
    for order in &mut data.orders {
        order.container = OrderContainer::Carton;
        order.exit_path = ExitPath::Dock;
        order.lines.truncate(1);
        order.arrival = None;
    }
    let induction_cells: Vec<_> = data
        .map
        .stations
        .iter()
        .filter(|station| station.station_type == StationType::BoxInduction)
        .flat_map(|station| station.presentation_cells())
        .collect();
    data.map.stations.retain(|station| station.station_type != StationType::BoxInduction);
    let mut definition = config.composition.resolve().unwrap();
    definition.tasks.iter_mut().find(|task| task.name() == "FulfillOrders").unwrap().config["bypass_box_induction"] =
        json!(true);
    config.composition = Composition::Inline(definition);
    let (reports, events) = run(data, &config);
    assert_eq!(events.iter().filter(|event| matches!(event, SimEvent::OrderComplete(_))).count(), 3);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, SimEvent::CycleComplete(cycle)
        if matches!(cycle.kind, CycleKind::OrderExtract { .. }) && cycle.dwell_ms > 0))
            .count(),
        3
    );
    assert!(!events.iter().any(|event| matches!(event, SimEvent::Nav(nav)
        if nav.waypoints.last().is_some_and(|waypoint| induction_cells.contains(&waypoint.cell)))));
    let births: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::ToteCreated(created) if created.tote_type == ToteType::Carton => Some(created),
            _ => None,
        })
        .collect();
    assert_eq!(births.len(), 3);
    for birth in births {
        let loaded = events
            .iter()
            .find_map(|event| match event {
                SimEvent::ToteMoved(moved)
                    if moved.tote_id == birth.tote_id && matches!(moved.new_state, ToteVisualState::OnBot { .. }) =>
                {
                    Some(moved.time)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(loaded, birth.time, "box availability and pickup are instantaneous");
    }
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, SimEvent::OrderStationPhase(phase)
        if phase.phase == OrderStationPhase::Admitted))
            .count(),
        3
    );
    reports.existing::<BotService>().unwrap().for_each(BotKind::Htm, |_, plan| assert!(plan.carrying_tote().is_none()));
}

#[test]
fn a_hundred_unit_inventory_tote_serves_two_orders_and_returns_the_remaining_stock() {
    let (mut data, mut config) = fixture();
    config.fidelity = FidelityMode::CollisionDes;
    let (&(sku, space), _) = data.inventory.first_key_value().unwrap();
    let source_id = ToteId(space.0);
    data.inventory.clear();
    data.inventory.insert((sku, space), 100);
    for item in &mut data.skus {
        item.inventory_units = if item.id == sku { 100 } else { 0 };
    }
    data.orders.truncate(2);
    let order_ids: Vec<_> = data.orders.iter().map(|order| order.id).collect();
    for (order, quantity) in data.orders.iter_mut().zip([3, 4]) {
        order.lines = vec![OrderLine { sku, quantity }];
        order.arrival = Some(SimTime::ZERO);
        order.container = OrderContainer::Tote;
        order.exit_path = ExitPath::Dock;
    }
    let (reports, events) = run(data, &config);
    let inventory = reports.existing::<InventoryService>().unwrap();
    let stock = inventory.tote(source_id).unwrap();
    let mut picked = Vec::new();
    for event in &events {
        let SimEvent::CycleComplete(cycle) = event else { continue };
        let CycleKind::Pick { qty, .. } = cycle.kind else { continue };
        let presentation = cycle.tote_presentation.unwrap();
        assert_eq!(presentation.tote_id, source_id, "both orders use the same physical stock tote");
        let start = SimTime::ms(cycle.sim_time_ms);
        let end = SimTime::ms(cycle.sim_time_ms + cycle.dwell_ms);
        assert_eq!(stock.sku_qty_at(sku, start) - stock.sku_qty_at(sku, end), qty);
        picked.push((presentation.order_id, qty));
    }
    picked.sort_unstable();
    assert_eq!(picked, [(order_ids[0], 3), (order_ids[1], 4)]);
    assert_eq!(stock.sku_qty_at(sku, reports.planned_horizon()), 93);
    assert!(matches!(stock.location(), ToteLocation::InRack { .. }));
    let returns: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::InventoryMove(moved)
                if moved.tote_id == source_id
                    && moved.at > SimTime::ZERO
                    && matches!(moved.to, ToteLocation::InRack { .. }) =>
            {
                Some(moved.at)
            }
            _ => None,
        })
        .collect();
    assert_eq!(returns.len(), 2, "each current conveyor visit returns the same nonempty tote to storage");
    assert!(returns.iter().all(|&at| stock.sku_qty_at(sku, at) >= 93));
}

#[test]
fn mixed_inventory_tote_serves_different_skus_without_losing_its_other_contents() {
    let (mut data, config) = fixture();
    let (&(_, space), _) = data.inventory.first_key_value().unwrap();
    let source_id = ToteId(space.0);
    let [first, second, untouched] = [data.skus[0].id, data.skus[1].id, data.skus[2].id];
    let initial = [(first, 5), (second, 4), (untouched, 2)];
    data.inventory.clear();
    for (sku, quantity) in initial {
        data.inventory.insert((sku, space), quantity);
    }
    for item in &mut data.skus {
        item.inventory_units = initial.iter().find(|(sku, _)| *sku == item.id).map_or(0, |(_, qty)| *qty);
    }
    data.orders.truncate(2);
    let mut expected_picks = FxHashMap::default();
    for (order, (sku, quantity)) in data.orders.iter_mut().zip([(first, 5), (second, 3)]) {
        order.lines = vec![OrderLine { sku, quantity }];
        order.arrival = Some(SimTime::ZERO);
        order.container = OrderContainer::Tote;
        order.exit_path = ExitPath::Dock;
        expected_picks.insert(order.id, (sku, quantity));
    }
    let (reports, events) = run(data, &config);
    let inventory = reports.existing::<InventoryService>().unwrap();
    let stock = inventory.tote(source_id).unwrap();
    let contents_at = |at| stock.contents_at(at).into_iter().collect::<FxHashMap<_, _>>();
    assert_eq!(contents_at(SimTime::ZERO), initial.into_iter().collect());
    let pairs: Vec<_> = reports
        .existing::<Conveyances>()
        .unwrap()
        .machines()
        .iter()
        .flat_map(|(_, conveyor)| conveyor.transfers())
        .collect();
    assert_eq!(pairs.len(), 2);
    let mut actual_picks = FxHashMap::default();
    for event in &events {
        let SimEvent::CycleComplete(cycle) = event else { continue };
        let CycleKind::Pick { sku, qty } = cycle.kind else { continue };
        let presentation = cycle.tote_presentation.unwrap();
        assert_eq!(presentation.tote_id, source_id, "both SKUs must come from the same physical tote");
        assert!(actual_picks.insert(presentation.order_id, (sku, qty)).is_none());
        let start = SimTime::ms(cycle.sim_time_ms);
        let end = SimTime::ms(cycle.sim_time_ms + cycle.dwell_ms);
        let mut expected_stock = contents_at(start);
        *expected_stock.get_mut(&sku).unwrap() -= qty;
        expected_stock.retain(|_, quantity| *quantity > 0);
        assert_eq!(contents_at(end), expected_stock, "a pick changes only its requested SKU");
        let pair = pairs
            .iter()
            .find(|pair| pair.order_robot == cycle.robot_id && pair.start == start && pair.end == end)
            .unwrap();
        assert_eq!(pair.inventory_tote, source_id);
        let order = inventory.tote(pair.order_tote).unwrap();
        assert!(order.contents_at(start).is_empty());
        assert_eq!(order.contents_at(end), [(sku, qty)], "no unrelated stock is transferred to the order");
    }
    assert_eq!(actual_picks, expected_picks);
    let mut completed: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::OrderComplete(done) => Some(done.order_id),
            _ => None,
        })
        .collect();
    completed.sort_unstable();
    let mut expected_completed: Vec<_> = expected_picks.keys().copied().collect();
    expected_completed.sort_unstable();
    assert_eq!(completed, expected_completed);
    let end = reports.planned_horizon();
    assert_eq!(contents_at(end), [(second, 1), (untouched, 2)].into_iter().collect());
    assert!(matches!(stock.location(), ToteLocation::InRack { .. }));
    for (sku, remaining) in [(first, 0), (second, 1), (untouched, 2)] {
        assert_eq!(inventory.sku_reserved_at(sku, end), 0);
        assert_eq!(inventory.sku_inventory_at(sku, end), remaining);
    }
    let returns: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::InventoryMove(moved)
                if moved.tote_id == source_id
                    && moved.at > SimTime::ZERO
                    && matches!(moved.to, ToteLocation::InRack { .. }) =>
            {
                Some(moved.at)
            }
            _ => None,
        })
        .collect();
    assert_eq!(returns.len(), 2, "the same tote is retrieved and returned for each SKU request");
    assert!(returns.iter().all(|&at| stock.sku_qty_at(untouched, at) == 2));
}

#[derive(Default)]
struct InventoryTrace(Mutex<Vec<InventoryEvent>>);

impl InventoryObserver for InventoryTrace {
    fn on_inventory_event(&self, event: &InventoryEvent) -> Result<(), InventoryObserverError> {
        self.0.lock().unwrap().push(event.clone());
        Ok(())
    }
}

#[test]
fn a_buffer_dwell_does_not_block_the_next_order_and_outbound_work_finishes() {
    let (mut data, mut config) = fixture();
    config.order.buffer_dwell_secs = 3600.0;
    data.orders.truncate(2);
    // Two HTMs require the next order to reuse the robot released by buffer
    // storage: retaining that robot until extraction would block fulfillment.
    data.htm_initial_positions.truncate(2);
    let cutoff = SimTime::seconds(36_000);
    let next_arrival = SimTime::seconds(1800);
    data.orders[0].exit_path = ExitPath::Buffer;
    data.orders[0].deadline = Some(cutoff);
    data.orders[1].exit_path = ExitPath::Dock;
    data.orders[1].arrival = Some(next_arrival);
    let buffered_order = data.orders[0].id;
    let next_order = data.orders[1].id;
    let mut expected_contents = FxHashMap::default();
    for line in &data.orders[0].lines {
        *expected_contents.entry(line.sku).or_insert(0_u32) += line.quantity;
    }
    let rack_use = data.rack_use.clone();

    let (tx, rx) = mpsc::channel();
    let pathfinder = common::fh_sipp_for_map(&data.map, &config);
    let (mut scheduler, reports, _) = composition::build(&mut data, &config, pathfinder, Some(Arc::new(tx))).unwrap();
    let inventory = scheduler.states().get::<InventoryService>().unwrap();
    let trace = Arc::new(InventoryTrace::default());
    // Order contents deliberately do not emit available-stock InventoryDelta
    // events, so observe their actual inventory mutations directly.
    let _observer = inventory.add_observer(trace.clone());
    scheduler.run().unwrap();
    reports.check_quiescence_invariants().unwrap();
    let events: Vec<_> = rx.try_iter().collect();

    let picks: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::CycleComplete(cycle)
                if cycle.tote_presentation.is_some_and(|pick| pick.order_id == buffered_order) =>
            {
                Some(cycle)
            }
            _ => None,
        })
        .collect();
    let last_pick = picks.iter().max_by_key(|pick| pick.sim_time_ms + pick.dwell_ms).unwrap();
    let last_pick_end = SimTime::ms(last_pick.sim_time_ms + last_pick.dwell_ms);
    let original_htm = last_pick.robot_id;
    let (tote_id, _) = inventory.tote_on_robot(original_htm, last_pick_end).unwrap();
    let tote = inventory.tote(tote_id).unwrap();
    let contents_at = |at| tote.contents_at(at).into_iter().collect::<FxHashMap<_, _>>();
    assert_eq!(contents_at(last_pick_end), expected_contents, "the buffer receives the filled order tote");

    let moves: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::InventoryMove(moved) if moved.tote_id == tote_id => Some(moved),
            _ => None,
        })
        .collect();
    let stored = moves
        .iter()
        .find(|moved| {
            moved.at > last_pick_end
                && matches!(moved.to, ToteLocation::InRack { rack_id, .. }
            if rack_use.get(&rack_id) == Some(&RackUse::OrderBuffer))
        })
        .expect("the same filled tote physically enters an order-buffer rack");
    let ToteLocation::InRack { rack_id: buffer_rack, .. } = stored.to else { unreachable!() };
    let retrieved = moves
        .iter()
        .filter(|moved| moved.at > stored.at && moved.from == stored.to)
        .min_by_key(|moved| moved.at)
        .expect("the stored tote leaves its buffer slot");
    assert!(stored.at < next_arrival && retrieved.at > next_arrival, "the second order arrives during storage");
    assert!(tote.location_is_through(stored.at, retrieved.at - SimDuration::ms(1), stored.to));
    assert_eq!(contents_at(stored.at), expected_contents);
    assert_eq!(contents_at(retrieved.at), expected_contents);

    let vtm_cycles: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::VtmCycle(cycle) if cycle.tote_id == tote_id && cycle.rack_id == buffer_rack => Some(cycle),
            _ => None,
        })
        .collect();
    assert_eq!(vtm_cycles.len(), 2, "the named tote makes one VTM roundtrip through its buffer rack");
    let store_cycle = vtm_cycles.iter().find(|cycle| cycle.direction == VtmCycleDirection::Return).unwrap();
    let retrieve_cycle = vtm_cycles.iter().find(|cycle| cycle.direction == VtmCycleDirection::Forward).unwrap();
    assert!(
        moves.iter().any(|moved| moved.at > last_pick_end
            && moved.at < stored.at
            && moved.from == ToteLocation::WithRobot { robot_id: original_htm }
            && moved.to == ToteLocation::AtRelay { relay_id: store_cycle.relay_id }),
        "the order HTM deposits the filled tote at the buffer return relay"
    );
    for (cycle, kinds) in [
        (*store_cycle, [VtmOperationKind::PickupRelay, VtmOperationKind::DropStorage]),
        (*retrieve_cycle, [VtmOperationKind::PickupStorage, VtmOperationKind::DropRelay]),
    ] {
        for kind in kinds {
            let operation = events
                .iter()
                .find_map(|event| match event {
                    SimEvent::VtmOperation(operation)
                        if operation.vtm_cycle_id == cycle.vtm_cycle_id && operation.kind == kind =>
                    {
                        Some(operation)
                    }
                    _ => None,
                })
                .expect("both rack and relay forks operate on the buffered tote");
            assert_eq!(operation.tote_id, tote_id);
            assert_eq!(operation.rack_id, buffer_rack);
            assert_eq!(operation.relay_id, cycle.relay_id);
            assert_eq!(operation.payload, TotePayloadState::NonEmpty);
            let at = SimTime::ms(operation.sim_time_ms);
            assert_eq!(contents_at(at), expected_contents);
            if kind == VtmOperationKind::DropRelay {
                let relay_location = ToteLocation::AtRelay { relay_id: cycle.relay_id };
                assert!(moves.iter().any(|moved| moved.at == at && moved.to == relay_location));
                let htm_pickup = moves
                    .iter()
                    .filter(|moved| {
                        moved.at >= at
                            && moved.from == relay_location
                            && matches!(moved.to, ToteLocation::WithRobot { robot_id } if robot_id != cycle.vtm_id)
                    })
                    .min_by_key(|moved| moved.at)
                    .expect("an HTM picks up the same tote from the relay");
                if htm_pickup.at > at {
                    assert!(tote.location_is_through(at, htm_pickup.at - SimDuration::ms(1), relay_location));
                }
            }
        }
    }

    let completed_next = events
        .iter()
        .find_map(|event| match event {
            SimEvent::OrderComplete(done) if done.order_id == next_order => Some(SimTime::ms(done.sim_time_ms)),
            _ => None,
        })
        .expect("second order was picked");
    assert!(
        completed_next > stored.at
            && completed_next < retrieved.at
            && completed_next < stored.at + SimDuration::seconds(3600),
        "machine admission must let the next order finish while the first tote is stored"
    );
    assert!(
        events.iter().any(|event| matches!(event, SimEvent::InventoryMove(moved)
        if moved.tote_id != tote_id && moved.at > stored.at && moved.at < retrieved.at
            && moved.to == ToteLocation::WithRobot { robot_id: original_htm })),
        "the first order HTM is released and carries another tote while the order is in the rack"
    );

    let extracted: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::CycleComplete(cycle) if matches!(cycle.kind, CycleKind::OrderExtract { .. }) => Some(cycle),
            _ => None,
        })
        .collect();
    assert_eq!(extracted.len(), 2, "the root must wait for both outbound children");
    let extraction = extracted
        .iter()
        .find(|cycle| {
            inventory.tote_on_robot(cycle.robot_id, SimTime::ms(cycle.sim_time_ms)).is_some_and(|(id, _)| id == tote_id)
        })
        .expect("the same buffered tote reaches extraction on an HTM");
    let extraction_start = SimTime::ms(extraction.sim_time_ms);
    let extraction_end = extraction_start + SimDuration::ms(extraction.dwell_ms);
    assert!(extraction_start >= stored.at + SimDuration::seconds(3600), "buffer dispatch honors its explicit dwell");
    assert!(extraction_start < cutoff, "order cutoff is metadata, not a dispatch not-before");
    assert!(tote.location_is_through(
        extraction_start,
        extraction_end,
        ToteLocation::WithRobot { robot_id: extraction.robot_id }
    ));
    assert_eq!(contents_at(extraction_start), expected_contents);
    assert_eq!(contents_at(extraction_end - SimDuration::ms(1)), expected_contents);
    assert!(tote.is_empty_at(extraction_end));

    // Contents must survive every intermediate handoff, not merely match at
    // the endpoints after an accidental removal and recreation.
    let trace = trace.0.lock().unwrap();
    let mut extracted_contents = FxHashMap::default();
    for event in trace.iter() {
        if let InventoryEvent::ContentsChanged { tote_id: changed, sku, delta, at, .. } = event {
            if *changed != tote_id || *at <= last_pick_end {
                continue;
            }
            assert!(*at >= extraction_end, "buffered contents changed before extraction completed: {event:?}");
            if *at == extraction_end {
                assert!(*delta < 0);
                *extracted_contents.entry(*sku).or_insert(0_u32) += delta.unsigned_abs();
            }
        }
    }
    assert_eq!(extracted_contents, expected_contents, "extraction removes exactly the originally picked items");
}

#[test]
fn throughput_overrides_release_late_buffer_order_at_t0_and_skip_buffer_storage() {
    let (mut data, mut config) = fixture();
    data.orders.truncate(1);
    let original_arrival = SimTime::seconds(36_000);
    let order_id = data.orders[0].id;
    data.orders[0].arrival = Some(original_arrival);
    data.orders[0].exit_path = ExitPath::Buffer;
    config.order.release_all_at_start = true;
    config.order.direct_extraction = true;
    let rack_use = data.rack_use.clone();

    let (_reports, events) = run(data, &config);
    let first_pick = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::CycleComplete(cycle)
                if cycle.tote_presentation.is_some_and(|presentation| presentation.order_id == order_id) =>
            {
                Some(SimTime::ms(cycle.sim_time_ms))
            }
            _ => None,
        })
        .min()
        .expect("the released order is picked");
    assert!(first_pick < original_arrival, "T0 override left the child gated until {original_arrival}");
    assert!(events.iter().any(|event| matches!(event, SimEvent::CycleComplete(cycle)
        if matches!(cycle.kind, CycleKind::OrderExtract { .. }))));
    assert!(!events.iter().any(|event| matches!(event, SimEvent::InventoryMove(moved)
        if matches!(moved.to, ToteLocation::InRack { rack_id, .. }
            if rack_use.get(&rack_id) == Some(&RackUse::OrderBuffer)))));
}

#[test]
fn order_approach_overlaps_known_inventory_conveyor_travel() {
    let (mut data, config) = fixture();
    data.orders.truncate(1);
    data.orders[0].container = OrderContainer::Carton;
    data.orders[0].exit_path = ExitPath::Dock;
    data.orders[0].lines.truncate(1);
    data.orders[0].lines[0].quantity = 1;
    let (reports, events) = run(data, &config);
    let conveyors = reports.existing::<Conveyances>().unwrap();
    let (layout, conveyor) = &conveyors.machines()[0];
    let pair = conveyor.transfers()[0];
    let presentation = layout.presentations.iter().find(|side| side.station_id == pair.station).unwrap();
    let inventory_ready = events
        .iter()
        .find_map(|event| match event {
            SimEvent::ToteTransit(travel)
                if travel.tote_id == pair.inventory_tote && travel.to_cell == presentation.inventory_cell =>
            {
                Some(travel.end)
            }
            _ => None,
        })
        .expect("the source tote travels to its selected presentation");
    let approach = events
        .iter()
        .find_map(|event| match event {
            SimEvent::Nav(nav)
                if nav.robot_id == pair.order_robot
                    && nav.waypoints.last().is_some_and(|end| end.cell == presentation.cell) =>
            {
                Some(nav)
            }
            _ => None,
        })
        .expect("the target HTM approaches the selected presentation");
    assert!(
        approach.time < inventory_ready,
        "order approach starts at {}, after inventory becomes ready at {inventory_ready}",
        approach.time
    );
    let arrival = approach.waypoints.last().unwrap().action.end_time();
    assert_eq!(pair.start, inventory_ready.max(arrival), "picking waits for both physical arrivals");
    assert_eq!(pair.end - pair.start, SimDuration::from_secs(config.operator.time_per_pick_secs));
    assert_eq!(conveyor.transfers().len(), 1);
    assert_eq!(events.iter().filter(|event| matches!(event, SimEvent::OrderComplete(_))).count(), 1);
}

#[test]
fn presentation_approach_honors_existing_traffic_and_both_arrivals() {
    let (mut data, config) = fixture();
    data.orders.truncate(1);
    data.orders[0].container = OrderContainer::Carton;
    data.orders[0].exit_path = ExitPath::Dock;
    data.orders[0].lines.truncate(1);
    data.orders[0].lines[0].quantity = 1;
    let pathfinder = common::fh_sipp_for_map(&data.map, &config);
    let (tx, rx) = mpsc::channel();
    let (mut scheduler, reports, _) = composition::build(&mut data, &config, pathfinder, Some(Arc::new(tx))).unwrap();
    let navigation = scheduler.states().get::<Navigation>().unwrap();
    let free_at = SimTime::seconds(100);
    // Pre-existing traffic occupies both target cells. Its finite reservations
    // are inputs to this route test, independent of the order being planned.
    let _traffic: Vec<_> = data.map.conveyor_machines[0]
        .presentations
        .iter()
        .enumerate()
        .map(|(index, side)| {
            navigation
                .oracle
                .commit_path(
                    RobotId(1000 + index as u32),
                    &CommitPlan::single(ReservationRequest::spanning(side.cell, SimTime::ZERO, Some(free_at))),
                    None,
                )
                .unwrap()
        })
        .collect();
    scheduler.run().unwrap();
    reports.check_quiescence_invariants().unwrap();
    let events: Vec<_> = rx.try_iter().collect();
    let conveyances = reports.existing::<Conveyances>().unwrap();
    let (layout, conveyor) = &conveyances.machines()[0];
    let pair = conveyor.transfers()[0];
    let side = layout.presentations.iter().find(|side| side.station_id == pair.station).unwrap();
    let approach = events
        .iter()
        .find_map(|event| match event {
            SimEvent::Nav(nav)
                if nav.robot_id == pair.order_robot
                    && nav.waypoints.last().is_some_and(|waypoint| waypoint.cell == side.cell) =>
            {
                Some(nav)
            }
            _ => None,
        })
        .unwrap();
    let target = approach.waypoints.last().unwrap();
    let target_hold = navigation
        .oracle
        .reservations_for_cell(side.cell)
        .into_iter()
        .find(|hold| hold.robot() == pair.order_robot)
        .expect("the order approach reserves its presentation cell");
    assert!(target_hold.window().start() >= free_at, "the target cell is occupied until existing traffic clears");
    let inventory_ready = events
        .iter()
        .find_map(|event| match event {
            SimEvent::ToteTransit(travel)
                if travel.tote_id == pair.inventory_tote && travel.to_cell == side.inventory_cell =>
            {
                Some(travel.end)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(pair.start, inventory_ready.max(target.action.end_time()));
    assert_eq!(pair.end - pair.start, SimDuration::from_secs(config.operator.time_per_pick_secs));
    assert_eq!(conveyor.transfers().len(), 1);
    assert_eq!(events.iter().filter(|event| matches!(event, SimEvent::OrderComplete(_))).count(), 1);
}

#[test]
fn fulfillment_divides_one_conveyor_circuit_without_removing_pick_or_fork_dwell() {
    let (mut data, config) = fixture();
    data.orders.truncate(1);
    data.orders[0].lines.truncate(1);
    data.orders[0].exit_path = ExitPath::Dock;
    let (reports, events) = run(data, &config);
    let machines = reports.existing::<Conveyances>().unwrap();
    let (layout, conveyor) = &machines.machines()[0];
    let visit = conveyor.visits()[0];
    let pair = conveyor.transfers()[0];
    let boarded = visit.boarded_at.unwrap();
    let TotePhase::Collected { at: collected, robot } = visit.phase else { panic!("inventory tote was not collected") };
    let nav_to = |cell, carrying| {
        events
            .iter()
            .find_map(|event| match event {
                SimEvent::Nav(nav)
                    if nav.robot_id == robot
                        && nav.carrying_cargo == carrying
                        && nav.waypoints.last().is_some_and(|waypoint| waypoint.cell == cell) =>
                {
                    Some(nav)
                }
                _ => None,
            })
            .expect("the retained HTM physically visits the conveyor port")
    };
    let delivered = nav_to(layout.dropoff_cell.unwrap(), true);
    let returning = nav_to(layout.collection_cell.unwrap(), false);
    let delivery_arrival = delivered.waypoints.last().unwrap().action.end_time();
    let collection_arrival = returning.waypoints.last().unwrap().action.end_time();
    assert_eq!(boarded, delivery_arrival + config.htm.conveyor_deposit_time());
    let inventory_ready = boarded + SimDuration::ms(22_500);
    assert_eq!(conveyor.visits().len(), 1);
    assert_eq!(conveyor.transfers().len(), 1);
    assert_eq!(conveyor.occupancy_at(inventory_ready - SimDuration::ms(1)).presentations, 0);
    assert_eq!(conveyor.occupancy_at(inventory_ready).presentations, 1);
    let presentation =
        layout.presentations.iter().find(|presentation| presentation.station_id == pair.station).unwrap();
    let order_arrival = events
        .iter()
        .find_map(|event| match event {
            SimEvent::Nav(nav)
                if nav.robot_id == pair.order_robot
                    && nav.carrying_cargo
                    && nav.waypoints.last().is_some_and(|waypoint| waypoint.cell == presentation.cell) =>
            {
                Some(nav.waypoints.last().unwrap().action.end_time())
            }
            _ => None,
        })
        .expect("the order HTM physically reaches the selected presentation");
    assert_eq!(pair.start, inventory_ready.max(order_arrival));
    assert_eq!(pair.end, pair.start + SimDuration::from_secs(config.operator.time_per_pick_secs));
    let returned = pair.end + SimDuration::ms(22_500);
    assert_eq!(collected, collection_arrival.max(returned) + config.htm.conveyor_pickup_time());
}

/// The two handoffs above happen at a station's belt, not at a relay's diving
/// board, so a spec's belt figures govern them, while the collector's drop at
/// the relay that returns the tote to stock keeps the relay figure. The run
/// states a relay deposit (3 s) that differs from the belt deposit (8 s), so a
/// relay drop that took the belt's time would show.
#[test]
fn the_belt_figures_govern_the_belt_handoffs_and_the_relay_drop_keeps_its_own() {
    let (mut data, mut config) = fixture();
    data.orders.truncate(1);
    data.orders[0].lines.truncate(1);
    data.orders[0].exit_path = ExitPath::Dock;
    config.htm.deposit_secs = 3.0;
    config.htm.conveyor_deposit_secs = Some(8.0);
    config.htm.conveyor_pickup_secs = Some(4.61);
    let (reports, events) = run(data, &config);
    let machines = reports.existing::<Conveyances>().unwrap();
    let (layout, conveyor) = &machines.machines()[0];
    let visit = conveyor.visits()[0];
    let boarded = visit.boarded_at.unwrap();
    let TotePhase::Collected { at: collected, robot } = visit.phase else { panic!("inventory tote was not collected") };
    let arrival_at = |cell, carrying| {
        events
            .iter()
            .find_map(|event| match event {
                SimEvent::Nav(nav)
                    if nav.robot_id == robot
                        && nav.carrying_cargo == carrying
                        && nav.waypoints.last().is_some_and(|waypoint| waypoint.cell == cell) =>
                {
                    Some(nav.waypoints.last().unwrap().action.end_time())
                }
                _ => None,
            })
            .expect("the retained HTM physically visits the conveyor port")
    };
    assert_eq!(boarded - arrival_at(layout.dropoff_cell.unwrap(), true), SimDuration::ms(8_000));
    // The collector waits on the collection cell; its pickup starts when the tote is back.
    let returned = events
        .iter()
        .find_map(|event| match event {
            SimEvent::ToteTransit(travel)
                if travel.tote_id == visit.tote && Some(travel.to_cell) == layout.collection_cell =>
            {
                Some(travel.end)
            }
            _ => None,
        })
        .expect("the tote rides back to the collection cell");
    let pickup_start = arrival_at(layout.collection_cell.unwrap(), false).max(returned);
    assert_eq!(collected - pickup_start, SimDuration::ms(4_610));

    // The collector then carries the tote to a relay. Its drop starts at the
    // later of its arrival and the slot opening, and the fork-Put it reports
    // says which.
    let to_relay = events
        .iter()
        .find_map(|event| match event {
            SimEvent::Nav(nav) if nav.robot_id == robot && nav.carrying_cargo && nav.time >= collected => Some(nav),
            _ => None,
        })
        .expect("the collector carries the tote back to a relay");
    let stop = to_relay.waypoints.last().unwrap();
    let (relay_cell, stored) = events
        .iter()
        .find_map(|event| match event {
            SimEvent::ToteMoved(moved) if moved.tote_id == visit.tote && moved.time > collected => {
                match moved.new_state {
                    ToteVisualState::AtRelay { cell_id } => Some((cell_id, moved.time)),
                    _ => None,
                }
            }
            _ => None,
        })
        .expect("the collected tote is set down at a relay");
    assert_eq!(stop.cell.0, relay_cell, "the tote is set down where the collector stopped");
    let arrival = stop.action.end_time();
    let fork_put = events
        .iter()
        .find_map(|event| match event {
            SimEvent::HtmRelayDeposit(put) if put.robot_id == robot && (arrival..=stored).contains(&put.start) => {
                Some(put)
            }
            _ => None,
        })
        .expect("the collector reports its fork-Put at the relay");
    assert_eq!(fork_put.start - arrival, fork_put.wait, "the wait runs from the arrival to the fork-Put");
    assert_eq!(
        (fork_put.end, stored - fork_put.start),
        (stored, SimDuration::seconds(3)),
        "the relay drop takes the relay figure"
    );
}

#[test]
fn actual_presentations_update_trip_context_for_both_pick_sides_and_extraction() {
    let (world, scenario, mut config) = common::load_world_spec("minimal_easyloop_flow_spec.json");
    config.fidelity = FidelityMode::CollisionDes;
    let (events, _, metrics) = common::run_world_capture_all(&world, &scenario, config);
    let mut trips = FxHashMap::default();
    let mut last_pick_station = FxHashMap::default();
    let mut used_sides = FxHashSet::default();
    let mut repeated_picks = 0;
    let mut extractions = 0;
    for event in events {
        match event {
            SimEvent::TripStart(trip) => {
                trips.insert(trip.robot_id, trip);
            }
            SimEvent::CycleComplete(cycle) => {
                let trip = trips.get(&cycle.robot_id).expect("station work has a declared approach");
                assert_eq!(trip.to, TripEndpoint::Station(cycle.station_id));
                assert_eq!(trip.presentation_id, cycle.presentation_id);
                let expected = if matches!(cycle.kind, CycleKind::OrderExtract { .. }) {
                    extractions += 1;
                    HtmJournalEndpoint::ExtractionStation(cycle.station_id)
                } else {
                    let order = cycle.tote_presentation.expect("pick retains inventory provenance").order_id;
                    if let Some(previous) = last_pick_station.insert(order, cycle.station_id) {
                        assert_eq!(previous, cycle.station_id, "an order remains in its assigned waiting block");
                        repeated_picks += 1;
                    }
                    used_sides.insert(cycle.station_id);
                    HtmJournalEndpoint::PickStation(cycle.station_id)
                };
                let arrival = metrics
                    .htm_leg_log
                    .iter()
                    .filter(|leg| {
                        leg.robot_id == cycle.robot_id
                            && leg.activity == Activity::Travel
                            && leg.start.as_ms() + leg.duration.as_ms() <= cycle.sim_time_ms
                    })
                    .max_by_key(|leg| leg.start + leg.duration)
                    .expect("observer records the station approach");
                assert_eq!(arrival.to, expected);
                assert_eq!(arrival.presentation_id, cycle.presentation_id.map(|id| id.0));
            }
            _ => {}
        }
    }
    assert_eq!(used_sides.len(), 2, "trip attribution covers both independent presentations");
    assert!(repeated_picks > 0, "trip attribution also covers re-presentation on the assigned side");
    assert!(extractions > 0);
}

struct FutureStock(Vec<(SkuId, ToteId, u32, SimTime)>);

#[task]
#[derive(Display)]
#[display("ScheduleFutureStock")]
struct ScheduleFutureStock {
    inventory: Arc<InventoryService>,
    stock: Arc<FutureStock>,
}

#[async_trait(?Send)]
impl Task for ScheduleFutureStock {
    fn create(ctx: &TaskContext) -> Result<Self, PlanError> {
        Ok(Self { inventory: ctx.state()?, stock: ctx.states().configuration()? })
    }
    type Proposal = EmptyProposal;
    type Schedule = ();
    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }
    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<(), PlanError> {
        assert_eq!(ctx.barrier().floor(), SimTime::ZERO);
        for &(sku, tote, quantity, at) in &self.stock.0 {
            self.inventory.tote(tote)?.add_item(sku, quantity, at.as_ms())?;
            for _ in 0..quantity {
                self.inventory.record_inventory_added(sku, at);
            }
        }
        Ok(())
    }
}

#[test]
fn recorded_future_stock_wakes_fulfillment_and_satisfies_all_duplicate_lines() {
    let (mut data, mut config) = fixture();
    let mut contents: Vec<_> =
        data.inventory.iter().map(|(&(sku, space), &qty)| ((sku, ToteId(space.0)), qty)).collect();
    data.inventory.clear();
    contents.sort_unstable_by_key(|&((sku, tote), _)| (sku, tote));
    let first = contents[0].0.0;
    let second = contents.iter().find(|((sku, _), _)| *sku != first).unwrap().0.0;
    assert!(contents.iter().filter(|((sku, _), _)| *sku == first).count() >= 2);
    data.orders.truncate(1);
    data.orders[0].arrival = None;
    data.orders[0].exit_path = ExitPath::Dock;
    data.orders[0].lines = vec![
        OrderLine { sku: first, quantity: 1 },
        OrderLine { sku: first, quantity: 1 },
        OrderLine { sku: second, quantity: 1 },
    ];
    let mut first_seen = false;
    let stock = contents
        .into_iter()
        .map(|((sku, tote), quantity)| {
            let at = if sku == first {
                if first_seen {
                    300
                } else {
                    first_seen = true;
                    100
                }
            } else if sku == second {
                200
            } else {
                100
            };
            (sku, tote, quantity, SimTime::seconds(at))
        })
        .collect();
    let mut definition = config.composition.resolve().unwrap();
    definition
        .tasks
        .push(serde_json::from_value(json!({ "type": "ScheduleFutureStock", "after": ["FillInventory"] })).unwrap());
    definition
        .tasks
        .iter_mut()
        .find(|task| task.name() == "FulfillOrders")
        .unwrap()
        .after
        .push("ScheduleFutureStock".into());
    config.composition = Composition::Inline(definition);
    let pathfinder = common::fh_sipp_for_map(&data.map, &config);
    let (tx, rx) = mpsc::channel();
    let (mut scheduler, reports, _) = composition::build(&mut data, &config, pathfinder, Some(Arc::new(tx))).unwrap();
    scheduler.states().configure(FutureStock(stock)).unwrap();
    scheduler.run().unwrap();
    reports.check_quiescence_invariants().unwrap();
    let inventory = reports.existing::<InventoryService>().unwrap();
    assert_eq!(inventory.sku_reserved_at(first, SimTime::seconds(299)), 0);
    assert_eq!(inventory.sku_reserved_at(first, SimTime::seconds(300)), 2);
    assert_eq!(inventory.sku_reserved_at(second, SimTime::seconds(300)), 1);
    let events: Vec<_> = rx.try_iter().collect();
    assert_eq!(events.iter().filter(|event| matches!(event, SimEvent::OrderComplete(_))).count(), 1);
    assert!(
        events
            .iter()
            .filter_map(|event| match event {
                SimEvent::Nav(nav) => Some(nav.time),
                _ => None,
            })
            .all(|at| at >= SimTime::seconds(300))
    );
}

#[test]
fn inventory_retrieval_tracks_future_order_container_readiness() {
    let (mut data, mut config) = fixture();
    data.htm_initial_positions.truncate(2);
    data.orders.truncate(1);
    data.orders[0].container = OrderContainer::Carton;
    data.orders[0].exit_path = ExitPath::Dock;
    data.orders[0].lines.truncate(1);
    data.orders[0].lines[0].quantity = 1;
    config.order.release_all_at_start = true;
    let induction =
        data.map.stations.iter().find(|s| s.station_type == StationType::BoxInduction).unwrap().delivery_cell();
    let (tx, rx) = mpsc::channel();
    let pathfinder = common::fh_sipp_for_map(&data.map, &config);
    let (mut scheduler, reports, _) = composition::build(&mut data, &config, pathfinder, Some(Arc::new(tx))).unwrap();
    let navigation = scheduler.states().get::<Navigation>().unwrap();
    let occupied_until = SimTime::seconds(200);
    let _traffic = navigation
        .oracle
        .commit_path(
            RobotId(1000),
            &CommitPlan::single(ReservationRequest::spanning(induction, SimTime::ZERO, Some(occupied_until))),
            None,
        )
        .unwrap();
    let bound = FetchBounds::new(&data, &navigation, &config)
        .unwrap()
        .for_machine(&navigation, &data.map.conveyor_machines[0])
        .unwrap()
        .total();
    scheduler.run().unwrap();
    reports.check_quiescence_invariants().unwrap();
    let events: Vec<_> = rx.try_iter().collect();
    let birth = events
        .iter()
        .find_map(|event| match event {
            SimEvent::ToteCreated(tote) if tote.tote_type == ToteType::Carton => Some(tote.time),
            _ => None,
        })
        .unwrap();
    assert!(birth >= occupied_until);
    let release = (birth + config.htm.conveyor_pickup_time()).saturating_sub(bound);
    assert!(release > SimTime::ZERO);
    assert!(
        events.iter().any(|event| matches!(event, SimEvent::InventoryVisitPhase(phase)
        if phase.phase == InventoryVisitPhase::Released && phase.at == SimTime::ZERO)),
        "logical CONWIP can precede physical retrieval"
    );
    let forward = events
        .iter()
        .find_map(|event| match event {
            SimEvent::VtmCycle(cycle) if cycle.direction == VtmCycleDirection::Forward => {
                Some(SimTime::ms(cycle.sim_time_ms))
            }
            _ => None,
        })
        .unwrap();
    assert!(
        forward >= release,
        "physical retrieval started at {forward}, before its consumer-derived release {release}"
    );
    assert!(forward < birth, "known future readiness still permits anticipatory retrieval");
    assert_eq!(events.iter().filter(|event| matches!(event, SimEvent::OrderComplete(_))).count(), 1);
}

#[test]
fn conveyor_animation_intervals_agree_with_physical_picks_and_collection() {
    let (mut data, mut config) = fixture();
    config.fidelity = FidelityMode::CollisionDes;
    data.orders.truncate(2);
    let (reports, events) = run(data, &config);
    let transits: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::ToteTransit(travel) => Some(travel),
            _ => None,
        })
        .collect();
    let conveyors = reports.existing::<Conveyances>().unwrap();
    let mut paired_picks = 0;
    for (machine, conveyor) in conveyors.machines() {
        for pair in conveyor.transfers() {
            paired_picks += 1;
            let seat =
                machine.presentations.iter().find(|seat| seat.station_id == pair.station).unwrap().inventory_cell;
            let incoming = transits
                .iter()
                .filter(|travel| {
                    travel.tote_id == pair.inventory_tote && travel.to_cell == seat && travel.end <= pair.start
                })
                .max_by_key(|travel| travel.end)
                .expect("recorded inbound travel before every paired pick");
            assert_eq!(Some(incoming.from_cell), machine.dropoff_cell);
            assert!(incoming.start < incoming.end);
            let outgoing = transits
                .iter()
                .find(|travel| travel.tote_id == pair.inventory_tote && travel.start == pair.end)
                .expect("return travel starts when the physical pick ends");
            assert_eq!(outgoing.from_cell, seat);
            assert_eq!(Some(outgoing.to_cell), machine.collection_cell);
            assert!(outgoing.start < outgoing.end);
            assert!(conveyor.visits().iter().any(|visit| visit.tote == pair.inventory_tote
                && visit.boarded_at == Some(incoming.start)
                && matches!(visit.phase, TotePhase::Collected { at, .. } if at >= outgoing.end)));
            for (at, cell) in [(incoming.end, seat), (outgoing.end, outgoing.to_cell)] {
                assert!(events.iter().any(|event| matches!(event, SimEvent::ToteMoved(moved)
                    if moved.tote_id == pair.inventory_tote && moved.time == at
                    && moved.new_state == (ToteVisualState::OnConveyor { cell_id: cell.0 }))));
            }
        }
    }
    assert!(paired_picks > 0);
    assert_eq!(transits.len(), 2 * paired_picks);
}

#[test]
fn inventory_conwip_spans_retrieval_and_picking_independently_of_order_waiting_capacity() {
    let mut peaks = Vec::new();
    for (queue_factor, mixed_containers) in [(0.0, false), (3.0, false), (3.0, true)] {
        let (mut data, mut config) = fixture();
        config.queue_factor = queue_factor;
        config.order.max_inflight_picks_override = None;
        config.order.release_all_at_start = true;
        config.order.direct_extraction = true;
        if mixed_containers {
            for (index, order) in data.orders.iter_mut().enumerate() {
                order.container = [OrderContainer::Tote, OrderContainer::Carton, OrderContainer::Tray][index % 3];
            }
            let mut definition = config.composition.resolve().unwrap();
            definition.tasks.iter_mut().find(|task| task.name() == "FillInventory").unwrap().config["order_trays"] =
                json!(2);
            config.composition = Composition::Inline(definition);
        }
        for machine in &mut data.map.conveyor_machines {
            for presentation in &mut machine.presentations {
                presentation.waiting_cells.truncate(1);
            }
            machine.waiting_cells = machine
                .presentations
                .iter()
                .flat_map(|presentation| presentation.waiting_cells.iter().copied())
                .collect();
        }
        let (reports, events) = run(data, &config);
        assert_eq!(events.iter().filter(|event| matches!(event, SimEvent::OrderComplete(_))).count(), 8);
        let mut visits = FxHashMap::default();
        let mut changes = Vec::new();
        for event in &events {
            if let SimEvent::InventoryVisitPhase(phase) = event {
                visits.entry((phase.order_id, phase.pick_index)).or_insert_with(Vec::new).push(phase);
                match phase.phase {
                    InventoryVisitPhase::Released => changes.push((phase.at, 1_i32)),
                    InventoryVisitPhase::PickCompleted => changes.push((phase.at, -1_i32)),
                    _ => {}
                }
            }
        }
        assert!(!visits.is_empty());
        for ((order, index), visit) in visits {
            let at = |phase| {
                let matching: Vec<_> = visit.iter().filter(|v| v.phase == phase).collect();
                assert_eq!(matching.len(), 1, "{order:?} visit {index} must report one {phase:?}");
                matching[0].at
            };
            assert!(at(InventoryVisitPhase::Released) <= at(InventoryVisitPhase::Loaded));
            assert!(at(InventoryVisitPhase::Loaded) <= at(InventoryVisitPhase::Boarded));
            assert!(at(InventoryVisitPhase::Boarded) < at(InventoryVisitPhase::PickCompleted));
        }
        changes.sort_unstable();
        let mut count = 0;
        let mut peak = 0;
        for (_, delta) in changes {
            count += delta;
            assert!(count >= 0);
            peak = peak.max(count);
        }
        assert_eq!(count, 0, "every admitted inventory visit finishes its pick");
        peaks.push(peak);
        let mut residents = FxHashMap::default();
        let mut phases: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                SimEvent::OrderStationPhase(phase) => Some(phase),
                _ => None,
            })
            .collect();
        phases.sort_by_key(|phase| (phase.at, phase.phase != OrderStationPhase::AreaExited));
        for phase in phases {
            let count = residents.entry(phase.presentation_id).or_insert(0_i32);
            match phase.phase {
                OrderStationPhase::Admitted => *count += 1,
                OrderStationPhase::AreaExited => *count -= 1,
                _ => {}
            }
            assert!((0..=1).contains(count), "each physical order area still has one return right");
        }
        assert!(residents.values().all(|&count| count == 0));
        assert!(
            reports
                .existing::<Conveyances>()
                .unwrap()
                .machines()
                .iter()
                .all(|(_, conveyor)| conveyor.unfinished() == 0)
        );
    }
    assert!(peaks[0] <= 3, "qf=0 retains only the admission law's base margin");
    assert!(
        peaks[1..].iter().all(|&peak| peak > 5),
        "inventory WIP exceeds both the two physical waiting places and the old five-worker fleet clamp: {peaks:?}"
    );
}
