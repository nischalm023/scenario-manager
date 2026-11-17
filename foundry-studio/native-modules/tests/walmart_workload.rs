//! Explicit full-workload acceptance gate on the reviewed Walmart map. Kept
//! separate from the smoke suite because it schedules 500 complete orders.

mod common;
#[path = "common/queueing.rs"]
mod queueing;

use std::env;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use dessim::composition::{self, Composition};
use dessim::events::{EventSink, SimEvent};
use dessim::model::bot::BotService;
use dessim::model::conveyance::{Conveyances, TotePhase};
use dessim::model::inventory::InventoryService;
use dessim::model::navigation::Navigation;
use dessim::model::{ExitPath, OrderContainer, ToteDesignation, ToteLocation, ToteType};
use dessim::pathfinding::MotionModel;
use dessim::prelude::{Display, EmptyProposal, PlanError, ProposalId, SchedCtx, Task, async_trait};
use dessim::runner::{self, RunnerHooks, resolve_scenario_with_racking};
use dessim::task;
use dessim::task::{TaskContext, parse_config};
use dessim::{FidelityMode, StopReason};
use native_modules as _;
use rustc_hash::FxHashSet;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuditConfig {
    cartons: usize,
    measurement_label: String,
}

#[task]
#[derive(Display)]
#[display("AuditWalmartWorkload")]
struct AuditWalmartWorkload {
    machines: Arc<Conveyances>,
    inventory: Arc<InventoryService>,
    bots: Arc<BotService>,
    navigation: Arc<Navigation>,
    cartons: usize,
    measurement_label: String,
}

#[async_trait(?Send)]
impl Task for AuditWalmartWorkload {
    fn validate_config(config: &Value) -> Result<(), PlanError> {
        let _: AuditConfig = parse_config(config)?;
        Ok(())
    }

    fn create(ctx: &TaskContext) -> Result<Self, PlanError> {
        let AuditConfig { cartons, measurement_label } = ctx.config()?;
        Ok(Self {
            machines: ctx.state()?,
            inventory: ctx.state()?,
            bots: ctx.state()?,
            navigation: ctx.state()?,
            cartons,
            measurement_label,
        })
    }

    type Proposal = EmptyProposal;
    type Schedule = ();

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, _: SchedCtx) -> Result<(), PlanError> {
        let mut picks = 0;
        let mut concurrent = false;
        let mut cartons = FxHashSet::default();
        for (layout, machine) in self.machines.machines() {
            assert_eq!(machine.unfinished(), 0, "unfinished conveyor work");
            assert!(machine.abandoned_custody().is_empty(), "lost conveyor or HTM custody");
            queueing::assert_inventory_custody(&machine, &self.inventory);
            let mut transfers = machine.transfers();
            transfers.sort_by_key(|pair| pair.start);
            assert!(transfers.windows(2).all(|pair| pair[0].end <= pair[1].start), "operator overlap");
            picks += transfers.len();
            let members: FxHashSet<_> = layout.cells.iter().copied().collect();
            let order_ids: FxHashSet<_> = transfers.iter().map(|pair| pair.order_tote).collect();
            let orders: Vec<_> = order_ids.iter().map(|&id| self.inventory.tote(id).unwrap()).collect();
            // Include calendar transitions as well as physical work milestones.
            let mut times: Vec<_> = machine
                .occupancy_change_times()
                .into_iter()
                .chain(machine.visits().iter().flat_map(|visit| {
                    let collected = match visit.phase {
                        TotePhase::Collected { at, .. } => Some(at),
                        _ => None,
                    };
                    visit.boarded_at.into_iter().chain(collected)
                }))
                .chain(transfers.iter().flat_map(|pair| [pair.start, pair.end]))
                .collect();
            times.sort_unstable();
            times.dedup();
            let mut max_conveyor = 0;
            let mut observed_order_htms = 0;
            let mut witness = None;
            for at in times {
                let aboard = machine.occupancy_at(at).conveyor;
                max_conveyor = max_conveyor.max(aboard);
                let robots: FxHashSet<_> = orders
                    .iter()
                    .filter_map(|tote| {
                        let ToteLocation::WithRobot { robot_id } = tote.location_at(at) else { return None };
                        self.bots.position_at(robot_id, at).filter(|cell| members.contains(cell)).map(|_| robot_id)
                    })
                    .collect();
                observed_order_htms = observed_order_htms.max(robots.len());
                if aboard > 1 && robots.len() > 1 {
                    witness.get_or_insert(at);
                }
            }
            assert!(max_conveyor <= 8, "conveyor capacity exceeded");
            eprintln!(
                "[workload] machine={} max_conveyor_totes={max_conveyor} observed_order_htms={observed_order_htms} concurrent_at={witness:?}",
                layout.conveyor_group
            );
            concurrent |= witness.is_some();
            for pair in transfers {
                let inventory = self.inventory.tote(pair.inventory_tote).unwrap();
                let order = self.inventory.tote(pair.order_tote).unwrap();
                assert_eq!(order.designation(), ToteDesignation::Order);
                if order.tote_type() == ToteType::Carton {
                    cartons.insert(pair.order_tote);
                    assert_eq!(order.location(), ToteLocation::Extracted);
                    assert!(order.contents().is_empty());
                }
                assert!(inventory.location_is_through(
                    pair.start,
                    pair.end,
                    ToteLocation::OnConveyance { conveyor_id: machine.id() },
                ));
                let location = order.location_at(pair.start);
                assert!(matches!(location, ToteLocation::WithRobot { .. }));
                assert!(order.location_is_through(pair.start, pair.end, location), "order tote left its HTM");
            }
        }
        queueing::report(&self.measurement_label, &self.machines, &self.navigation);
        assert_eq!(picks, 2_000, "every requested unit was transferred");
        assert!(concurrent, "no EasyLoop simultaneously held multiple order HTMs and inventory totes");
        assert_eq!(cartons.len(), self.cartons, "every carton order ends with its own extracted box");
        Ok(())
    }
}

