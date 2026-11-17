use super::*;
use desgraph::{BlockId, RelayId};
use desgraph::{CellId, StationId};
use desmap::{BlockSide, RelayPosition, VtmPosition};
use std::cell::Cell;
use std::rc::Rc;
use tokio::runtime::Builder;

use crate::events::ToteSource;
use crate::flows::fulfillment::induct_container::InductContainer;
use crate::flows::transport::Navigate;
use crate::flows::ttp::task::clearance::{ClearPresentation, ClearanceOutcome};
use crate::model::ToteType;
use crate::model::htm::registry::{PickupSelection, ProposedHtmClaim, ProposedHtmPickup};
use crate::model::ids::TaskId;
use crate::model::reservation::oracle::TableOracle;
use crate::model::test_model::{NeedVacated, TestModel, TestModelConfig, build_model, current_hold, register_idle_htm};
use crate::pathfinding::ReservationOracle;
use crate::pathfinding::WindowSink;
use crate::pathfinding::{
    CollisionConfig, DriveModel, FHSippPathfinder, Orientation, Path, Pathfinder, PathfinderError, ReservedPath,
    WaypointAction,
};
use crate::scheduler::task::proposal::ProposedReservation;
use async_trait::async_trait;

struct NoParkingSearch;
#[async_trait(?Send)]
impl Pathfinder for NoParkingSearch {
    fn find_path(
        &self,
        _: CellId,
        _: Orientation,
        _: CellId,
        _: &dyn DriveModel,
        _: SimTime,
    ) -> Result<Path, PathfinderError> {
        panic!("superseded Parking must not search for a bay or path");
    }
    async fn reserve_path(
        &self,
        _: CellId,
        _: Orientation,
        _: CellId,
        _: &dyn DriveModel,
        _: &dyn ReservationOracle,
        _: &CollisionConfig,
        _: &mut dyn WindowSink,
    ) -> Result<ReservedPath, PathfinderError> {
        panic!("superseded Parking must not reserve a path");
    }
    fn traverse_duration(&self) -> SimDuration {
        SimDuration::seconds(1)
    }
}

fn parking_task(model: &TestModel, scheduler: &Scheduler, registry: Option<ParkingRegistry>) -> Parking {
    Parking::new(
        model.available_htm.peek_matching(|entry| entry.robot() == RobotId(1)).pop().unwrap(),
        model.navigation.clone(),
        model.bot_service.clone(),
        model.inventory.clone(),
        model.available_htm.clone(),
        registry,
        model.events.clone(),
        scheduler.barrier().clock(),
    )
    .unwrap()
}

fn parking_bay(model: &TestModel, oracle: Arc<TableOracle>, cell: CellId) -> (Arc<Navigation>, ParkingRegistry) {
    parking_bays(model, oracle, &[cell])
}

fn parking_bays(model: &TestModel, oracle: Arc<TableOracle>, cells: &[CellId]) -> (Arc<Navigation>, ParkingRegistry) {
    let mut map = model.navigation.map.as_ref().clone();
    map.relays = cells
        .iter()
        .copied()
        .map(|cell| RelayPosition {
            id: RelayId(cell.0),
            position: cell,
            htm_access: cell,
            vtm_access: VtmPosition::Cell(CellId(0)),
            block: BlockId(0),
            side: BlockSide::Left,
        })
        .collect();
    let registry = ParkingRegistry::from_map(&map);
    let pathfinder =
        Arc::new(FHSippPathfinder::new(&map, 1000, SimDuration::seconds(1), cells.iter().copied().collect()));
    let navigation = Arc::new(Navigation::new(Arc::new(map), pathfinder, model.bot_service.clone(), oracle));
    (navigation, registry)
}

