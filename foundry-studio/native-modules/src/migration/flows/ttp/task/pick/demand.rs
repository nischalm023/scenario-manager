//! Root picking work: admit orders, select their next picks, and pass policy
//! and shared completion tracking to the resulting child tasks.

use super::{pick_item::*, station::*};
use crate::composition::PickTaskConfig;
use crate::composition::policy;
use crate::composition::resources::Orders;
use crate::composition::resources::inputs;
use crate::events::EventJournal;
use crate::model::ParkingRegistry;
use crate::model::bot::BotService;
use crate::model::factories::world_inputs;
use crate::model::htm::registry::AvailableHtmRegistry;
use crate::model::inventory::InventoryService;
use crate::model::navigation::Navigation;
use crate::model::relay::RelayService;
use crate::model::station_service::StationService;
use crate::task;
use crate::task::TaskContext;
use crate::task::parse_config;
use crate::{config::PickSequencing, flows::ttp::demand::*, prelude::*};
use serde_json::Value;
use tokio::select;

// Backpressure on the order builder: at most this many built-but-unstarted
// orders staged ahead of the stations.
const READY_QUEUE_DEPTH: usize = 2000;

#[derive(Display)]
#[display("PickDemand")]
#[task]
pub struct PickDemand {
    navigation: Arc<Navigation>,
    bots: BotService,
    inventory: InventoryService,
    available: AvailableHtmRegistry,
    parking: Option<ParkingRegistry>,
    events: EventJournal,
    station_service: Arc<StationService>,
    station_arrivals: StationArrivalDiagnostics,
    tripwires: bool,
    delivery_max_wait_override: Option<SimDuration>,
    relay: RelayService,
    orders: OrderReadyQueue,
    /// Every order in the run, however it was produced.
    registry: OrderService,
    builder: Option<OrderBuilder>,
    stations: Vec<Arc<StationState>>,
    completion: OrderCompletionTracker,
    close_out: bool,
    sequencing: PickSequencing,
    embargo_until: SimTime,
    inventory_generation_seen: u64,
    station_version: Version,
    boarding: BeltBoarding,
}

impl PickDemand {
    /// Rechecks blocked orders and builds new ones against the stock putaway
    /// has scheduled. `after` is the earliest simtime a built order may
    /// reserve at: never behind the settled frontier, where the ledger may
    /// show units free that another order books at a later simtime.
    fn absorb_new_inventory(&mut self, after: SimTime) {
        let generation = self.inventory.version().seen();
        if generation == self.inventory_generation_seen {
            return;
        }
        self.inventory_generation_seen = generation;
        let blocked_skus: Vec<SkuId> = self.orders.blocked_by_sku.keys().copied().collect();
        for sku in blocked_skus {
            self.orders.recheck_blocked_for_sku(sku, &self.inventory, SimTime::ZERO);
        }
        let Some(builder) = self.builder.as_mut() else {
            return;
        };
        while self.orders.ready.len() < READY_QUEUE_DEPTH {
            let Some(built) = builder.try_build_one(&self.inventory, after) else {
                break;
            };
            // Register before enqueueing: once it is dispatchable, close-out
            // and reporting must be able to look the order up.
            self.registry.insert(built.order.clone());
            self.orders.enqueue_reserved(&built.order, built.reservation_time);
        }
    }

    fn order_not_before(&self, order_id: OrderId) -> SimTime {
        self.orders
            .orders
            .get(&order_id)
            .and_then(|order| order.reserved_at)
            .map_or(self.embargo_until, |at| at.max(self.embargo_until))
    }

    fn stations_by_fewest_orders(&self) -> Vec<Arc<StationState>> {
        let mut stations = self.stations.clone();
        stations.sort_by_key(|station| {
            (self.orders.station_active.get(&station.id).map_or(0, |orders| orders.len()), station.id)
        });
        stations
    }

