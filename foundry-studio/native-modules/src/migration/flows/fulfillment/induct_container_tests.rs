use crate::model::ids::TaskId;
use crate::pathfinding::Orientation;
use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

use tokio::runtime::Builder;

use super::*;
use crate::events::SimEvent;
use crate::model::reservation::oracle::{ReservationOracle, TableOracle};
use crate::model::test_model::{TestModel, TestModelConfig, build_model, register_idle_htm};
use crate::simtime::TimeRange;
use pathfinder::ReservationOracle as _;

struct Harness {
    model: Arc<TestModel>,
    oracle: Arc<TableOracle>,
    events: EventJournal,
    received: mpsc::Receiver<SimEvent>,
}

impl Harness {
    fn new() -> Self {
        let mut floor_model = crate::model::test_model::reference_model();
        Arc::get_mut(&mut floor_model).unwrap().figures.conveyor_pickup_secs = Some(5.0);
        let (model, oracle) = build_model(TestModelConfig { floor_model: Some(floor_model), ..Default::default() });
        let (sender, received) = mpsc::channel();
        Self { model, oracle, events: EventJournal::new(Some(Arc::new(sender))), received }
    }

    fn task(&self, scheduler: &Scheduler, after: SimTime) -> InductContainer {
        InductContainer::new(
            CellId(10),
            ToteType::Carton,
            ToteDesignation::Order,
            after,
            self.model.navigation.clone(),
            self.model.bot_service.clone(),
            self.model.inventory.clone(),
            self.model.available_htm.clone(),
            None,
            self.events.clone(),
            scheduler.barrier().clock(),
        )
        .with_station(StationId(1))
    }
}

#[test]
fn resident_induction_keeps_the_available_htm_heading() {
    let h = Harness::new();
    let robot = RobotId(1);
    register_idle_htm(&h.model, &h.oracle, robot, CellId(10), SimTime::ZERO);
    let mut entry = h.model.available_htm.take(robot).unwrap();
    entry.orientation = Orientation::North;
    h.model.available_htm.put_back(entry).unwrap();
    let mut scheduler = Scheduler::new();
    let completion = scheduler.add_task(h.task(&scheduler, SimTime::ZERO));
    scheduler.run().unwrap();
    let handoff = Builder::new_current_thread().build().unwrap().block_on(completion);
    assert_eq!(handoff.orientation, Orientation::North, "picking up a box does not rotate its HTM");
}

#[test]
fn induction_creates_at_arrival_and_moves_custody_only_after_full_pickup_dwell() {
    let h = Harness::new();
    let robot = RobotId(1);
    register_idle_htm(&h.model, &h.oracle, robot, CellId(0), SimTime::ZERO);
    let mut scheduler = Scheduler::new();
    let completion = scheduler.add_task(h.task(&scheduler, SimTime::seconds(100)));
    assert_eq!(h.model.inventory.tote_count(), 0, "construction must not create a box");
    scheduler.run().unwrap();
    let handoff = Builder::new_current_thread().build().unwrap().block_on(completion);
    let emitted: Vec<_> = h.received.try_iter().collect();
    let births: Vec<_> = emitted
        .iter()
        .filter_map(|event| match event {
            SimEvent::ToteCreated(event) => Some(event),
            _ => None,
        })
        .collect();
    assert_eq!(births.len(), 1);
    let birth = births[0];
    assert_eq!(birth.tote_id, handoff.tote_id);
    assert_eq!(birth.tote_type, ToteType::Carton);
    assert_eq!(birth.designation, ToteDesignation::Order);
    assert_eq!(birth.initial_state, ToteVisualState::AtCell { cell_id: 10 });
    assert!(birth.time >= SimTime::seconds(100));
    assert_eq!(handoff.action_ready_at, birth.time + SimDuration::seconds(5));
    let tote = h.model.inventory.tote(handoff.tote_id).unwrap();
    assert_eq!(tote.location_at(birth.time - SimDuration::ms(1)), ToteLocation::Uninitialized);
    assert_eq!(tote.location_at(birth.time), ToteLocation::AtCell { cell_id: CellId(10) });
    assert_eq!(
        tote.location_at(handoff.action_ready_at - SimDuration::ms(1)),
        ToteLocation::AtCell { cell_id: CellId(10) }
    );
    assert_eq!(tote.location_at(handoff.action_ready_at), ToteLocation::WithRobot { robot_id: robot });
    assert_eq!(handoff.cell_reservation.robot(), robot);
    assert_eq!(handoff.cell_reservation.cell(), CellId(10));
    assert!(handoff.cell_reservation.is_open_ended());
    assert_eq!(h.model.available_htm.len(), 0, "parent retains the loaded HTM");
    assert!(handoff.source_relay_id.is_none());
    assert!(emitted.iter().any(|event| matches!(event, SimEvent::ToteMoved(event)
        if event.tote_id == handoff.tote_id && event.time == handoff.action_ready_at
        && event.new_state == (ToteVisualState::OnBot { bot_id: robot.0 }))));
    h.model.bot_service.for_each(BotKind::Htm, |_, plan| {
        assert_eq!(plan.carrying_tote(), Some(handoff.tote_id));
        assert_eq!(plan.availability(), Some((handoff.action_ready_at, CellId(10))));
    });
}