#[test]
fn later_productive_induction_supersedes_due_parking_without_scheduling_it() {
    let (model, oracle) = build_model(TestModelConfig::default());
    register_idle_htm(&model, &oracle, RobotId(1), CellId(10), SimTime::seconds(10));
    let original = model.available_htm.peek_matching(|_| true).pop().unwrap();
    let mut scheduler = Scheduler::new();
    let (_, bay) = parking_bay(&model, oracle.clone(), CellId(12));
    let mut task = parking_task(&model, &scheduler, Some(bay));
    task.navigation = Arc::new(Navigation::new(
        model.navigation.map.clone(),
        Arc::new(NoParkingSearch),
        model.bot_service.clone(),
        oracle.clone(),
    ));
    let need_by =
        NeedVacated { navigation: task.navigation.clone(), hold: original.cell_reservation.clone(), by: SimTime::ZERO };
    let parking = scheduler.add_task(task);
    scheduler.add_service(need_by);
    let productive = scheduler.add_task(
        InductContainer::new(
            CellId(20),
            ToteType::Carton,
            ToteDesignation::Order,
            SimTime::seconds(50),
            model.navigation.clone(),
            model.bot_service.clone(),
            model.inventory.clone(),
            model.available_htm.clone(),
            None,
            model.events.clone(),
            scheduler.barrier().clock(),
        )
        .with_station(StationId(1)),
    );
    scheduler.run().unwrap();
    let runtime = Builder::new_current_thread().build().unwrap();
    assert_eq!(
        runtime.block_on(parking),
        ParkingOutcome::Superseded,
        "the last resort retires before invoking schedule or searching a parking path"
    );
    let container = runtime.block_on(productive);
    assert_eq!(container.cell_reservation.robot(), RobotId(1));
    assert!(container.action_ready_at >= SimTime::seconds(52));
    assert!(
        model
            .navigation
            .oracle
            .reservations_for_cell(original.cell())
            .iter()
            .filter(|hold| hold.robot() == original.robot())
            .all(|hold| !hold.is_open_ended())
    );
    assert!(scheduler.dump_group(scheduler.group_of(TaskId(0)).unwrap()).unwrap().contains("[dissolved]"));
}

/// A productive task's claim on one stay: accepted, it takes the
/// stay and plans nothing.
#[derive(Display)]
#[display("ProductiveClaim")]
struct ProductiveClaim {
    available: AvailableHtmRegistry,
    entry: AvailableHtm,
    clock: Mut<SimTime>,
}

#[async_trait(?Send)]
impl Task for ProductiveClaim {
    type Proposal = ProposedHtmClaim;
    type Schedule = RobotId;
    async fn propose(&self, owner: ProposalId) -> Result<ProposedHtmClaim, PlanError> {
        Ok(self.available.propose_productive(&self.entry, owner, self.entry.action_ready_at, self.clock.clone()))
    }
    async fn schedule(&mut self, proposal: ProposedHtmClaim, _: SchedCtx) -> Result<RobotId, PlanError> {
        Ok(proposal.into_accepted()?.0.robot())
    }
}

/// Parking and a clearance each make a last-resort claim on one empty HTM's
/// stay. Neither outranks the other, so the one the scheduler ranks first
/// parks the HTM and the other dissolves: the stay is parked exactly once.
/// A productive claim on the same stay beats both, and nothing is parked.
#[test]
fn two_last_resorts_on_one_empty_stay_park_it_once_and_a_productive_claim_beats_both() {
    for (clear_first, productive) in [(true, false), (false, false), (true, true)] {
        let (model, oracle) = build_model(TestModelConfig::default());
        register_idle_htm(&model, &oracle, RobotId(1), CellId(10), SimTime::seconds(10));
        let original = model.available_htm.get(RobotId(1)).unwrap();
        let (navigation, registry) = parking_bay(&model, oracle.clone(), CellId(12));
        let mut scheduler = Scheduler::new();
        let states = scheduler.states();
        states.provide(navigation.clone());
        states.provide(Arc::new(model.bot_service.clone()));
        states.provide(Arc::new(model.inventory.clone()));
        states.provide(Arc::new(model.available_htm.clone()));
        states.provide(Arc::new(model.events.clone()));
        states.provide(Arc::new(registry.clone()));
        let mut parking = parking_task(&model, &scheduler, Some(registry));
        parking.navigation = navigation.clone();
        let clear = ClearPresentation::new(
            original.clone(),
            StationId(1),
            ToteSource::Pick,
            navigation,
            model.available_htm.clone(),
            scheduler.barrier().clock(),
        );
        let (clear, parking) = if clear_first {
            let clear = scheduler.add_task(clear);
            (clear, scheduler.add_task(parking))
        } else {
            let parking = scheduler.add_task(parking);
            (scheduler.add_task(clear), parking)
        };
        let used = productive.then(|| {
            scheduler.add_task(ProductiveClaim {
                available: model.available_htm.clone(),
                entry: original.clone(),
                clock: scheduler.barrier().clock(),
            })
        });
        scheduler.run().unwrap();
        let runtime = Builder::new_current_thread().build().unwrap();
        let outcomes = (runtime.block_on(clear), runtime.block_on(parking));
        if let Some(used) = used {
            assert_eq!(outcomes, (ClearanceOutcome::Superseded, ParkingOutcome::Superseded));
            assert_eq!(runtime.block_on(used), RobotId(1), "the productive claim took the stay");
            assert!(current_hold(&oracle, &original.cell_reservation).is_open_ended(), "nothing parked the HTM");
            continue;
        }
        assert!(
            matches!(
                outcomes,
                (ClearanceOutcome::Cleared, ParkingOutcome::Superseded)
                    | (ClearanceOutcome::Superseded, ParkingOutcome::Parked)
            ),
            "clearance first: {clear_first}, outcomes {outcomes:?}"
        );
        assert_eq!(model.available_htm.get(RobotId(1)).unwrap().cell(), CellId(12), "parked in the bay");
        assert!(!current_hold(&oracle, &original.cell_reservation).is_open_ended());
    }
}

