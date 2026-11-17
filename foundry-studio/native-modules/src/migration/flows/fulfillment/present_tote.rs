//! Select a source presentation in readiness order before its paired target
//! must depart. Intentions are revocable; only an accepted approach claims a
//! side, and the subsequent pick still requires both physical arrivals.

use std::cell::Cell;
use std::fmt;

use desgraph::{CellId, StationId};
use tokio::select;

use crate::flows::transport::HtmPosition;
use crate::model::conveyance::{
    Conveyance, ConveyedTote, InventoryPresentation, PresentationApproach, PresentationClaim, PresentationIntent,
    PresentationQuote,
};
use crate::model::navigation::Navigation;
use crate::model::reservation::oracle::ReservationOracle;
use crate::pathfinding::{PathCacheKey, PathfinderError};
use crate::prelude::*;
use crate::scheduler::task::proposal::{AcceptError, ProposedReservation};

#[derive(Display)]
#[display("PrepareOrderPresentation")]
pub(super) struct PrepareOrderPresentation {
    tote: ConveyedTote,
    conveyance: Conveyance,
    inventory_ready: SimTime,
    position: HtmPosition,
    navigation: Arc<Navigation>,
    destinations: Vec<(StationId, CellId)>,
}

pub(super) struct PreparedOrderPresentation {
    pub inventory: InventoryPresentation,
    pub depart_at: SimTime,
}

impl PrepareOrderPresentation {
    pub fn new(
        tote: ConveyedTote,
        conveyance: Conveyance,
        inventory_ready: SimTime,
        position: HtmPosition,
        navigation: Arc<Navigation>,
        destinations: Vec<(StationId, CellId)>,
    ) -> Self {
        Self { tote, conveyance, inventory_ready, position, navigation, destinations }
    }
}

#[async_trait(?Send)]
impl Task for PrepareOrderPresentation {
    type Proposal = EmptyProposal;
    type Schedule = PreparedOrderPresentation;

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<Self::Schedule, PlanError> {
        let approach = OrderApproach::estimate(&self.position, &self.navigation, &self.destinations)?;
        prepare(&self.tote, &self.conveyance, self.inventory_ready, Approach::Order(Arc::new(approach)), &ctx).await
    }
}

struct OrderApproach {
    robot: RobotId,
    ready: SimTime,
    oracle: Arc<dyn ReservationOracle>,
    destinations: Vec<ApproachDestination>,
}

struct ApproachDestination {
    station: StationId,
    cell: CellId,
    travel: SimDuration,
}

impl OrderApproach {
    fn estimate(
        position: &HtmPosition,
        navigation: &Navigation,
        destinations: &[(StationId, CellId)],
    ) -> Result<Self, PlanError> {
        let from = position.cell_reservation.cell();
        let cache = navigation.cache(position.cell_reservation.robot())?;
        let mut estimated = Vec::new();
        for &(station, cell) in destinations {
            let travel = if from == cell {
                SimDuration::ZERO
            } else {
                match cache.travel_time(PathCacheKey {
                    from,
                    from_orientation: position.orientation,
                    to: cell,
                    carrying_cargo: true,
                }) {
                    Ok(travel) => travel,
                    Err(PathfinderError::Unreachable { .. } | PathfinderError::NoStaticRoute { .. }) => continue,
                    Err(error) => return Err(error.into()),
                }
            };
            estimated.push(ApproachDestination { station, cell, travel });
        }
        if estimated.is_empty() {
            return Err(super::invalid("order HTM has no reachable presentation"));
        }
        Ok(Self {
            robot: position.cell_reservation.robot(),
            ready: position.action_ready_at,
            oracle: navigation.oracle.clone(),
            destinations: estimated,
        })
    }

    fn available_at(&self, cell: CellId) -> Option<SimTime> {
        let holds = self.oracle.reservations_for_cell(cell);
        if holds.iter().any(|hold| hold.robot() != self.robot && hold.is_open_ended()) {
            None
        } else {
            Some(
                holds
                    .iter()
                    .filter(|hold| hold.robot() != self.robot)
                    .filter_map(|hold| hold.window().end())
                    .max()
                    .unwrap_or(SimTime::ZERO),
            )
        }
    }
}

#[derive(Clone)]
enum Approach {
    #[cfg(test)]
    Fixed(Vec<PresentationApproach>),
    Order(Arc<OrderApproach>),
}

