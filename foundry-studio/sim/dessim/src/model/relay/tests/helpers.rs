//! Shared test harness for relay service tests.

use crate::model::navigation::Navigation;
use crate::model::{BotKind, BotService};
use crate::pathfinding::Pathfinder;
use desmap::VtmPosition;

use std::collections::{BTreeMap, HashSet};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use desgraph::graph::Edge;
use desgraph::{Cell, CellId, CellKind, WarehouseGraphBuilder, WorldPos};
use desgraph::{GridPos, RelayId, ZoneId};
use desmap::BlockSide;
use desmap::{SystemType, WarehouseMapBuilder, Zone, ZoneHolds};

use crate::config::SimConfig;
use crate::config::VtmConfig;
use crate::model::RackId;
use crate::model::RobotId;
use crate::model::ToteDesignation;
use crate::model::htm::registry::AvailableHtmRegistry;
use crate::model::ids::ToteId;
use crate::model::inventory::InventoryService;
use crate::model::location::{ToteLocation, ToteType};
use crate::model::relay::*;
use crate::model::reservation::cell_reservation::CellReservation;
use crate::model::reservation::oracle::GhostOracle;
use crate::model::reservation::oracle::ReservationOracle;
use crate::model::{SimDuration, SimTime};
use crate::pathfinding::{Constant, DriveModel, FHSippPathfinder};
use crate::robots::reference_htm;
use crate::robots::runtime::RuntimeRobots;
use crate::scenario_gen::{RelaySetupData, ScenarioData, ZoneSetupData};

/// Build an empty `AvailableHtmRegistry` for tests that don't exercise HTM
/// residency. `RelayService::new` requires one to subscribe its
/// `RelayHtmIndex`; tests that don't seed HTMs see every relay scored at
/// `PICK_HTM_TO_RELAY_DEFAULT` (the index's "non-resident" default).
pub fn empty_htm_registry(data: &ScenarioData, inventory: &InventoryService) -> AvailableHtmRegistry {
    AvailableHtmRegistry::new(inventory.clone(), &data.map, HashSet::new(), None)
}

/// Build RelayZones + InventoryService from a compact spec.
///
/// `zone_spec`: &[(zone_id, &[(relay_id, cell_x)])]
///   Each relay gets a cell at grid pos (cell_x, 0).
/// `rack_spec`: &[(rack_id, cell_x, zone_id)]
///   Each rack gets a cell at grid pos (cell_x, 0) in the given zone.
pub fn build_test_env(
    zone_spec: &[(u32, &[(u32, u32)])],
    rack_spec: &[(u32, u32, u32)],
) -> (ScenarioData, InventoryService, Arc<FHSippPathfinder>) {
    let max_cell: u32 = zone_spec
        .iter()
        .flat_map(|(_, relays)| relays.iter().map(|&(_, x)| x))
        .chain(rack_spec.iter().map(|&(_, x, _)| x))
        .max()
        .unwrap_or(0);

    let mut builder = WarehouseGraphBuilder::new();
    for id in 0..=max_cell + 100 {
        builder.add_cell(Cell {
            id: CellId(id),
            kind: CellKind::Open,
            zone: ZoneId(0),
            sector: None,
            grid_pos: Some(GridPos { x: id, y: 0 }),
            // 1 grid unit = 1m for tests.
            world_pos: Some(WorldPos { x_mm: id as f64 * 1000.0, y_mm: 0.0 }),
            barcode: None,
            htm_rotation_forbidden: false,
            rack_orientation: None,
            station_id: None,
        });
    }
    for id in 0..max_cell + 100 {
        builder.add_edge_bidi(CellId(id), CellId(id + 1), Edge::uniform(1000));
    }
    let graph = builder.build();

    let mut relay_data = Vec::new();
    let mut zone_data = Vec::new();

    for &(zid, relays) in zone_spec {
        let mut relay_ids = BTreeSet::new();
        for &(rid, cell_x) in relays {
            relay_ids.insert(RelayId(rid));
            relay_data.push(RelaySetupData {
                relay_id: RelayId(rid),
                vtm_access: CellId(cell_x),
                htm_access: CellId(cell_x),
                // Single-sided test geometry: all relays and racks are Left.
                // Dedicated dual-side tests construct their own fixtures.
                side: BlockSide::Left,
            });
        }
        zone_data.push(ZoneSetupData { zone_id: ZoneId(zid), relay_ids });
    }

    // Rack → cell and zone → storage cells.
    let mut rack_cells: BTreeMap<RackId, CellId> = BTreeMap::new();
    let mut rack_side: BTreeMap<RackId, BlockSide> = BTreeMap::new();
    let mut zone_storage: BTreeMap<ZoneId, BTreeSet<CellId>> = BTreeMap::new();
    for &(rack_id, cell_x, zone_id) in rack_spec {
        let cid = CellId(cell_x);
        rack_cells.insert(RackId(rack_id), cid);
        rack_side.insert(RackId(rack_id), BlockSide::Left);
        zone_storage.entry(ZoneId(zone_id)).or_default().insert(cid);
    }

    let rack_grid_pos: HashMap<RackId, (u32, u32)> =
        rack_spec.iter().map(|&(rid, cell_x, _)| (RackId(rid), (cell_x, 0))).collect();

    // Register one VTM per zone so scoring can query the VTM frontier.
    // Production scenarios always have VTMs per zone; tests must mirror that
    // since `select_relay_for_retrieve_*` and `reserve_relay_for_deposit`
    // now compute deposit time from the zone's earliest VTM availability.
    // Zone with no VTMs ⇒ candidate is correctly filtered out (production-
    // realistic: no VTM, no chain).
    let mut vtm_zone_assignments: Vec<BTreeSet<ZoneId>> = Vec::new();
    let mut vtm_initial_positions: Vec<VtmPosition> = Vec::new();
    for &(zid, relays) in zone_spec {
        let mut zones_set = BTreeSet::new();
        zones_set.insert(ZoneId(zid));
        vtm_zone_assignments.push(zones_set);
        // Park the VTM at the first relay's cell — any in-zone cell works
        // for VtmService::new's world-pos lookup. An empty relay list for a
        // zone is a malformed test input; panic with context.
        let first_relay_cell =
            relays.first().map(|&(_, x)| CellId(x)).unwrap_or_else(|| panic!("test fixture: zone {zid} has no relays"));
        vtm_initial_positions.push(VtmPosition::Cell(first_relay_cell));
    }

    let map = WarehouseMapBuilder::new(SystemType::Relay).graph(graph).build();
    let robot_models = RuntimeRobots::legacy(&SimConfig::default(), 0, vtm_zone_assignments.len());
    let mut data = ScenarioData {
        robot_models,
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
        rack_use: Default::default(),
        board_floors_m: BTreeMap::new(),
        rack_placements: Vec::new(),
        vtm_zone_assignments,
        vtm_initial_positions,
        htm_initial_positions: Vec::new(),
        rack_to_vtm_access: BTreeMap::new(),
        vertical_tote_capacity: 10,
        relay_zones: RelayZones::default(),
        demand_pool: None,
    };

    data.relay_zones = RelayZones::from_scenario(&data).expect("test scenario should be valid");
    let pathfinder = Arc::new(FHSippPathfinder::new(&data.map, 1000, SimDuration::seconds(1), HashSet::new()));

    let inventory = InventoryService::new();

    (data, inventory, pathfinder)
}

