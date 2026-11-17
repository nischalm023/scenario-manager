//! Direct putaway supply claims a standing empty's stay as ordinary use: it
//! outranks that stay's clearance when it departs at the empty's release
//! (simtime), and loses to the clearance when it would depart later. A
//! staging whose source the last lot's fill overtakes, in scheduler time,
//! stages nothing. A direct claim whose drive cannot depart at the release
//! hands the HTM back to the clearance.

use std::sync::atomic::Ordering;

use desgraph::{CellId, StationId};

use super::super::tests::{OPENING, PICK_CELL, STATION, STATION_CELL, idle_empty, supply};
use super::*;
use crate::flows::ttp::task::clearance::ClearanceOutcome;
use crate::model::ids::{RackId, TaskId};
use crate::model::reservation::oracle::TableOracle;
use crate::model::test_model::{
    TestModel, TestModelConfig, block, bounded_scheduler, build_model, current_hold, provide_states,
    register_loaded_htm,
};
use crate::model::{ToteLocation, ToteType};
use crate::simtime::TimeRange;

/// One lot still to fill: the arrival sequence a staging serves.
pub(super) fn one_lot() -> ArrivalQueue {
    Mut::new(VecDeque::from([(SkuId(1), 1, SimTime::ZERO)]))
}

/// The station's opening at `OPENING`, searching for its next empty.
pub(super) fn stage_empty(
    model: &TestModel,
    supply: &Arc<PutawaySupply>,
    arrivals: ArrivalQueue,
    clock: Mut<SimTime>,
) -> StageEmpty {
    StageEmpty {
        arrivals,
        clock,
        opening: StagingOpening {
            station_id: STATION,
            station_cell: STATION_CELL,
            after: OPENING,
            search_after: SimTime::ZERO,
        },
        ticket: Some(supply.begin_staging(STATION).unwrap()),
        supply: supply.clone(),
        direct_bias: SimOffset::ZERO,
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
    }
}

fn clearance(model: &TestModel, entry: AvailableHtm, clock: Mut<SimTime>) -> ClearPresentation {
    ClearPresentation::new(
        entry,
        STATION,
        ToteSource::Pick,
        model.navigation.clone(),
        model.available_htm.clone(),
        clock,
    )
}

/// An empty tote taken out of rack 1, carried by `robot` standing at `cell`
/// since simtime zero: an emptied tote that a clearance can take home.
pub(super) fn racked_empty(model: &TestModel, oracle: &TableOracle, robot: RobotId, tote: ToteId, cell: CellId) {
    let racked = ToteLocation::InRack { rack_id: RackId(1), z_index: 0 };
    model.inventory.register_tote(tote, ToteType::Tote, ToteDesignation::Inventory, racked).unwrap();
    model.inventory.tote(tote).unwrap().move_to(ToteLocation::WithRobot { robot_id: robot }, SimTime::ZERO).unwrap();
    register_loaded_htm(model, oracle, robot, cell, tote);
}

/// Direct supply's claim is an ordinary use of the empty's stay: it ranks
/// ahead of the stay's clearance when it departs at the empty's release, and
/// behind it when it would depart later.
#[test]
fn a_direct_claim_outranks_the_clearance_only_when_it_departs_at_the_release() {
    let (model, oracle) = build_model(TestModelConfig::default());
    idle_empty(&model, &oracle, RobotId(60), ToteId(700), CellId(5));
    let entry = model.available_htm.get(RobotId(60)).unwrap();
    let clock = Mut::new(SimTime::ZERO);
    let [clear_id, late_id, on_time_id] = [1, 2, 3].map(|task| ProposalId { task: TaskId(task), attempt: 0 });
    let clearance = model.available_htm.propose_clearance(&entry, clear_id, SimTime::ZERO, clock.clone());
    let direct = |owner, depart| {
        ProposedDirectEmpty::new(
            model.available_htm.clone(),
            DirectCandidate { entry: entry.clone(), depart, travel: SimDuration::seconds(5) },
            clock.clone(),
            owner,
        )
    };
    let late = direct(late_id, SimTime::seconds(20));
    assert_eq!(ProposedReservation::ranked_competitors(&late), Some(vec![clear_id, late_id]));
    let _on_time = direct(on_time_id, SimTime::ZERO);
    assert_eq!(ProposedReservation::ranked_competitors(&clearance), Some(vec![on_time_id, clear_id, late_id]));
}

#[derive(Display)]
#[display("DirectPutawayTest")]
struct DirectPutawayTest {
    registry: AvailableHtmRegistry,
    entry: AvailableHtm,
    depart: SimTime,
    clock: Mut<SimTime>,
}