impl Approach {
    fn order_ready(&self) -> SimTime {
        match self {
            #[cfg(test)]
            Self::Fixed(_) => SimTime::ZERO,
            Self::Order(order) => order.ready,
        }
    }

    fn oracle_version(&self) -> Option<Version> {
        match self {
            #[cfg(test)]
            Self::Fixed(_) => None,
            Self::Order(order) => order.oracle.version().cloned(),
        }
    }

    fn choices(&self) -> Vec<PresentationApproach> {
        match self {
            #[cfg(test)]
            Self::Fixed(choices) => choices.clone(),
            Self::Order(order) => order
                .destinations
                .iter()
                .map(|destination| PresentationApproach {
                    station: destination.station,
                    travel: destination.travel,
                    available_at: order.available_at(destination.cell),
                })
                .collect(),
        }
    }

    fn snapshot(&self) -> ApproachAvailability {
        let oracle_version = self.oracle_version().map(|version| {
            let seen = Cell::new(version.seen());
            (version, seen)
        });
        ApproachAvailability { approach: self.clone(), choices: self.choices(), oracle_version }
    }
}

struct ApproachAvailability {
    approach: Approach,
    choices: Vec<PresentationApproach>,
    oracle_version: Option<(Version, Cell<u64>)>,
}

impl ApproachAvailability {
    fn changed(&self) -> bool {
        let epoch = self.oracle_version.as_ref().map(|(version, checked)| (version.seen(), checked));
        if epoch.is_some_and(|(current, checked)| current == checked.get()) {
            return false;
        }
        // A different side becoming available can change the choice too.
        // Unrelated path commits only advance the notification epoch.
        if self.approach.choices() != self.choices {
            return true;
        }
        if let Some((current, checked)) = epoch {
            checked.set(current);
        }
        false
    }
}

async fn prepare(
    tote: &ConveyedTote,
    conveyance: &Conveyance,
    inventory_ready: SimTime,
    approach: Approach,
    ctx: &SchedCtx,
) -> Result<PreparedOrderPresentation, PlanError> {
    let intent = conveyance
        .request_presentation(tote, inventory_ready, approach.order_ready(), approach.choices())
        .map_err(|error| super::invalid(error.to_string()))?;
    Ok(ctx.add_task(SelectPresentation { intent, approach, clock: ctx.barrier().clock() }).await)
}

#[derive(Display)]
#[display("SelectPresentation")]
struct SelectPresentation {
    intent: PresentationIntent,
    approach: Approach,
    clock: Mut<SimTime>,
}

struct ProposedPresentation {
    intent: PresentationIntent,
    quote: PresentationQuote,
    seen: u64,
    availability: ApproachAvailability,
    clock: Mut<SimTime>,
    accepted: Option<(PresentationClaim, SimTime)>,
}

impl fmt::Display for ProposedPresentation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "presentation{{station={:?} depart={} source={}}}",
            self.quote.station, self.quote.depart_at, self.quote.inventory_ready
        )
    }
}

impl ProposedReservation for ProposedPresentation {
    fn start_time(&self) -> Option<SimTime> {
        Some(self.quote.depart_at.max(self.clock.value()))
    }

    fn poisoned(&self) -> bool {
        self.intent.version().seen() != self.seen || self.availability.changed()
    }

    fn accept(&mut self) -> Result<(), AcceptError> {
        if ProposedReservation::poisoned(self) {
            return Err(AcceptError::Poisoned { detail: "presentation selection changed".into() });
        }
        let now = self.clock.value();
        let claim = self
            .intent
            .accept(self.quote, now)
            .map_err(|error| AcceptError::ActualConflict { detail: error.to_string() })?;
        self.accepted = Some((claim, self.quote.depart_at.max(now)));
        Ok(())
    }
}

#[async_trait(?Send)]
impl Task for SelectPresentation {
    type Proposal = ProposedPresentation;
    type Schedule = PreparedOrderPresentation;

