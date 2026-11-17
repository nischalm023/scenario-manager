//! Test-only `TestModel` scaffold for task-level scheduling tests.
//!
//! Within `model`, `test_model` assembles a real `TestModel` — line-map
//! warehouse, relay service, inventory, reservation oracle, HTM registry —
//! small enough for a unit test to stage one `propose`/`schedule` decision and
//! inspect its effects. It exists so every task module's tests drive the
//! SAME construction path instead of each growing a divergent private
//! copy (extracted from `htm_store`'s tests when `htm_pickup` and
//! `htm_deliver` gained characterization tests of their own).
//!
//! Geometry: cells 0..=60 in a line (bidirectional edges between
//! neighbors), relays 1..=4 with both access cells at x=10/20/30/40,
//! racks 1..=4 at x=12/22/32/42, one zone. `island_cells` adds cells
//! that have a grid position but NO edges — statically unreachable
//! targets for infeasibility tests.

use crate::events::{EventJournal, EventSink, SimEvent};
use crate::model::navigation::Navigation;
use crate::model::parking::ParkingRegistry;
use desmap::{RelayPosition, VtmPosition};

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::error::Error;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use derive_more::Display;
use pathfinder::ReservationOracle as _;
use pathfinder::WindowSink;
use tokio::runtime::Builder;
use tokio::time::sleep;

use crate::error::{PlanError, ReservationError};
use crate::scheduler::Scheduler;
use crate::scheduler::task::proposal::{EmptyProposal, ProposalId};
use crate::scheduler::task::{SchedCtx, Task};
use crate::signals::Mut;
use crate::sim_types::{StopReason, StopSignal};

use desgraph::graph::Edge;
use desgraph::{BlockId, Cell, CellId, CellKind, GridPos, RelayId, StationId, WarehouseGraphBuilder, WorldPos, ZoneId};
use desmap::{BlockSide, SystemType, WarehouseMapBuilder};

use crate::config::SimConfig;
use crate::config::VtmConfig;
use crate::model::bot::plan::{BotExpectation, Leg, TaskKind, WaypointKind};
use crate::model::bot::{BotKind, BotService, LegSpec};
use crate::model::conveyor::ConveyorLoops;
use crate::model::factories::WorldInputs;
use crate::model::htm::registry::{AvailableHtm, AvailableHtmRegistry, Observer};
use crate::model::ids::{RackId, RackSlot, RobotId, ToteId};
use crate::model::inventory::InventoryService;
use crate::model::location::ToteDesignation;
use crate::model::location::{ToteLocation, ToteType};
use crate::model::relay::{RelayReservation, RelayService, RelayZones};
use crate::model::reservation::oracle::{ReservationOracle, TableOracle};
use crate::model::reservation::{CellReservation, ReservationTable};
use crate::model::station::StationSet;
use crate::model::station_service::StationService;
use crate::model::{SimDuration, SimTime, StationArrivalDiagnostics, StationFetchBounds};
use crate::pathfinding::{Constant, DriveModel, FHSippPathfinder, MotionModel, Orientation, Pathfinder, ReservedPath};
use crate::robots::reference_htm;
use crate::robots::runtime::{FloorRobotModel, RuntimeRobots};
use crate::scenario_gen::{RelaySetupData, ScenarioData, ZoneSetupData};
use crate::simtime::TimeRange;

/// The test model's HTM drives, `[empty, laden]`: the reference HTM at its
/// constant speed. The model runs at `ConstantSpeed`, so the same pair serves its
/// estimates and its reservations.
pub(crate) fn reference_drives() -> [Arc<dyn DriveModel>; 2] {
    let reference = reference_htm();
    [Arc::new(Constant::htm(&reference, false)), Arc::new(Constant::htm(&reference, true))]
}

pub fn reference_model() -> Arc<FloorRobotModel> {
    let mut model = FloorRobotModel::legacy(&SimConfig::default());
    model.execution = reference_drives();
    model.estimate = reference_drives();
    model.kinematics = reference_htm();
    model.kind = MotionModel::ConstantSpeed;
    Arc::new(model)
}

/// The side of `reserve_path` for a test's journey of one window: the journal's simtime barrier, and a window short of
/// the target refused, as the tasks' own one-window journey refuses it.
pub struct OneWindow<'a>(pub &'a EventJournal);