    fn start_ready_orders(&mut self) -> Result<(), PlanError> {
        'restart: loop {
            for station in self.stations_by_fewest_orders() {
                let Some(bin_floor) = station.bin_floor(self.embargo_until) else { continue };
                if self.start_one_order(&station, bin_floor)? {
                    continue 'restart;
                }
            }
            return Ok(());
        }
    }

    fn start_one_order(&mut self, station: &StationState, bin_floor: SimTime) -> Result<bool, PlanError> {
        let ready: Vec<OrderId> = self.orders.ready.iter().copied().collect();
        for order_id in ready {
            let Some(order) = self.orders.orders.get(&order_id) else {
                continue;
            };
            if order.lines.is_empty() || order.station_id.is_some_and(|id| id != station.id) {
                continue;
            }
            let needs_reservation = order.reserved_at.is_none();
            let demand = order.aggregated_demand();
            let total_picks: u32 = order.lines.iter().map(|line| line.qty).sum();
            let reservation_time = match order.reserved_at {
                Some(at) => at.max(self.embargo_until),
                // A line that stays available from its simtime on stays so at
                // any later one, so every line holds from the latest of them.
                None => demand
                    .iter()
                    .filter_map(|&(sku, qty)| self.inventory.earliest_sku_qty_lasting(sku, qty, SimTime::ZERO))
                    .max()
                    .unwrap_or(SimTime::ZERO)
                    .max(self.embargo_until),
            };
            if station.admission_after(reservation_time).is_none() {
                continue;
            }
            if needs_reservation && !self.inventory.reserve_for_order(&demand, reservation_time) {
                continue;
            }
            self.orders.ready.retain(|&id| id != order_id);
            self.orders.start_picking(order_id, station.id, reservation_time);
            let registration = if self.close_out {
                self.completion.register_for_close_out(order_id, station.id, total_picks)
            } else {
                self.completion.register(order_id, station.id, total_picks)
            };
            registration.map_err(|id| PlanError::TaskGraphInvariant {
                context: "PickDemand",
                detail: format!("order {id:?} registered twice"),
            })?;
            station.open_bin(order_id, bin_floor);
            return Ok(true);
        }
        Ok(false)
    }

    fn ready_lines(&self, order_id: OrderId) -> Vec<usize> {
        let Some(order) = self.orders.orders.get(&order_id) else {
            return Vec::new();
        };
        let undispatched = order.lines.iter().enumerate().filter(|(_, line)| !line.dispatched).map(|(idx, _)| idx);
        match self.sequencing {
            PickSequencing::AllTotes => undispatched.collect(),
            PickSequencing::Sequenced => undispatched.take(1).collect(),
        }
    }

    fn spawn_ready_picks(&mut self, ctx: &SchedCtx) -> Result<(), PlanError> {
        'stations: for station in self.stations.clone() {
            let active: Vec<OrderId> = self.orders.station_active.get(&station.id).cloned().unwrap_or_default();
            for order_id in active {
                let not_before = self.order_not_before(order_id);
                for line_idx in self.ready_lines(order_id) {
                    let Some(after) = station.admission_after(not_before) else {
                        continue 'stations;
                    };
                    self.spawn_pick(ctx, &station, order_id, line_idx, after)?;
                }
            }
        }
        Ok(())
    }

    fn spawn_pick(
        &mut self,
        ctx: &SchedCtx,
        station: &Arc<StationState>,
        order_id: OrderId,
        line_idx: usize,
        after: SimTime,
    ) -> Result<(), PlanError> {
        let invariant = |detail: String| PlanError::TaskGraphInvariant { context: "PickDemand", detail };
        let line = self
            .orders
            .dispatch_line(order_id, line_idx)
            .ok_or_else(|| invariant(format!("order {order_id:?} line {line_idx} is not dispatchable")))?;
        let bin_hold =
            station.bin(order_id).ok_or_else(|| invariant(format!("active order {order_id:?} has no bin")))?;
        let arrival_floor = if line.first { bin_hold.bin_free() } else { SimTime::ZERO };

        station.reserve_pick_slot();
        if line.last {
            station.take_bin(order_id);
        }
        ctx.add_task(PickItem {
            line: PickLine { order: order_id, sku: line.sku, qty: line.qty, dc: line.dc },
            station: station.clone(),
            relay: self.relay.clone(),
            navigation: self.navigation.clone(),
            bots: self.bots.clone(),
            inventory: self.inventory.clone(),
            available: self.available.clone(),
            parking: self.parking.clone(),
            events: self.events.clone(),
            station_service: self.station_service.clone(),
            station_arrivals: self.station_arrivals.clone(),
            tripwires: self.tripwires,
            delivery_max_wait_override: self.delivery_max_wait_override,
            after,
            arrival_floor,
            bin_hold,
            completion: self.completion.clone(),
            boarding: self.boarding.clone(),
        });
        Ok(())
    }

    fn finished(&self) -> bool {
        self.orders.ready.is_empty()
            && self.orders.blocked.is_empty()
            && self.orders.station_active.values().all(|orders| orders.is_empty())
            && self.builder.as_ref().map_or(true, |builder| builder.pool_is_empty())
            && self.stations.iter().all(|station| station.idle())
    }
}