    async fn propose(&self, _: ProposalId) -> Result<Self::Proposal, PlanError> {
        loop {
            let availability = self.approach.snapshot();
            self.intent
                .update_approaches(availability.choices.clone())
                .map_err(|error| super::invalid(error.to_string()))?;
            let seen = self.intent.version().seen();
            if let Some(quote) = self.intent.quote(self.clock.value()) {
                return Ok(ProposedPresentation {
                    intent: self.intent.clone(),
                    quote,
                    seen,
                    availability,
                    clock: self.clock.clone(),
                    accepted: None,
                });
            }
            if let Some((version, oracle_seen)) = availability.oracle_version {
                select! {
                    _ = self.intent.version().past(seen) => {},
                    _ = version.past(oracle_seen.get()) => {},
                }
            } else {
                self.intent.version().past(seen).await;
            }
        }
    }

    async fn schedule(&mut self, mut proposal: Self::Proposal, _: SchedCtx) -> Result<Self::Schedule, PlanError> {
        let (mut claim, depart_at) =
            proposal.accepted.take().ok_or_else(|| super::invalid("presentation was not accepted"))?;
        let inventory = claim.present(claim.at()).map_err(|error| super::invalid(error.to_string()))?;
        Ok(PreparedOrderPresentation { inventory, depart_at })
    }
}

#[cfg(test)]
mod tests {
    use crate::robots::runtime::RobotModelId;
    use pathfinder::ReservationOracle as _;
    use tokio::runtime::Builder;

    use super::*;
    use crate::model::conveyance::RetainedHtm;
    use crate::model::conveyor::ConveyorId;
    use crate::model::reservation::cell_reservation::CellReservation;
    use crate::model::reservation::oracle::TableOracle;
    use crate::model::test_model::{TestModelConfig, build_model};
    use crate::pathfinding::{CommitPlan, MoveOp, Orientation, ReservationRequest};
    use crate::simtime::TimeRange;

    struct PresentationFixture {
        task: SelectPresentation,
        navigation: Arc<Navigation>,
        oracle: Arc<TableOracle>,
        tote: ConveyedTote,
        retained: RetainedHtm,
        conveyance: Conveyance,
    }

    impl PresentationFixture {
        fn new(blocked_side: Option<CellId>) -> Self {
            let (model, oracle) =
                build_model(TestModelConfig { bot_htms: vec![(RobotId(100), CellId(0))], ..Default::default() });
            let position = HtmPosition {
                cell_reservation: hold(&oracle, RobotId(100), CellId(0), TimeRange::open_ended(SimTime::ZERO)),
                action_ready_at: SimTime::ZERO,
                orientation: Orientation::East,
            };
            if let Some(cell) = blocked_side {
                hold(&oracle, RobotId(101), cell, TimeRange::open_ended(SimTime::ZERO));
            }
            let order = OrderApproach::estimate(
                &position,
                &model.navigation,
                &[(StationId(1), CellId(10)), (StationId(2), CellId(20))],
            )
            .unwrap();
            let approach = Approach::Order(Arc::new(order));
            let conveyance = Conveyance::new(ConveyorId(0), 1, 1, [StationId(1), StationId(2)]).unwrap();
            let (tote, retained) = conveyance
                .try_admit(ToteId(1), RobotId(1), SimTime::ZERO)
                .unwrap()
                .unwrap()
                .board(SimTime::ZERO)
                .unwrap();
            let intent = conveyance
                .request_presentation(&tote, SimTime::seconds(100), approach.order_ready(), approach.choices())
                .unwrap();
            Self {
                task: SelectPresentation { intent, approach, clock: Mut::new(SimTime::ZERO) },
                navigation: model.navigation.clone(),
                oracle,
                tote,
                retained,
                conveyance,
            }
        }

        fn propose(&self) -> ProposedPresentation {
            Builder::new_current_thread()
                .build()
                .unwrap()
                .block_on(Task::propose(&self.task, ProposalId { task: TaskId(0), attempt: 0 }))
                .unwrap()
        }

        fn finish(self) {
            let mut proposal = self.propose();
            ProposedReservation::accept(&mut proposal).unwrap();
            let (mut claim, _) = proposal.accepted.take().unwrap();
            let presentation = claim.present(claim.at()).unwrap();
            presentation.clear(SimTime::seconds(110), SimTime::seconds(110)).unwrap();
            self.tote.collect(&self.retained, SimTime::seconds(110)).unwrap();
            assert_eq!(self.conveyance.unfinished(), 0);
            assert!(self.conveyance.abandoned_custody().is_empty());
        }
    }

    fn hold(oracle: &TableOracle, robot: RobotId, cell: CellId, window: TimeRange) -> CellReservation {
        oracle.commit_path(robot, &CommitPlan::single(ReservationRequest::new(cell, window)), None).unwrap().remove(0)
    }

