//! The Walmart floor must put robots beneath the shelves they exchange with
//! and give independent station ports independent approaches. These checks use
//! physical coordinates and compiled edges, so a visual adjustment cannot pass.

use std::path::Path;

use desgraph::HtmTraversal;
use dessim::worldmap::{self, WorldMap};
use rustc_hash::{FxHashMap, FxHashSet};
use site_model::components::product::Port;
use site_model::components::spatial::{Cell, Pose};
use site_model::components::structure::{Flow, FlowRule, Relay};
use site_model::entity::Entity;
use site_model::ids::EntityId;
use site_model::pipeline::World;
use site_model::pipeline::graph::{CellGeometry, cell_contributions};
use site_model::units::{Coord, Steps};

const APRON: &str = "b1f0a2c4-7d3e-5a91-8c62-4e5f0a1b2c3d";

fn load() -> WorldMap {
    worldmap::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/walmart_dfw6.json"))
        .expect("the full Walmart map loads")
}

fn physical_shelf(world: &World, shelf: &Entity) -> [Steps; 2] {
    let relay: Relay = shelf.require("relay").unwrap();
    let pose: Pose = world.entities[&relay.rack].require("pose").unwrap();
    let origin = pose.xy().unwrap();
    let offset = pose.heading_deg.rotate(relay.offset.map(|v| v.0 as f64));
    let point = [0, 1].map(|axis| Coord(Steps::round_half_up(origin[axis].0 as f64 + offset[axis]).0));
    world.frames.to_primary(pose.frame.as_ref().unwrap(), point).unwrap()
}

#[test]
fn every_relay_has_its_own_aligned_htm_stand_and_vtm_aisle_row() {
    let w = load();
    let world = &w.world;
    let unit = world.frames.unit_mm().unwrap();
    let contributions = cell_contributions(&world.entities, &world.cells);
    let mut occupied_stands = FxHashSet::default();
    assert_eq!(w.map.relays.len(), 2_132, "inventory and buffer handoff shelves remain present");
    assert_eq!(w.index.zone_aisles.len(), 28, "subdivision must not turn one aisle into multiple service zones");

    for relay in &w.map.relays {
        assert!(occupied_stands.insert(relay.htm_access), "two relays share stand {:?}", relay.htm_access);
        let stand_id = &w.index.cell_entities[&relay.htm_access];
        let stand = &world.cells[stand_id];
        let capability: Cell = world.entities[stand_id].require("cell").unwrap();
        assert!(capability.traversable_by.iter().any(|role| role == "htm"), "{stand_id} must admit HTMs");
        let runtime_stand = w.map.graph.cell(relay.htm_access).unwrap().world_pos.unwrap();
        let zone = w.map.graph.cell(relay.position).unwrap().zone;
        let aisle = &w.index.zone_aisles[&zone];
        let aisle_cells = contributions.get(aisle).expect("the serving aisle contributes physical floor cells");

        for shelf_id in &w.index.relay_shelves[&relay.id] {
            let shelf = physical_shelf(world, &world.entities[shelf_id]);
            for axis in 0..2 {
                assert!(
                    (stand.centre[axis].0 - shelf[axis].0).abs() <= 1,
                    "relay {shelf_id} and HTM stand {stand_id} disagree on axis {axis}: shelf {:?}, stand {:?}",
                    shelf,
                    stand.centre
                );
            }
            assert!((runtime_stand.x_mm - shelf[0].0 as f64 * unit).abs() <= unit);
            assert!((runtime_stand.y_mm - shelf[1].0 as f64 * unit).abs() <= unit);
            let aligned: Vec<_> =
                aisle_cells.iter().filter(|cell| (world.cells[*cell].centre[1].0 - shelf[1].0).abs() <= 1).collect();
            assert_eq!(aligned.len(), 1, "relay {shelf_id} needs exactly one same-Y VTM cell in aisle {aisle}");
            let vtm_cell: Cell = world.entities[aligned[0]].require("cell").unwrap();
            assert!(vtm_cell.traversable_by.iter().any(|role| role == "vtm"));
            assert!(!vtm_cell.traversable_by.iter().any(|role| role == "htm"), "rack aisles remain VTM-only");
        }
    }
}