/// A won claim hands its HTM over: ParkClaimedHtm plans only the drive to a
/// bay, departing at the claim's departure. With no bay to take, it restores
/// the stay it was given, for the clearance to compete again or stop the run.
#[test]
fn a_claimed_htm_parks_at_its_claims_departure_or_restores_its_stay() {
    use crate::events::SimEvent;
    use std::sync::mpsc;

    for bay in [true, false] {
        let (model, oracle) = build_model(TestModelConfig::default());
        register_idle_htm(&model, &oracle, RobotId(1), CellId(10), SimTime::ZERO);
        let entry = model.available_htm.take(RobotId(1)).unwrap();
        let (navigation, registry) = parking_bay(&model, oracle.clone(), CellId(12));
        let depart = SimTime::seconds(20);
        let (tx, rx) = mpsc::channel();
        let mut scheduler = Scheduler::new();
        let parked = scheduler.add_task(ParkClaimedHtm {
            entry: Some(entry.clone()),
            depart,
            navigation,
            bots: model.bot_service.clone(),
            inventory: model.inventory.clone(),
            available: model.available_htm.clone(),
            parking: bay.then_some(registry),
            events: EventJournal::new(Some(Arc::new(tx))),
        });
        scheduler.run().unwrap();
        let outcome = Builder::new_current_thread().build().unwrap().block_on(parked);
        let stay = model.available_htm.get(RobotId(1)).expect("the HTM is idle again");
        if bay {
            assert_eq!(outcome, ParkingOutcome::Parked);
            assert_eq!(stay.cell(), CellId(12));
            let first_move = rx.try_iter().find_map(|event| match event {
                SimEvent::Nav(nav) => nav.waypoints.iter().find_map(|waypoint| match waypoint.action {
                    WaypointAction::Move { start, .. } => Some(start),
                    _ => None,
                }),
                _ => None,
            });
            assert_eq!(first_move, Some(depart), "it left the cell at the claim's departure");
        } else {
            assert_eq!(outcome, ParkingOutcome::Unavailable);
            assert_eq!(stay.availability_id(), entry.availability_id(), "the stay it was given is restored");
        }
    }
}

#[test]
fn parking_without_productive_demand_clears_the_original_hold_when_the_htm_is_available() {
    let (model, oracle) = build_model(TestModelConfig::default());
    register_idle_htm(&model, &oracle, RobotId(1), CellId(10), SimTime::seconds(10));
    let original = model.available_htm.peek_matching(|_| true).pop().unwrap();
    let (navigation, registry) = parking_bay(&model, oracle.clone(), CellId(12));
    let mut scheduler = Scheduler::new();
    let mut task = parking_task(&model, &scheduler, Some(registry));
    task.navigation = navigation;
    let parked = scheduler.add_task(task);
    scheduler.run().unwrap();
    assert_eq!(Builder::new_current_thread().build().unwrap().block_on(parked), ParkingOutcome::Parked);
    let entry = model.available_htm.peek_matching(|_| true).pop().unwrap();
    assert_eq!(entry.cell(), CellId(12));
    assert_ne!(entry.availability_id(), original.availability_id());
    assert_eq!(scheduler.barrier().floor(), SimTime::seconds(10));
    assert!(
        model
            .navigation
            .oracle
            .reservations_for_cell(original.cell())
            .iter()
            .filter(|hold| hold.robot() == original.robot())
            .all(|hold| !hold.is_open_ended())
    );
    assert!(entry.action_ready_at > SimTime::seconds(10));
    assert!(entry.cell_reservation.is_open_ended());
}

#[test]
fn no_bay_restores_the_same_availability_and_original_hold() {
    let (model, oracle) = build_model(TestModelConfig::default());
    register_idle_htm(&model, &oracle, RobotId(1), CellId(10), SimTime::ZERO);
    let original = model.available_htm.peek_matching(|_| true).pop().unwrap();
    let mut scheduler = Scheduler::new();
    let completion = scheduler.add_task(parking_task(&model, &scheduler, Some(ParkingRegistry::default())));
    scheduler.run().unwrap();
    assert_eq!(Builder::new_current_thread().build().unwrap().block_on(completion), ParkingOutcome::Unavailable);
    let restored = model.available_htm.peek_matching(|_| true).pop().unwrap();
    assert_eq!(restored.availability_id(), original.availability_id());
    assert_eq!(restored.cell_reservation.id(), original.cell_reservation.id());
    assert!(original.cell_reservation.is_open_ended());
}