impl WindowSink for OneWindow<'_> {
    fn barrier(&self) -> SimTime {
        self.0.barrier()
    }

    fn committed(
        &mut self,
        window: &ReservedPath,
        _closed: Option<&CellReservation>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let end = window.path.waypoints.last().map(|waypoint| waypoint.cell);
        Err(format!("the test's journey is one window, and it stopped short on {end:?}").into())
    }
}

/// Test fixture ownership only; production tasks receive its individual handles.
pub struct TestModel {
    pub navigation: Arc<Navigation>,
    pub bot_service: BotService,
    pub relay_service: RelayService,
    pub inventory: InventoryService,
    pub(crate) stations: StationSet,
    pub(crate) conveyors: ConveyorLoops,
    pub(crate) relay_htm_access: HashMap<RelayId, CellId>,

    pub(crate) station_presentation: HashMap<StationId, CellId>,
    pub(crate) station_order_presentation: HashMap<StationId, CellId>,
    pub station_service: Arc<StationService>,
    pub station_fetch_bounds: StationFetchBounds,
    pub station_arrivals: StationArrivalDiagnostics,
    pub(crate) parking: Option<ParkingRegistry>,
    pub available_htm: AvailableHtmRegistry,
    pub events: EventJournal,
    pub(crate) tripwires: bool,
    pub(crate) delivery_max_wait_override: Option<SimDuration>,
    /// The scenario the fixture was built from, for `provide_states`.
    pub(crate) data: Arc<ScenarioData>,
}

/// Knobs a test can turn on the shared scaffold. `Default` reproduces the
/// original `htm_store` test model minus its one BotService HTM.
#[derive(Default)]
pub struct TestModelConfig {
    pub floor_model: Option<Arc<FloorRobotModel>>,
    /// HTMs registered with `BotService` only (id, cell) — present in the
    /// world but NOT idle-available. For an idle HTM holding its resting
    /// cell, use [`register_idle_htm`] instead.
    pub bot_htms: Vec<(RobotId, CellId)>,
    /// Cells added with a grid position but no edges: statically
    /// unreachable from the line.
    pub island_cells: Vec<CellId>,
    /// `TestModel::station_presentation` entries.
    pub stations: Vec<(StationId, CellId)>,
    /// Cells of the line that are queue cells, in order along the line, each
    /// feeding the next and the last the cell after it. As the map builders
    /// make a station's queue, it runs one way, entered only at its first cell,
    /// and the cell it feeds is entered only from it and left only onward.
    pub queue_cells: Vec<CellId>,
    /// `pick.delivery_max_wait_secs` override (`HtmDeliver`'s search
    /// horizon).
    pub delivery_max_wait_override: Option<SimDuration>,
}

