//! Publish orders and the unpreloaded share of automatic stock as inbound work.
//! A supplied bulk arrival file retains its own quantities and release times.

use super::induction_schedule::order_aligned;
use crate::composition::resources::{InventorySeed, Orders, inputs};
use crate::composition::{InductionSequence, InductionTaskConfig, policy};
use crate::events::EventJournal;
use crate::flows::ttp::demand::feed_putaway_stations::{ArrivalQueue, Arrivals, build_arrival_queues};
use crate::flows::ttp::demand::striping;
use crate::model::navigation::Navigation;
use crate::model::station::StationSet;
use crate::prelude::*;
use crate::scenario_gen::{DemandPool, SkuInfo};
use crate::task;
use crate::task::{TaskContext, invalid};
use desmap::WarehouseMap;
use rustc_hash::FxHashMap;

#[task]
#[derive(Display)]
#[display("LoadOrders")]
pub struct LoadOrders {
    orders: OrderService,
    arrivals: ArrivalQueue,
    initial_orders: Vec<OrderSpec>,
    skus: Vec<SkuInfo>,
    pool: Option<DemandPool>,
    stations: StationSet,
    map: Arc<WarehouseMap>,
    events: EventJournal,
    units_per_tote: u32,
    induct_stock: bool,
    sequence: InductionSequence,
    preloaded: FxHashMap<SkuId, u64>,
    release_all_at_start: bool,
    lead: Option<SimDuration>,
}

#[async_trait(?Send)]
impl Task for LoadOrders {
    fn create(ctx: &TaskContext) -> Result<Self, PlanError> {
        let input = inputs(ctx.states())?;
        let params: InductionTaskConfig = input.definition.parameters("PutawayInduction")?;
        let seed: Arc<InventorySeed> = ctx.states().configuration()?;
        let mut skus = input.skus.clone();
        let mut seeded = FxHashMap::default();
        for &(sku, _, quantity) in &seed.contents {
            *seeded.entry(sku).or_insert(0_u64) += u64::from(quantity);
        }
        if input.demand_pool.is_none() {
            for sku in &mut skus {
                let loaded = seeded.get(&sku.id).copied().unwrap_or(0);
                sku.inventory_units = u64::from(sku.inventory_units)
                    .checked_sub(loaded)
                    .ok_or_else(|| invalid(format!("preload exceeds stock for {:?}", sku.id)))?
                    as u32;
            }
        }
        Ok(Self {
            orders: ctx.state::<Orders>()?.service.clone(),
            arrivals: ctx.state::<Arrivals>()?.0.clone(),
            initial_orders: input.orders.clone(),
            skus,
            pool: input.demand_pool.clone(),
            stations: ctx.state::<StationSet>()?.as_ref().clone(),
            map: ctx.state::<Navigation>()?.map.clone(),
            events: ctx.state::<EventJournal>()?.as_ref().clone(),
            units_per_tote: params.units_per_tote.unwrap_or(1),
            induct_stock: input.definition.tasks.iter().any(|task| task.name() == "PutawayInduction"),
            sequence: params.sequence,
            preloaded: seeded,
            release_all_at_start: input.config.order.release_all_at_start,
            lead: params
                .lead_time_secs
                .map(|secs| policy::duration("PutawayInduction.lead_time_secs", secs))
                .transpose()?,
        })
    }

    type Proposal = EmptyProposal;
    type Schedule = ();
    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }
    async fn schedule(&mut self, _: EmptyProposal, _: SchedCtx) -> Result<(), PlanError> {
        self.orders.extend(self.initial_orders.iter().cloned());
        if self.induct_stock {
            let queue = if self.pool.is_none() && self.sequence == InductionSequence::OrderRelease {
                Mut::new(order_aligned(
                    &self.skus,
                    self.preloaded.clone(),
                    &self.initial_orders,
                    self.units_per_tote,
                    self.release_all_at_start,
                    self.lead,
                ))
            } else {
                build_arrival_queues(&self.skus, &self.orders, self.pool.as_ref(), self.units_per_tote)
            };
            queue.with(|items| {
                eprintln!(
                    "[inbound] {} totes, {} units not preloaded",
                    items.len(),
                    items.iter().map(|(_, qty, _)| u64::from(*qty)).sum::<u64>()
                )
            });
            self.arrivals.set(queue.value());
        }
        striping::emit(&self.stations, &self.map, &self.pool, self.events.sink().as_deref());
        Ok(())
    }
}