/// The HTM drives that go with `build_test_env`'s pathfinder, `[empty, laden]`:
/// the M5F's figures at constant speed. The relay tests run at `ConstantSpeed`,
/// so the same pair serves estimates and reservations.
pub fn test_drives() -> [Arc<dyn DriveModel>; 2] {
    let reference = reference_htm();
    [Arc::new(Constant::htm(&reference, false)), Arc::new(Constant::htm(&reference, true))]
}

pub fn test_navigation(
    data: &ScenarioData,
    pathfinder: Arc<dyn Pathfinder>,
    oracle: Arc<dyn ReservationOracle>,
) -> Arc<Navigation> {
    let bots = BotService::new(crate::model::test_model::reference_model());
    // These named clients are used by the independent relay deposit/custody fixtures.
    for id in [0, 1, 2, 99, 101] {
        bots.register(RobotId(id), BotKind::Htm, CellId(0)).unwrap();
    }
    Arc::new(Navigation::new(Arc::new(data.map.clone()), pathfinder, bots, oracle))
}

/// Build a RelayService with auto-generated racks and pre-seeded empty slots.
///
/// For each relay at (relay_id, cell_x), creates a rack at (relay_id, cell_x+2)
/// in the same zone. Then registers a tote per rack and moves it out, creating
/// empty slots so `reserve_relay_for_deposit` has candidates to score.
pub fn test_service(zone_spec: &[(u32, &[(u32, u32)])]) -> RelayService {
    test_service_with_oracle(zone_spec, Arc::new(GhostOracle))
}

pub fn test_service_with_oracle(
    zone_spec: &[(u32, &[(u32, u32)])],
    oracle: Arc<dyn ReservationOracle>,
) -> RelayService {
    let rack_spec: Vec<(u32, u32, u32)> =
        zone_spec.iter().flat_map(|&(zid, relays)| relays.iter().map(move |&(rid, x)| (rid, x + 2, zid))).collect();
    let (data, inv, pathfinder) = build_test_env(zone_spec, &rack_spec);
    let registry = empty_htm_registry(&data, &inv);
    let svc = RelayService::new(
        &data,
        inv.clone(),
        super::helpers::test_navigation(&data, pathfinder, oracle),
        &registry,
        &VtmConfig::default(),
    )
    .expect("relay inventory index");
    // Seed empty slots: register a tote per rack, then move it out.
    for &(rack_id, _, _) in &rack_spec {
        let tid = ToteId(9000 + rack_id);
        inv.register_tote(
            tid,
            ToteType::Tote,
            ToteDesignation::Inventory,
            ToteLocation::InRack { rack_id: RackId(rack_id), z_index: 0 },
        )
        .expect("seed: register_tote");
        inv.tote(tid)
            .expect("seed: tote")
            .move_to(ToteLocation::WithRobot { robot_id: RobotId(9000 + rack_id) }, SimTime::ZERO)
            .expect("seed: move_to");
    }
    svc
}