#[test]
fn instant_supply_claims_an_empty_htm_at_its_own_future_availability_without_visiting_induction() {
    let h = Harness::new();
    let robot = RobotId(1);
    register_idle_htm(&h.model, &h.oracle, robot, CellId(0), SimTime::seconds(200));
    register_idle_htm(&h.model, &h.oracle, RobotId(2), CellId(10), SimTime::ZERO);
    let _busy_holder = h.model.available_htm.take(RobotId(2)).unwrap();
    let mut scheduler = Scheduler::new();
    let task = h.task(&scheduler, SimTime::seconds(100)).with_instant_supply();
    assert_eq!(h.model.inventory.tote_count(), 0, "no box exists before acceptance");
    let completion = scheduler.add_task(task);
    scheduler.run().unwrap();
    let handoff = Builder::new_current_thread().build().unwrap().block_on(completion);
    assert_eq!(handoff.action_ready_at, SimTime::seconds(200));
    assert_eq!(handoff.cell_reservation.cell(), CellId(0));
    assert_eq!(handoff.cell_reservation.robot(), robot);
    assert!(handoff.cell_reservation.is_open_ended());
    assert!(handoff.deliver_from.is_none(), "this box did not visit a station");
    let tote = h.model.inventory.tote(handoff.tote_id).unwrap();
    assert_eq!(tote.location_at(SimTime::seconds(199)), ToteLocation::Uninitialized);
    assert_eq!(tote.location_at(SimTime::seconds(200)), ToteLocation::WithRobot { robot_id: robot });
    let events: Vec<_> = h.received.try_iter().collect();
    assert!(!events.iter().any(|event| matches!(event, SimEvent::Nav(_))), "supply does not move the HTM");
    assert_eq!(events.iter().filter(|event| matches!(event, SimEvent::ToteCreated(_))).count(), 1);
    assert_eq!(h.model.available_htm.len(), 0, "the acquired HTM stays with its order");
}

#[derive(Display)]
#[display("AcquireAndLeave")]
struct AcquireAndLeave {
    induction: Option<InductContainer>,
    navigation: Arc<Navigation>,
    inventory: InventoryService,
    bots: BotService,
    events: EventJournal,
}

#[async_trait(?Send)]
impl Task for AcquireAndLeave {
    type Proposal = EmptyProposal;
    type Schedule = HtmCarryingTote;
    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }
    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<HtmCarryingTote, PlanError> {
        let mut order = ctx.add_task(self.induction.take().unwrap()).await;
        let position = ctx
            .add_task(Navigate::new(
                HtmPosition {
                    cell_reservation: order.cell_reservation,
                    action_ready_at: order.action_ready_at,
                    orientation: order.orientation,
                },
                CellId(11),
                Some(order.tote_id),
                self.navigation.clone(),
                self.bots.clone(),
                self.inventory.clone(),
                None,
                self.events.clone(),
                ctx.barrier().clock(),
            ))
            .await;
        order.cell_reservation = position.cell_reservation;
        order.action_ready_at = position.action_ready_at;
        order.orientation = position.orientation;
        Ok(order)
    }
}

