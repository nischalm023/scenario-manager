//! A putaway fill is committed only once its tote's return holds a deposit
//! relay. A declined fill commits nothing and awaits (scheduler time) the
//! barrier reaching the start of the first fill whose departure brings a
//! relay that opens beyond the access horizon within it, or passing the
//! declined fill's start, since its next attempt departs later.

use std::collections::VecDeque;
use std::sync::mpsc;

use desgraph::{CardinalDirection, RelayId, StationType};
use serde_json::json;

use super::*;
use crate::model::ParkingRegistry;
use crate::model::relay::DEPOSIT_ACCESS_MAX_WAIT;
use crate::model::reservation::oracle::TableOracle;
use crate::model::station::{Station, StationSet};
use crate::model::test_model::{
    Takes, TestModel, TestModelConfig, bay_held_by_a_busy_htm, bounded_scheduler, build_model, empty_rack_slots,
    one_bay, provide_states, register_loaded_htm, relays_busy_until, squatter_ready_at,
};
use crate::model::{ToteDesignation, ToteLocation, ToteType};
use crate::simtime::TimeRange;

const STATION: StationId = StationId(1);
const STATION_CELL: CellId = CellId(15);
const PACING: SimDuration = SimDuration::seconds(5);
/// When every relay, or every relay's access cell, is next free: more than
/// the access horizon after a fill departing at its first release.
const RELAYS_FREE: SimTime = SimTime::seconds(400);

/// Holds an advance target at its simtime and changes nothing else: the
/// barrier moves there with no relay, inventory, fleet or path change.
#[derive(Display)]
#[display("BarrierTarget")]
struct BarrierTarget(SimTime);

#[async_trait(?Send)]
impl Task for BarrierTarget {
    type Proposal = EmptyProposal;
    type Schedule = ();

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<(), PlanError> {
        ctx.barrier().wait_until(self.0).await
    }
}

/// The induction for one putaway station on `STATION_CELL`, with one lot
/// pending since simtime zero.
fn induction(model: &TestModel, scheduler: &Scheduler) -> PutawayInduction {
    let station = Station::new(STATION, StationType::Putaway, STATION_CELL, vec![], CardinalDirection::East, 1);
    let supply = PutawaySupply::new(
        &StationSet::new(vec![station]),
        &model.station_fetch_bounds,
        model.navigation.clone(),
        model.relay_service.clone(),
        model.inventory.clone(),
        model.available_htm.clone(),
        None,
    );
    let context = TaskContext::new::<PutawayInduction>(scheduler.states().clone(), &Default::default(), json!({}))
        .expect("the fixture's states");
    PutawayInduction {
        inventory: model.inventory.clone(),
        bots: model.bot_service.clone(),
        available: model.available_htm.clone(),
        events: model.events.clone(),
        supply: Arc::new(supply),
        arrival_queue: Mut::new(VecDeque::from([(SkuId(1), 1, SimTime::ZERO)])),
        station_pace: [(STATION, (STATION_CELL, TemporalCounter::new()))].into_iter().collect(),
        pacing_interval: PACING,
        return_relays: ReturnRelays::new(&context).expect("the fixture's services"),
    }
}

/// Robot 60 on the station's cell with empty tote 700 since simtime zero, an
/// empty slot in each rack, the run's `parking` bays, and a scheduler whose
/// journal hears each advance of the barrier; the facts stream to the
/// returned receiver.
fn staged(
    model: &mut Arc<TestModel>,
    oracle: &TableOracle,
    parking: ParkingRegistry,
) -> (Scheduler, mpsc::Receiver<SimEvent>) {
    let (tx, facts) = mpsc::channel();
    Arc::get_mut(model).unwrap().events = EventJournal::new(Some(Arc::new(tx)));
    let journal = model.events.clone();
    let scheduler = bounded_scheduler().with_advance_listener(Box::new(move |at| journal.barrier_advanced(at)));
    model.relay_service.set_admission_floor(scheduler.barrier().clock());
    model.relay_service.vtm_service.set_admission_floor(scheduler.barrier().clock());
    provide_states(&scheduler, model, parking);
    model
        .inventory
        .register_tote(
            ToteId(700),
            ToteType::Tote,
            ToteDesignation::Inventory,
            ToteLocation::WithRobot { robot_id: RobotId(60) },
        )
        .unwrap();
    register_loaded_htm(model, oracle, RobotId(60), STATION_CELL, ToteId(700));
    empty_rack_slots(model, 1..=4);
    (scheduler, facts)
}

/// Runs the induction, beside `others`, to the end; the releases of the
/// fills it committed.
fn fills_released(
    model: &TestModel,
    mut scheduler: Scheduler,
    facts: mpsc::Receiver<SimEvent>,
    others: impl IntoIterator<Item = BarrierTarget>,
) -> Vec<SimTime> {
    let induction = induction(model, &scheduler);
    let lots = induction.arrival_queue.clone();
    scheduler.add_task(induction);
    for other in others {
        scheduler.add_task(other);
    }
    scheduler.run().unwrap();
    assert!(lots.with(|queue| queue.is_empty()), "the lot is spent");
    facts
        .try_iter()
        .filter_map(|fact| match fact {
            SimEvent::CycleComplete(cycle) if matches!(cycle.kind, CycleKind::Putaway { .. }) => {
                Some(SimTime::ms(cycle.sim_time_ms))
            }
            _ => None,
        })
        .collect()
}

