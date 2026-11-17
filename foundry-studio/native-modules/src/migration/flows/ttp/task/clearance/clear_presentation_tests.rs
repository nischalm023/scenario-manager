//! A presentation's clearance is a last-resort claim that a need-by can make
//! due at the barrier without moving its start: a productive claim on the
//! same stay that departs when the presentation ends wins it, and a delivery
//! waiting in the station's queue moves up once the winner's departure is
//! scheduled.
//! An empty HTM's clearance that finds no parking bay competes again at its
//! departure when it was decided early, and after that on each path commit
//! until the barrier passes its departure, when it stops the run.

use std::sync::{Arc, Mutex, mpsc};

use desgraph::{CellId, ZoneId};

use super::*;
use crate::events::{EventJournal, SimEvent};
use crate::flows::transport::{HtmPosition, Navigate};
use crate::flows::ttp::handoff::HtmCarryingTote;
use crate::flows::ttp::inflight::InflightSpan;
use crate::flows::ttp::task::HtmDeliver;
use crate::model::ParkingRegistry;
use crate::model::htm::registry::ProposedHtmClaim;
use crate::model::ids::{PresentationId, TaskId};
use crate::model::reservation::oracle::TableOracle;
use crate::model::test_model::{
    NeedVacated, TestModel, TestModelConfig, block, bounded_scheduler, build_model, current_hold, provide_states,
    register_idle_htm, register_loaded_htm,
};
use crate::model::{ToteDesignation, ToteLocation, ToteType};
use crate::pathfinding::{FHSippPathfinder, Orientation};
use crate::utils::temporal_counter::TemporalCounter;
use desgraph::{BlockId, RelayId};
use desmap::{BlockSide, RelayPosition, VtmPosition};
use pathfinder::ReservationOracle as _;

pub(super) const STATION: StationId = StationId(1);
pub(super) const STATION_CELL: CellId = CellId(10);
pub(super) const PRESENTATION_END: SimTime = SimTime::seconds(30);

/// Robot 61 standing on the station cell since simtime zero, available once
/// its presentation ends: the stay a finished presentation registers.
pub(super) fn standing_after_presentation(model: &TestModel, oracle: &TableOracle) -> AvailableHtm {
    register_idle_htm(model, oracle, RobotId(61), STATION_CELL, SimTime::ZERO);
    let stay = model.available_htm.take(RobotId(61)).unwrap();
    model.available_htm.register(stay.cell_reservation, PRESENTATION_END, stay.orientation, None).unwrap();
    model.available_htm.get(RobotId(61)).unwrap()
}

pub(super) fn clearance(model: &TestModel, entry: AvailableHtm, clock: Mut<SimTime>) -> ClearPresentation {
    ClearPresentation::new(
        entry,
        STATION,
        ToteSource::Pick,
        model.navigation.clone(),
        model.available_htm.clone(),
        clock,
    )
}

#[test]
fn a_need_by_makes_the_clearance_due_at_the_barrier_without_moving_its_start() {
    let (model, oracle) = build_model(TestModelConfig::default());
    let entry = standing_after_presentation(&model, &oracle);
    let barrier = SimTime::seconds(5);
    let clear = clearance(&model, entry.clone(), Mut::new(barrier));
    let proposal = block(Task::propose(&clear, ProposalId { task: TaskId(9), attempt: 0 })).unwrap();
    assert!(model.available_htm.is_available(entry.availability_id()), "proposing the clearance spends no stay");
    assert!(current_hold(&oracle, &entry.cell_reservation).is_open_ended(), "proposing the clearance spends no hold");
    let start = PRESENTATION_END;
    assert_eq!(Proposal::start_time(&proposal), Some(start));
    assert_eq!(Proposal::resolve_by(&proposal), Some(start), "with no need-by, it is decided at its own start");

    let vacating = model.navigation.cells.vacating(&entry.cell_reservation).expect("the clearance's Vacate");
    assert_eq!(vacating.at, start);
    let need_by = model.navigation.cells.need_by(vacating, barrier).unwrap();
    assert_eq!(Proposal::resolve_by(&proposal), Some(barrier), "a need-by makes it due at the barrier");
    assert_eq!(Proposal::start_time(&proposal), Some(start), "the physical start does not move");
    drop(need_by);
    assert_eq!(Proposal::resolve_by(&proposal), Some(start));
}

