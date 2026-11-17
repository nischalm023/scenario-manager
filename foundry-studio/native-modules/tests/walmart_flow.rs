//! Run the Walmart composition on both reviewed maps and audit physical work.
//! Completion counts alone cannot certify a pick made at the wrong location.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

use desgraph::CellId;
use desmap::ConveyorMachine;
use dessim::composition::Composition;
use dessim::events::{CycleKind, EventSink, ResourceEventKind, SimEvent};
use dessim::model::bot::BotService;
use dessim::model::conveyance::{Conveyances, PairedTransfer, TotePhase};
use dessim::model::inventory::InventoryService;
use dessim::model::{SimDuration, ToteDesignation, ToteLocation};
use dessim::observer::output::MetricsOutput;
use dessim::pathfinding::MotionModel;
use dessim::prelude::{Display, EmptyProposal, PlanError, ProposalId, SchedCtx, Task, async_trait};
use dessim::runner::{self, RunnerHooks, resolve_scenario_with_racking};
use dessim::scenario_gen::ScenarioData;
use dessim::task;
use dessim::task::TaskContext;
use dessim::{FidelityMode, SimConfig, StopReason, composition};
use native_modules as _;
use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::json;

const MINIMAL: &str = "minimal_easyloop_flow_spec.json";
const FULL: &str = "walmart_dfw6_smoke_spec.json";

fn prepared(spec: &str, fidelity: FidelityMode) -> (ScenarioData, SimConfig) {
    let (world, scenario, mut config) = common::load_world_spec(spec);
    config.fidelity = fidelity;
    config.motion_model = MotionModel::HtmKinematic;
    config.wall_timeout_secs = Some(60.0);
    let data = resolve_scenario_with_racking(world.map, world.racking, &scenario, &config).unwrap();
    (data, config)
}

#[test]
fn fixtures_supply_stock_order_totes_and_fleet_for_real_pairing() {
    for (spec, machines, orders, picks) in [(MINIMAL, 1, 8, 24), (FULL, 4, 5, 10)] {
        let (data, _) = prepared(spec, FidelityMode::Ghost);
        assert_eq!(data.orders.len(), orders);
        assert_eq!(data.orders.iter().map(|order| order.unit_count()).sum::<u32>(), picks);
        let mut stock = FxHashMap::default();
        for (&(sku, _), &quantity) in &data.inventory {
            *stock.entry(sku).or_insert(0) += quantity;
        }
        let mut demand = FxHashMap::default();
        for line in data.orders.iter().flat_map(|order| &order.lines) {
            *demand.entry(line.sku).or_insert(0) += line.quantity;
        }
        for (sku, quantity) in demand {
            assert!(stock.get(&sku).copied().unwrap_or(0) >= quantity, "fixture stock must cover {sku:?} demand");
        }
        assert_eq!(data.map.conveyor_machines.len(), machines);
        assert!(data.htm_initial_positions.len() >= 10, "eight retained HTMs leave robots for order totes");
        assert!(data.orders.iter().any(|order| order.unit_count() > 1), "repeated presentation has real demand");
        let order_totes = data.tote_placements.iter().filter(|tote| tote.designation == ToteDesignation::Order).count();
        assert!(order_totes >= data.orders.len(), "this bounded fixture does not rely on empty-tote recycling");
        assert!(!data.relay_data.is_empty());
        assert!(!data.vtm_initial_positions.is_empty());
        for machine in &data.map.conveyor_machines {
            assert_eq!(machine.inventory_holding_capacity(), 8);
            assert_eq!(machine.presentations.len(), 2);
            assert!(!machine.waiting_cells.is_empty());
            assert!(machine.dropoff_cell.is_some() && machine.collection_cell.is_some());
            assert!(
                machine
                    .presentations
                    .iter()
                    .all(|place| place.approach_cell.is_some() && place.departure_cell.is_some())
            );
        }
    }
}