#[test]
fn an_old_parking_task_cannot_claim_a_new_idle_episode_of_the_same_robot() {
    let (model, oracle) = build_model(TestModelConfig::default());
    register_idle_htm(&model, &oracle, RobotId(1), CellId(10), SimTime::ZERO);
    let mut scheduler = Scheduler::new();
    let task = parking_task(&model, &scheduler, None);
    let old = model.available_htm.take(RobotId(1)).unwrap();
    model.available_htm.register(old.cell_reservation.clone(), SimTime::seconds(20), old.orientation, None).unwrap();
    let replacement = model.available_htm.peek_matching(|_| true).pop().unwrap();
    let completion = scheduler.add_task(task);
    scheduler.run().unwrap();
    assert_eq!(Builder::new_current_thread().build().unwrap().block_on(completion), ParkingOutcome::Superseded);
    assert!(model.available_htm.is_available(replacement.availability_id()));
    assert_ne!(replacement.availability_id(), old.availability_id());
}

#[test]
fn dropped_old_attempt_does_not_remove_its_replacement_claim() {
    let (model, oracle) = build_model(TestModelConfig::default());
    register_idle_htm(&model, &oracle, RobotId(1), CellId(10), SimTime::ZERO);
    let entry = model.available_htm.peek_matching(|_| true).pop().unwrap();
    let clock = Mut::new(SimTime::ZERO);
    let old_id = ProposalId { task: TaskId(1), attempt: 0 };
    let new_id = ProposalId { task: TaskId(1), attempt: 1 };
    let old = model.available_htm.propose_parking(&entry, old_id, entry.action_ready_at, clock.clone());
    let new = model.available_htm.propose_parking(&entry, new_id, entry.action_ready_at, clock.clone());
    let productive_id = ProposalId { task: TaskId(2), attempt: 0 };
    let productive = ProposedHtmPickup::new(
        productive_id,
        model.available_htm.clone(),
        model.navigation.clone(),
        CellId(20),
        PickupSelection {
            htm: entry,
            travel: SimDuration::seconds(5),
            depart_anchor: SimTime::seconds(50),
            resident: false,
        },
        model.available_htm.version().seen(),
        clock,
    );
    assert_eq!(ProposedReservation::ranked_competitors(&old), Some(vec![productive_id, old_id, new_id]));
    drop(old);
    assert_eq!(ProposedReservation::ranked_competitors(&new), Some(vec![productive_id, new_id]));
    drop(new);
    assert!(
        ProposedReservation::ranked_competitors(&productive).is_none(),
        "no last resort changes ordinary pickup arbitration"
    );
}

#[test]
fn a_rejected_stale_restore_preserves_the_replacement_entry() {
    let (model, oracle) = build_model(TestModelConfig::default());
    register_idle_htm(&model, &oracle, RobotId(1), CellId(10), SimTime::ZERO);
    let old = model.available_htm.take(RobotId(1)).unwrap();
    model.available_htm.register(old.cell_reservation.clone(), SimTime::seconds(20), old.orientation, None).unwrap();
    let replacement = model.available_htm.peek_matching(|_| true).pop().unwrap();
    let version = model.available_htm.version().seen();
    assert!(model.available_htm.put_back(old).is_err());
    let retained = model.available_htm.peek_matching(|_| true).pop().unwrap();
    assert_eq!(retained.availability_id(), replacement.availability_id());
    assert_eq!(retained.action_ready_at, replacement.action_ready_at);
    assert_eq!(model.available_htm.version().seen(), version);
}