fn run_full_workload(fidelity: FidelityMode, mixed: bool) {
    let (world, scenario, mut config) = common::load_world_spec("walmart_dfw6_generated_spec.json");
    config.fidelity = fidelity;
    config.motion_model = MotionModel::HtmKinematic;
    config.sim_duration = Duration::from_secs(48 * 60 * 60);
    config.wall_timeout_secs = Some(180.0);
    let mut definition = config.composition.resolve().unwrap();
    if let Ok(value) = env::var("WALMART_ACTIVE_ORDER_LIMIT") {
        let limit: usize = value.parse().expect("WALMART_ACTIVE_ORDER_LIMIT is a positive integer");
        assert!(limit > 0);
        definition.tasks.iter_mut().find(|task| task.name() == "FulfillOrders").unwrap().config["max_active_orders_per_machine"] =
            json!(limit);
        eprintln!("[queueing] max_active_orders_per_machine={limit}");
    }
    if mixed {
        // Explicit synthetic population, not an inferred site tray count.
        definition.tasks.iter_mut().find(|task| task.name() == "FillInventory").unwrap().config["order_trays"] =
            json!(24);
    }
    definition
        .tasks
        .push(serde_json::from_value(json!({ "type": "AuditWalmartWorkload", "after": ["FulfillOrders"], "config": { "cartons": if mixed { 167 } else { 0 }, "measurement_label": format!("generated-{}-{fidelity:?}", if mixed { "mixed" } else { "tote" }) } })).unwrap());
    config.composition = Composition::Inline(definition);
    let mut data = resolve_scenario_with_racking(world.map, world.racking, &scenario, &config).unwrap();
    if mixed {
        for (index, order) in data.orders.iter_mut().enumerate() {
            order.container = [OrderContainer::Tote, OrderContainer::Carton, OrderContainer::Tray][index % 3];
            order.is_container_shared = order.container != OrderContainer::Carton;
            order.exit_path = [ExitPath::Buffer, ExitPath::Dock, ExitPath::Sorter][(index / 3) % 3];
        }
    }
    assert_eq!(data.orders.len(), 500);
    assert_eq!(data.orders.iter().map(|order| order.unit_count()).sum::<u32>(), 2_000);
    assert_eq!(data.inventory.values().sum::<u32>(), 50_000);
    let pathfinder = common::fh_sipp_for_map(&data.map, &config);
    let (event_tx, event_rx) = mpsc::channel::<SimEvent>();
    let sink: Arc<dyn EventSink> = Arc::new(event_tx.clone());
    let (scheduler, reports, bias) = composition::build(&mut data, &config, pathfinder, Some(sink)).unwrap();
    let (result, metrics) = runner::run(
        scheduler,
        reports,
        &data,
        config,
        event_tx,
        event_rx,
        bias,
        RunnerHooks { live: None, subscribers: Vec::new(), recording: None },
    )
    .expect("the full Walmart workload schedules without errors");
    assert!(matches!(result.stop_reason, StopReason::Complete), "{:?}", result.stop_reason);
    assert_eq!(result.total_picks, 2_000);
    assert_eq!(result.total_orders_completed, 500);
    assert_eq!(metrics.cumulative_scalars.order_extract_presentations as u64, 500);
    if mixed {
        let cartons: Vec<_> = metrics
            .time_slices
            .iter()
            .flat_map(|slice| &slice.tote_state.creations)
            .filter(|created| created.tote_type == ToteType::Carton)
            .collect();
        assert_eq!(cartons.len(), 167, "each carton order inducts one physical box");
    }
}

#[test]
#[ignore = "full 500-order workload; run explicitly with --ignored --nocapture"]
fn full_walmart_500_orders_complete_in_ghost_mode() {
    run_full_workload(FidelityMode::Ghost, false);
}

#[test]
#[ignore = "full 500-order workload with collisions; run explicitly with --ignored --nocapture"]
fn full_walmart_500_orders_complete_with_collisions() {
    run_full_workload(FidelityMode::CollisionDes, false);
}

#[test]
#[ignore = "full 500-order mixed-container workload with collisions; run explicitly"]
fn full_walmart_500_mixed_orders_complete_with_collisions() {
    run_full_workload(FidelityMode::CollisionDes, true);
}