fn run_and_audit(spec: &str, fidelity: FidelityMode, require_repeated_picks: bool) {
    let (mut data, mut config) = prepared(spec, fidelity);
    let expected_orders = data.orders.len() as u64;
    let expected_picks: u64 = data.orders.iter().map(|order| u64::from(order.unit_count())).sum();
    let mut definition = config.composition.resolve().unwrap();
    definition.tasks.push(
        serde_json::from_value(json!({
            "type": "AuditWalmartFlow", "after": ["FulfillOrders"]
        }))
        .unwrap(),
    );
    config.composition = Composition::Inline(definition);
    let pathfinder = common::fh_sipp_for_map(&data.map, &config);
    let (capture_tx, capture_rx) = mpsc::channel::<SimEvent>();
    let capture = std::thread::spawn(move || capture_rx.iter().collect::<Vec<_>>());
    let (event_tx, event_rx) = mpsc::channel();
    let sink: Arc<dyn EventSink> = Arc::new(event_tx.clone());
    let (scheduler, reports, bias) = composition::build(&mut data, &config, pathfinder, Some(sink)).unwrap();
    let audit = Arc::new(AtomicBool::new(false));
    scheduler
        .states()
        .configure(AuditRequest { completed: audit.clone(), expected_picks, require_repeated_picks })
        .unwrap();
    let (result, metrics) = runner::run(
        scheduler,
        reports,
        &data,
        config,
        event_tx,
        event_rx,
        bias,
        RunnerHooks { live: None, subscribers: vec![capture_tx], recording: None },
    )
    .expect("Walmart workload completes without planner errors");
    let events = capture.join().unwrap();
    assert!(
        matches!(result.stop_reason, StopReason::Complete),
        "workload did not finish naturally: {:?}",
        result.stop_reason
    );
    assert_eq!(result.total_picks as u64, expected_picks);
    assert_eq!(metrics.cumulative_scalars.orders_completed as u64, expected_orders);
    assert_eq!(metrics.cumulative_scalars.order_extract_presentations as u64, expected_orders);
    assert!(!events.iter().any(|event| matches!(event, SimEvent::WatchdogTimeout(_))));
    assert!(!events.iter().any(|event| matches!(event, SimEvent::Resource(resource) if resource.kind == ResourceEventKind::DeadlockHeadroomTriggered)));
    assert!(audit.load(Ordering::SeqCst), "physical audit completed before runner teardown");
    audit_playback(&data, &metrics, &events);
}

fn audit_playback(data: &ScenarioData, metrics: &MetricsOutput, events: &[SimEvent]) {
    for event in events {
        let SimEvent::CycleComplete(pick) = event else { continue };
        if !matches!(pick.kind, CycleKind::Pick { .. }) {
            continue;
        }
        let place = data
            .map
            .conveyor_machines
            .iter()
            .flat_map(|machine| &machine.presentations)
            .find(|place| place.station_id == pick.station_id)
            .unwrap();
        let expected = data.map.cell_world_pos(place.cell).unwrap();
        for at in [pick.sim_time_ms + 1, pick.sim_time_ms + pick.dwell_ms - 1] {
            let slice = metrics.time_slices.iter().rev().find(|slice| slice.start_ms <= at).unwrap();
            let location =
                slice.bot_locations.get(&pick.robot_id.0).expect("playback retains the order HTM during its pick");
            let mut position = location.start_pos;
            let mut cursor = slice.start_ms;
            for step in &location.steps {
                if cursor + step.delta_ms > at {
                    let fraction = (at - cursor) as f64 / step.delta_ms as f64;
                    position.x_mm += (step.pos.x_mm - position.x_mm) * fraction;
                    position.y_mm += (step.pos.y_mm - position.y_mm) * fraction;
                    break;
                }
                cursor += step.delta_ms;
                position = step.pos;
            }
            assert!(
                (position.x_mm - expected.x_mm).hypot(position.y_mm - expected.y_mm) < 1.0,
                "playback hides presentation: robot {:?}, station {:?}, time {at}, actual {position:?}, expected {expected:?}",
                pick.robot_id,
                pick.station_id
            );
        }
    }
}