pub fn build_model(cfg: TestModelConfig) -> (Arc<TestModel>, Arc<TableOracle>) {
    let mut builder = WarehouseGraphBuilder::new();
    let island: BTreeSet<CellId> = cfg.island_cells.iter().copied().collect();
    for id in (0..=60).map(CellId).chain(island.iter().copied()) {
        builder.add_cell(Cell {
            id,
            kind: if cfg.queue_cells.contains(&id) { CellKind::QueueCell } else { CellKind::Open },
            zone: ZoneId(0),
            sector: None,
            grid_pos: Some(GridPos { x: id.0, y: 0 }),
            world_pos: Some(WorldPos { x_mm: id.0 as f64 * 1000.0, y_mm: 0.0 }),
            barcode: None,
            htm_rotation_forbidden: false,
            rack_orientation: None,
            station_id: None,
        });
    }
    let fed = cfg.queue_cells.last().map(|last| CellId(last.0 + 1));
    for (here, next) in (0..60).map(|id| (CellId(id), CellId(id + 1))) {
        if cfg.queue_cells.contains(&here) || cfg.queue_cells.contains(&next) || fed == Some(here) {
            builder.add_edge(here, next, Edge::uniform(1000));
        } else {
            builder.add_edge_bidi(here, next, Edge::uniform(1000));
        }
    }
    let graph = builder.build();
    let map = WarehouseMapBuilder::new(SystemType::Relay).graph(graph).build();

    let relays = [(1, 10), (2, 20), (3, 30), (4, 40)];
    let relay_data: Vec<_> = relays
        .iter()
        .map(|&(relay_id, cell_x)| RelaySetupData {
            relay_id: RelayId(relay_id),
            vtm_access: CellId(cell_x),
            htm_access: CellId(cell_x),
            side: BlockSide::Left,
        })
        .collect();
    let zone_data = vec![ZoneSetupData {
        zone_id: ZoneId(1),
        relay_ids: relays.iter().map(|&(relay_id, _)| RelayId(relay_id)).collect::<BTreeSet<_>>(),
    }];

    let rack_spec = [(1, 12), (2, 22), (3, 32), (4, 42)];
    let rack_cells: BTreeMap<_, _> =
        rack_spec.iter().map(|&(rack_id, cell_x)| (RackId(rack_id), CellId(cell_x))).collect();
    let rack_side: BTreeMap<_, _> = rack_spec.iter().map(|&(rack_id, _)| (RackId(rack_id), BlockSide::Left)).collect();
    let zone_storage: BTreeMap<_, _> =
        [(ZoneId(1), rack_spec.iter().map(|&(_, cell_x)| CellId(cell_x)).collect::<BTreeSet<_>>())].into();
    let rack_grid_pos: HashMap<_, _> =
        rack_spec.iter().map(|&(rack_id, cell_x)| (RackId(rack_id), (cell_x, 0))).collect();
    let relay_grid_pos: HashMap<_, _> =
        relays.iter().map(|&(relay_id, cell_x)| (RelayId(relay_id), (cell_x, 0))).collect();
    let cell_grid_pos: HashMap<_, _> =
        (0..=60).map(CellId).chain(island.iter().copied()).map(|id| (id, (id.0, 0))).collect();

    let robot_models = RuntimeRobots::legacy(&SimConfig::default(), 0, 1);
    let mut data = ScenarioData {
        robot_models,
        board_floors_m: BTreeMap::new(),
        rack_use: BTreeMap::new(),
        map,
        skus: Vec::new(),
        orders: Vec::new(),
        inventory: BTreeMap::new(),
        zone_data,
        relay_data,
        station_data: Vec::new(),
        zone_storage,
        tote_placements: Vec::new(),
        rack_cells,
        rack_world_pos: Default::default(),
        rack_floor_m: Default::default(),
        relay_exchange_pos: Default::default(),
        rack_side,
        rack_placements: Vec::new(),
        vtm_zone_assignments: vec![BTreeSet::from([ZoneId(1)])],
        vtm_initial_positions: vec![VtmPosition::Cell(CellId(10))],
        htm_initial_positions: Vec::new(),
        rack_to_vtm_access: BTreeMap::new(),
        vertical_tote_capacity: 10,
        relay_zones: RelayZones::default(),
        demand_pool: None,
    };
    data.relay_zones = RelayZones::from_scenario(&data).expect("relay zones");

    let pathfinder = Arc::new(FHSippPathfinder::new(&data.map, 1000, SimDuration::seconds(1), HashSet::new()));
    let pathfinder_dyn: Arc<dyn Pathfinder> = pathfinder.clone();
    let oracle = Arc::new(TableOracle::new(ReservationTable::default()));
    let oracle_dyn: Arc<dyn ReservationOracle> = oracle.clone();
    let inventory = InventoryService::new();
    let available_htm = AvailableHtmRegistry::new(inventory.clone(), &data.map, HashSet::new(), None);
    let bot_service = BotService::new(cfg.floor_model.clone().unwrap_or_else(reference_model));
    let navigation = Arc::new(Navigation::new(
        Arc::new(data.map.clone()),
        pathfinder_dyn.clone(),
        bot_service.clone(),
        oracle_dyn.clone(),
    ));
    let relay_service =
        RelayService::new(&data, inventory.clone(), navigation.clone(), &available_htm, &VtmConfig::default())
            .expect("relay inventory index");

    for &(robot, cell) in &cfg.bot_htms {
        bot_service.register(robot, BotKind::Htm, cell).unwrap();
    }
    inventory.set_bot_service(bot_service.clone());

    let model = Arc::new(TestModel {
        conveyors: ConveyorLoops::default(),
        relay_htm_access: HashMap::new(),

        station_presentation: HashMap::new(),
        station_order_presentation: HashMap::new(),
        navigation,
        bot_service,
        relay_service,
        inventory,
        stations: StationSet::new(vec![]),

        station_service: Arc::new(StationService::new(cfg.stations.iter().copied().collect(), oracle_dyn)),
        station_fetch_bounds: StationFetchBounds::default(),
        station_arrivals: StationArrivalDiagnostics::new(),
        parking: None,
        available_htm,
        events: EventJournal::new(None),
        tripwires: false,
        delivery_max_wait_override: cfg.delivery_max_wait_override,
        data: Arc::new(data),
    });
    (model, oracle)
}

