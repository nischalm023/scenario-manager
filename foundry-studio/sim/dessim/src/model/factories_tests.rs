use std::path::PathBuf;
use std::sync::mpsc;

use tokio::runtime::Builder;
use tokio::task::yield_now;

use super::*;
use crate::events::SimEvent;
use crate::model::bot::BotKind;
use crate::model::relay::ToteSearchCriteria;
use crate::model::{RackId, RobotId, SkuId, ToteDesignation, ToteId, ToteLocation, ToteType};
use crate::pathfinding::{CommitPlan, FHSippPathfinder, MotionModel, ReservationRequest, holding_cells};
use crate::robots::runtime::FloorRobotModel;
use crate::runner::resolve_scenario_with_racking;
use crate::scenario::ScenarioConfig;
use crate::world_spec;

fn fixture() -> (WorldInputs, mpsc::Receiver<SimEvent>) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal_pick_only_spec.json");
    let mut loaded = world_spec::load(&path).expect("world fixture");
    // Use a fulfillable workload on this small map; the test exercises manager
    // construction, while the fixture's ten-SKU stock leaves some orders short.
    let ScenarioConfig::Generated(generated) = &mut loaded.scenario else { panic!("generated fixture") };
    generated.num_skus = 1;
    let config = loaded.settings;
    let data = resolve_scenario_with_racking(loaded.world.map, loaded.world.racking, &loaded.scenario, &config)
        .expect("scenario input");
    assert!(!data.htm_initial_positions.is_empty());
    assert!(!data.tote_placements.is_empty());
    let holding = holding_cells(&data.map);
    let pathfinder = Arc::new(FHSippPathfinder::new(
        &data.map,
        (config.vtm.speed * 1000.0) as u64,
        SimDuration::seconds(1),
        holding,
    ));
    let (sender, receiver) = mpsc::channel();
    (
        WorldInputs {
            data: Arc::new(data),
            config,
            pathfinder,
            event_sink: Some(Arc::new(sender)),
            clock: Mut::new(SimTime::ZERO),
        },
        receiver,
    )
}

#[test]
fn constructing_equipment_does_not_start_physical_work() {
    Builder::new_current_thread().build().unwrap().block_on(async {
        let (inputs, events) = fixture();
        let states = States::default();
        states.configure(inputs).unwrap();
        let relay = states.get::<RelayService>().unwrap();
        let inventory = states.get::<InventoryService>().unwrap();
        let bots = states.get::<BotService>().unwrap();
        let available = states.get::<AvailableHtmRegistry>().unwrap();
        let navigation = states.get::<Navigation>().unwrap();
        let parking = states.get::<ParkingRegistry>().unwrap();
        states.get::<EventJournal>().unwrap();
        states.get::<StationSet>().unwrap();
        states.get::<StationService>().unwrap();
        states.get::<StationFetchBounds>().unwrap();
        states.get::<StationArrivalDiagnostics>().unwrap();

        assert!(bots.is_empty(), "fleet registration belongs to setup tasks");
        assert_eq!(inventory.tote_count(), 0, "inventory fill belongs to setup tasks");
        assert_eq!(available.len(), 0, "construction must not make robots available");
        assert!(relay.htm_access_cells().next().is_some(), "topology is available before work begins");
        for cell in navigation.map.graph.cells() {
            assert!(navigation.oracle.reservations_for_cell(cell.id).is_empty());
            if parking.is_candidate(cell.id) {
                let free = parking.find_candidates(cell.id, cell.world_pos, SimTime::ZERO, RobotId(u32::MAX));
                assert!(free.contains(&cell.id), "factory must not book a parking bay");
            }
        }
        assert!(events.try_iter().next().is_none(), "factories must not announce simulated work");
    });
}

#[test]
fn late_managers_share_inventory_and_robot_state() {
    Builder::new_current_thread().build().unwrap().block_on(async {
        let (inputs, _events) = fixture();
        let cell = inputs.data.htm_initial_positions[0];
        let robot = RobotId(inputs.data.vtm_zone_assignments.len() as u32);
        let states = States::default();
        states.configure(inputs).unwrap();
        let bots = states.get::<BotService>().unwrap();
        let inventory = states.get::<InventoryService>().unwrap();
        let navigation = states.get::<Navigation>().unwrap();
        let tote = ToteId(1000);
        bots.register(robot, BotKind::Htm, cell).unwrap();
        inventory
            .register_tote(
                tote,
                ToteType::Tote,
                ToteDesignation::Inventory,
                ToteLocation::WithRobot { robot_id: robot },
            )
            .unwrap();

        yield_now().await;
        let later = states.clone();
        let available = later.get::<AvailableHtmRegistry>().unwrap();
        let hold = navigation
            .oracle
            .commit_path(robot, &CommitPlan::single(ReservationRequest::spanning(cell, SimTime::ZERO, None)), None)
            .unwrap()
            .pop()
            .unwrap();
        available.register(hold, SimTime::ZERO, Default::default(), None).unwrap();
        let carried = available.peek_matching(|htm| htm.robot() == robot);
        assert_eq!(carried.len(), 1);
        assert_eq!(carried[0].tote.as_ref().unwrap().tote_id, tote, "late registry must use existing inventory");
        assert!(Arc::ptr_eq(&bots, &later.get::<BotService>().unwrap()));
        assert!(Arc::ptr_eq(&inventory, &later.get::<InventoryService>().unwrap()));
        assert!(Arc::ptr_eq(&available, &states.get::<AvailableHtmRegistry>().unwrap()));
        assert_eq!(later.get::<BotService>().unwrap().bot_kind(robot), Some(BotKind::Htm));
    });
}