    fn move_out(oracle: &TableOracle, predecessor: &CellReservation, to: CellId) {
        let window = TimeRange::new(SimTime::seconds(9), SimTime::seconds(10));
        oracle
            .commit_path(
                predecessor.robot(),
                &CommitPlan {
                    requests: vec![ReservationRequest::new(to, TimeRange::open_ended(window.start()))],
                    move_ops: vec![Some(MoveOp { source_cell: predecessor.cell(), dest_cell: to, window })],
                    close_predecessor_at: window.end(),
                },
                Some(predecessor),
            )
            .unwrap();
    }

    #[test]
    fn unrelated_movement_preserves_the_proposal_and_cached_travel() {
        let fixture = PresentationFixture::new(None);
        let other = hold(&fixture.oracle, RobotId(101), CellId(50), TimeRange::open_ended(SimTime::ZERO));
        let proposal = fixture.propose();
        assert!(!fixture.navigation.cache_for_model(RobotModelId(0)).unwrap().is_empty());
        fixture.navigation.cache_for_model(RobotModelId(0)).unwrap().clear();
        let before = fixture.oracle.version().unwrap().seen();
        move_out(&fixture.oracle, &other, CellId(51));
        let after = fixture.oracle.version().unwrap().seen();
        assert!(after > before);
        assert!(!ProposedReservation::poisoned(&proposal));
        assert_eq!(proposal.availability.oracle_version.as_ref().unwrap().1.get(), after);
        assert!(!ProposedReservation::poisoned(&proposal));
        assert_eq!(fixture.propose().quote, proposal.quote);
        assert!(
            fixture.navigation.cache_for_model(RobotModelId(0)).unwrap().is_empty(),
            "refreshes must reuse the task's static travel estimates"
        );
        fixture.finish();
    }

    #[test]
    fn a_foreign_target_hold_invalidates_the_proposal() {
        for window in [TimeRange::open_ended(SimTime::ZERO), TimeRange::new(SimTime::ZERO, SimTime::seconds(120))] {
            let fixture = PresentationFixture::new(None);
            let proposal = fixture.propose();
            assert_eq!(proposal.quote.station, StationId(1));
            hold(&fixture.oracle, RobotId(101), CellId(10), window);
            assert!(ProposedReservation::poisoned(&proposal));
            assert_eq!(fixture.propose().quote.station, StationId(2));
            fixture.finish();
        }
    }

    #[test]
    fn opening_an_alternative_side_invalidates_the_proposal() {
        let fixture = PresentationFixture::new(Some(CellId(10)));
        let proposal = fixture.propose();
        assert_eq!(proposal.quote.station, StationId(2));
        let blocker = fixture.oracle.reservations_for_cell(CellId(10)).remove(0);
        move_out(&fixture.oracle, &blocker, CellId(11));
        assert!(ProposedReservation::poisoned(&proposal), "a change on an unselected side can alter the winner");
        assert_eq!(fixture.propose().quote.station, StationId(1));
        fixture.finish();
    }

    #[test]
    fn the_order_robots_own_target_holds_do_not_invalidate_the_proposal() {
        let fixture = PresentationFixture::new(None);
        let proposal = fixture.propose();
        hold(&fixture.oracle, RobotId(100), CellId(10), TimeRange::open_ended(SimTime::ZERO));
        assert!(!ProposedReservation::poisoned(&proposal));
        assert_eq!(fixture.propose().quote, proposal.quote);
        fixture.finish();
    }

    fn present_in_registration_order(readiness: [u64; 2]) -> [(SimTime, SimTime); 2] {
        let service = Conveyance::new(ConveyorId(0), 2, 2, [StationId(1)]).unwrap();
        let mut scheduler = Scheduler::new();
        let tasks: Vec<_> = readiness
            .into_iter()
            .enumerate()
            .map(|(index, ready)| {
                let (tote, retained) = service
                    .try_admit(ToteId(index as u32), RobotId(index as u32), SimTime::ZERO)
                    .unwrap()
                    .unwrap()
                    .board(SimTime::ZERO)
                    .unwrap();
                scheduler.add_task(PrepareAndClear {
                    tote,
                    retained,
                    conveyance: service.clone(),
                    ready: SimTime::seconds(ready),
                    travel: SimDuration::ZERO,
                })
            })
            .collect();
        scheduler.run().unwrap();
        assert_eq!(service.unfinished(), 0);
        assert!(service.abandoned_custody().is_empty());
        let result = Builder::new_current_thread().build().unwrap().block_on(async {
            let mut times = Vec::new();
            for task in tasks {
                let (_, pick_start) = task.await;
                times.push((pick_start, pick_start + SimDuration::seconds(10)));
            }
            times
        });
        result.try_into().unwrap()
    }