#[test]
fn concurrent_induction_waits_for_the_loaded_htm_to_clear_the_source() {
    let h = Harness::new();
    register_idle_htm(&h.model, &h.oracle, RobotId(1), CellId(10), SimTime::ZERO);
    register_idle_htm(&h.model, &h.oracle, RobotId(2), CellId(0), SimTime::ZERO);
    let mut scheduler = Scheduler::new();
    let first = scheduler.add_task(AcquireAndLeave {
        induction: Some(h.task(&scheduler, SimTime::ZERO)),
        navigation: h.model.navigation.clone(),
        inventory: h.model.inventory.clone(),
        bots: h.model.bot_service.clone(),
        events: h.events.clone(),
    });
    let second = scheduler.add_task(h.task(&scheduler, SimTime::seconds(1)));
    scheduler.run().unwrap();
    let runtime = Builder::new_current_thread().build().unwrap();
    let first = runtime.block_on(first);
    let second = runtime.block_on(second);
    assert_ne!(first.tote_id, second.tote_id);
    assert_ne!(first.cell_reservation.robot(), second.cell_reservation.robot());
    let source_holds = h.model.navigation.oracle.reservations_for_cell(CellId(10));
    let first_hold = source_holds.iter().find(|hold| hold.robot() == first.cell_reservation.robot()).unwrap();
    let second_hold = source_holds.iter().find(|hold| hold.robot() == second.cell_reservation.robot()).unwrap();
    assert!(first_hold.window().end().unwrap() <= second_hold.window().start());
    let mut births: Vec<_> = h
        .received
        .try_iter()
        .filter_map(|event| match event {
            SimEvent::ToteCreated(event) => Some(event),
            _ => None,
        })
        .collect();
    births.sort_by_key(|event| event.time);
    assert_eq!(births.len(), 2);
    assert!(births[0].time + SimDuration::seconds(5) < births[1].time);
    assert_eq!(h.model.available_htm.len(), 0);
}

#[derive(Display)]
#[display("StagedInduction")]
struct StagedInduction {
    task: InductContainer,
    first: RefCell<Option<ProposedHtmPickup>>,
    proposals: Arc<AtomicUsize>,
}

#[async_trait(?Send)]
impl Task for StagedInduction {
    type Proposal = ProposedHtmPickup;
    type Schedule = HtmCarryingTote;

    async fn propose(&self, id: ProposalId) -> Result<Self::Proposal, PlanError> {
        self.proposals.fetch_add(1, Ordering::SeqCst);
        let first = self.first.borrow_mut().take();
        if let Some(first) = first { Ok(first) } else { Task::propose(&self.task, id).await }
    }

    async fn schedule(&mut self, proposal: Self::Proposal, ctx: SchedCtx) -> Result<Self::Schedule, PlanError> {
        Task::schedule(&mut self.task, proposal, ctx).await
    }
}

#[derive(Display)]
#[display("VisitSourceBeforeInduction")]
struct VisitSourceBeforeInduction {
    incoming: Option<Navigate>,
    navigation: Arc<Navigation>,
    inventory: InventoryService,
    bots: BotService,
    events: EventJournal,
}

#[async_trait(?Send)]
impl Task for VisitSourceBeforeInduction {
    type Proposal = EmptyProposal;
    type Schedule = HtmPosition;

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<Self::Schedule, PlanError> {
        let mut position = ctx.add_task(self.incoming.take().unwrap()).await;
        position.action_ready_at = SimTime::seconds(200);
        Ok(ctx
            .add_task(Navigate::new(
                position,
                CellId(11),
                None,
                self.navigation.clone(),
                self.bots.clone(),
                self.inventory.clone(),
                None,
                self.events.clone(),
                ctx.barrier().clock(),
            ))
            .await)
    }
}

#[test]
fn an_induction_proposal_losing_its_clear_source_does_not_block_the_occupants_later_departure() {
    let h = Harness::new();
    register_idle_htm(&h.model, &h.oracle, RobotId(1), CellId(0), SimTime::ZERO);
    register_idle_htm(&h.model, &h.oracle, RobotId(2), CellId(11), SimTime::ZERO);
    let visitor = h.model.available_htm.take(RobotId(2)).unwrap();
    let mut scheduler = Scheduler::new();
    let task = h.task(&scheduler, SimTime::seconds(100));
    let selection = task.select().unwrap().unwrap();
    let first = ProposedHtmPickup::new(
        ProposalId { task: TaskId(9000), attempt: 0 },
        task.available.clone(),
        task.navigation.clone(),
        task.source,
        selection,
        task.available.version().seen(),
        task.clock.clone(),
    );
    assert!(!ProposedReservation::poisoned(&first), "the provisional selection observes an empty source");
    let proposals = Arc::new(AtomicUsize::new(0));
    let induction =
        scheduler.add_task(StagedInduction { task, first: RefCell::new(Some(first)), proposals: proposals.clone() });
    let visit = scheduler.add_task(VisitSourceBeforeInduction {
        incoming: Some(Navigate::new(
            HtmPosition {
                cell_reservation: visitor.cell_reservation,
                action_ready_at: SimTime::ZERO,
                orientation: Orientation::East,
            },
            CellId(10),
            None,
            h.model.navigation.clone(),
            h.model.bot_service.clone(),
            h.model.inventory.clone(),
            None,
            h.events.clone(),
            scheduler.barrier().clock(),
        )),
        navigation: h.model.navigation.clone(),
        inventory: h.model.inventory.clone(),
        bots: h.model.bot_service.clone(),
        events: h.events.clone(),
    });
    scheduler.run().unwrap();
    let runtime = Builder::new_current_thread().build().unwrap();
    let visit = runtime.block_on(visit);
    let induction = runtime.block_on(induction);
    assert!(proposals.load(Ordering::SeqCst) >= 2, "arrival invalidates the pending induction selection");
    assert_eq!(visit.cell_reservation.cell(), CellId(11));
    assert!(induction.action_ready_at > SimTime::seconds(200));
    assert_eq!(induction.cell_reservation.robot(), RobotId(1));
    assert_eq!(h.model.inventory.tote_count(), 1);
    let holds = h.model.navigation.oracle.reservations_for_cell(CellId(10));
    let departed = holds.iter().find(|hold| hold.robot() == RobotId(2)).unwrap();
    assert!(departed.window().end().unwrap() <= induction.cell_reservation.window().start());
}