#[test]
fn clearance_retries_an_evicted_proposal_and_accepts_without_a_competitor() {
    let (model, oracle) = build_model(TestModelConfig::default());
    let robot = RobotId(60);
    model
        .inventory
        .register_tote(
            ToteId(700),
            ToteType::Tote,
            ToteDesignation::Inventory,
            ToteLocation::WithRobot { robot_id: robot },
        )
        .expect("carried tote");
    register_idle_htm(&model, &oracle, robot, CellId(5), SimTime::ZERO);
    let entry = model.available_htm.get(robot).unwrap();
    let clear = clearance(&model, entry.clone(), Mut::new(SimTime::ZERO));
    let attempt = block(Task::propose(&clear, ProposalId { task: TaskId(90), attempt: 0 })).unwrap();
    drop(attempt);
    assert_eq!(Task::dissolve(&clear), None, "eviction alone does not cancel clearance");
    assert!(model.available_htm.is_available(entry.availability_id()));
    let mut retry = block(Task::propose(&clear, ProposalId { task: TaskId(90), attempt: 1 })).unwrap();
    assert_eq!(Proposal::start_time(&retry), Some(SimTime::ZERO));
    ProposedReservation::accept(&mut retry).unwrap();
    assert!(
        !model.available_htm.is_available(entry.availability_id()),
        "a last-resort claim alone takes the stay at once"
    );
}

/// The end-to-end handover. Robot 61 holds the station cell open-ended, its
/// presentation ending at `PRESENTATION_END` (simtime); a delivery to that
/// cell drives at once to the queue cell next to it and stands there. When the
/// barrier reaches the presentation's end, the productive claim on the same
/// stay wins the clearance and drives the HTM away; the delivery moves up onto
/// the station cell only once that departure is scheduled, and enters only
/// after the hold closes.
#[test]
fn a_productive_claim_wins_the_clearance_at_the_release_and_the_delivery_waiting_in_the_queue_moves_up() {
    let (mut model, oracle) = build_model(TestModelConfig {
        stations: vec![(STATION, STATION_CELL)],
        queue_cells: vec![CellId(7), CellId(8), CellId(9)],
        ..Default::default()
    });
    let (tx, rx) = mpsc::channel();
    let markers = tx.clone();
    Arc::get_mut(&mut model).unwrap().events = EventJournal::new(Some(Arc::new(tx)));
    let incumbent = standing_after_presentation(&model, &oracle);
    let follower = RobotId(60);
    let tote = ToteId(1);
    model
        .inventory
        .register_tote(tote, ToteType::Tote, ToteDesignation::Inventory, ToteLocation::WithRobot { robot_id: follower })
        .unwrap();
    register_loaded_htm(&model, &oracle, follower, CellId(5), tote);
    let source = model.available_htm.take(follower).unwrap();

    let journal = model.events.clone();
    let mut scheduler = bounded_scheduler().with_advance_listener(Box::new(move |at| {
        journal.barrier_advanced(at);
        markers.send(SimEvent::BarrierAdvanced(at)).unwrap();
    }));
    scheduler.add_task(HtmDeliver::new(
        STATION,
        InflightSpan::open(Arc::new(Mutex::new(TemporalCounter::new())), SimTime::ZERO, Version::new()),
        HtmCarryingTote {
            tote_id: tote,
            zone_id: ZoneId(1),
            cell_reservation: source.cell_reservation,
            action_ready_at: SimTime::ZERO,
            orientation: Orientation::East,
            station_arrival_plan: None,
            presentation_id: PresentationId(1),
            source_relay_id: None,
            deliver_from: None,
        },
        scheduler.barrier().clock(),
        model.navigation.clone(),
        model.bot_service.clone(),
        model.inventory.clone(),
        None,
        model.events.clone(),
        model.station_service.clone(),
        model.station_arrivals.clone(),
        None,
        false,
        None,
    ));
    let cleared = scheduler.add_task(clearance(&model, incumbent.clone(), scheduler.barrier().clock()));
    scheduler.add_task(ProductiveClaim {
        model: model.clone(),
        entry: incumbent.clone(),
        to: CellId(20),
        clock: scheduler.barrier().clock(),
    });
    scheduler.run().unwrap();

    assert_eq!(block(cleared), ClearanceOutcome::Superseded, "the productive claim wins the stay");
    let mut barrier = SimTime::ZERO;
    let mut windows = Vec::new();
    for event in rx.try_iter() {
        match event {
            SimEvent::BarrierAdvanced(at) => barrier = at,
            SimEvent::Nav(event) => windows.push((barrier, event)),
            _ => {}
        }
    }
    let of = |robot| windows.iter().filter(move |(_, event)| event.robot_id == robot).collect::<Vec<_>>();
    let (inbound, outbound) = (of(follower), of(RobotId(61)));
    let ends: Vec<_> = inbound.iter().map(|(_, event)| event.waypoints.last().unwrap().cell).collect();
    assert_eq!(ends, [CellId(9), STATION_CELL], "into the queue, then onto the station's cell");
    assert_eq!(inbound[0].1.time, SimTime::ZERO, "the delivery drives into the queue at once");
    let (outbound_barrier, outbound) = outbound.first().expect("the incumbent left");
    assert!(outbound.time >= PRESENTATION_END, "the incumbent left before its presentation ended");
    let (move_up_barrier, move_up) = inbound[1];
    assert!(
        *move_up_barrier >= *outbound_barrier,
        "the delivery moved up at B={move_up_barrier}, before the incumbent's departure was scheduled at B={outbound_barrier}"
    );
    assert!(move_up.time >= outbound.time, "the delivery moved up before the incumbent departed (simtime)");
    let holds = oracle.reservations_for_cell(STATION_CELL);
    let cleared_at =
        holds.iter().find(|hold| hold.id() == incumbent.cell_reservation.id()).unwrap().window().end().unwrap();
    let arrival = holds.iter().find(|hold| hold.robot() == follower).unwrap().window().start();
    assert!(arrival >= cleared_at, "the delivery entered before the incumbent left");
    assert_eq!(model.navigation.cells.vacate_count(), 0, "a Vacate outlived its proposal");
}