#[test]
fn easyloop_dropoff_and_collection_have_independent_straight_apron_approaches() {
    let w = load();
    let world = &w.world;
    let apron: EntityId = APRON.parse().unwrap();
    let contributions = cell_contributions(&world.entities, &world.cells);
    let apron_cells: FxHashSet<_> = contributions[&apron].iter().collect();
    let mut approaches = FxHashMap::default();
    let mut port_count = 0;

    for entity in world.entities.values() {
        let Some(port) = entity.component::<Port>("port").unwrap() else { continue };
        let Some(machine) = port.into.as_ref().or(port.from.as_ref()) else { continue };
        if !world.entities[machine].has("go.easyloop") {
            continue;
        }
        let port_cell = w.index.cells_by_entity[&entity.id];
        let mut connected = Vec::new();
        for (from, to, edge) in w.map.graph.edges() {
            let candidate = match port.direction.as_str() {
                "sink" if to == port_cell => from,
                "source" if from == port_cell => to,
                _ => continue,
            };
            let source = &w.index.cell_entities[&candidate];
            if apron_cells.contains(source) && edge.traversability.htm_traversal == HtmTraversal::Both {
                connected.push(source);
            }
        }
        connected.sort();
        connected.dedup();
        assert_eq!(connected.len(), 1, "port {} needs one usable apron approach; got {connected:?}", entity.id);
        let port_centre = world.cells[&entity.id].centre;
        let approach = &world.cells[connected[0]];
        assert!(
            (port_centre[0].0 - approach.centre[0].0).abs() <= 1,
            "port {} and apron {} require a straight X-aligned approach: {:?} vs {:?}",
            entity.id,
            approach.id,
            port_centre,
            approach.centre
        );
        assert!(approach.centre[1] > port_centre[1], "the apron approach is north of the EasyLoop port");
        assert!(
            approaches.insert(approach.id.clone(), entity.id.clone()).is_none(),
            "two ports share apron cell {}",
            approach.id
        );
        port_count += 1;
    }
    assert_eq!(port_count, 8, "both inventory ports of every EasyLoop are checked");
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FloorRole {
    RackLane,
    OpenFloor,
    Highway,
    Other,
}

fn floor_role(world: &World, cell: &EntityId) -> FloorRole {
    // These are named fixture regions, not a simulator dispatch rule. Following
    // ownership makes the assertion independent of each region's subdivision.
    let root: EntityId = cell.root().into();
    let name = world.entities[&root].name.as_deref().unwrap_or("");
    if name.starts_with("under-rack service strip") || name.starts_with("under-rack travel lane") {
        FloorRole::RackLane
    } else if name.starts_with("open floor") {
        FloorRole::OpenFloor
    } else if name == "south highway" || name == "north highway" {
        FloorRole::Highway
    } else {
        FloorRole::Other
    }
}

#[test]
fn rack_floor_lateral_seams_and_highway_approaches_are_orthogonal() {
    let w = load();
    let world = &w.world;
    let mut lateral = 0;
    let mut highway = 0;
    let mut reached_highways: FxHashMap<EntityId, FxHashSet<EntityId>> = FxHashMap::default();
    for edge in &world.edges {
        if edge.from.root() == edge.to.root() {
            continue;
        }
        let a_role = floor_role(world, &edge.from);
        let b_role = floor_role(world, &edge.to);
        let rack_floor = |role| matches!(role, FloorRole::RackLane | FloorRole::OpenFloor);
        let axis = if rack_floor(a_role) && rack_floor(b_role) {
            lateral += 1;
            1
        } else if (rack_floor(a_role) && b_role == FloorRole::Highway)
            || (rack_floor(b_role) && a_role == FloorRole::Highway)
        {
            highway += 1;
            let (lane, cross_floor) =
                if a_role == FloorRole::Highway { (&edge.to, &edge.from) } else { (&edge.from, &edge.to) };
            reached_highways.entry(lane.root().into()).or_default().insert(cross_floor.root().into());
            0
        } else {
            continue;
        };
        let a = world.cells[&edge.from].centre;
        let b = world.cells[&edge.to].centre;
        assert!(
            (a[axis].0 - b[axis].0).abs() <= 1,
            "seam {} from {} to {} is diagonal: {a:?} -> {b:?}",
            edge.owner,
            edge.from,
            edge.to
        );
    }
    assert!(lateral > 2_000, "the site-wide service/travel/open-floor connections must exist");
    assert!(highway >= 154, "both ends of all 74 rack lanes and three open floors must connect");
    assert_eq!(reached_highways.len(), 77);
    for (region, highways) in reached_highways {
        assert_eq!(highways.len(), 2, "rack-block region {region} must reach both cross-floor bands");
    }
}

#[test]
fn aligned_floor_sections_do_not_create_overlapping_navigation_cells() {
    let w = load();
    let world = &w.world;
    let apron: EntityId = APRON.parse().unwrap();
    let mut tiles: Vec<_> = world
        .cells
        .values()
        .filter(|cell| floor_role(world, &cell.id) != FloorRole::Other || cell.id.root() == apron.root())
        .collect();
    tiles.sort_by_key(|cell| (cell.bounds().min[0].0, cell.id.clone()));
    assert!(tiles.len() > 7_000, "check the full station/rack/cross-floor footprint");
    for (index, a) in tiles.iter().enumerate() {
        let a_bounds = a.bounds();
        for b in &tiles[index + 1..] {
            let b_bounds = b.bounds();
            if b_bounds.min[0] >= a_bounds.max[0] {
                break;
            }
            // Odd-width tiles round their centres to primary steps; their
            // reconstructed edges may differ by one step (0.1 mm).
            assert!(
                a_bounds.separation(b_bounds, 0).0 >= -2 || a_bounds.separation(b_bounds, 1).0 >= -2,
                "floor cells {} and {} overlap after alignment: {:?} and {:?}",
                a.id,
                b.id,
                a_bounds,
                b_bounds
            );
        }
    }
}

#[test]
fn every_bay_gap_preserves_the_corridor_direction_without_breaking_the_route() {
    let w = load();
    let world = &w.world;
    let contributions = cell_contributions(&world.entities, &world.cells);
    let mut arcs = FxHashSet::default();
    for edge in &world.edges {
        arcs.insert((&edge.from, &edge.to));
        if edge.bidirectional {
            arcs.insert((&edge.to, &edge.from));
        }
    }
    let mut corridors = 0;
    let mut directed_corridors = 0;
    for corridor in world.declared.values() {
        let name = corridor.name.as_deref().unwrap_or("");
        if !corridor.has("region")
            || !(name.starts_with("aisle ") || name.starts_with("under-rack ") || name.starts_with("open floor "))
        {
            continue;
        }
        let flow = corridor.component::<Flow>("flow").unwrap();
        let (north, south) = match flow {
            Some(flow) if flow.rule == FlowRule::OneWay => {
                let direction = flow.direction.expect("one-way corridor declares its direction");
                assert_eq!(direction.axis(), 1, "the corridor's one-way policy follows its length");
                directed_corridors += 1;
                (direction.forward(), !direction.forward())
            }
            None => (true, true),
            Some(flow) if flow.rule == FlowRule::TwoWay => (true, true),
            Some(flow) => panic!("unexpected corridor policy: {flow:?}"),
        };
        let mut columns: FxHashMap<i64, Vec<&CellGeometry>> = FxHashMap::default();
        for id in &contributions[&corridor.id] {
            let cell = &world.cells[id];
            columns.entry(cell.centre[0].0).or_default().push(cell);
        }
        assert!(!columns.is_empty(), "corridor {} has no floor", corridor.id);
        for column in columns.values_mut() {
            column.sort_by_key(|cell| cell.centre[1]);
            assert_eq!(column.len(), 82, "21 steel bays provide the full corridor's rows");
            let mut bay_gaps = 0;
            for pair in column.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                assert_eq!(
                    arcs.contains(&(&a.id, &b.id)),
                    north,
                    "corridor {} northward connection {} -> {} does not match its policy",
                    corridor.id,
                    a.id,
                    b.id
                );
                assert_eq!(
                    arcs.contains(&(&b.id, &a.id)),
                    south,
                    "corridor {} southward connection {} -> {} does not match its policy",
                    corridor.id,
                    b.id,
                    a.id
                );
                if a.bounds().separation(b.bounds(), 1).0 > 2 {
                    bay_gaps += 1;
                }
            }
            assert_eq!(bay_gaps, 20, "the full route crosses all 20 physical gaps between 21 bay chunks");
        }
        corridors += 1;
    }
    assert_eq!(corridors, 97, "20 VTM aisles, 74 rack lanes and three open floors");
    assert_eq!(directed_corridors, 26, "all original one-way travel lanes retain their policy");
}