struct AuditRequest {
    completed: Arc<AtomicBool>,
    expected_picks: u64,
    require_repeated_picks: bool,
}

/// Retaining a run registry outside runner::run also retains its event senders,
/// preventing observer shutdown. Audit after fulfillment, before teardown.
#[task]
#[derive(Display)]
#[display("AuditWalmartFlow")]
struct AuditWalmartFlow {
    machines: Arc<Conveyances>,
    inventory: Arc<InventoryService>,
    bots: Arc<BotService>,
    completed: Arc<AtomicBool>,
    expected_picks: u64,
    require_repeated_picks: bool,
}

#[async_trait(?Send)]
impl Task for AuditWalmartFlow {
    fn create(ctx: &TaskContext) -> Result<Self, PlanError> {
        let request = ctx.states().configuration::<AuditRequest>()?;
        Ok(Self {
            machines: ctx.state()?,
            inventory: ctx.state()?,
            bots: ctx.state()?,
            completed: request.completed.clone(),
            expected_picks: request.expected_picks,
            require_repeated_picks: request.require_repeated_picks,
        })
    }
    type Proposal = EmptyProposal;
    type Schedule = ();
    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }
    async fn schedule(&mut self, _: EmptyProposal, _: SchedCtx) -> Result<(), PlanError> {
        audit_transfers(&self.machines, &self.inventory, &self.bots, self.expected_picks, self.require_repeated_picks);
        self.completed.store(true, Ordering::SeqCst);
        Ok(())
    }
}

