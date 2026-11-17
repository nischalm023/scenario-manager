//! After a pick, a tote the station's belt boards stays on its HTM for
//! `LoopDeposit` to set on the belt; any other picked tote's HTM gets a
//! clearance, which takes it home.

use desgraph::{CardinalDirection, CellId, StationId, StationType};
use desmap::{PresentationPoint, Station as MapStation};

use super::*;
use crate::composition::policy::StationPolicy;
use crate::model::admission::AdmissionLaw;
use crate::model::ids::RackId;
use crate::model::reservation::oracle::TableOracle;
use crate::model::station::Station;
use crate::model::test_model::{
    TestModel, TestModelConfig, bounded_scheduler, build_model, current_hold, provide_states, register_idle_htm,
    register_loaded_htm,
};
use crate::model::{ToteDesignation, ToteLocation, ToteType};

const STATION: StationId = StationId(1);
const DROPOFF: CellId = CellId(35);
const TOTE: ToteId = ToteId(700);
const SKU: SkuId = SkuId(1);

/// The station's belt, boarding totes of `accepts` at `DROPOFF`.
fn belt(accepts: &str) -> MapStation {
    MapStation {
        id: STATION,
        outbound_role: None,
        position: DROPOFF,
        presentation_points: vec![PresentationPoint { presentation_cell: DROPOFF, queue_path: vec![] }],
        edge: None,
        handoff_queues: vec![],
        exit_direction: CardinalDirection::East,
        bins: None,
        dropoff_cell: Some(DROPOFF),
        collection_cell: None,
        order_presentation_cell: None,
        conveyance_accepts: Some(accepts.into()),
        conveyor_transit_ms: None,
        conveyor_group: None,
        station_type: StationType::Pick,
    }
}

/// A test model with the station's presentation cell at `DROPOFF`, and a
/// scheduler wired to it as a run wires it.
fn station_model() -> (Arc<TestModel>, Arc<TableOracle>, Scheduler) {
    let (model, oracle) = build_model(TestModelConfig { stations: vec![(STATION, DROPOFF)], ..Default::default() });
    let scheduler = bounded_scheduler();
    provide_states(&scheduler, &model, ParkingRegistry::default());
    model.relay_service.set_admission_floor(scheduler.barrier().clock());
    model.relay_service.vtm_service.set_admission_floor(scheduler.barrier().clock());
    (model, oracle, scheduler)
}

/// One pick of `SKU` for order 1 at the station, whose belt boards totes of
/// `accepts`.
fn pick(model: &TestModel, accepts: &str) -> PickItem {
    let station = Station::new(STATION, StationType::Pick, DROPOFF, vec![], CardinalDirection::East, 1);
    let policy = StationPolicy {
        admission: AdmissionLaw { multiplier: 1.0, period: SimDuration::seconds(10), override_cap: None },
        pick_dwell: SimDuration::seconds(10),
        pallet_swap: SimDuration::ZERO,
        aim_arrivals: false,
    };
    let station = Arc::new(StationState::new(&station, policy, SimDuration::seconds(60), Version::new()));
    station.open_bin(OrderId(1), SimTime::ZERO);
    PickItem {
        line: PickLine { order: OrderId(1), sku: SKU, qty: 1, dc: None },
        bin_hold: station.bin(OrderId(1)).unwrap(),
        station,
        relay: model.relay_service.clone(),
        navigation: model.navigation.clone(),
        bots: model.bot_service.clone(),
        inventory: model.inventory.clone(),
        available: model.available_htm.clone(),
        parking: None,
        events: model.events.clone(),
        station_service: model.station_service.clone(),
        station_arrivals: model.station_arrivals.clone(),
        tripwires: false,
        delivery_max_wait_override: None,
        after: SimTime::ZERO,
        arrival_floor: SimTime::ZERO,
        completion: OrderCompletionTracker::default(),
        boarding: BeltBoarding::of(&[belt(accepts)]),
    }
}

/// Runs a pick's last step for the stay its presentation released.
#[derive(Display)]
#[display("AfterPick")]
struct AfterPick {
    pick: PickItem,
    stay: Option<AvailableHtm>,
}

