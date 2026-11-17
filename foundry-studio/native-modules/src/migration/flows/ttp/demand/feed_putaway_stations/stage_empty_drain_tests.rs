//! The drain: an empty staged before the last lot's fill is scheduled, which
//! nothing will fill, goes home with a clearance once a deposit relay is
//! reserved for its tote (scheduler time), and stands on the station's cell
//! (simtime) until the departure that relay was reserved for.

use desgraph::CellId;
use pathfinder::ReservationOracle as _;

use super::super::tests::{STATION, STATION_CELL, supply};
use super::tests::{LastFill, one_lot, racked_empty, stage_empty};
use super::*;
use crate::model::ToteLocation;
use crate::model::test_model::{
    TestModelConfig, bounded_scheduler, build_model, claim_the_zones_deposits, empty_rack_slots, provide_states,
};

/// The empty the staging delivers never gets its fill: the last lot's fill
/// is scheduled, once the barrier reaches 200 s, after the empty was staged.
/// The StageEmpty hands it to a clearance, which takes it home instead of
/// leaving it on the station's cell.
#[test]
fn a_staged_empty_that_nothing_will_fill_is_cleared_home() {
    let (model, oracle) =
        build_model(TestModelConfig { stations: vec![(STATION, STATION_CELL)], ..Default::default() });
    let tote = ToteId(700);
    racked_empty(&model, &oracle, RobotId(60), tote, CellId(5));
    let supply = Arc::new(supply(&model));
    let arrivals = one_lot();
    let mut scheduler = bounded_scheduler();
    provide_states(&scheduler, &model, ParkingRegistry::default());
    model.relay_service.set_admission_floor(scheduler.barrier().clock());
    model.relay_service.vtm_service.set_admission_floor(scheduler.barrier().clock());
    scheduler.add_task(stage_empty(&model, &supply, arrivals.clone(), scheduler.barrier().clock()));
    scheduler.add_task(LastFill { arrivals, at: SimTime::seconds(200) });
    scheduler.run().unwrap();
    assert!(!scheduler.stopped(), "the run lasted until the test deadline: the staged empty never went to a clearance");
    assert!(
        matches!(model.inventory.tote(tote).unwrap().location(), ToteLocation::InRack { .. }),
        "the staged empty stayed on the station's cell"
    );
}

/// A decision due once the barrier reaches simtime `at`: the barrier does not
/// pass `at` until the scheduler has made it (scheduler time). A target held
/// at `at` is reached first, and the tasks it wakes run before the decision.
#[derive(Display)]
#[display("DueAt({_0})")]
struct DueAt(SimTime);

impl ProposedReservation for DueAt {
    fn start_time(&self) -> Option<SimTime> {
        Some(self.0)
    }

    fn accept(&mut self) -> Result<(), AcceptError> {
        Ok(())
    }
}

/// Cancels `deposit` (scheduler time) once its decision, due at simtime
/// `at`, is made, which leaves that relay free to be reserved.
#[derive(Display)]
#[display("GiveBackAt")]
struct GiveBackAt {
    deposit: Option<RelayReservation<RackSlot>>,
    at: SimTime,
}

#[async_trait(?Send)]
impl Task for GiveBackAt {
    type Proposal = DueAt;
    type Schedule = ();
    async fn propose(&self, _: ProposalId) -> Result<DueAt, PlanError> {
        Ok(DueAt(self.at))
    }
    async fn schedule(&mut self, _: DueAt, _: SchedCtx) -> Result<(), PlanError> {
        self.deposit.take().expect("built holding the deposit it gives back, and scheduled once").cancel();
        Ok(())
    }
}

/// An empty staged and never filled, the last lot's fill scheduled once the
/// barrier reaches 200 s, while deposits off the station's cell hold every
/// relay the zone admits, each with an open reservation: the drain's attempt
/// is declined, no departure is scheduled for the empty, and the run goes
/// on. One of those reservations is given back once its decision, due at
/// `relay_free`, is made. Runs to the end, checks that the empty went home,
/// and returns when it departed (simtime) the station's cell.
fn drained_departure(relay_free: SimTime) -> SimTime {
    let (model, oracle) =
        build_model(TestModelConfig { stations: vec![(STATION, STATION_CELL)], ..Default::default() });
    let (robot, tote) = (RobotId(60), ToteId(700));
    racked_empty(&model, &oracle, robot, tote, CellId(5));
    empty_rack_slots(&model, 1..=4);
    let mut others = claim_the_zones_deposits(&model, STATION_CELL, SimTime::ZERO);
    let freed = others.pop().expect("a deposit holding a relay");
    let supply = Arc::new(supply(&model));
    let arrivals = one_lot();
    let mut scheduler = bounded_scheduler();
    provide_states(&scheduler, &model, ParkingRegistry::default());
    model.relay_service.set_admission_floor(scheduler.barrier().clock());
    model.relay_service.vtm_service.set_admission_floor(scheduler.barrier().clock());
    scheduler.add_task(stage_empty(&model, &supply, arrivals.clone(), scheduler.barrier().clock()));
    scheduler.add_task(LastFill { arrivals, at: SimTime::seconds(200) });
    scheduler.add_task(GiveBackAt { deposit: Some(freed), at: relay_free });
    scheduler.run().unwrap();
    assert!(!scheduler.stopped(), "the run waited until the test deadline");
    assert!(
        matches!(model.inventory.tote(tote).unwrap().location(), ToteLocation::InRack { .. }),
        "the staged empty stayed on the station's cell"
    );
    others.into_iter().for_each(RelayReservation::cancel);
    let holds = oracle.reservations_for_cell(STATION_CELL).into_iter().filter(|hold| hold.robot() == robot);
    holds.filter_map(|hold| hold.window().end()).max().expect("the empty stood on the station's cell")
}

/// The relay is given back once the barrier reaches 300 s. Whether the
/// drain's attempt there comes before or after that (scheduler time), the
/// drain reserves the relay for a drive departing (simtime) at 300 s, and the
/// clearance takes the empty home, departing the station's cell then.
#[test]
fn a_drained_empty_departs_at_the_barrier_at_which_a_relay_is_reserved_for_its_tote() {
    let relay_free = SimTime::seconds(300);
    let left = drained_departure(relay_free);
    assert!(left >= relay_free && left < relay_free + SimDuration::seconds(1), "it departed at {left}");
}

/// The relay is given back once the barrier reaches 200 s, after the drain's
/// first attempt there was declined (scheduler time), and the barrier does
/// not pass 200 s before it is: only the relay's change can wake the drain,
/// which reserves that relay, and the clearance takes the empty home,
/// departing (simtime) at 200 s.
#[test]
fn a_relay_given_back_before_the_barrier_moves_wakes_a_declined_drain() {
    let relay_free = SimTime::seconds(200);
    let left = drained_departure(relay_free);
    assert!(left >= relay_free && left < relay_free + SimDuration::seconds(1), "it departed at {left}");
}