/// A wired service whose racks are not all the same kind of steel.
///
/// `uses` names the racking each rack is, so a test can put a row of order
/// racking inside an ordinary storage zone -- which is what DFW6's row of
/// empty order totes is, and what no zone-level distinction can express.
/// Every rack starts with a tote in it and the tote is then moved out, so
/// every slot is an empty candidate for a deposit.
pub fn wired_service_with_uses(uses: &[(u32, RackUse)]) -> (RelayService, InventoryService) {
    let zone_spec: &[(u32, &[(u32, u32)])] = &[(1, &[(1, 10), (2, 20), (3, 30), (4, 40)])];
    let rack_spec: Vec<(u32, u32, u32)> =
        uses.iter().enumerate().map(|(i, &(rid, _))| (rid, 12 + 10 * i as u32, 1)).collect();
    let (mut data, inventory, pathfinder) = build_test_env(zone_spec, &rack_spec);
    data.rack_use = uses.iter().map(|&(rid, u)| (RackId(rid), u)).collect();
    if !uses.is_empty() && uses.iter().all(|(_, uses)| *uses == RackUse::OrderBuffer) {
        data.map.zones.push(Zone {
            id: ZoneId(1),
            name: None,
            cells: data.rack_cells.values().copied().collect(),
            relays: data.relay_data.iter().map(|relay| relay.relay_id).collect(),
            vtm_count: 1,
            holds: ZoneHolds::Order,
            vtm_configuration: None,
            zone_height_mm: 10_000,
            zone_width_mm: None,
            real_shelf_count: 10,
            slots: None,
        });
    }
    let registry = empty_htm_registry(&data, &inventory);
    let svc = RelayService::new(
        &data,
        inventory.clone(),
        super::helpers::test_navigation(&data, pathfinder, Arc::new(GhostOracle)),
        &registry,
        &VtmConfig::default(),
    )
    .expect("relay inventory index");
    for &(rack_id, uses) in uses {
        // A buffer slot starts with no tote at all; the index seeds those
        // itself. Everywhere else the empty slot has to be made by moving a
        // tote out of it, which is how the index learns one in production.
        if uses == RackUse::OrderBuffer {
            continue;
        }
        let tid = ToteId(9000 + rack_id);
        let designation = match uses {
            RackUse::OrderDepot => ToteDesignation::Order,
            _ => ToteDesignation::Inventory,
        };
        inventory
            .register_tote(
                tid,
                ToteType::Tote,
                designation,
                ToteLocation::InRack { rack_id: RackId(rack_id), z_index: 0 },
            )
            .expect("seed: register_tote");
        inventory
            .tote(tid)
            .expect("seed: tote")
            .move_to(ToteLocation::WithRobot { robot_id: RobotId(9000 + rack_id) }, SimTime::ZERO)
            .expect("seed: move_to");
    }
    (svc, inventory)
}

/// Standard 2-zone layout (4 relays each).
pub fn two_zone_service() -> RelayService {
    test_service(&[(1, &[(1, 10), (2, 20), (3, 30), (4, 40)]), (2, &[(5, 50), (6, 60), (7, 70), (8, 80)])])
}

/// RelayService wired to InventoryService via observer hook.
/// Zone 1: relays 1,2,3,4 at x=10,20,30,40.
/// Racks 1-5 at x=12,22,32,42,52 in zone 1.
pub fn wired_service() -> (RelayService, InventoryService) {
    let zone_spec: &[(u32, &[(u32, u32)])] = &[(1, &[(1, 10), (2, 20), (3, 30), (4, 40)])];
    // (rack_id, cell_x, zone_id)
    let rack_spec: &[(u32, u32, u32)] = &[(1, 12, 1), (2, 22, 1), (3, 32, 1), (4, 42, 1), (5, 52, 1)];
    let (data, inventory, pathfinder) = build_test_env(zone_spec, rack_spec);
    let registry = empty_htm_registry(&data, &inventory);
    let svc = RelayService::new(
        &data,
        inventory.clone(),
        super::helpers::test_navigation(&data, pathfinder, Arc::new(GhostOracle)),
        &registry,
        &VtmConfig::default(),
    )
    .expect("relay inventory index");
    (svc, inventory)
}

/// Records a pickup unit for `zone` riding `hold`, so a test outside the relay can put one on its ledger.
pub fn record_pickup_unit(navigation: &Navigation, zone: ZoneId, hold: CellReservation) {
    navigation.pickup_units.record(zone, hold);
}

/// Counts `zone`'s pickup units whose hold the relay counts as open, so a test outside the relay can read them.
pub fn count_open_pickup_units(navigation: &Navigation, relay: &RelayService, zone: ZoneId) -> u32 {
    navigation.pickup_units.count(zone, |hold| relay.pickup_hold_is_open(hold))
}