#[async_trait(?Send)]
impl Task for AfterPick {
    type Proposal = EmptyProposal;
    type Schedule = ();

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<(), PlanError> {
        self.pick.clear_station(self.stay.take().expect("scheduled once"), &ctx)
    }
}

/// A pick on the station's dropoff cell leaves robot 60 standing there with
/// the inventory tote it presented. When the belt boards inventory totes,
/// the HTM keeps its stay and its hold for LoopDeposit; when it boards only
/// order totes, a clearance claims the stay and takes the tote home.
#[test]
fn a_tote_the_belt_boards_stays_on_its_htm_and_any_other_is_cleared() {
    for (accepts, boards) in [("inventory", true), ("order", false)] {
        let (model, oracle, mut scheduler) = station_model();
        let racked = ToteLocation::InRack { rack_id: RackId(1), z_index: 0 };
        model.inventory.register_tote(TOTE, ToteType::Tote, ToteDesignation::Inventory, racked).unwrap();
        model
            .inventory
            .tote(TOTE)
            .unwrap()
            .move_to(ToteLocation::WithRobot { robot_id: RobotId(60) }, SimTime::ZERO)
            .unwrap();
        register_loaded_htm(&model, &oracle, RobotId(60), DROPOFF, TOTE);
        let stay = model.available_htm.get(RobotId(60)).unwrap();
        scheduler.add_task(AfterPick { pick: pick(&model, accepts), stay: Some(stay.clone()) });
        scheduler.run().unwrap();
        assert!(!scheduler.stopped(), "{accepts} belt: the run reached the test deadline");
        assert_eq!(model.available_htm.is_available(stay.availability_id()), boards, "{accepts} belt: the stay");
        assert_eq!(current_hold(&oracle, &stay.cell_reservation).is_open_ended(), boards, "{accepts} belt: the hold");
        let home = matches!(model.inventory.tote(TOTE).unwrap().location(), ToteLocation::InRack { .. });
        assert_eq!(home, !boards, "{accepts} belt: the tote went home");
    }
}

/// A whole pick: robot 60, idle at cell 5, fetches the tote holding `SKU`
/// from rack 1 through a relay, presents it at the station's dropoff cell
/// and stays there once the operator releases it. When the belt boards
/// inventory totes, the HTM keeps the tote there for LoopDeposit; when it
/// boards only order totes, the pick's clearance takes the tote home.
#[test]
fn a_whole_pick_leaves_a_tote_the_belt_boards_on_its_htm_and_clears_any_other() {
    for (accepts, boards) in [("inventory", true), ("order", false)] {
        let (model, oracle, mut scheduler) = station_model();
        let racked = ToteLocation::InRack { rack_id: RackId(1), z_index: 0 };
        model.inventory.register_tote(TOTE, ToteType::Tote, ToteDesignation::Inventory, racked).unwrap();
        model.inventory.tote(TOTE).unwrap().add_item(SKU, 5, 0).unwrap();
        model.inventory.seed_inventory_from_totes();
        assert!(model.inventory.reserve_for_order(&[(SKU, 1)], SimTime::ZERO), "order 1 holds its unit");
        register_idle_htm(&model, &oracle, RobotId(60), CellId(5), SimTime::ZERO);
        let pick = pick(&model, accepts);
        pick.completion.register(OrderId(1), STATION, 1).unwrap();
        scheduler.add_task(pick);
        scheduler.run().unwrap();
        assert!(!scheduler.stopped(), "{accepts} belt: the run reached the test deadline");
        let stay = model.available_htm.get(RobotId(60));
        assert_eq!(stay.as_ref().map(AvailableHtm::cell) == Some(DROPOFF), boards, "{accepts} belt: the stay");
        let location = model.inventory.tote(TOTE).unwrap().location();
        let home = matches!(location, ToteLocation::InRack { .. });
        assert_eq!(home, !boards, "{accepts} belt: the tote went home, not {location:?}");
    }
}
