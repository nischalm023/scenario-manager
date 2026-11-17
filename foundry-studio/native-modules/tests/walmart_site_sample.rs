//! Full-map custody gates for finite source-order backlogs. All arrivals are
//! intentionally zero; these exercise sustained flow and container routing,
//! not the site's arrival schedule, cutoff policy, or measured tray population.

mod common;
#[path = "common/queueing.rs"]
mod queueing;

use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use dessim::composition::{self, Composition};
use dessim::events::{CycleKind, EventSink, InventoryMoveEvent, SimEvent, TotePayloadState, VtmOperationKind};
use dessim::model::conveyance::{Conveyances, PairedTransfer};
use dessim::model::navigation::Navigation;
use dessim::model::{
    BotService, ExitPath, InventoryService, OrderContainer, OrderId, RobotId, SimTime, SkuId, ToteDesignation, ToteId,
    ToteLocation, ToteType,
};
use dessim::observer::time_slice::ToteVisualState;
use dessim::pathfinding::MotionModel;
use dessim::prelude::{Display, EmptyProposal, PlanError, ProposalId, SchedCtx, Task, async_trait};
use dessim::runner::{self, RunnerHooks, resolve_scenario_with_racking};
use dessim::scenario_gen::RackUse;
use dessim::task;
use dessim::task::TaskContext;
use dessim::{FidelityMode, StopReason};
use native_modules as _;
use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::json;

#[derive(Default)]
struct PhysicalFacts {
    transfers: Vec<AuditedTransfer>,
    containers: FxHashMap<ToteId, (ToteType, ToteLocation)>,
    contents_at_moves: FxHashMap<(ToteId, SimTime), Vec<(SkuId, u32)>>,
}

struct AuditedTransfer {
    pair: PairedTransfer,
    sku: SkuId,
    qty: u32,
}

struct AuditOutput {
    facts: Arc<Mutex<Option<PhysicalFacts>>>,
    move_times: Arc<Mutex<Vec<(ToteId, SimTime)>>>,
    expected_units: u32,
    initial_stock: FxHashMap<SkuId, u32>,
    inventory_totes: Vec<ToteId>,
    measurement_label: String,
}

struct CaptureMoveTimes {
    sender: mpsc::Sender<SimEvent>,
    times: Arc<Mutex<Vec<(ToteId, SimTime)>>>,
    planned_picks: AtomicUsize,
}

impl EventSink for CaptureMoveTimes {
    fn send(&self, event: SimEvent) {
        if let SimEvent::InventoryMove(moved) = &event {
            self.times.lock().unwrap().push((moved.tote_id, moved.at));
        }
        if let SimEvent::CycleComplete(cycle) = &event
            && let CycleKind::Pick { qty, .. } = cycle.kind
        {
            let previous = self.planned_picks.fetch_add(qty as usize, Ordering::Relaxed);
            let count = previous + qty as usize;
            if count / 250 > previous / 250 {
                eprintln!(
                    "[source-progress] planned_picks={count} last_pick_end={:.3}s",
                    (cycle.sim_time_ms + cycle.dwell_ms) as f64 / 1000.0,
                );
            }
        }
        self.sender.send(event).unwrap();
    }
}

#[task]
#[derive(Display)]
#[display("AuditWalmartSiteSample")]
struct AuditWalmartSiteSample {
    conveyors: Arc<Conveyances>,
    inventory: Arc<InventoryService>,
    bots: Arc<BotService>,
    navigation: Arc<Navigation>,
    output: Arc<AuditOutput>,
}

#[async_trait(?Send)]
impl Task for AuditWalmartSiteSample {
    fn create(ctx: &TaskContext) -> Result<Self, PlanError> {
        Ok(Self {
            conveyors: ctx.state()?,
            inventory: ctx.state()?,
            bots: ctx.state()?,
            navigation: ctx.state()?,
            output: ctx.states().configuration()?,
        })
    }