fn audit_transfers(
    machines: &Conveyances,
    inventory: &InventoryService,
    bots: &BotService,
    expected_picks: u64,
    require_repeated_picks: bool,
) {
    let mut transfers = 0;
    let mut repeated_picks = false;
    let mut concurrent_orders = false;
    let mut concurrent_inventory = false;
    for (layout, service) in machines.machines() {
        assert!(service.abandoned_custody().is_empty(), "lost custody on conveyor {:?}", service.id());
        assert_eq!(service.unfinished(), 0, "every boarded tote and retained HTM must leave");
        let mut pairs = service.transfers();
        assert!(!pairs.is_empty(), "the workload exercises every resolved machine");
        pairs.sort_by_key(|pair| (pair.start, pair.order_tote));
        transfers += pairs.len() as u64;
        assert!(
            pairs.windows(2).all(|pair| pair[0].end <= pair[1].start),
            "one operator cannot perform two picks at once"
        );
        let mut per_order: FxHashMap<_, Vec<PairedTransfer>> = FxHashMap::default();
        for pair in &pairs {
            audit_pair(pair, layout, &inventory, &bots);
            per_order.entry(pair.order_tote).or_default().push(*pair);
        }
        for order in per_order.values() {
            let first = &order[0];
            let presentation = layout.presentations.iter().find(|p| p.station_id == first.station).unwrap();
            repeated_picks |= order.len() > 1;
            assert!(
                order.iter().all(|pair| pair.station == first.station && pair.order_robot == first.order_robot),
                "an order stays on its assigned HTM and presentation side for every pick"
            );
            bots.for_each_bot(|robot, _, plan| {
                if robot == first.order_robot {
                    for waypoint in plan.iter().filter(|waypoint| {
                        waypoint.enter >= first.start
                            && waypoint.enter < order.last().unwrap().end
                            && layout.waiting_cells.contains(&waypoint.cell)
                    }) {
                        assert!(
                            presentation.waiting_cells.contains(&waypoint.cell),
                            "order HTM used the other presentation's waiting block"
                        );
                    }
                }
            });
        }
        let visits = service.visits();
        let mut moments: Vec<_> = pairs.iter().flat_map(|pair| [pair.start, pair.end]).collect();
        for visit in &visits {
            let boarded = visit.boarded_at.expect("no provisional admission remains");
            let TotePhase::Collected { at, .. } = visit.phase else { panic!("unfinished inventory tote") };
            moments.extend([boarded, at]);
            let prior = boarded.checked_sub(SimDuration::ms(1)).expect("pickup precedes conveyor boarding");
            let ToteLocation::WithRobot { robot_id } = inventory.tote(visit.tote).unwrap().location_at(prior) else {
                panic!("inventory tote {:?} did not board from an HTM", visit.tote);
            };
            let collected = visits
                .iter()
                .filter_map(|visit| match visit.phase {
                    TotePhase::Collected { at, robot } if robot == robot_id && at >= boarded => Some(at),
                    _ => None,
                })
                .min()
                .expect("delivering HTM remained available to collect a return");
            let lane = layout.inventory_queue.as_ref().unwrap();
            let allowed: FxHashSet<CellId> =
                lane.path.iter().copied().chain(layout.dropoff_cell).chain(layout.collection_cell).collect();
            let mut found = false;
            bots.for_each_bot(|robot, _, plan| {
                if robot != robot_id {
                    return;
                }
                found = true;
                let dropoff = plan.position_at(prior).expect("delivering HTM holds the dropoff while unloading");
                assert_eq!(Some(dropoff.cell), layout.dropoff_cell, "inventory HTM is at dropoff until boarding completes");
                let start = plan.position_at(boarded).expect("retained HTM remains positioned when boarding completes");
                let mut traversed: Vec<_> = Some(start)
                    .into_iter()
                    .chain(plan.iter().filter(|waypoint| waypoint.enter > boarded && waypoint.enter < collected))
                    .map(|waypoint| waypoint.cell)
                    .collect();
                let last = collected.checked_sub(SimDuration::ms(1)).expect("collection follows boarding");
                let collection = plan.position_at(last).expect("inventory HTM remains positioned through collection");
                assert_eq!(Some(collection.cell), layout.collection_cell);
                traversed.push(collection.cell);
                assert!(
                    traversed.iter().all(|cell| allowed.contains(cell)),
                    "inventory HTM {robot_id:?} exited conveyor {:?} before collection: tote {:?}, boarded {boarded:?}, collected {collected:?}, visited {traversed:?}",
                    service.id(),
                    visit.tote
                );
                traversed.dedup();
                let queue_only: Vec<_> = traversed.into_iter().filter(|cell| lane.path.contains(cell)).collect();
                assert_eq!(queue_only, lane.path, "retained HTM must traverse all eight queue cells in order");
            });
            assert!(found, "delivering inventory HTM exists in the physical plan");
        }
        moments.sort();
        moments.dedup();
        let order_cells: FxHashSet<_> =
            layout.waiting_cells.iter().copied().chain(layout.presentations.iter().map(|place| place.cell)).collect();
        let mut peak_orders = 0;
        let mut peak_inventory = 0;
        for at in moments {
            let occupancy = service.occupancy_at(at);
            peak_inventory = peak_inventory.max(occupancy.conveyor);
            let order_robots: FxHashSet<_> = per_order
                .keys()
                .filter_map(|&tote| {
                    let ToteLocation::WithRobot { robot_id } = inventory.tote(tote).unwrap().location_at(at) else {
                        return None;
                    };
                    let mut inside = false;
                    bots.for_each_bot(|robot, _, plan| {
                        if robot == robot_id {
                            inside = plan.position_at(at).is_some_and(|position| order_cells.contains(&position.cell));
                        }
                    });
                    inside.then_some(robot_id)
                })
                .collect();
            peak_orders = peak_orders.max(order_robots.len());
            assert!(occupancy.conveyor <= 8 && occupancy.retained_htms <= 8);
            assert!(occupancy.presentations <= 2 && occupancy.operator <= 1);
        }
        eprintln!(
            "[concurrency] machine {:?}: peak order HTMs={peak_orders}, conveyor totes={peak_inventory}",
            service.id()
        );
        concurrent_orders |= peak_orders > 1;
        concurrent_inventory |= peak_inventory > 1;
    }
    assert_eq!(transfers, expected_picks, "every pick has a committed paired-tote transfer");
    if require_repeated_picks {
        assert!(concurrent_orders, "the minimal workload must use multiple waiting berths concurrently");
        assert!(concurrent_inventory, "inventory return must overlap later conveyor visits");
        assert!(repeated_picks, "the minimal workload must exercise repeated picks into an order container");
    }
}