/// Make `robot` an idle available HTM resting at `cell`, the way the real
/// flow leaves one behind: registered with `BotService`, holding an
/// open-ended oracle reservation on its resting cell, and entered in the
/// available registry (which snapshots any tote registered
/// `WithRobot { robot }` beforehand — register the tote first to model a
/// cargo-carrying holder).
pub fn register_idle_htm(model: &TestModel, oracle: &TableOracle, robot: RobotId, cell: CellId, at: SimTime) {
    model.bot_service.register(robot, BotKind::Htm, cell).unwrap();
    let hold = oracle.table().reserve_cell(robot, cell, TimeRange::open_ended(at)).expect("idle HTM resting-cell hold");
    model.available_htm.register(hold, at, Default::default(), None).expect("idle HTM registry entry");
}

/// Seed an empty tote resting in `rack`, retrievable by
/// `ToteSearchCriteria::Empty` — the cheapest honest input for
/// `reserve_relay_for_retrieve` in pickup tests.
pub fn seed_empty_rack_tote(model: &TestModel, tote: ToteId, rack: RackId) {
    model
        .inventory
        .register_tote(
            tote,
            ToteType::Tote,
            ToteDesignation::Inventory,
            ToteLocation::InRack { rack_id: rack, z_index: 0 },
        )
        .expect("seed rack tote");
}

/// Publish the fixture's handles as `scheduler`'s states, so a task that
/// builds its children from a task context (a clearance's return, its
/// parking, a pick's pickup) reaches the world this fixture staged.
/// `parking` stands in for the run's parking registry.
pub fn provide_states(scheduler: &Scheduler, model: &TestModel, parking: ParkingRegistry) {
    let states = scheduler.states();
    let pathfinder = model.navigation.pathfinder.clone();
    let world =
        WorldInputs::new(model.data.clone(), SimConfig::default(), pathfinder, None, scheduler.barrier().clock());
    states.configure(world).expect("one world per scheduler");
    states.provide(model.navigation.clone());
    states.provide(Arc::new(model.bot_service.clone()));
    states.provide(Arc::new(model.inventory.clone()));
    states.provide(Arc::new(model.available_htm.clone()));
    states.provide(Arc::new(model.relay_service.clone()));
    states.provide(Arc::new(model.events.clone()));
    states.provide(Arc::new(parking));
}

/// An entrant blocked on `hold` in scheduler time, as a delivery or a
/// journey is: it answers the hold's earliest `Vacate` with a need-by at
/// `by`, and answers again once that `Vacate` is dropped. Added as a
/// service, it never holds the run open.
#[derive(Display)]
#[display("NeedVacated")]
pub struct NeedVacated {
    pub navigation: Arc<Navigation>,
    pub hold: CellReservation,
    pub by: SimTime,
}

#[async_trait(?Send)]
impl Task for NeedVacated {
    type Proposal = EmptyProposal;
    type Schedule = ();

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, _: SchedCtx) -> Result<(), PlanError> {
        match self.navigation.cells.keep_need_by(&self.hold, self.by).await {}
    }
}

/// Bound a missing-wakeup regression without delaying a successful run.
pub fn bounded_scheduler() -> Scheduler {
    let stop = StopSignal::new();
    let mut scheduler = Scheduler::new().with_stop_signal(stop.clone());
    scheduler.add_service(TestDeadline(stop));
    scheduler
}

#[derive(Display)]
#[display("TestDeadline")]
struct TestDeadline(Arc<StopSignal>);

#[async_trait(?Send)]
impl Task for TestDeadline {
    type Proposal = EmptyProposal;
    type Schedule = ();

    async fn propose(&self, _: ProposalId) -> Result<EmptyProposal, PlanError> {
        Ok(EmptyProposal)
    }

    async fn schedule(&mut self, _: EmptyProposal, _: SchedCtx) -> Result<(), PlanError> {
        sleep(Duration::from_secs(1)).await;
        self.0.stop(StopReason::WallTimeout);
        Ok(())
    }
}