    type Proposal = EmptyProposal;
    type Schedule = ();

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, _: SchedCtx) -> Result<(), PlanError> {
        let mut facts = PhysicalFacts::default();
        for (layout, conveyor) in self.conveyors.machines() {
            assert_eq!(conveyor.unfinished(), 0);
            assert!(conveyor.abandoned_custody().is_empty());
            queueing::assert_inventory_custody(&conveyor, &self.inventory);
            let mut pairs = conveyor.transfers();
            pairs.sort_by_key(|pair| pair.start);
            assert!(pairs.windows(2).all(|pair| pair[0].end <= pair[1].start), "operator overlap");
            for pair in &pairs {
                let stock = self.inventory.tote(pair.inventory_tote).unwrap();
                let order = self.inventory.tote(pair.order_tote).unwrap();
                let presentation = layout.presentations.iter().find(|place| place.station_id == pair.station).unwrap();
                assert_eq!(order.designation(), ToteDesignation::Order);
                assert!(stock.location_is_through(
                    pair.start,
                    pair.end,
                    ToteLocation::OnConveyance { conveyor_id: conveyor.id() }
                ));
                assert!(order.location_is_through(
                    pair.start,
                    pair.end,
                    ToteLocation::WithRobot { robot_id: pair.order_robot }
                ));
                assert_eq!(self.bots.position_at(pair.order_robot, pair.start), Some(presentation.cell));
                facts.containers.insert(pair.order_tote, (order.tote_type(), order.location()));
                let (sku, qty) = transferred_contents(
                    &stock.contents_at(pair.start),
                    &stock.contents_at(pair.end),
                    &order.contents_at(pair.start),
                    &order.contents_at(pair.end),
                );
                facts.transfers.push(AuditedTransfer { pair: *pair, sku, qty });
            }
        }
        assert_eq!(facts.transfers.iter().map(|transfer| transfer.qty).sum::<u32>(), self.output.expected_units);
        let mut expected_remaining = self.output.initial_stock.clone();
        for transfer in &facts.transfers {
            let remaining = expected_remaining.get_mut(&transfer.sku).expect("physical transfer uses seeded SKU");
            *remaining = remaining.checked_sub(transfer.qty).expect("physical transfers cannot exceed stock");
        }
        let mut actual_remaining = FxHashMap::default();
        for &id in &self.output.inventory_totes {
            let tote = self.inventory.tote(id)?;
            assert_eq!(tote.designation(), ToteDesignation::Inventory);
            for (sku, qty) in tote.contents() {
                *actual_remaining.entry(sku).or_insert(0_u32) += qty;
            }
        }
        let last_pick = facts.transfers.iter().map(|transfer| transfer.pair.end).max().unwrap();
        for (&sku, &qty) in &expected_remaining {
            assert_eq!(self.inventory.sku_inventory_at(sku, last_pick), qty as isize, "stock ledger for {sku:?}");
            assert_eq!(self.inventory.sku_reserved_at(sku, last_pick), 0, "completed orders release SKU claims");
        }
        expected_remaining.retain(|_, qty| *qty > 0);
        actual_remaining.retain(|_, qty| *qty > 0);
        assert_eq!(actual_remaining, expected_remaining, "initial stock minus physical transfers equals final stock");
        queueing::report(&self.output.measurement_label, &self.conveyors, &self.navigation);
        // InventoryMove.contents values stock-chart flux, excluding order
        // goods. Audit physical contents from their authoritative timelines.
        for &(tote, at) in self.output.move_times.lock().unwrap().iter() {
            if facts.containers.contains_key(&tote) {
                facts.contents_at_moves.insert((tote, at), self.inventory.tote(tote)?.contents_at(at));
            }
        }
        *self.output.facts.lock().unwrap() = Some(facts);
        Ok(())
    }
}

fn transferred_contents(
    stock_before: &[(SkuId, u32)],
    stock_after: &[(SkuId, u32)],
    order_before: &[(SkuId, u32)],
    order_after: &[(SkuId, u32)],
) -> (SkuId, u32) {
    let delta = |before: &[(SkuId, u32)], after: &[(SkuId, u32)]| {
        let mut changes = FxHashMap::default();
        for &(sku, qty) in before {
            *changes.entry(sku).or_insert(0_i64) -= i64::from(qty);
        }
        for &(sku, qty) in after {
            *changes.entry(sku).or_insert(0_i64) += i64::from(qty);
        }
        changes.retain(|_, qty| *qty != 0);
        changes
    };
    let removed = delta(stock_after, stock_before);
    let added = delta(order_before, order_after);
    assert_eq!(removed.len(), 1, "one physical transfer removes exactly one SKU from inventory");
    assert_eq!(removed, added, "the order receives exactly the goods removed from inventory");
    let (&sku, &qty) = removed.iter().next().unwrap();
    assert!(qty > 0, "a physical pick transfers positive quantity into the order");
    (sku, u32::try_from(qty).unwrap())
}

