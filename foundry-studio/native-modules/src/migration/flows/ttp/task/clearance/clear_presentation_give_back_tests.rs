//! A clearance gives back what it no longer needs: the deposit relay its
//! putaway fill reserved, when a productive claim wins the stay; and it takes
//! back a stay a direct supply could not depart on only at the stay's
//! release.

use desgraph::{CellId, RelayId};

use super::tests::{PRESENTATION_END, ProductiveClaim, STATION, STATION_CELL, clearance, standing_after_presentation};
use super::*;
use crate::model::ParkingRegistry;
use crate::model::bot::BotKind;
use crate::model::ids::PresentationId;
use crate::model::relay::RelayReservation;
use crate::model::test_model::{
    TestModel, TestModelConfig, block, bounded_scheduler, build_model, claim_the_zones_deposits, empty_rack_slots,
    provide_states, register_loaded_htm,
};
use crate::model::{ToteDesignation, ToteLocation, ToteType};
use crate::pathfinding::Orientation;

/// The relay a deposit off `from` at the presentation's end reserves for
/// `robot`.
fn reserve_off(model: &TestModel, from: CellId, robot: RobotId) -> Option<RelayReservation<RackSlot>> {
    block(model.relay_service.reserve_relay_for_deposit(from, Orientation::East, PRESENTATION_END, robot, None)).ok()
}

/// Reserves a deposit off the station's cell for robot 91 once the
/// clearance has resolved, while the run goes on.
#[derive(Display)]
#[display("ReserveOnceCleared")]
struct ReserveOnceCleared {
    model: Arc<TestModel>,
    cleared: Option<Completion<ClearanceOutcome>>,
}

#[async_trait(?Send)]
impl Task for ReserveOnceCleared {
    type Proposal = EmptyProposal;
    type Schedule = (ClearanceOutcome, Option<RelayId>);

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, _: SchedCtx) -> Result<Self::Schedule, PlanError> {
        let outcome = self.cleared.take().expect("scheduled once").await;
        let relay = &self.model.relay_service;
        let after =
            relay.reserve_relay_for_deposit(STATION_CELL, Orientation::East, PRESENTATION_END, RobotId(91), None);
        Ok((
            outcome,
            after.await.ok().map(|deposit| {
                let relay_id = deposit.relay_id;
                deposit.cancel();
                relay_id
            }),
        ))
    }
}

/// Robot 61's fill reserved a relay for its tote's return, and other
/// deposits then claimed whatever was left, so the next is declined. A
/// productive claim on the same stay, departing at the release, wins it; the
/// clearance dissolves and gives the relay back as it resolves, so a
/// deposit for robot 91 reserves it while the run goes on.
#[test]
fn a_clearance_a_productive_claim_supersedes_gives_its_return_relay_back() {
    let (model, oracle) = build_model(TestModelConfig::default());
    let entry = standing_after_presentation(&model, &oracle);
    empty_rack_slots(&model, 1..=4);
    model.bot_service.register(RobotId(91), BotKind::Htm, CellId(1)).unwrap();
    let mut scheduler = bounded_scheduler();
    let held = reserve_off(&model, STATION_CELL, RobotId(61)).expect("a free relay for the fill's return");
    let held_relay = held.relay_id;
    let others = claim_the_zones_deposits(&model, CellId(15), PRESENTATION_END);
    let mut clear = clearance(&model, entry.clone(), scheduler.barrier().clock());
    clear.return_relay = Some(held);
    let cleared = scheduler.add_task(clear);
    let watched = scheduler.add_task(ReserveOnceCleared { model: model.clone(), cleared: Some(cleared) });
    let used = scheduler.add_task(ProductiveClaim {
        model: model.clone(),
        entry: entry.clone(),
        to: CellId(25),
        clock: scheduler.barrier().clock(),
    });
    scheduler.run().unwrap();
    block(used);
    let (outcome, after) = block(watched);
    assert_eq!(outcome, ClearanceOutcome::Superseded);
    assert_eq!(after, Some(held_relay), "the superseded clearance gave its relay back");
    others.into_iter().for_each(RelayReservation::cancel);
}

/// Hands `entry` back to its clearance as a refused direct supply does,
/// refused at `departure`.
#[derive(Display)]
#[display("HandBack")]
struct HandBack {
    entry: AvailableHtm,
    departure: SimTime,
}

#[async_trait(?Send)]
impl Task for HandBack {
    type Proposal = EmptyProposal;
    type Schedule = Result<(), PlanError>;

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<Result<(), PlanError>, PlanError> {
        Ok(ClearPresentation::take_back(&ctx, self.entry.clone(), STATION, ToteSource::Pick, self.departure))
    }
}

/// Robot 62 stands on cell 5 with a tote, its presentation released at its
/// end. A direct supply's refusal names a departure 5 s later: no refusal
/// can be made then, so the hand-back stops the run with a task-graph
/// invariant error and no return is planned.
#[test]
fn a_hand_back_refused_after_the_release_stops_the_run() {
    let (model, oracle) = build_model(TestModelConfig::default());
    let (robot, tote) = (RobotId(62), ToteId(1));
    let carried = ToteLocation::WithRobot { robot_id: robot };
    model.inventory.register_tote(tote, ToteType::Tote, ToteDesignation::Inventory, carried).unwrap();
    register_loaded_htm(&model, &oracle, robot, CellId(5), tote);
    let stay = model.available_htm.take(robot).unwrap();
    let presentation = Some(PresentationId(1));
    model.available_htm.register(stay.cell_reservation, PRESENTATION_END, stay.orientation, presentation).unwrap();
    let entry = model.available_htm.take(robot).unwrap();
    let mut scheduler = bounded_scheduler();
    provide_states(&scheduler, &model, ParkingRegistry::default());
    let handed = scheduler.add_task(HandBack { entry, departure: PRESENTATION_END + SimDuration::seconds(5) });
    scheduler.run().unwrap();
    let refused = block(handed).unwrap_err();
    assert!(
        matches!(&refused, PlanError::TaskGraphInvariant { context: "ClearPresentation", detail } if detail.contains("not at its release")),
        "{refused}"
    );
    assert!(matches!(model.inventory.tote(tote).unwrap().location(), ToteLocation::WithRobot { .. }), "no return");
}