fn audit_pair(pair: &PairedTransfer, layout: &ConveyorMachine, inventory: &InventoryService, bots: &BotService) {
    let stock = inventory.tote(pair.inventory_tote).unwrap();
    let order = inventory.tote(pair.order_tote).unwrap();
    assert_eq!(stock.designation(), ToteDesignation::Inventory);
    assert_eq!(order.designation(), ToteDesignation::Order);
    let last = pair.end.checked_sub(SimDuration::ms(1)).expect("positive pick duration");
    let place = layout.presentations.iter().find(|place| place.station_id == pair.station).unwrap();
    for at in [pair.start, last] {
        assert_eq!(
            stock.location_at(at),
            ToteLocation::OnConveyance { conveyor_id: pair.conveyor },
            "inventory tote must be off its HTM on the correct machine during the pick"
        );
        assert_eq!(order.location_at(at), ToteLocation::WithRobot { robot_id: pair.order_robot });
        let mut found = false;
        bots.for_each_bot(|robot, _, plan| {
            if robot == pair.order_robot {
                assert_eq!(plan.position_at(at).map(|waypoint| waypoint.cell), Some(place.cell));
                let waypoints: Vec<_> = plan.iter().collect();
                let index = waypoints.partition_point(|waypoint| waypoint.enter <= at) - 1;
                let approach = waypoints[..index].iter().rev().find(|waypoint| waypoint.cell != place.cell);
                let departure = waypoints[index + 1..].iter().find(|waypoint| waypoint.cell != place.cell);
                assert_eq!(
                    approach.map(|waypoint| waypoint.cell),
                    place.approach_cell,
                    "order HTM enters on the declared curve"
                );
                assert_eq!(
                    departure.map(|waypoint| waypoint.cell),
                    place.departure_cell,
                    "order HTM completes the declared U-turn after picking"
                );
                found = true;
            }
        });
        assert!(found, "order HTM exists and holds the physical presentation cell");
    }
    let before = stock.contents_at(last);
    let removed: u32 = before
        .iter()
        .map(|(sku, quantity)| {
            quantity.checked_sub(stock.sku_qty_at(*sku, pair.end)).expect("stock cannot increase during a pick")
        })
        .sum();
    assert_eq!(removed, 1, "one-unit fixture transfers stock at pick end");
    for (sku, quantity) in before {
        let taken = quantity - stock.sku_qty_at(sku, pair.end);
        assert_eq!(
            order.sku_qty_at(sku, pair.end),
            order.sku_qty_at(sku, last) + taken,
            "the same SKU reaches the order tote at pick end"
        );
    }
}

#[test]
fn minimal_walmart_completes_with_paired_totes_in_ghost_mode() {
    run_and_audit(MINIMAL, FidelityMode::Ghost, true);
}

#[test]
fn minimal_walmart_completes_with_paired_totes_and_collisions() {
    run_and_audit(MINIMAL, FidelityMode::CollisionDes, true);
}

#[test]
#[cfg_attr(not(feature = "slow-gate"), ignore = "whole-fixture simulation; slow gate: scripts/test-slow-gate.sh")]
fn full_walmart_smoke_completes_with_paired_totes_in_ghost_mode() {
    run_and_audit(FULL, FidelityMode::Ghost, false);
}

#[test]
#[cfg_attr(not(feature = "slow-gate"), ignore = "whole-fixture simulation; slow gate: scripts/test-slow-gate.sh")]
fn full_walmart_smoke_completes_with_paired_totes_and_collisions() {
    run_and_audit(FULL, FidelityMode::CollisionDes, false);
}