#[test]
fn discarding_an_induction_proposal_creates_nothing_and_keeps_the_htm_available() {
    let h = Harness::new();
    register_idle_htm(&h.model, &h.oracle, RobotId(1), CellId(10), SimTime::ZERO);
    let scheduler = Scheduler::new();
    let task = h.task(&scheduler, SimTime::seconds(10));
    let selection = task.select().unwrap().unwrap();
    let proposal = ProposedHtmPickup::new(
        ProposalId { task: TaskId(9000), attempt: 0 },
        task.available.clone(),
        task.navigation.clone(),
        task.source,
        selection,
        task.available.version().seen(),
        task.clock.clone(),
    );
    drop(proposal);
    assert_eq!(h.model.inventory.tote_count(), 0);
    assert_eq!(h.model.available_htm.len(), 1);
    assert_eq!(h.received.try_iter().count(), 0);
}

#[test]
fn induction_refuses_a_second_container_on_an_already_loaded_htm() {
    let h = Harness::new();
    let robot = RobotId(1);
    register_idle_htm(&h.model, &h.oracle, robot, CellId(10), SimTime::ZERO);
    let existing = h
        .model
        .inventory
        .create_tote(ToteType::Tote, ToteDesignation::Order, ToteLocation::WithRobot { robot_id: robot }, SimTime::ZERO)
        .unwrap();
    h.model
        .bot_service
        .commit_task(
            robot,
            TaskKind::Deliver,
            &[LegSpec {
                leg: Leg::StationWork { cell: CellId(10) },
                requests: &[],
                action_ready_arrival: SimTime::ZERO,
                wp_kind: WaypointKind::Present,
                expect: BotExpectation::empty(),
                carrying_after: Some(existing),
            }],
            &h.model.inventory,
            None,
        )
        .unwrap();
    let htm = h.model.available_htm.take(robot).unwrap();
    let mut scheduler = Scheduler::new();
    scheduler.add_task(CreateContainer {
        position: Some(HtmPosition {
            cell_reservation: htm.cell_reservation,
            action_ready_at: SimTime::ZERO,
            orientation: Orientation::East,
        }),
        tote_type: ToteType::Carton,
        designation: ToteDesignation::Order,
        inventory: h.model.inventory.clone(),
        bots: h.model.bot_service.clone(),
        events: h.events.clone(),
    });
    assert!(scheduler.run().is_err());
    assert_eq!(h.model.inventory.tote_count(), 1);
    assert_eq!(h.received.try_iter().count(), 0);
    h.model.bot_service.for_each(BotKind::Htm, |_, plan| assert_eq!(plan.carrying_tote(), Some(existing)));
}

#[test]
fn induction_skips_a_boxed_in_nearer_htm_without_claiming_it() {
    let h = Harness::new();
    let trapped = RobotId(1);
    let free = RobotId(2);
    register_idle_htm(&h.model, &h.oracle, trapped, CellId(5), SimTime::ZERO);
    register_idle_htm(&h.model, &h.oracle, free, CellId(20), SimTime::ZERO);
    for cell in [CellId(4), CellId(6)] {
        h.oracle.table().reserve_cell(RobotId(90 + cell.0), cell, TimeRange::open_ended(SimTime::ZERO)).unwrap();
    }
    let mut scheduler = Scheduler::new();
    let completion = scheduler.add_task(h.task(&scheduler, SimTime::ZERO));
    scheduler.run().unwrap();
    let handoff = Builder::new_current_thread().build().unwrap().block_on(completion);
    assert_eq!(handoff.cell_reservation.robot(), free);
    assert!(h.model.available_htm.contains(trapped), "selection must not claim and put back the trapped robot");
    assert!(
        h.oracle.reservations_for_cell(CellId(5)).iter().any(|hold| hold.robot() == trapped && hold.is_open_ended())
    );
}