#[async_trait(?Send)]
impl Task for DirectPutawayTest {
    type Proposal = StageProposal;
    /// The empty and the departure its claim was accepted for; `None` once
    /// another claim took the stay.
    type Schedule = Option<(RobotId, SimTime)>;
    fn dissolve(&self) -> Option<Self::Schedule> {
        (!self.registry.is_available(self.entry.availability_id())).then_some(None)
    }
    async fn propose(&self, owner: ProposalId) -> Result<StageProposal, PlanError> {
        Ok(StageProposal {
            source: EmptySource::Direct(ProposedDirectEmpty::new(
                self.registry.clone(),
                DirectCandidate { entry: self.entry.clone(), depart: self.depart, travel: SimDuration::seconds(5) },
                self.clock.clone(),
                owner,
            )),
            arrivals: one_lot(),
        })
    }
    async fn schedule(&mut self, proposal: StageProposal, _: SchedCtx) -> Result<Self::Schedule, PlanError> {
        let EmptySource::Direct(direct) = proposal.source else { panic!("direct") };
        let (entry, depart, _) = direct.into_accepted()?;
        Ok(Some((entry.robot(), depart)))
    }
}

/// An emptied tote's HTM is released at simtime zero, and its clearance and a
/// live direct claim compete for its stay, whichever is published first. A
/// direct claim that departs at the release takes the stay, and the clearance
/// plans no departure. One that would depart 20 s later loses: the clearance
/// takes the tote home, and the direct claim's task dissolves.
#[test]
fn a_live_direct_claim_wins_the_clearance_only_when_it_departs_at_the_release() {
    let tote = ToteId(700);
    for (depart, direct_first) in
        [(SimTime::ZERO, false), (SimTime::ZERO, true), (SimTime::seconds(20), false), (SimTime::seconds(20), true)]
    {
        let (model, oracle) = build_model(TestModelConfig::default());
        racked_empty(&model, &oracle, RobotId(60), tote, CellId(5));
        let entry = model.available_htm.get(RobotId(60)).unwrap();
        let mut scheduler = bounded_scheduler();
        provide_states(&scheduler, &model, ParkingRegistry::default());
        model.relay_service.set_admission_floor(scheduler.barrier().clock());
        model.relay_service.vtm_service.set_admission_floor(scheduler.barrier().clock());
        let clear = clearance(&model, entry.clone(), scheduler.barrier().clock());
        let direct = DirectPutawayTest {
            registry: model.available_htm.clone(),
            entry: entry.clone(),
            depart,
            clock: scheduler.barrier().clock(),
        };
        let (clear, direct) = if direct_first {
            let direct = scheduler.add_task(direct);
            (scheduler.add_task(clear), direct)
        } else {
            let clear = scheduler.add_task(clear);
            (clear, scheduler.add_task(direct))
        };
        scheduler.run().unwrap();
        assert!(!scheduler.stopped(), "the run reached the test deadline");
        assert!(!model.available_htm.is_available(entry.availability_id()));
        if depart == SimTime::ZERO {
            assert_eq!(block(clear), ClearanceOutcome::Superseded);
            assert_eq!(block(direct), Some((RobotId(60), SimTime::ZERO)));
            assert!(
                current_hold(&oracle, &entry.cell_reservation).is_open_ended(),
                "the losing clearance planned no departure"
            );
            assert!(model.navigation.cells.vacating(&entry.cell_reservation).is_none());
        } else {
            assert_eq!(block(clear), ClearanceOutcome::Cleared);
            assert_eq!(block(direct), None, "the late direct claim's task dissolves");
            assert!(
                matches!(model.inventory.tote(tote).unwrap().location(), ToteLocation::InRack { .. }),
                "the clearance took the tote home"
            );
        }
    }
}

/// Induction's last fill, scheduled once the barrier reaches `at`: the
/// arrival sequence is exhausted.
#[derive(Display)]
#[display("LastFill")]
pub(super) struct LastFill {
    pub(super) arrivals: ArrivalQueue,
    pub(super) at: SimTime,
}

#[async_trait(?Send)]
impl Task for LastFill {
    type Proposal = EmptyProposal;
    type Schedule = ();
    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }
    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<(), PlanError> {
        ctx.barrier().wait_until(self.at).await?;
        self.arrivals.update(VecDeque::clear);
        Ok(())
    }
}