#[test]
fn an_incoming_journey_can_be_planned_before_the_known_future_release() {
    use crate::events::SimEvent;
    use crate::flows::transport::{HtmPosition, Navigate};
    use std::sync::mpsc;

    let (model, oracle) = build_model(TestModelConfig::default());
    register_idle_htm(&model, &oracle, RobotId(1), CellId(10), SimTime::ZERO);
    let held = model.available_htm.take(RobotId(1)).unwrap();
    model.available_htm.register(held.cell_reservation, SimTime::seconds(100), held.orientation, None).unwrap();
    register_idle_htm(&model, &oracle, RobotId(2), CellId(0), SimTime::ZERO);
    let incoming = model.available_htm.take(RobotId(2)).unwrap();
    let (navigation, bay) = parking_bay(&model, oracle.clone(), CellId(12));
    let (tx, rx) = mpsc::channel();
    let events = EventJournal::new(Some(Arc::new(tx)));
    let mut scheduler = Scheduler::new();
    let mut task = parking_task(&model, &scheduler, Some(bay.clone()));
    task.navigation = navigation.clone();
    task.events = events.clone();
    let parking = scheduler.add_task(task);
    let movement = scheduler.add_task(Navigate::new(
        HtmPosition {
            cell_reservation: incoming.cell_reservation,
            action_ready_at: SimTime::ZERO,
            orientation: Orientation::East,
        },
        CellId(10),
        None,
        navigation,
        model.bot_service.clone(),
        model.inventory.clone(),
        Some(bay),
        events,
        scheduler.barrier().clock(),
    ));
    scheduler.run().unwrap();
    let runtime = Builder::new_current_thread().build().unwrap();
    assert_eq!(runtime.block_on(parking), ParkingOutcome::Parked);
    let arrived = runtime.block_on(movement);
    assert!(arrived.action_ready_at >= SimTime::seconds(100), "the occupied destination is never entered early");
    let incoming_nav = rx
        .try_iter()
        .find_map(|event| match event {
            SimEvent::Nav(nav) if nav.robot_id == RobotId(2) => Some(nav),
            _ => None,
        })
        .expect("incoming HTM has a physical journey");
    let first_move = incoming_nav
        .waypoints
        .iter()
        .find_map(|waypoint| match waypoint.action {
            WaypointAction::Move { start, .. } => Some(start),
            _ => None,
        })
        .expect("incoming HTM physically changes cells");
    assert!(first_move < SimTime::seconds(100), "physical approach is deferred until {first_move}");
    let source_hold =
        oracle.reservations_for_cell(CellId(0)).into_iter().find(|hold| hold.robot() == RobotId(2)).unwrap();
    assert!(source_hold.window().end().unwrap() < SimTime::seconds(100));
    let destination_holds = oracle.reservations_for_cell(CellId(10));
    let incumbent = destination_holds.iter().find(|hold| hold.robot() == RobotId(1)).unwrap();
    let arriving = destination_holds.iter().find(|hold| hold.robot() == RobotId(2)).unwrap();
    assert!(incumbent.window().end().unwrap() <= arriving.window().start());
    assert!(
        incoming_nav.time < SimTime::seconds(100),
        "incoming departure {} waited until the future release was physically due instead of planning against it",
        incoming_nav.time
    );
}

#[test]
fn withdrawn_and_stale_need_bys_do_not_advance_other_work() {
    let (model, oracle) = build_model(TestModelConfig::default());
    register_idle_htm(&model, &oracle, RobotId(1), CellId(10), SimTime::seconds(100));
    let entry = model.available_htm.peek_matching(|_| true).pop().unwrap();
    let clock = Mut::new(SimTime::seconds(10));
    let fleet_version = model.available_htm.version().seen();
    let cells = &model.navigation.cells;
    let last_resort = |entry: &AvailableHtm, task| {
        let owner = ProposalId { task: TaskId(task), attempt: 0 };
        ProposedLastResort::parking(&model.available_htm, &model.navigation, entry, owner, clock.clone())
    };
    let parking = last_resort(&entry, 1);
    let need_by = cells.need_by(cells.vacating(&entry.cell_reservation).unwrap(), SimTime::ZERO).unwrap();
    assert_eq!(ProposedReservation::resolve_by(&parking), Some(SimTime::seconds(10)));
    assert_eq!(ProposedReservation::start_time(&parking), Some(SimTime::seconds(100)));
    assert_eq!(
        model.available_htm.version().seen(),
        fleet_version,
        "a need-by does not poison productive fleet choices"
    );
    drop(need_by);
    assert_eq!(ProposedReservation::resolve_by(&parking), Some(SimTime::seconds(100)));
    let stale = cells.need_by(cells.vacating(&entry.cell_reservation).unwrap(), SimTime::ZERO).unwrap();
    assert!(stale.is_live());
    drop(parking);
    assert!(!stale.is_live(), "a withdrawn Vacate must not keep a need-by live");
    let old = model.available_htm.take(RobotId(1)).unwrap();
    model.available_htm.register(old.cell_reservation, SimTime::seconds(200), old.orientation, None).unwrap();
    let replacement = model.available_htm.peek_matching(|_| true).pop().unwrap();
    let next = last_resort(&replacement, 2);
    assert_eq!(ProposedReservation::resolve_by(&next), Some(SimTime::seconds(200)));
}