#[async_trait(?Send)]
impl Task for PickDemand {
    fn create(ctx: &TaskContext) -> Result<Self, PlanError> {
        let params: PickTaskConfig = ctx.config()?;
        let input = inputs(ctx.states())?;
        let orders = ctx.state::<Orders>()?;
        let world = world_inputs(ctx.states())?;
        let inventory = ctx.state::<InventoryService>()?;
        let stations = ctx.state::<StationSet>()?;
        let bounds = ctx.state::<StationFetchBounds>()?;
        let parking = ctx.state::<ParkingRegistry>()?;
        let station_service = ctx.state::<StationService>()?;
        let queue = OrderReadyQueue::build(&orders.service, &inventory, &stations);
        let station_version = station_service.version().clone();
        let policy = policy::pick(&input.config, &params)?;
        policy::echo_admission_caps(&stations, &bounds, Some(&policy.admission), None, None);
        let stations = stations
            .pick_stations()
            .iter()
            .map(|station| {
                let f_max = bounds.f_max(station.id);
                Arc::new(StationState::new(station, policy, f_max, station_version.clone()))
            })
            .collect();
        let navigation = ctx.state::<Navigation>()?;
        Ok(Self {
            boarding: BeltBoarding::of(&navigation.map.stations),
            navigation,
            relay: ctx.state::<RelayService>()?.as_ref().clone(),
            bots: ctx.state::<BotService>()?.as_ref().clone(),
            inventory: inventory.as_ref().clone(),
            available: ctx.state::<AvailableHtmRegistry>()?.as_ref().clone(),
            parking: (parking.candidate_count() > 0).then(|| parking.as_ref().clone()),
            events: ctx.state::<EventJournal>()?.as_ref().clone(),
            station_service,
            station_arrivals: ctx.state::<StationArrivalDiagnostics>()?.as_ref().clone(),
            tripwires: world.config.tripwires,
            delivery_max_wait_override: world
                .config
                .pick
                .delivery_max_wait_secs
                .map(|secs| SimDuration::ms((secs * 1000.0) as u64)),
            orders: queue,
            registry: orders.service.clone(),
            builder: input.demand_pool.clone().map(OrderBuilder::new),
            stations,
            completion: ctx.state::<OrderCompletionTracker>()?.as_ref().clone(),
            close_out: close_out_enabled(&input, ctx.states())?,
            sequencing: params.sequencing.unwrap_or(input.config.pick.sequencing),
            embargo_until: policy::pick_start(&input.config, &params)?,
            inventory_generation_seen: 0,
            station_version,
        })
    }

    fn validate_config(config: &Value) -> Result<(), PlanError> {
        parse_config::<PickTaskConfig>(config).map(|_| ())
    }

    type Proposal = EmptyProposal;
    type Schedule = ();

    async fn propose(&self, _owner: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _proposal: EmptyProposal, ctx: SchedCtx) -> Result<(), PlanError> {
        // Picks may not begin scheduling until the embargo instant is in the
        // settled past. The per-order embargo floors below remain as local
        // honesty; once this gate opens the barrier floor subsumes them.
        ctx.barrier().wait_until(self.embargo_until).await?;
        loop {
            let inventory_seen = self.inventory.version().seen();
            let station_seen = self.station_version.seen();
            self.absorb_new_inventory(self.embargo_until.max(ctx.barrier().floor()));
            self.start_ready_orders()?;
            self.spawn_ready_picks(&ctx)?;
            if self.finished() {
                return Ok(());
            }
            select! {
                biased;
                _ = self.inventory.version().past(inventory_seen) => {}
                _ = self.station_version.past(station_seen) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::order::{OrderLine, OrderSpec};
    use crate::pathfinding::holding_cells;
    use crate::runner::resolve_scenario_with_racking;
    use crate::{FHSippPathfinder, ScenarioConfig, composition, world_spec};
    use serde_json::json;
    use std::path::Path;

    #[test]
    fn preformed_picks_keep_the_reservation_time_as_their_lower_bound() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal_pick_only_spec.json");
        let mut loaded = world_spec::load(&fixture).unwrap();
        if let ScenarioConfig::Generated(g) = &mut loaded.scenario {
            g.num_skus = 1;
        }
        let mut data =
            resolve_scenario_with_racking(loaded.world.map, loaded.world.racking, &loaded.scenario, &loaded.settings)
                .unwrap();
        let pathfinder =
            Arc::new(FHSippPathfinder::new(&data.map, 1000, SimDuration::seconds(1), holding_cells(&data.map)));
        let (scheduler, _, _) = composition::build(&mut data, &loaded.settings, pathfinder, None).unwrap();
        let states = scheduler.states();
        let context = TaskContext::new::<PickDemand>(states.clone(), &Default::default(), json!({})).unwrap();
        let mut pick = PickDemand::create(&context).unwrap();
        let inventory = InventoryService::new();
        let sku = SkuId(10);
        inventory.record_inventory_added(sku, SimTime::ZERO);
        assert!(inventory.reserve_for_order(&[(sku, 1)], SimTime::ms(2_000)));
        inventory.record_pick_consumed(sku, SimTime::ms(3_000));
        inventory.record_inventory_added(sku, SimTime::ms(4_000));
        let registry = OrderService::new();
        let order = OrderId(999);
        registry.insert(OrderSpec::unstated(order, vec![OrderLine { sku, quantity: 1 }]));
        pick.orders = OrderReadyQueue::build(&registry, &inventory, &states.get::<StationSet>().unwrap());
        pick.inventory = inventory.clone();
        pick.registry = registry;
        let station = pick.stations[0].clone();
        let times = [0, 2000, 3000, 4000].map(SimTime::ms);
        let reserved_before = times.map(|at| inventory.sku_reserved_at(sku, at));
        assert!(pick.start_one_order(&station, SimTime::ZERO).unwrap());
        let reservation_at = times
            .into_iter()
            .zip(reserved_before)
            .find(|&(at, count)| inventory.sku_reserved_at(sku, at) > count)
            .unwrap()
            .0;
        let pick_not_before = pick.order_not_before(order);
        assert_eq!(reservation_at, SimTime::ms(4_000));
        assert!(pick_not_before >= reservation_at, "picks can be dispatched before their reservation begins");
    }
}
