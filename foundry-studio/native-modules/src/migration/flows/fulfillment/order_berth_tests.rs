//! Logical replacement starts at the final pick; physical admission waits
//! for the committed area exit, independently of later outbound completion.

use tokio::runtime::Builder;

use super::*;
use crate::model::test_model::{TestModelConfig, build_model};
use crate::simtime::TimeRange;

#[derive(Display)]
#[display("DelayedDeparture")]
struct DelayedDeparture {
    position: Option<HtmPosition>,
    navigation: Arc<Navigation>,
    bots: BotService,
    inventory: InventoryService,
    events: EventJournal,
    admission: AreaAdmission<StationId, OrderId>,
}

#[async_trait(?Send)]
impl Task for DelayedDeparture {
    type Proposal = EmptyProposal;
    type Schedule = ();
    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }
    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<(), PlanError> {
        ctx.barrier().wait_until(SimTime::seconds(20)).await?;
        let admission = self.admission.clone();
        let area = OrderHtmArea { entry: CellId(9), egress: CellId(11), resident_cells: vec![CellId(10)] };
        let _position = ctx
            .add_task(
                Navigate::new(
                    self.position.take().unwrap(),
                    CellId(11),
                    None,
                    self.navigation.clone(),
                    self.bots.clone(),
                    self.inventory.clone(),
                    None,
                    self.events.clone(),
                    ctx.barrier().clock(),
                )
                .with_commit_observer(Box::new({
                    let mut exit = BoundaryCrossing::exit(area);
                    move |window, which| {
                        if let Some(at) = exit.in_window(window, which)? {
                            admission.commit_exit(&StationId(1), &OrderId(0), at).unwrap();
                        }
                        Ok(())
                    }
                })),
            )
            .await;
        ctx.barrier().wait_until(SimTime::seconds(100)).await?;
        Ok(())
    }
}

#[derive(Display)]
#[display("ReplaceOrder")]
struct ReplaceOrder {
    outgoing: Option<DelayedDeparture>,
}

#[async_trait(?Send)]
impl Task for ReplaceOrder {
    type Proposal = EmptyProposal;
    type Schedule = (SimTime, SimTime, Completion<()>);
    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }
    async fn schedule(&mut self, _: EmptyProposal, ctx: SchedCtx) -> Result<Self::Schedule, PlanError> {
        let outgoing = self.outgoing.take().unwrap();
        let admission = outgoing.admission.clone();
        let ready = outgoing.position.as_ref().unwrap().action_ready_at;
        let dispatch = ctx.add_task(outgoing);
        let pending = PendingOrders::new(vec![OrderSpec::unstated(OrderId(1), Vec::new())])?;
        let (_, assigned) = ctx.add_task(ClaimNextOrder::new(pending, ready, ctx.barrier().clock())).await.unwrap();
        let claim =
            ctx.add_task(AdmitArea::new(admission, StationId(1), OrderId(1), assigned, ctx.barrier().clock())).await;
        Ok((assigned, claim.entry(), dispatch))
    }
}

#[test]
fn logical_replacement_precedes_physical_exit_and_admission_precedes_outbound_completion() {
    let (model, oracle) =
        build_model(TestModelConfig { bot_htms: vec![(RobotId(1), CellId(10))], ..Default::default() });
    let ready = SimTime::seconds(10);
    let current = oracle.table().reserve_cell(RobotId(1), CellId(10), TimeRange::open_ended(ready)).unwrap();
    let admission = AreaAdmission::new([(StationId(1), 1)]).unwrap();
    let mut member =
        admission.claim(StationId(1), OrderId(0), admission.quote(&StationId(1), ready).unwrap().unwrap()).unwrap();
    member.finalize(ready).unwrap();
    let mut scheduler = Scheduler::new();
    let completed = scheduler.add_task(ReplaceOrder {
        outgoing: Some(DelayedDeparture {
            position: Some(HtmPosition {
                cell_reservation: current,
                action_ready_at: ready,
                orientation: Orientation::East,
            }),
            navigation: model.navigation.clone(),
            bots: model.bot_service.clone(),
            inventory: model.inventory.clone(),
            events: model.events.clone(),
            admission: admission.clone(),
        }),
    });
    scheduler.run().unwrap();
    let runtime = Builder::new_current_thread().build().unwrap();
    let (assigned, admitted, dispatch) = runtime.block_on(completed);
    assert_eq!(assigned, ready, "the final pick immediately makes logical room for the next order");
    assert!(admitted > SimTime::seconds(20), "physical admission waits for the committed exit");
    assert!(admitted < SimTime::seconds(100), "downstream cleanup remains independent");
    assert_eq!(admission.occupancy(&StationId(1), admitted - SimDuration::ms(1)).unwrap(), 1);
    assert_eq!(admission.occupancy(&StationId(1), admitted).unwrap(), 0);
    assert_eq!(scheduler.barrier().floor(), SimTime::seconds(100));
    runtime.block_on(dispatch);
}