    #[test]
    fn future_ready_parent_cannot_claim_a_slot_ahead_of_earlier_ready_work() {
        let times = present_in_registration_order([100, 50]);
        assert_eq!(times[1], (SimTime::seconds(50), SimTime::seconds(60)));
        assert_eq!(times[0], (SimTime::seconds(100), SimTime::seconds(110)));
    }

    #[test]
    fn a_waiting_presentation_rechecks_the_clock_after_slot_clearance() {
        let times = present_in_registration_order([0, 0]);
        assert_eq!(times[0], (SimTime::ZERO, SimTime::seconds(10)));
        assert_eq!(times[1], (SimTime::seconds(10), SimTime::seconds(20)));
    }

    #[derive(Display)]
    #[display("PrepareAndClear({ready})")]
    struct PrepareAndClear {
        tote: ConveyedTote,
        retained: RetainedHtm,
        conveyance: Conveyance,
        ready: SimTime,
        travel: SimDuration,
    }

    #[async_trait(?Send)]
    impl Task for PrepareAndClear {
        type Proposal = EmptyProposal;
        type Schedule = (SimTime, SimTime);

        async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
            Ok(EmptyProposal)
        }

        async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<Self::Schedule, PlanError> {
            let selected = prepare(
                &self.tote,
                &self.conveyance,
                self.ready,
                Approach::Fixed(vec![PresentationApproach {
                    station: StationId(1),
                    travel: self.travel,
                    available_at: Some(SimTime::ZERO),
                }]),
                &ctx,
            )
            .await?;
            let actual_arrival = selected.depart_at + self.travel;
            let mut pick = self
                .conveyance
                .try_pair(
                    &selected.inventory,
                    ToteId(100 + self.tote.tote().0),
                    RobotId(100 + self.tote.tote().0),
                    actual_arrival,
                    ctx.barrier().floor(),
                    SimDuration::seconds(10),
                )
                .unwrap();
            let pair = pick.commit().unwrap();
            ctx.barrier().wait_until(pair.end).await?;
            selected.inventory.clear(pair.end, pair.end).unwrap();
            self.tote.collect(&self.retained, pair.end).unwrap();
            Ok((selected.depart_at, pair.start))
        }
    }

    #[test]
    fn unequal_approaches_do_not_pin_the_clock_behind_an_unselected_earlier_departure() {
        let service = Conveyance::new(ConveyorId(0), 2, 2, [StationId(1)]).unwrap();
        let mut scheduler = Scheduler::new();
        // Register the T100 source first. Its ninety-second approach would
        // publish T10 if the unselected request incorrectly entered the barrier.
        let tasks: Vec<_> = [(100, 90), (50, 5)]
            .into_iter()
            .enumerate()
            .map(|(id, (ready, travel))| {
                let (tote, retained) = service
                    .try_admit(ToteId(id as u32), RobotId(id as u32), SimTime::ZERO)
                    .unwrap()
                    .unwrap()
                    .board(SimTime::ZERO)
                    .unwrap();
                scheduler.add_task(PrepareAndClear {
                    tote,
                    retained,
                    conveyance: service.clone(),
                    ready: SimTime::seconds(ready),
                    travel: SimDuration::seconds(travel),
                })
            })
            .collect();
        scheduler.run().unwrap();
        let times = Builder::new_current_thread().build().unwrap().block_on(async {
            let mut times = Vec::new();
            for task in tasks {
                times.push(task.await);
            }
            times
        });
        assert_eq!(times[1], (SimTime::seconds(45), SimTime::seconds(50)));
        assert_eq!(times[0], (SimTime::seconds(60), SimTime::seconds(150)));
        assert_eq!(scheduler.barrier().floor(), SimTime::seconds(160));
        assert_eq!(service.unfinished(), 0);
        assert!(service.abandoned_custody().is_empty());
    }
}