/// A productive task's claim on one stay; once accepted it drives
/// the HTM off its cell, departing at the claim's start.
#[derive(Display)]
#[display("ProductiveClaim")]
pub(super) struct ProductiveClaim {
    pub(super) model: Arc<TestModel>,
    pub(super) entry: AvailableHtm,
    pub(super) to: CellId,
    pub(super) clock: Mut<SimTime>,
}

#[async_trait(?Send)]
impl Task for ProductiveClaim {
    type Proposal = ProposedHtmClaim;
    type Schedule = ();

    async fn propose(&self, owner: ProposalId) -> Result<ProposedHtmClaim, PlanError> {
        Ok(self.model.available_htm.propose_productive(
            &self.entry,
            owner,
            self.entry.action_ready_at,
            self.clock.clone(),
        ))
    }

    async fn schedule(&mut self, proposal: ProposedHtmClaim, ctx: SchedCtx) -> Result<(), PlanError> {
        let (entry, depart) = proposal
            .into_accepted()
            .map_err(|error| PlanError::TaskGraphInvariant { context: "ProductiveClaim", detail: error.to_string() })?;
        ctx.add_task(Navigate::new(
            HtmPosition {
                cell_reservation: entry.cell_reservation,
                action_ready_at: depart,
                orientation: entry.orientation,
            },
            self.to,
            None,
            self.model.navigation.clone(),
            self.model.bot_service.clone(),
            self.model.inventory.clone(),
            None,
            self.model.events.clone(),
            ctx.barrier().clock(),
        ))
        .await;
        Ok(())
    }
}

/// Adds `task` to the graph once the barrier reaches `at`: work published
/// later in scheduler time.
#[derive(Display)]
#[display("Later")]
struct Later<T> {
    at: SimTime,
    task: Option<T>,
}

#[async_trait(?Send)]
impl<T: Task + 'static> Task for Later<T> {
    type Proposal = EmptyProposal;
    type Schedule = T::Schedule;

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<T::Schedule, PlanError> {
        ctx.barrier().wait_until(self.at).await?;
        Ok(ctx.add_task(self.task.take().expect("scheduled once")).await)
    }
}

/// An empty HTM's clearance wins its stay when the barrier reaches the
/// presentation's end, and no parking bay is free. No path commit follows
/// that could free one, so once the barrier passes the presentation's end the
/// run stops, naming the station, the HTM and the departure.
#[test]
fn an_empty_htm_with_no_free_bay_stops_the_run_once_the_barrier_passes_its_departure() {
    let (model, oracle) = build_model(TestModelConfig::default());
    let entry = standing_after_presentation(&model, &oracle);
    let mut scheduler = bounded_scheduler();
    provide_states(&scheduler, &model, ParkingRegistry::default());
    scheduler.add_task(clearance(&model, entry, scheduler.barrier().clock()));
    let error = scheduler.run().unwrap_err();
    assert!(
        matches!(
            error,
            PlanError::ClearanceCannotDepart {
                station: STATION,
                robot: RobotId(61),
                departure: PRESENTATION_END,
                cause: ClearanceBlock::NoParkingBay,
            }
        ),
        "{error}"
    );
    assert_eq!(
        scheduler.barrier().floor(),
        PRESENTATION_END + SimDuration::ms(1),
        "the run stops as the barrier passes it"
    );
}