#[test]
fn induction_rechecks_departure_after_its_exact_travel_estimate() {
    let h = Harness::new();
    register_idle_htm(&h.model, &h.oracle, RobotId(1), CellId(5), SimTime::ZERO);
    for cell in [CellId(4), CellId(6)] {
        h.oracle.table().reserve_cell(RobotId(90 + cell.0), cell, TimeRange::open_ended(SimTime::seconds(50))).unwrap();
    }
    let scheduler = Scheduler::new();
    assert!(h.task(&scheduler, SimTime::seconds(20)).select().unwrap().is_some());
    assert!(h.task(&scheduler, SimTime::seconds(100)).select().unwrap().is_none());
    assert_eq!(h.model.available_htm.len(), 1);
}

#[test]
fn a_new_departure_blocker_invalidates_an_induction_claim_without_fleet_changes() {
    let h = Harness::new();
    let robot = RobotId(1);
    register_idle_htm(&h.model, &h.oracle, robot, CellId(5), SimTime::ZERO);
    let scheduler = Scheduler::new();
    let task = h.task(&scheduler, SimTime::seconds(100));
    let selection = task.select().unwrap().unwrap();
    let seen = task.available.version().seen();
    let mut proposal = ProposedHtmPickup::new(
        ProposalId { task: TaskId(9000), attempt: 0 },
        task.available.clone(),
        task.navigation.clone(),
        task.source,
        selection,
        seen,
        task.clock.clone(),
    );
    assert!(!ProposedReservation::poisoned(&proposal));
    for cell in [CellId(4), CellId(6)] {
        h.oracle.table().reserve_cell(RobotId(90 + cell.0), cell, TimeRange::open_ended(SimTime::ZERO)).unwrap();
    }
    assert_eq!(task.available.version().seen(), seen);
    assert!(ProposedReservation::poisoned(&proposal));
    assert!(matches!(proposal.accept(), Err(AcceptError::Poisoned { .. })));
    assert!(task.available.contains(robot));
    assert!(ProposedReservation::admissible(&proposal), "a poisoned claim must not pin the planning clock");
}

#[test]
fn boxed_in_induction_wakes_when_an_exit_holder_commits_departure() {
    let h = Harness::new();
    let picker = RobotId(1);
    let blocker = RobotId(2);
    register_idle_htm(&h.model, &h.oracle, picker, CellId(5), SimTime::ZERO);
    register_idle_htm(&h.model, &h.oracle, blocker, CellId(6), SimTime::ZERO);
    let entry = h.model.available_htm.take(blocker).unwrap();
    h.oracle.table().reserve_cell(RobotId(94), CellId(4), TimeRange::open_ended(SimTime::ZERO)).unwrap();
    let mut scheduler = Scheduler::new();
    let seen = h.model.available_htm.version().seen();
    assert!(h.task(&scheduler, SimTime::ZERO).select().unwrap().is_none());
    let completion = scheduler.add_task(h.task(&scheduler, SimTime::ZERO));
    scheduler.add_task(Navigate::new(
        HtmPosition {
            cell_reservation: entry.cell_reservation,
            action_ready_at: SimTime::seconds(30),
            orientation: Orientation::East,
        },
        CellId(20),
        None,
        h.model.navigation.clone(),
        h.model.bot_service.clone(),
        h.model.inventory.clone(),
        None,
        h.events.clone(),
        scheduler.barrier().clock(),
    ));
    scheduler.run().unwrap();
    let handoff = Builder::new_current_thread().build().unwrap().block_on(completion);
    assert_eq!(handoff.cell_reservation.robot(), picker);
    assert!(handoff.action_ready_at > SimTime::seconds(30));
    assert_eq!(h.model.available_htm.version().seen(), seen + 1, "only the final pickup claim changes the fleet");
    let holds = h.oracle.reservations_for_cell(CellId(6));
    let departing = holds.iter().find(|hold| hold.robot() == blocker).unwrap();
    let following = holds.iter().find(|hold| hold.robot() == picker).unwrap();
    assert!(departing.window().end().unwrap() <= following.window().start());
}