#[test]
fn productive_only_claims_keep_their_ordinary_deadline() {
    let (model, oracle) = build_model(TestModelConfig::default());
    register_idle_htm(&model, &oracle, RobotId(1), CellId(10), SimTime::ZERO);
    let entry = model.available_htm.peek_matching(|_| true).pop().unwrap();
    let hold = entry.cell_reservation.clone();
    let productive = ProposedHtmPickup::new(
        ProposalId { task: TaskId(1), attempt: 0 },
        model.available_htm.clone(),
        model.navigation.clone(),
        CellId(20),
        PickupSelection {
            htm: entry,
            travel: SimDuration::seconds(5),
            depart_anchor: SimTime::seconds(50),
            resident: false,
        },
        model.available_htm.version().seen(),
        Mut::new(SimTime::ZERO),
    );
    assert!(model.navigation.cells.vacating(&hold).is_none(), "a productive claim registers no Vacate to need");
    assert_eq!(ProposedReservation::resolve_by(&productive), Some(SimTime::seconds(50)));
    assert!(ProposedReservation::ranked_competitors(&productive).is_none());
}

#[derive(Display)]
#[display("ObservedParking")]
struct ObservedParking {
    inner: Parking,
    decided_at: Rc<Cell<Option<SimTime>>>,
}
#[async_trait(?Send)]
impl Task for ObservedParking {
    type Proposal = ProposedLastResort;
    type Schedule = ParkingOutcome;
    fn dissolve(&self) -> Option<Self::Schedule> {
        Task::dissolve(&self.inner)
    }
    async fn propose(&self, owner: ProposalId) -> Result<Self::Proposal, PlanError> {
        Task::propose(&self.inner, owner).await
    }
    async fn schedule(&mut self, proposal: Self::Proposal, ctx: SchedCtx) -> Result<Self::Schedule, PlanError> {
        self.decided_at.set(Some(ctx.barrier().floor()));
        Task::schedule(&mut self.inner, proposal, ctx).await
    }
}

#[test]
fn destination_alternatives_request_only_one_useful_release_and_none_when_free() {
    use crate::events::TripEndpoint;
    use crate::flows::transport::{HtmPosition, Navigate};
    use std::cell::Cell;
    use std::rc::Rc;
    for free_alternative in [false, true] {
        let (model, oracle) = build_model(TestModelConfig::default());
        for (robot, cell, ready) in [(1, 10, 100), (3, 30, 150)] {
            register_idle_htm(&model, &oracle, RobotId(robot), CellId(cell), SimTime::ZERO);
            let entry = model.available_htm.take(RobotId(robot)).unwrap();
            model
                .available_htm
                .register(entry.cell_reservation, SimTime::seconds(ready), entry.orientation, None)
                .unwrap();
        }
        register_idle_htm(&model, &oracle, RobotId(2), CellId(0), SimTime::ZERO);
        let incoming = model.available_htm.take(RobotId(2)).unwrap();
        let (navigation, bays) = parking_bays(&model, oracle.clone(), &[CellId(12), CellId(32)]);
        let mut scheduler = Scheduler::new();
        let mut observed = Vec::new();
        for robot in [1, 3] {
            let entry = model.available_htm.peek_matching(|entry| entry.robot() == RobotId(robot)).pop().unwrap();
            let decided_at = Rc::new(Cell::new(None));
            scheduler.add_task(ObservedParking {
                inner: Parking::new(
                    entry,
                    navigation.clone(),
                    model.bot_service.clone(),
                    model.inventory.clone(),
                    model.available_htm.clone(),
                    Some(bays.clone()),
                    model.events.clone(),
                    scheduler.barrier().clock(),
                )
                .unwrap(),
                decided_at: decided_at.clone(),
            });
            observed.push(decided_at);
        }
        let mut destinations =
            vec![(CellId(10), TripEndpoint::Station(StationId(1))), (CellId(30), TripEndpoint::Station(StationId(2)))];
        if free_alternative {
            destinations.push((CellId(5), TripEndpoint::Station(StationId(3))));
        }
        let completion = scheduler.add_task(
            Navigate::new(
                HtmPosition {
                    cell_reservation: incoming.cell_reservation,
                    action_ready_at: SimTime::ZERO,
                    orientation: Orientation::East,
                },
                CellId(10),
                None,
                navigation,
                model.bot_service.clone(),
                model.inventory.clone(),
                Some(bays),
                model.events.clone(),
                scheduler.barrier().clock(),
            )
            .with_destinations(destinations),
        );
        scheduler.run().unwrap();
        let arrived = Builder::new_current_thread().build().unwrap().block_on(completion);
        assert_eq!(arrived.cell_reservation.cell(), CellId(if free_alternative { 5 } else { 10 }));
        assert_eq!(observed[0].get(), Some(if free_alternative { SimTime::ms(100_000) } else { SimTime::ZERO }));
        assert_eq!(observed[1].get(), Some(SimTime::ms(150_000)), "the unneeded alternative keeps its normal deadline");
    }
}