/// A direct proposal for the opening is published, then the last lot's fill
/// is scheduled while the proposal is pending, its decision due when the
/// barrier reaches the opening. Nothing will fill the empty it would stage:
/// the exhausted arrival sequence poisons the proposal, eviction dissolves
/// the StageEmpty, and the empty never leaves the cell it stands on.
#[test]
fn a_staging_proposal_dissolves_when_the_last_fill_is_scheduled_before_its_acceptance() {
    let (model, oracle) =
        build_model(TestModelConfig { stations: vec![(STATION, STATION_CELL)], ..Default::default() });
    idle_empty(&model, &oracle, RobotId(60), ToteId(700), CellId(5));
    let entry = model.available_htm.get(RobotId(60)).unwrap();
    let supply = Arc::new(supply(&model));
    let arrivals = one_lot();
    let mut scheduler = bounded_scheduler();
    scheduler.add_task(stage_empty(&model, &supply, arrivals.clone(), scheduler.barrier().clock()));
    scheduler.add_task(LastFill { arrivals, at: SimTime::seconds(50) });
    let run = scheduler.run();
    assert!(model.available_htm.is_available(entry.availability_id()), "direct supply took the empty's stay");
    assert!(current_hold(&oracle, &entry.cell_reservation).is_open_ended(), "a delivery bounded the empty's hold");
    assert_eq!(supply.stations[&STATION].pending.load(Ordering::Relaxed), 0, "the staging ticket is released");
    run.unwrap();
    assert!(!scheduler.stopped(), "the run lasted until the test deadline: the StageEmpty never dissolved");
}

/// The pick station's operator releases an emptied tote's HTM on the pick
/// cell at simtime zero, and the putaway station's cell is held open-ended by
/// another robot whose departure nothing has scheduled (scheduler time), with
/// no queue cell to wait on. Run with and without a staging for the putaway
/// station, the clearance's outcome, when the HTM's hold on the pick cell
/// ends (simtime), and the station's in-flight count.
fn release_with_putaway_cell_held(direct: bool) -> (ClearanceOutcome, Option<SimTime>, (isize, isize)) {
    let tote = ToteId(700);
    let stations = vec![(StationId(2), PICK_CELL), (STATION, STATION_CELL)];
    let (model, oracle) = build_model(TestModelConfig { stations, ..TestModelConfig::default() });
    racked_empty(&model, &oracle, RobotId(60), tote, PICK_CELL);
    oracle.table().reserve_cell(RobotId(61), STATION_CELL, TimeRange::open_ended(SimTime::ZERO)).unwrap();
    let entry = model.available_htm.get(RobotId(60)).unwrap();
    let supply = Arc::new(supply(&model));
    let mut scheduler = bounded_scheduler();
    provide_states(&scheduler, &model, ParkingRegistry::default());
    model.relay_service.set_admission_floor(scheduler.barrier().clock());
    model.relay_service.vtm_service.set_admission_floor(scheduler.barrier().clock());
    let clear = scheduler.add_task(clearance(&model, entry.clone(), scheduler.barrier().clock()));
    let staged = direct.then(|| {
        let mut staging = stage_empty(&model, &supply, one_lot(), scheduler.barrier().clock());
        staging.opening.after = SimTime::ZERO;
        scheduler.add_task(staging)
    });
    scheduler.run().unwrap();
    assert!(!scheduler.stopped(), "the run reached the test deadline");
    staged.map(block);
    assert!(
        matches!(model.inventory.tote(tote).unwrap().location(), ToteLocation::InRack { .. }),
        "the clearance took the tote home"
    );
    let left = current_hold(&oracle, &entry.cell_reservation).window().end();
    let inflight = supply.stations[&STATION].inflight.lock().unwrap();
    (block(clear), left, (inflight.count_at(SimTime::ZERO), inflight.frontier().1))
}

/// The direct claim wins the stay at acceptance (scheduler time), but its
/// drive could only wait (simtime) on the pick cell: it is refused, and in
/// the same scheduler-time instant the clearance takes the HTM home,
/// departing the pick cell at the release exactly as a clearance that won
/// outright does (its planned turn on the cell included). The staging's
/// in-flight span is withdrawn.
#[test]
fn a_direct_supply_that_cannot_depart_at_the_release_hands_the_htm_to_the_clearance() {
    let (outcome, left, inflight) = release_with_putaway_cell_held(true);
    let (alone, left_alone, _) = release_with_putaway_cell_held(false);
    assert_eq!((outcome, alone), (ClearanceOutcome::Superseded, ClearanceOutcome::Cleared));
    assert_eq!(left, left_alone, "the HTM leaves the pick cell as the clearance alone plans it");
    assert!(left.is_some_and(|left| left < SimTime::seconds(1)), "it leaves within its turn, not after a wait");
    assert_eq!(inflight, (0, 0), "the in-flight span is withdrawn");
}