#[derive(Default)]
struct OrderWork {
    tote: Option<ToteId>,
    first_pick: Option<SimTime>,
    last_pick_end: SimTime,
    units: u32,
    transfers: usize,
    sku_units: FxHashMap<SkuId, u32>,
}

#[test]
fn physical_transfer_audit_counts_multiple_units_with_existing_order_contents() {
    let sku = SkuId(1);
    let other = SkuId(2);
    assert_eq!(
        transferred_contents(&[(sku, 100)], &[(sku, 97)], &[(other, 2), (sku, 4)], &[(other, 2), (sku, 7)]),
        (sku, 3)
    );
}

#[test]
#[should_panic(expected = "the order receives exactly the goods removed from inventory")]
fn physical_transfer_audit_rejects_equal_quantity_of_the_wrong_sku() {
    let sku = SkuId(1);
    let other = SkuId(2);
    transferred_contents(&[(sku, 100)], &[(sku, 97)], &[], &[(other, 3)]);
}

#[test]
#[ignore = "full 500-order source sample with collisions; run explicitly with --ignored --nocapture"]
fn source_sample_completes_with_real_container_and_buffer_custody() {
    run_source_backlog("walmart_dfw6_mixed_spec.json", "source-sample", BacklogCheck::ReferenceSample);
}

#[test]
#[ignore = "4,000+ finite source orders; set WALMART_LONG_SPEC using scripts/prepare_walmart_throughput.py"]
fn finite_source_backlog_completes_with_real_container_and_buffer_custody() {
    let spec = env::var("WALMART_LONG_SPEC").expect("set WALMART_LONG_SPEC to the generated absolute spec path");
    assert!(PathBuf::from(&spec).is_absolute());
    run_source_backlog(&spec, "source-finite", BacklogCheck::Finite);
}