#[test]
fn an_early_no_bay_failure_waits_until_departure_before_reporting_unavailable() {
    let (model, oracle) = build_model(TestModelConfig::default());
    register_idle_htm(&model, &oracle, RobotId(1), CellId(10), SimTime::seconds(100));
    let original = model.available_htm.peek_matching(|_| true).pop().unwrap();
    let mut scheduler = Scheduler::new();
    scheduler.add_service(NeedVacated {
        navigation: model.navigation.clone(),
        hold: original.cell_reservation.clone(),
        by: SimTime::ZERO,
    });
    let completion = scheduler.add_task(parking_task(&model, &scheduler, Some(ParkingRegistry::default())));
    scheduler.run().unwrap();
    assert_eq!(Builder::new_current_thread().build().unwrap().block_on(completion), ParkingOutcome::Unavailable);
    assert_eq!(scheduler.barrier().floor(), SimTime::seconds(100));
    let restored = model.available_htm.peek_matching(|_| true).pop().unwrap();
    assert_eq!(restored.availability_id(), original.availability_id());
    assert_eq!(restored.cell_reservation.id(), original.cell_reservation.id());
    assert!(original.cell_reservation.is_open_ended());
}

#[derive(Display)]
#[display("ReleaseBayLater")]
struct ReleaseBayLater {
    departure: Option<Navigate>,
    parking: ParkingRegistry,
}

#[async_trait(?Send)]
impl Task for ReleaseBayLater {
    type Proposal = EmptyProposal;
    type Schedule = ();
    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }
    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<(), PlanError> {
        ctx.barrier().wait_until(SimTime::seconds(50)).await?;
        let arrived = ctx.add_task(self.departure.take().unwrap()).await;
        // This fixture owns the bay reservation it seeded. Release it only
        // after a real departure plan exists, conservatively at full arrival.
        self.parking.cancel(CellId(12), RobotId(3), arrived.action_ready_at);
        Ok(())
    }
}

#[test]
fn a_bay_cleared_during_the_original_dwell_is_available_to_the_single_retry() {
    use crate::flows::transport::{HtmPosition, Navigate};

    let (model, oracle) = build_model(TestModelConfig::default());
    register_idle_htm(&model, &oracle, RobotId(1), CellId(10), SimTime::seconds(100));
    register_idle_htm(&model, &oracle, RobotId(3), CellId(12), SimTime::ZERO);
    let occupant = model.available_htm.take(RobotId(3)).unwrap();
    let (navigation, registry) = parking_bay(&model, oracle.clone(), CellId(12));
    registry.reserve(CellId(12), RobotId(3), SimTime::ZERO);
    let mut scheduler = Scheduler::new();
    let mut task = parking_task(&model, &scheduler, Some(registry.clone()));
    let original = task.entry.clone();
    task.navigation = navigation.clone();
    scheduler.add_service(NeedVacated {
        navigation: navigation.clone(),
        hold: original.cell_reservation.clone(),
        by: SimTime::ZERO,
    });
    let completion = scheduler.add_task(task);
    let departure = Navigate::new(
        HtmPosition {
            cell_reservation: occupant.cell_reservation,
            action_ready_at: occupant.action_ready_at,
            orientation: Orientation::East,
        },
        CellId(14),
        None,
        navigation,
        model.bot_service.clone(),
        model.inventory.clone(),
        Some(registry.clone()),
        model.events.clone(),
        scheduler.barrier().clock(),
    );
    scheduler.add_task(ReleaseBayLater { departure: Some(departure), parking: registry });
    scheduler.run().unwrap();
    assert_eq!(Builder::new_current_thread().build().unwrap().block_on(completion), ParkingOutcome::Parked);
    assert_eq!(scheduler.barrier().floor(), SimTime::seconds(100));
    let parked = model.available_htm.peek_matching(|entry| entry.robot() == RobotId(1)).pop().unwrap();
    assert_eq!(parked.cell(), CellId(12));
    assert_ne!(parked.availability_id(), original.availability_id());
    let holds = oracle.reservations_for_cell(CellId(12));
    let prior = holds.iter().find(|hold| hold.robot() == RobotId(3)).unwrap();
    let next = holds.iter().find(|hold| hold.robot() == RobotId(1)).unwrap();
    assert!(prior.window().end().unwrap() <= next.window().start());
}