#[test]
fn separate_runs_have_separate_managers() {
    // BotService builds its HTMs' drives from the run's configuration, so each run gets its inputs.
    let run = || {
        let states = States::default();
        states.configure(fixture().0).unwrap();
        states.get::<BotService>().unwrap()
    };
    let (first, second) = (run(), run());
    assert!(!Arc::ptr_eq(&first, &second));
}

/// The inventory reports through the run's journal rather than straight into
/// the event stream, so a fact it reports stamped behind B is refused as it is
/// emitted, as one a task emits is.
#[test]
#[should_panic(expected = "stamped at 5s was emitted after B had reached 20s")]
fn the_inventory_reports_through_the_journal() {
    let (inputs, _events) = fixture();
    let states = States::default();
    states.configure(inputs).unwrap();
    let inventory = states.get::<InventoryService>().unwrap();
    let rack = ToteLocation::InRack { rack_id: RackId(1), z_index: 0 };
    inventory.register_tote(ToteId(1000), ToteType::Tote, ToteDesignation::Inventory, rack).unwrap();
    states.get::<EventJournal>().unwrap().barrier_advanced(SimTime::ms(20_000));
    inventory.tote(ToteId(1000)).unwrap().add_item(SkuId(1), 1, 5_000).unwrap();
}

#[test]
fn relay_requested_after_inventory_fill_finds_the_same_stock() {
    Builder::new_current_thread().build().unwrap().block_on(async {
        let mut choices = Vec::new();
        for construct_early in [true, false] {
            let (inputs, _events) = fixture();
            let data = inputs.data.clone();
            let states = States::default();
            states.configure(inputs).unwrap();
            if construct_early {
                states.get::<RelayService>().unwrap();
            }
            let inventory = states.get::<InventoryService>().unwrap();
            for placement in &data.tote_placements {
                inventory
                    .register_tote(placement.tote_id, ToteType::Tote, placement.designation, placement.location)
                    .unwrap();
            }
            for (&(sku, space), &qty) in &data.inventory {
                inventory.tote(ToteId(space.0)).unwrap().add_item(sku, qty, 0).unwrap();
            }
            inventory.seed_inventory_from_totes();
            yield_now().await;
            let relay = states.get::<RelayService>().unwrap();
            let sku = data.inventory.keys().next().unwrap().0;
            let target = data.station_data[0].presentation_cell;
            let selection = relay
                .select_relay_for_retrieve(ToteSearchCriteria::ContainsSku { sku }, target, SimTime::ZERO)
                .expect("filled inventory is visible to a late relay manager");
            choices.push((selection.relay_id, selection.recommendation, selection.start, selection.completion_time));
        }
        assert_eq!(choices[0], choices[1]);
    });
}

#[test]
fn station_lookback_uses_catalog_cruise_and_an_independent_planning_allowance() {
    let bound = |speed, acceleration, kind, actual_vtm_handling, extra_allowance| {
        let (mut inputs, _) = fixture();
        let mut config = inputs.config.clone();
        config.htm.loaded_speed = speed;
        config.htm.loaded_accel = acceleration;
        config.motion_model = kind;
        let model = Arc::new(FloorRobotModel::legacy(&config));
        let data = Arc::make_mut(&mut inputs.data);
        data.robot_models.htm.fill(model);
        for model in &mut data.robot_models.vtm {
            Arc::make_mut(model).handling.pick_secs = actual_vtm_handling;
        }
        let station = data.station_data[0].station_id;
        inputs.config.vtm.load_time_secs += extra_allowance;
        let states = States::default();
        states.configure(inputs).unwrap();
        states.get::<StationFetchBounds>().unwrap().f_max(station)
    };
    let ordinary = bound(4.0, 1.0, MotionModel::HtmKinematic, 5.0, 0.0);
    assert_eq!(ordinary, bound(4.0, 0.01, MotionModel::HtmKinematic, 5.0, 0.0));
    assert_eq!(ordinary, bound(4.0, 1.0, MotionModel::ConstantSpeed, 5.0, 0.0));
    assert_eq!(ordinary, bound(4.0, 1.0, MotionModel::HtmKinematic, 500.0, 0.0));
    assert!(bound(2.0, 1.0, MotionModel::HtmKinematic, 5.0, 0.0) > ordinary);
    assert_eq!(bound(4.0, 1.0, MotionModel::HtmKinematic, 5.0, 3.0), ordinary + SimDuration::seconds(3));
}