/// An empty slot at the bottom of each of `racks`, left by the inventory tote
/// that robot 100 + rack took out of it: room for deposits, so that a
/// declined one wants a relay, not a slot.
pub fn empty_rack_slots(model: &TestModel, racks: impl IntoIterator<Item = u32>) {
    for rack in racks {
        let tote = ToteId(720 + rack);
        let racked = ToteLocation::InRack { rack_id: RackId(rack), z_index: 0 };
        model.inventory.register_tote(tote, ToteType::Tote, ToteDesignation::Inventory, racked).unwrap();
        let carried = ToteLocation::WithRobot { robot_id: RobotId(100 + rack) };
        model.inventory.tote(tote).unwrap().move_to(carried, SimTime::ZERO).unwrap();
    }
}

/// One parking bay, on the access cell of a relay 9 at cell 3, with the
/// navigation that knows it as a resting cell.
pub fn one_bay(model: &TestModel) -> (Arc<Navigation>, ParkingRegistry) {
    let mut map = model.navigation.map.as_ref().clone();
    map.relays = vec![RelayPosition {
        id: RelayId(9),
        position: CellId(3),
        htm_access: CellId(3),
        vtm_access: VtmPosition::Cell(CellId(0)),
        block: BlockId(0),
        side: BlockSide::Left,
    }];
    let bay = ParkingRegistry::from_map(&map);
    let resting_at_bay = Arc::new(FHSippPathfinder::new(&map, 1000, SimDuration::seconds(1), [CellId(3)].into()));
    let navigation = Arc::new(Navigation::new(
        model.navigation.map.clone(),
        resting_at_bay,
        model.bot_service.clone(),
        model.navigation.oracle.clone(),
    ));
    (navigation, bay)
}

/// Robot 65 standing open-ended on `one_bay`'s bay since simtime zero, busy,
/// so the bay takes no squatter until robot 65's next path commits.
pub fn bay_held_by_a_busy_htm(model: &TestModel, oracle: &TableOracle) -> AvailableHtm {
    register_idle_htm(model, oracle, RobotId(65), CellId(3), SimTime::ZERO);
    model.available_htm.take(RobotId(65)).expect("the bay's HTM is busy")
}

/// `robot`, empty, standing on `cell` from simtime zero, its stay in the
/// registry idle only from simtime `ready`.
pub fn squatter_ready_at(model: &TestModel, oracle: &TableOracle, robot: RobotId, cell: CellId, ready: SimTime) {
    register_idle_htm(model, oracle, robot, cell, SimTime::ZERO);
    let stay = model.available_htm.take(robot).unwrap();
    model.available_htm.register(stay.cell_reservation, ready, stay.orientation, None).unwrap();
}

/// The robots whose stays the registry handed out, in order.
#[derive(Default)]
pub struct Takes(pub Mutex<Vec<RobotId>>);

impl Observer for Takes {
    fn on_register(&self, _: &AvailableHtm) {}

    fn on_take(&self, entry: &AvailableHtm) {
        self.0.lock().unwrap().push(entry.robot());
    }
}

/// Every relay's slot but `free`'s busy until `until`: a deposit off `from`
/// for each of HTMs 80 to 83, finalized during setup with occupancy ending at
/// simtime `until`, which leaves the racks' empty slots free; `free`'s is
/// given back once all four are reserved.
pub fn relays_busy_until(model: &TestModel, from: CellId, until: SimTime, free: Option<RelayId>) {
    let mut relays = Vec::new();
    let mut given_back = None;
    for robot in (80..84).map(RobotId) {
        model.bot_service.register(robot, BotKind::Htm, CellId(1)).unwrap();
        let deposit =
            block(model.relay_service.reserve_relay_for_deposit(from, Orientation::East, SimTime::ZERO, robot, None))
                .expect("a relay not yet reserved by this fixture");
        relays.push(deposit.relay_id);
        if Some(deposit.relay_id) == free {
            given_back = Some(deposit);
        } else {
            deposit.finalize(until).unwrap();
        }
    }
    if let Some(deposit) = given_back {
        deposit.cancel();
    }
    relays.sort();
    relays.dedup();
    assert_eq!(relays.len(), 4, "each relay reserved once");
}