#[derive(Display)]
#[display("PublishInductionLater")]
struct PublishInductionLater {
    task: Option<InductContainer>,
    at: SimTime,
}

#[async_trait(?Send)]
impl Task for PublishInductionLater {
    type Proposal = EmptyProposal;
    type Schedule = RobotId;
    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }
    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<RobotId, PlanError> {
        ctx.barrier().wait_until(self.at).await?;
        Ok(ctx.add_task(self.task.take().unwrap()).await.cell_reservation.robot())
    }
}

#[test]
fn productive_work_published_after_an_early_failure_wins_the_retry_contest() {
    let (model, oracle) = build_model(TestModelConfig::default());
    register_idle_htm(&model, &oracle, RobotId(1), CellId(10), SimTime::seconds(100));
    let original = model.available_htm.peek_matching(|_| true).pop().unwrap();
    let (_, registry) = parking_bay(&model, oracle.clone(), CellId(12));
    registry.reserve(CellId(12), RobotId(3), SimTime::ZERO);
    let mut scheduler = Scheduler::new();
    let mut task = parking_task(&model, &scheduler, Some(registry));
    task.navigation = Arc::new(Navigation::new(
        model.navigation.map.clone(),
        Arc::new(NoParkingSearch),
        model.bot_service.clone(),
        oracle.clone(),
    ));
    let need_by =
        NeedVacated { navigation: task.navigation.clone(), hold: original.cell_reservation.clone(), by: SimTime::ZERO };
    let parking = scheduler.add_task(task);
    scheduler.add_service(need_by);
    let induction = InductContainer::new(
        CellId(20),
        ToteType::Carton,
        ToteDesignation::Order,
        SimTime::seconds(200),
        model.navigation.clone(),
        model.bot_service.clone(),
        model.inventory.clone(),
        model.available_htm.clone(),
        None,
        model.events.clone(),
        scheduler.barrier().clock(),
    );
    let productive = scheduler.add_task(PublishInductionLater { task: Some(induction), at: SimTime::seconds(50) });
    scheduler.run().unwrap();
    let runtime = Builder::new_current_thread().build().unwrap();
    assert_eq!(runtime.block_on(parking), ParkingOutcome::Superseded);
    assert_eq!(runtime.block_on(productive), RobotId(1));
    assert!(scheduler.dump_group(scheduler.group_of(TaskId(0)).unwrap()).unwrap().contains("[dissolved]"));
    assert!(
        oracle
            .reservations_for_cell(original.cell())
            .iter()
            .filter(|hold| hold.robot() == original.robot())
            .all(|hold| !hold.is_open_ended())
    );
}

#[test]
fn instant_box_supply_reuses_a_parked_htm_without_moving_or_replaying_its_parking_leg() {
    let (model, oracle) = build_model(TestModelConfig::default());
    register_idle_htm(&model, &oracle, RobotId(1), CellId(0), SimTime::ZERO);
    let (navigation, registry) = parking_bay(&model, oracle.clone(), CellId(12));
    let mut scheduler = Scheduler::new();
    let mut parking = parking_task(&model, &scheduler, Some(registry.clone()));
    parking.navigation = navigation.clone();
    let parked = scheduler.add_task(parking);
    scheduler.run().unwrap();
    assert_eq!(Builder::new_current_thread().build().unwrap().block_on(parked), ParkingOutcome::Parked);
    let entry = model.available_htm.peek_matching(|_| true).pop().unwrap();
    assert_eq!(entry.cell(), CellId(12));
    let ready = entry.available_at();
    let supply = InductContainer::new(
        CellId(10),
        ToteType::Carton,
        ToteDesignation::Order,
        ready,
        navigation,
        model.bot_service.clone(),
        model.inventory.clone(),
        model.available_htm.clone(),
        Some(registry),
        model.events.clone(),
        scheduler.barrier().clock(),
    )
    .with_instant_supply();
    let completion = scheduler.add_task(supply);
    scheduler.run().unwrap();
    let order = Builder::new_current_thread().build().unwrap().block_on(completion);
    assert_eq!(order.cell_reservation.cell(), CellId(12));
    assert_eq!(order.action_ready_at, ready, "neither another journey nor pickup dwell is introduced");
    model.bot_service.for_each(BotKind::Htm, |_, plan| {
        assert_eq!(plan.position_at(ready).unwrap().cell, CellId(12));
        assert_eq!(plan.availability(), Some((ready, CellId(12))));
        assert_eq!(plan.carrying_tote(), Some(order.tote_id));
    });
}