#[test]
#[ignore = "4,000+ source orders; set WALMART_LONG_SPEC using scripts/prepare_walmart_throughput.py"]
fn sustained_source_backlog_completes_with_real_container_and_buffer_custody() {
    let spec = env::var("WALMART_LONG_SPEC").expect("set WALMART_LONG_SPEC to the generated absolute spec path");
    assert!(PathBuf::from(&spec).is_absolute());
    run_source_backlog(&spec, "source-long", BacklogCheck::Sustained);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BacklogCheck {
    ReferenceSample,
    Finite,
    Sustained,
}

fn run_source_backlog(spec: &str, label: &str, check: BacklogCheck) {
    let (world, scenario, mut config) = common::load_world_spec(spec);
    assert_eq!(config.fidelity, FidelityMode::CollisionDes);
    assert_eq!(config.motion_model, MotionModel::HtmKinematic);
    assert_eq!(config.order.buffer_dwell_secs, 0.0);
    config.sim_duration = Duration::from_secs(48 * 60 * 60);
    // The bay-aligned full map needs about thirty wall minutes for 4,000
    // orders on the development host. Full-stock preload adds 313,220 SKU/tote
    // records even to the 500-order audit; allow ten minutes for setup, capture
    // and final returns. This guard does not change simulated time limits.
    config.wall_timeout_secs = Some(if check == BacklogCheck::ReferenceSample { 600.0 } else { 3600.0 });
    let mut definition = config.composition.resolve().unwrap();
    if let Ok(value) = env::var("WALMART_ACTIVE_ORDER_LIMIT") {
        let limit: usize = value.parse().expect("WALMART_ACTIVE_ORDER_LIMIT is a positive integer");
        assert!(limit > 0);
        definition.tasks.iter_mut().find(|task| task.name() == "FulfillOrders").unwrap().config["max_active_orders_per_machine"] =
            json!(limit);
        eprintln!("[queueing] max_active_orders_per_machine={limit}");
    }
    assert_eq!(
        definition.tasks.iter().find(|task| task.name() == "FillInventory").unwrap().config["order_trays"],
        json!(24)
    );
    definition.tasks.push(
        serde_json::from_value(json!({
            "type": "AuditWalmartSiteSample", "after": ["FulfillOrders"]
        }))
        .unwrap(),
    );
    config.composition = Composition::Inline(definition);
    let mut data = resolve_scenario_with_racking(world.map, world.racking, &scenario, &config).unwrap();
    let expected_orders = data.orders.len();
    let expected_units = data.orders.iter().map(|order| order.unit_count()).sum::<u32>();
    let expected_boxes = data.orders.iter().filter(|order| order.container == OrderContainer::Carton).count();
    let expected_buffered = data.orders.iter().filter(|order| order.exit_path == ExitPath::Buffer).count();
    assert!(data.orders.iter().all(|order| order.arrival == Some(SimTime::ZERO)));
    if check != BacklogCheck::ReferenceSample {
        assert!(expected_orders >= 4000, "the backlog gate requires a larger source cohort");
    } else {
        assert_eq!(expected_orders, 500);
        assert_eq!(data.orders.iter().map(|order| order.lines.len()).sum::<usize>(), 562);
        assert_eq!(expected_units, 576);
        for (container, expected) in
            [(OrderContainer::Tote, 224), (OrderContainer::Carton, 246), (OrderContainer::Tray, 30)]
        {
            assert_eq!(data.orders.iter().filter(|order| order.container == container).count(), expected);
        }
        for (exit, expected) in [(ExitPath::Buffer, 316), (ExitPath::Sorter, 91), (ExitPath::Dock, 93)] {
            assert_eq!(data.orders.iter().filter(|order| order.exit_path == exit).count(), expected);
        }
    }
    let mut initial_stock = FxHashMap::default();
    for (&(sku, _), &qty) in &data.inventory {
        *initial_stock.entry(sku).or_insert(0_u32) += qty;
    }
    let mut demand = FxHashMap::default();
    for line in data.orders.iter().flat_map(|order| &order.lines) {
        *demand.entry(line.sku).or_insert(0_u32) += line.quantity;
    }
    for (sku, qty) in demand {
        assert!(initial_stock.get(&sku).copied().unwrap_or(0) >= qty, "seeded stock covers demand for {sku:?}");
    }
    let inventory_totes = data
        .tote_placements
        .iter()
        .filter(|placement| placement.designation == ToteDesignation::Inventory)
        .map(|placement| placement.tote_id)
        .collect();

    let pathfinder = common::fh_sipp_for_map(&data.map, &config);
    let (capture_tx, capture_rx) = mpsc::channel::<SimEvent>();
    let capture = std::thread::spawn(move || capture_rx.iter().collect::<Vec<_>>());
    let (event_tx, event_rx) = mpsc::channel::<SimEvent>();
    let physical = Arc::new(Mutex::new(None));
    let move_times = Arc::new(Mutex::new(Vec::new()));
    let sink: Arc<dyn EventSink> = Arc::new(CaptureMoveTimes {
        sender: event_tx.clone(),
        times: move_times.clone(),
        planned_picks: AtomicUsize::new(0),
    });
    let (scheduler, reports, bias) = composition::build(&mut data, &config, pathfinder, Some(sink)).unwrap();
    scheduler
        .states()
        .configure(AuditOutput {
            facts: physical.clone(),
            move_times,
            expected_units,
            initial_stock,
            inventory_totes,
            measurement_label: label.to_owned(),
        })
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
    .expect("the source sample completes without planner errors");
    let events = capture.join().unwrap();
    assert!(matches!(result.stop_reason, StopReason::Complete), "{:?}", result.stop_reason);
    assert_eq!(result.total_picks as u32, expected_units);
    assert_eq!(result.total_orders_completed as usize, expected_orders);
    assert_eq!(metrics.cumulative_scalars.order_extract_presentations as usize, expected_orders);
    let facts = physical.lock().unwrap().take().expect("physical audit ran before manager teardown");

    let mut work = FxHashMap::<OrderId, OrderWork>::default();
    let mut paired = FxHashSet::default();
    let pairs_by_identity: FxHashMap<_, _> = facts
        .transfers
        .iter()
        .enumerate()
        .map(|(index, transfer)| {
            let pair = &transfer.pair;
            ((pair.station, pair.order_robot, pair.inventory_tote, pair.start, pair.end), index)
        })
        .collect();
    assert_eq!(pairs_by_identity.len(), facts.transfers.len(), "physical pair identities are unique");
    for event in &events {
        let SimEvent::CycleComplete(cycle) = event else { continue };
        let CycleKind::Pick { sku, qty } = cycle.kind else { continue };
        let source = cycle.tote_presentation.expect("pick has its order and inventory tote identities");
        let start = SimTime::ms(cycle.sim_time_ms);
        let end = SimTime::ms(cycle.sim_time_ms + cycle.dwell_ms);
        let &index = pairs_by_identity
            .get(&(cycle.station_id, cycle.robot_id, source.tote_id, start, end))
            .expect("each reported pick corresponds to an actual simultaneous presentation");
        let transfer = &facts.transfers[index];
        let pair = &transfer.pair;
        assert_eq!((sku, qty), (transfer.sku, transfer.qty), "pick event matches physical goods transferred");
        assert!(paired.insert(index), "a physical paired pick was reported twice");
        let order = work.entry(source.order_id).or_default();
        assert!(
            order.tote.is_none_or(|id| id == pair.order_tote),
            "an order changed physical containers during picking"
        );
        order.tote = Some(pair.order_tote);
        order.first_pick = Some(order.first_pick.map_or(start, |prior| prior.min(start)));
        order.last_pick_end = order.last_pick_end.max(end);
        order.units += qty;
        order.transfers += 1;
        *order.sku_units.entry(sku).or_insert(0) += qty;
    }
    assert_eq!(paired.len(), facts.transfers.len(), "every physical transfer has one reported pick event");
    assert_eq!(work.len(), expected_orders);
    let trace_directory = env::var_os("WALMART_QUEUE_TRACE_DIR").map(PathBuf::from);
    let mut order_rows = Vec::new();

    let mut moves = FxHashMap::<ToteId, Vec<&InventoryMoveEvent>>::default();
    for event in &events {
        if let SimEvent::InventoryMove(moved) = event {
            moves.entry(moved.tote_id).or_default().push(moved);
        }
    }
    for history in moves.values_mut() {
        history.sort_by_key(|moved| moved.at);
    }
    let location_at = |tote, at| {
        let history = &moves[&tote];
        history
            .partition_point(|moved| moved.at <= at)
            .checked_sub(1)
            .map_or(ToteLocation::Uninitialized, |index| history[index].to)
    };
    let mut robot_custody = FxHashMap::<RobotId, Vec<(ToteId, SimTime, Option<SimTime>)>>::default();
    for &tote in facts.containers.keys() {
        let history = &moves[&tote];
        for (index, moved) in history.iter().enumerate() {
            if let ToteLocation::WithRobot { robot_id } = moved.to {
                robot_custody.entry(robot_id).or_default().push((
                    tote,
                    moved.at,
                    history.get(index + 1).map(|next| next.at),
                ));
            }
        }
    }
    let mut storage_pickups = FxHashMap::default();
    for event in &events {
        if let SimEvent::VtmOperation(operation) = event {
            if operation.kind == VtmOperationKind::PickupStorage {
                let key = (operation.tote_id, operation.rack_id, operation.sim_time_ms, operation.vtm_id);
                assert!(storage_pickups.insert(key, operation).is_none(), "duplicate physical VTM pickup");
            }
        }
    }
    let mut extraction_by_tote = FxHashMap::<ToteId, Vec<(SimTime, SimTime)>>::default();
    let mut extraction_location = FxHashMap::default();
    let mut extraction_units = FxHashMap::default();
    for event in &events {
        let SimEvent::CycleComplete(cycle) = event else { continue };
        let CycleKind::OrderExtract { qty } = cycle.kind else { continue };
        let start = SimTime::ms(cycle.sim_time_ms);
        let matching: Vec<_> = robot_custody[&cycle.robot_id]
            .iter()
            .filter(|&&(_, held_from, held_until)| held_from <= start && held_until.is_none_or(|end| start < end))
            .map(|&(tote, _, _)| tote)
            .collect();
        assert_eq!(matching.len(), 1, "extraction must identify one physical order container");
        extraction_by_tote
            .entry(matching[0])
            .or_default()
            .push((start, SimTime::ms(cycle.sim_time_ms + cycle.dwell_ms)));
        extraction_location.insert((matching[0], start), (cycle.station_id, cycle.robot_id));
        assert!(
            extraction_units.insert((matching[0], start), qty).is_none(),
            "one extraction event per container visit"
        );
    }
    for extractions in extraction_by_tote.values_mut() {
        extractions.sort_unstable();
    }
    assert_eq!(extraction_by_tote.values().map(Vec::len).sum::<usize>(), expected_orders);
    let mut assigned_extractions = FxHashSet::default();
    let mut buffered_orders = FxHashSet::default();
    for spec in &data.orders {
        let order = &work[&spec.id];
        assert_eq!(order.units, spec.unit_count());
        let mut expected = FxHashMap::default();
        for line in &spec.lines {
            *expected.entry(line.sku).or_insert(0_u32) += line.quantity;
        }
        assert_eq!(order.sku_units, expected, "each order receives exactly its requested SKU quantities");
        let tote = order.tote.unwrap();
        let expected_type = spec.container;
        assert_eq!(facts.containers[&tote].0, expected_type);
        let &(extracted_start, extracted_end) = extraction_by_tote[&tote]
            .iter()
            .find(|&&(start, _)| start >= order.last_pick_end)
            .expect("every order reaches extraction after its last pick");
        assert!(assigned_extractions.insert((tote, extracted_start)), "two orders claimed the same extraction");
        assert_eq!(extraction_units[&(tote, extracted_start)], spec.unit_count());
        let (extraction_station, extraction_robot) = extraction_location[&(tote, extracted_start)];
        let mut row = trace_directory.as_ref().map(|_| {
            json!({
                "order_id": spec.id.0, "order_tote": tote.0,
                "container_type": spec.container, "physical_tote_type": expected_type,
                "first_pick_ms": order.first_pick.unwrap().as_ms(),
                "picked_units": order.units, "physical_transfers": order.transfers,
                "last_pick_end_ms": order.last_pick_end.as_ms(), "exit_path": spec.exit_path,
                "extraction_start_ms": extracted_start.as_ms(), "extraction_end_ms": extracted_end.as_ms(),
                "extraction_station_id": extraction_station.0, "extraction_robot_id": extraction_robot.0,
                "buffer_stored_ms": null, "buffer_storage_pickup_ms": null,
                "buffer_relay_deposit_ms": null, "buffer_htm_pickup_ms": null,
                "buffer_zone_id": null, "buffer_rack_id": null, "buffer_z_index": null,
                "buffer_vtm_id": null, "buffer_relay_id": null,
            })
        });
        let history = &moves[&tote];
        let buffer_entries: Vec<_> = history.iter().filter(|moved| {
            moved.at >= order.last_pick_end && moved.at < extracted_start
                && matches!(moved.to, ToteLocation::InRack { rack_id, .. } if data.rack_use.get(&rack_id) == Some(&RackUse::OrderBuffer))
        }).collect();
        if spec.exit_path == ExitPath::Buffer {
            assert_eq!(buffer_entries.len(), 1, "each DS order enters one actual buffer rack");
            let stored = buffer_entries[0];
            let ToteLocation::InRack { rack_id, z_index } = stored.to else { unreachable!() };
            let reserved = ToteLocation::Reserved { rack_id, z_index };
            assert_eq!(
                facts.contents_at_moves[&(tote, stored.at)].iter().copied().collect::<FxHashMap<_, _>>(),
                expected,
                "buffer receives the filled order"
            );
            let returned = history
                .iter()
                .find(|moved| moved.at >= stored.at && moved.at < extracted_start
                    && (moved.from == stored.to || moved.from == reserved)
                    && matches!(moved.to, ToteLocation::WithRobot { .. }))
                .unwrap_or_else(|| panic!(
                    "order {:?}, tote {tote:?}: no physical buffer pickup between {} and {extracted_start}; moves={history:?}",
                    spec.id, stored.at
                ));
            assert_eq!(
                facts.contents_at_moves[&(tote, returned.at)].iter().copied().collect::<FxHashMap<_, _>>(),
                expected,
                "retrieval retains the filled order"
            );
            let ToteLocation::WithRobot { robot_id } = returned.to else { unreachable!() };
            let pickup = storage_pickups.get(&(tote, rack_id, returned.at.as_ms(), robot_id)).unwrap_or_else(|| {
                panic!("order {:?}, tote {tote:?}: no matching VTM pickup at {}", spec.id, returned.at)
            });
            assert_eq!(pickup.payload, TotePayloadState::NonEmpty);
            let relay = ToteLocation::AtRelay { relay_id: pickup.relay_id };
            let relayed = history
                .iter()
                .find(|moved| {
                    moved.at >= returned.at
                        && moved.at < extracted_start
                        && moved.from == returned.to
                        && moved.to == relay
                })
                .unwrap_or_else(|| panic!("order {:?}, tote {tote:?}: VTM never deposits at {relay:?}", spec.id));
            let extraction_htm = location_at(tote, extracted_start);
            let collected = history
                .iter()
                .find(|moved| {
                    moved.at >= relayed.at
                        && moved.at < extracted_start
                        && moved.from == relay
                        && moved.to == extraction_htm
                })
                .unwrap_or_else(|| {
                    panic!("order {:?}, tote {tote:?}: extraction HTM never collects from {relay:?}", spec.id)
                });
            for at in [relayed.at, collected.at] {
                assert_eq!(
                    facts.contents_at_moves[&(tote, at)].iter().copied().collect::<FxHashMap<_, _>>(),
                    expected,
                    "order {:?}, tote {tote:?}: filled custody through relay at {at}",
                    spec.id
                );
            }
            if let Some(row) = row.as_mut() {
                row["buffer_stored_ms"] = json!(stored.at.as_ms());
                row["buffer_storage_pickup_ms"] = json!(returned.at.as_ms());
                row["buffer_relay_deposit_ms"] = json!(relayed.at.as_ms());
                row["buffer_htm_pickup_ms"] = json!(collected.at.as_ms());
                row["buffer_zone_id"] = json!(pickup.zone_id.0);
                row["buffer_rack_id"] = json!(rack_id.0);
                row["buffer_z_index"] = json!(z_index);
                row["buffer_vtm_id"] = json!(pickup.vtm_id.0);
                row["buffer_relay_id"] = json!(pickup.relay_id.0);
            }
            buffered_orders.insert(spec.id);
        } else {
            assert!(buffer_entries.is_empty(), "SC and 3P use direct extraction in this throughput sample");
        }
        if expected_type == ToteType::Carton {
            assert_eq!(location_at(tote, extracted_end), ToteLocation::Extracted);
            assert_eq!(facts.containers[&tote].1, ToteLocation::Extracted);
        } else {
            assert!(
                matches!(facts.containers[&tote].1, ToteLocation::InRack { rack_id, .. }
                if data.rack_use.get(&rack_id) == Some(&RackUse::OrderDepot)),
                "reusable containers return to their depot"
            );
        }
        if let Some(row) = row {
            order_rows.push(row);
        }
    }
    assert_eq!(buffered_orders.len(), expected_buffered);
    assert_eq!(assigned_extractions.len(), expected_orders);
    let cartons: FxHashSet<_> = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::ToteCreated(created) if created.tote_type == ToteType::Carton => {
                assert!(created.time > SimTime::ZERO, "boxes are inducted after travel, not seeded at startup");
                assert_eq!(created.designation, ToteDesignation::Order);
                assert!(matches!(created.initial_state, ToteVisualState::AtCell { .. }));
                Some(created.tote_id)
            }
            _ => None,
        })
        .collect();
    let removed: FxHashSet<_> = events
        .iter()
        .filter_map(|event| match event {
            SimEvent::ToteMoved(moved) if moved.new_state == ToteVisualState::Extracted => Some(moved.tote_id),
            _ => None,
        })
        .collect();
    assert_eq!(cartons.len(), expected_boxes);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, SimEvent::ToteCreated(created)
        if created.tote_type == ToteType::Carton))
            .count(),
        expected_boxes,
        "each box has exactly one birth event"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, SimEvent::ToteCreated(created)
        if created.tote_type == ToteType::Tray && created.time == SimTime::ZERO))
            .count(),
        24,
        "the configured reusable tray population is actually seeded"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, SimEvent::ToteMoved(moved)
        if moved.new_state == ToteVisualState::Extracted))
            .count(),
        expected_boxes,
        "each box has exactly one removal event"
    );
    assert_eq!(removed, cartons, "every newly inducted box physically leaves at extraction");
    let last_pick_ms = facts.transfers.iter().map(|transfer| transfer.pair.end.as_ms()).max().unwrap();
    let mut hourly = Vec::new();
    for start_ms in (0..last_pick_ms).step_by(3_600_000) {
        let end_ms = (start_ms + 3_600_000).min(last_pick_ms);
        let (completed_units, completed_transfers) = facts
            .transfers
            .iter()
            .filter(|transfer| transfer.pair.end.as_ms() > start_ms && transfer.pair.end.as_ms() <= end_ms)
            .fold((0_u32, 0_usize), |(units, count), transfer| (units + transfer.qty, count + 1));
        let (remaining_units, remaining_transfers) = facts
            .transfers
            .iter()
            .filter(|transfer| transfer.pair.end.as_ms() > end_ms)
            .fold((0_u32, 0_usize), |(units, count), transfer| (units + transfer.qty, count + 1));
        let rate_factor = 3_600_000.0 / (end_ms - start_ms) as f64;
        let units_per_hour = completed_units as f64 * rate_factor;
        let transfers_per_hour = completed_transfers as f64 * rate_factor;
        eprintln!(
            "[{label}] pick window {}..{} min: completed_units={completed_units} units_per_hour={units_per_hour:.1} remaining_units={remaining_units} completed_transfers={completed_transfers} transfers_per_hour={transfers_per_hour:.1} remaining_transfers={remaining_transfers}",
            start_ms / 60_000,
            end_ms as f64 / 60_000.0
        );
        hourly.push(json!({
            "start_ms": start_ms, "end_ms": end_ms,
            "completed_units": completed_units, "units_per_hour": units_per_hour, "remaining_units": remaining_units,
            "completed_transfers": completed_transfers, "transfers_per_hour": transfers_per_hour,
            "remaining_transfers": remaining_transfers,
            "completed_picks": completed_units, "picks_per_hour": units_per_hour, "remaining_picks": remaining_units,
        }));
    }
    if check == BacklogCheck::Sustained {
        assert!(last_pick_ms > 8 * 3_600_000, "backlog must last beyond eight hours to validate sustained flow");
        let unstarted_at_eight_hours =
            work.values().filter(|order| order.first_pick.unwrap().as_ms() > 8 * 3_600_000).count();
        let picking_berths: usize = data.map.conveyor_machines.iter().map(|layout| layout.waiting_cells.len()).sum();
        eprintln!(
            "[{label}] orders not yet picked at eight hours={unstarted_at_eight_hours}; picking berths={picking_berths}"
        );
        assert!(
            unstarted_at_eight_hours > picking_berths,
            "eight hours must retain demand beyond the admitted picking work"
        );
        let mut completions: Vec<_> = facts.transfers.iter().map(|transfer| transfer.pair.end.as_ms()).collect();
        completions.push(0);
        completions.sort_unstable();
        let longest_gap = completions.windows(2).map(|times| times[1] - times[0]).max().unwrap();
        eprintln!("[{label}] longest physical pick completion gap={:.3}s", longest_gap as f64 / 1000.0);
        assert!(longest_gap < 3_600_000, "an entire hour passed without a physical pick while demand remained");
    }
    if let Some(directory) = trace_directory {
        fs::write(directory.join(format!("{label}-orders.json")), serde_json::to_vec_pretty(&order_rows).unwrap())
            .expect("write audited per-order physical milestones in input order");
        fs::write(directory.join(format!("{label}-hourly.json")), serde_json::to_vec_pretty(&hourly).unwrap())
            .expect("write physical completion windows with remaining demand");
    }
    eprintln!(
        "[{label}] complete: orders={expected_orders} picked_units={expected_units} paired_transfers={} boxes={expected_boxes} buffered_orders={expected_buffered} sim_secs={}",
        facts.transfers.len(),
        result.sim_duration.as_secs_f64()
    );
}