/// Deposits off `from` departing at `at`, for HTMs 80 onward that the bot
/// service learns of here, until one is declined for want of a relay, a
/// slot or headroom: no other deposit is admitted until one of these, or a
/// reservation made before them, is given back.
pub fn claim_the_zones_deposits(model: &TestModel, from: CellId, at: SimTime) -> Vec<RelayReservation<RackSlot>> {
    let mut claimed = Vec::new();
    for robot in (80..).map(RobotId) {
        model.bot_service.register(robot, BotKind::Htm, CellId(1)).unwrap();
        match block(model.relay_service.reserve_relay_for_deposit(from, Orientation::East, at, robot, None)) {
            Ok(deposit) => claimed.push(deposit),
            Err(ReservationError::NoCandidates { .. } | ReservationError::HeadroomExhausted { .. }) => return claimed,
            Err(error) => panic!("a deposit for {robot:?} was refused for another reason: {error}"),
        }
    }
    unreachable!("robot ids are unbounded")
}

/// Stage a completed pickup at the source of a storage leg. The inventory
/// location must already identify this HTM; the bot plan needs the same cargo.
pub fn register_loaded_htm(model: &TestModel, oracle: &TableOracle, robot: RobotId, cell: CellId, tote: ToteId) {
    register_idle_htm(model, oracle, robot, cell, SimTime::ZERO);
    model
        .bot_service
        .commit_task(
            robot,
            TaskKind::Deliver,
            &[LegSpec {
                leg: Leg::StationWork { cell },
                requests: &[],
                action_ready_arrival: SimTime::ZERO,
                wp_kind: WaypointKind::Travel,
                expect: BotExpectation::empty(),
                carrying_after: Some(tote),
            }],
            &model.inventory,
            None,
        )
        .expect("completed pickup boundary");
}

/// `hold` as the oracle holds it after the run. A reservation handle is a
/// snapshot: the end a later task's commit gives the reservation does not
/// reach it.
pub fn current_hold(oracle: &TableOracle, hold: &CellReservation) -> CellReservation {
    oracle
        .reservations_for_cell(hold.cell())
        .into_iter()
        .find(|current| current.id() == hold.id())
        .expect("the oracle holds the reservation")
}

/// Run one future to completion on a current-thread runtime, for tests that
/// call the async model from a plain `#[test]` body.
pub fn block<F: Future>(future: F) -> F::Output {
    Builder::new_current_thread().build().expect("a current-thread runtime").block_on(future)
}

/// Every fact a run emits, with the barrier B as it stood when the fact was
/// emitted, so a test can find a fact announced behind B.
pub struct EmittedFacts {
    clock: Mut<SimTime>,
    facts: Mutex<Vec<(SimEvent, SimTime)>>,
}

impl EventSink for EmittedFacts {
    fn send(&self, event: SimEvent) {
        self.facts.lock().unwrap().push((event, self.clock.value()));
    }
}

impl EmittedFacts {
    pub fn new(clock: Mut<SimTime>) -> Self {
        Self { clock, facts: Mutex::default() }
    }

    /// Where each window of `robot`'s journeys ends, and when it starts.
    pub fn windows_of(&self, robot: RobotId) -> Vec<(CellId, SimTime)> {
        let facts = self.facts.lock().unwrap();
        facts
            .iter()
            .filter_map(|(event, _)| match event {
                SimEvent::Nav(nav) if nav.robot_id == robot => Some((nav.waypoints.last()?.cell, nav.time)),
                _ => None,
            })
            .collect()
    }

    /// `robot`'s station assignments and arrivals, in the order emitted.
    pub fn assignments_and_arrivals_of(&self, robot: RobotId) -> Vec<&'static str> {
        let facts = self.facts.lock().unwrap();
        facts
            .iter()
            .filter_map(|(event, _)| match event {
                SimEvent::HtmAssignment(assignment) if assignment.robot_id == robot => Some("assignment"),
                SimEvent::HtmArrival(arrival) if arrival.robot_id == robot => Some("arrival"),
                _ => None,
            })
            .collect()
    }

    pub fn behind_the_barrier(&self) -> Vec<String> {
        let facts = self.facts.lock().unwrap();
        facts
            .iter()
            .filter(|(event, barrier)| event.stamp().is_some_and(|stamp| stamp < *barrier))
            .map(|(event, barrier)| format!("{event} behind B={barrier}"))
            .collect()
    }
}