/// The one parking bay, at cell 12, is held by robot 3 when the barrier
/// reaches the presentation's end and the empty HTM's clearance wins its stay,
/// so the search finds no bay. Robot 3's drive out of the bay commits later
/// at that barrier; the clearance competes again on that commit, before the
/// barrier passes the presentation's end, and parks the HTM in the bay.
#[test]
fn an_empty_htm_parks_in_a_bay_freed_before_the_barrier_passes_its_departure() {
    const BAY: CellId = CellId(12);
    let (model, oracle) = build_model(TestModelConfig::default());
    let entry = standing_after_presentation(&model, &oracle);
    register_idle_htm(&model, &oracle, RobotId(3), BAY, SimTime::ZERO);
    let occupant = model.available_htm.take(RobotId(3)).unwrap();
    let mut map = model.navigation.map.as_ref().clone();
    map.relays = vec![RelayPosition {
        id: RelayId(12),
        position: BAY,
        htm_access: BAY,
        vtm_access: VtmPosition::Cell(CellId(0)),
        block: BlockId(0),
        side: BlockSide::Left,
    }];
    let resting_at_bay = Arc::new(FHSippPathfinder::new(&map, 1000, SimDuration::seconds(1), [BAY].into()));
    let navigation = Arc::new(Navigation::new(
        model.navigation.map.clone(),
        resting_at_bay,
        model.bot_service.clone(),
        oracle.clone(),
    ));
    let mut scheduler = bounded_scheduler();
    let states = scheduler.states();
    states.provide(navigation.clone());
    states.provide(Arc::new(model.bot_service.clone()));
    states.provide(Arc::new(model.inventory.clone()));
    states.provide(Arc::new(model.available_htm.clone()));
    states.provide(Arc::new(model.events.clone()));
    states.provide(Arc::new(ParkingRegistry::from_map(&map)));
    let cleared = scheduler.add_task(clearance(&model, entry, scheduler.barrier().clock()));
    scheduler.add_task(Later {
        at: PRESENTATION_END,
        task: Some(Navigate::new(
            HtmPosition {
                cell_reservation: occupant.cell_reservation,
                action_ready_at: PRESENTATION_END,
                orientation: Orientation::East,
            },
            CellId(14),
            None,
            navigation,
            model.bot_service.clone(),
            model.inventory.clone(),
            None,
            model.events.clone(),
            scheduler.barrier().clock(),
        )),
    });
    scheduler.run().unwrap();
    assert!(!scheduler.stopped(), "the run reached the test deadline");
    assert_eq!(block(cleared), ClearanceOutcome::Cleared);
    assert_eq!(model.available_htm.get(RobotId(61)).expect("parked").cell(), BAY);
    assert_eq!(
        scheduler.barrier().floor(),
        PRESENTATION_END,
        "parked before the barrier passed the presentation's end"
    );
}

/// A blocked entrant's need-by makes an empty HTM's clearance due at the
/// barrier, long before its stay starts (simtime), and no parking bay is free
/// then. That early failure cannot speak for the whole stay: the clearance
/// competes again once the barrier reaches the planned departure, by which
/// time a productive claim published in the meantime has taken the stay.
#[test]
fn an_early_parking_failure_competes_again_at_the_departure_and_yields_to_productive_work() {
    let (model, oracle) = build_model(TestModelConfig::default());
    let entry = standing_after_presentation(&model, &oracle);
    let mut scheduler = bounded_scheduler();
    provide_states(&scheduler, &model, ParkingRegistry::default());
    scheduler.add_service(NeedVacated {
        navigation: model.navigation.clone(),
        hold: entry.cell_reservation.clone(),
        by: SimTime::ZERO,
    });
    let cleared = scheduler.add_task(clearance(&model, entry.clone(), scheduler.barrier().clock()));
    let used = scheduler.add_task(Later {
        at: SimTime::seconds(10),
        task: Some(ProductiveClaim {
            model: model.clone(),
            entry: entry.clone(),
            to: CellId(20),
            clock: scheduler.barrier().clock(),
        }),
    });
    scheduler.run().unwrap();
    assert_eq!(block(cleared), ClearanceOutcome::Superseded, "the second contest yields the stay to productive work");
    block(used);
    assert!(!model.available_htm.is_available(entry.availability_id()), "the productive claim took the stay");
    assert!(
        !current_hold(&oracle, &entry.cell_reservation).is_open_ended(),
        "the productive claim's drive closed the station cell's hold"
    );
}