/// Every relay's slot is busy until 400 s, beyond the access horizon of a
/// fill released at 5 s, so that fill is declined and nothing is committed,
/// and no other task moves the barrier. The induction holds the start of the
/// fill released at 100 s, whose horizon reaches 400 s, as an advance
/// target: the barrier moves there, and that fill reserves a relay.
#[test]
fn a_declined_fill_whose_relays_open_beyond_its_horizon_uses_the_first_release_that_reaches_them() {
    let (mut model, oracle) = build_model(TestModelConfig::default());
    let (scheduler, facts) = staged(&mut model, &oracle, ParkingRegistry::default());
    relays_busy_until(&model, STATION_CELL, RELAYS_FREE, None);
    let fills = fills_released(&model, scheduler, facts, []);
    assert_eq!(fills, [RELAYS_FREE - DEPOSIT_ACCESS_MAX_WAIT], "released as the horizon first reaches 400 s");
}

/// Relay 1 is free, but robot 95, empty, stands on its access cell 10 from
/// simtime zero, idle from 60 s, and a busy HTM holds the one parking bay;
/// relays 2 to 4 are busy until 400 s, beyond the access horizon of a fill
/// released at 5 s. That fill is declined for the held access cell, and robot
/// 95's parking, departing at 60 s, finds no bay. Each later fill released
/// before 60 s would park it departing at 60 s again, so none tries, and no
/// other task moves the barrier. The decline keeps the other relays' opening:
/// the induction holds the start of the fill released at 100 s as an advance
/// target, and that fill reserves a relay that opens at 400 s.
#[test]
fn a_fill_declined_for_a_squatted_access_cell_tries_one_parking_and_uses_the_first_release_reaching_the_other_relays() {
    let (mut model, oracle) = build_model(TestModelConfig::default());
    let (navigation, bay) = one_bay(&model);
    Arc::get_mut(&mut model).unwrap().navigation = navigation;
    let (scheduler, facts) = staged(&mut model, &oracle, bay);
    relays_busy_until(&model, STATION_CELL, RELAYS_FREE, Some(RelayId(1)));
    squatter_ready_at(&model, &oracle, RobotId(95), CellId(10), SimTime::seconds(60));
    bay_held_by_a_busy_htm(&model, &oracle);
    let takes = Arc::new(Takes::default());
    model.available_htm.add_observer(takes.clone());
    let fills = fills_released(&model, scheduler, facts, []);
    assert_eq!(fills, [RELAYS_FREE - DEPOSIT_ACCESS_MAX_WAIT], "released as the horizon first reaches 400 s");
    let parkings = takes.0.lock().unwrap().iter().filter(|&&robot| robot == RobotId(95)).count();
    assert_eq!(parkings, 1, "robot 95's parking is tried once");
    assert_eq!(model.available_htm.get(RobotId(95)).map(|stay| stay.cell()), Some(CellId(10)), "the squatter stays");
}

/// Relay 1 is free, but robot 95, empty, stands on its access cell 10 from
/// simtime zero, idle from 60 s; robot 65 holds the one parking bay until
/// 200 s, and robots 91 to 93 hold relays 2 to 4's access cells until
/// 1,000 s, beyond the access horizon of every fill here, so no decline
/// names an opening. Robot 95's parking departing at 60 s finds no bay, as
/// the bay frees more than 120 s after it; one departing at 100 s finds it.
/// Another task moves the barrier to 100 s and changes nothing else: the fill
/// released at 105 s parks robot 95 departing at 100 s and reserves relay 1.
#[test]
fn a_squatter_whose_parking_found_no_bay_is_parked_when_its_parking_would_depart_later() {
    let (mut model, oracle) = build_model(TestModelConfig::default());
    let (navigation, bay) = one_bay(&model);
    Arc::get_mut(&mut model).unwrap().navigation = navigation;
    let (scheduler, facts) = staged(&mut model, &oracle, bay);
    squatter_ready_at(&model, &oracle, RobotId(95), CellId(10), SimTime::seconds(60));
    oracle.table().reserve_cell(RobotId(65), CellId(3), TimeRange::new(SimTime::ZERO, SimTime::seconds(200))).unwrap();
    for (robot, cell) in (91..).map(RobotId).zip([20, 30, 40].map(CellId)) {
        oracle.table().reserve_cell(robot, cell, TimeRange::new(SimTime::ZERO, SimTime::seconds(1000))).unwrap();
    }
    let fills = fills_released(&model, scheduler, facts, [BarrierTarget(SimTime::seconds(100))]);
    assert_eq!(fills, [SimTime::seconds(100) + PACING], "released a pacing interval after the barrier's 100 s");
    let stand = model.available_htm.get(RobotId(95)).map(|stay| stay.cell());
    assert_eq!(stand, Some(CellId(3)), "robot 95 stands in the bay");
}

/// Robots 90 to 93 hold the relays' access cells (10, 20, 30, 40) until 400
/// s, beyond the access horizon of a fill released at 5 s, so that fill is
/// declined and names no relay opening. Another task moves the barrier to
/// 150 s and changes nothing else: the induction wakes, and the fill
/// released at 155 s reserves a relay whose access cell frees within its
/// horizon.
#[test]
fn a_declined_fill_is_committed_once_the_barrier_passes_its_start_with_its_release_moved() {
    let (mut model, oracle) = build_model(TestModelConfig::default());
    let (scheduler, facts) = staged(&mut model, &oracle, ParkingRegistry::default());
    for (robot, cell) in (90..).map(RobotId).zip([10, 20, 30, 40].map(CellId)) {
        oracle.table().reserve_cell(robot, cell, TimeRange::new(SimTime::ZERO, RELAYS_FREE)).unwrap();
    }
    let fills = fills_released(&model, scheduler, facts, [BarrierTarget(SimTime::seconds(150))]);
    assert_eq!(fills, [SimTime::seconds(150) + PACING], "released a pacing interval after the barrier's 150 s");
}
