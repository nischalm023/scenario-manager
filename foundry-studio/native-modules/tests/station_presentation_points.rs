//! A station's presentation points -- each cell it presents at, fed by a lane
//! of its own or by none: what the loader builds from them, and the documents
//! it refuses.
//! Dematic's pick station presents at two cells, one arm between two mirrored
//! lanes; the seeded Cencora NDC2 map states ten of them.
//!
//! The other documents are hand-built and tiny: an apron of four cells, and
//! above it two lanes of two queue cells, a presentation cell and an egress
//! cell each.

use std::path::{Path, PathBuf};

use desgraph::{CellId, CellKind, DestGate, StationType};
use desmap::{PresentationPoint, WarehouseMap};
use dessim::worldmap::{self, WorldMap, WorldMapError};
use rustc_hash::FxHashSet;
use serde_json::{Value, json};

const SITE: &str = "20000000-0000-4000-8000-000000000001";
const STATION: &str = "20000000-0000-4000-8000-000000000002";
const APRON: [&str; 4] = [
    "20000000-0000-4000-8000-000000000010",
    "20000000-0000-4000-8000-000000000011",
    "20000000-0000-4000-8000-000000000012",
    "20000000-0000-4000-8000-000000000013",
];
const WEST_QUEUE: [&str; 2] = ["20000000-0000-4000-8000-000000000020", "20000000-0000-4000-8000-000000000021"];
const WEST_PRESENTATION: &str = "20000000-0000-4000-8000-000000000022";
const WEST_EGRESS: &str = "20000000-0000-4000-8000-000000000023";
const EAST_QUEUE: [&str; 2] = ["20000000-0000-4000-8000-000000000030", "20000000-0000-4000-8000-000000000031"];
const EAST_PRESENTATION: &str = "20000000-0000-4000-8000-000000000032";
const EAST_EGRESS: &str = "20000000-0000-4000-8000-000000000033";
const WEST_LANE: &str = "20000000-0000-4000-8000-000000000040";
const EAST_LANE: &str = "20000000-0000-4000-8000-000000000041";
const FLOOR_ARCS: &str = "20000000-0000-4000-8000-000000000050";
const LANE_ARCS: &str = "20000000-0000-4000-8000-000000000051";
const SECOND_POINT: &str = "20000000-0000-4000-8000-000000000060";
const OTHER_STATION: &str = "20000000-0000-4000-8000-000000000003";
const PAD: &str = "20000000-0000-4000-8000-000000000070";
const PORT: &str = "20000000-0000-4000-8000-000000000071";
const RACK: &str = "20000000-0000-4000-8000-000000000072";

fn load(name: &str, entities: Vec<Value>) -> Result<WorldMap, WorldMapError> {
    let document = json!({
        "proto_version": "0.7.0",
        "doc": { "id": "20000000-0000-4000-8000-0000000000ff", "kind": "world", "name": name },
        "primary_frame": SITE,
        "required_components": ["frame", "presentation_point", "queue", "station", "stitch"],
        "entities": entities,
    });
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.json"));
    std::fs::write(&path, serde_json::to_vec_pretty(&document).expect("the document serializes"))
        .expect("the scratch document is writable");
    worldmap::load(&path)
}

fn refused(name: &str, entities: Vec<Value>, what: &str) -> WorldMapError {
    match load(name, entities) {
        Ok(_) => panic!("{what} loaded without a word"),
        Err(refusal) => refusal,
    }
}

fn cell(id: &str, x: i64, y: i64, extra: Value) -> Value {
    let mut components = json!({
        "pose": { "frame": SITE, "position": [x, y] },
        "cell": { "traversable_by": ["htm"], "size": [800, 800] },
    });
    for (key, value) in extra.as_object().expect("the extra components are an object") {
        components[key] = value.clone();
    }
    json!({ "id": id, "components": components })
}

fn lane(id: &str, path: [&str; 3]) -> Value {
    json!({ "id": id, "components": { "queue": { "path": path, "discipline": "no_reverse" } } })
}

/// The station, its two lanes and the apron they open onto. Each lane runs up
/// from the apron, across to its presentation cell, and down its egress back
/// to the apron; `-y` is down. An arc also joins the two presentation cells
/// each way, which the station's exit direction rules out.
fn two_lane_station() -> Vec<Value> {
    let presents = json!({ "presentation_point": { "for": "inventory_tote", "station": STATION } });
    vec![
        json!({ "id": SITE, "components": { "frame": { "dims": 2, "units": "mm", "unit_mm": 1 } } }),
        cell(APRON[0], 0, 0, json!({})),
        cell(APRON[1], 800, 0, json!({})),
        cell(APRON[2], 1600, 0, json!({})),
        cell(APRON[3], 2400, 0, json!({})),
        cell(WEST_QUEUE[0], 0, 800, json!({})),
        cell(WEST_QUEUE[1], 0, 1600, json!({})),
        cell(WEST_PRESENTATION, 800, 1600, presents.clone()),
        cell(WEST_EGRESS, 800, 800, json!({})),
        cell(EAST_QUEUE[0], 2400, 800, json!({})),
        cell(EAST_QUEUE[1], 2400, 1600, json!({})),
        cell(EAST_PRESENTATION, 1600, 1600, presents),
        cell(EAST_EGRESS, 1600, 800, json!({})),
        json!({ "id": STATION, "components": {
            "pose": { "frame": SITE, "position": [1200, 2400] },
            "station": { "kind": "pick", "exit_direction": "-y" },
        }}),
        json!({ "id": FLOOR_ARCS, "components": { "stitch": {
            "a": APRON[0], "b": APRON[3], "rule": "explicit",
            "pairs": [[APRON[0], APRON[1]], [APRON[1], APRON[2]], [APRON[2], APRON[3]]],
        }}}),
        json!({ "id": LANE_ARCS, "components": { "stitch": {
            "a": APRON[0], "b": APRON[3], "rule": "explicit", "bidirectional": false,
            "pairs": [
                [APRON[0], WEST_QUEUE[0]], [WEST_QUEUE[0], WEST_QUEUE[1]], [WEST_QUEUE[1], WEST_PRESENTATION],
                [WEST_PRESENTATION, WEST_EGRESS], [WEST_EGRESS, APRON[1]],
                [APRON[3], EAST_QUEUE[0]], [EAST_QUEUE[0], EAST_QUEUE[1]], [EAST_QUEUE[1], EAST_PRESENTATION],
                [EAST_PRESENTATION, EAST_EGRESS], [EAST_EGRESS, APRON[2]],
                [WEST_PRESENTATION, EAST_PRESENTATION], [EAST_PRESENTATION, WEST_PRESENTATION],
            ],
        }}}),
        lane(WEST_LANE, [WEST_QUEUE[0], WEST_QUEUE[1], WEST_PRESENTATION]),
        lane(EAST_LANE, [EAST_QUEUE[0], EAST_QUEUE[1], EAST_PRESENTATION]),
    ]
}

/// The same floor with one presentation point and one lane, the west's: the
/// east cells are bare floor.
fn one_lane_station() -> Vec<Value> {
    let mut entities: Vec<Value> = two_lane_station().into_iter().filter(|e| e["id"] != json!(EAST_LANE)).collect();
    for entity in &mut entities {
        if entity["id"] == json!(EAST_PRESENTATION) {
            entity["components"].as_object_mut().expect("a cell has components").remove("presentation_point");
        }
    }
    entities
}

/// The two-lane station with one more thing said about one entity.
fn two_lane_station_where(id: &str, change: impl Fn(&mut Value)) -> Vec<Value> {
    let mut entities = two_lane_station();
    entities.iter_mut().filter(|entity| entity["id"] == json!(id)).for_each(change);
    entities
}

fn cell_of(world: &WorldMap, id: &str) -> CellId {
    let found = world.index.cells_by_entity.iter().find(|(entity, _)| entity.to_string() == id);
    found.map(|(_, cell)| *cell).unwrap_or_else(|| panic!("{id} is a cell"))
}

/// Whether a robot bound for `goal` can drive from `from` to it, taking only
/// the arcs whose gate admits it.
fn reaches(map: &WarehouseMap, from: CellId, goal: CellId) -> bool {
    let mut seen = FxHashSet::default();
    let mut pending = vec![from];
    while let Some(cell) = pending.pop() {
        if cell == goal {
            return true;
        }
        if seen.insert(cell) {
            let open =
                map.graph.neighbors(cell).into_iter().filter(|(_, edge)| edge.dest_only.is_none_or(|g| g.admits(goal)));
            pending.extend(open.map(|(next, _)| next));
        }
    }
    false
}

/// What a lane into a cell a station presents at is: entered from the apron,
/// gated to that cell all the way in, so that a robot bound for it reaches it
/// and no other robot drives the lane, and left down its egress alone -- not
/// across into the other presentation cell, which an arc also leads to.
fn assert_approach(world: &WorldMap, entry: &str, queue: [&str; 2], presentation: &str, egress: &str) {
    let graph = &world.map.graph;
    let [entry, q0, q1, presentation, egress] =
        [entry, queue[0], queue[1], presentation, egress].map(|id| cell_of(world, id));
    for (from, to) in [(entry, q0), (q0, q1), (q1, presentation)] {
        let arc = graph.edge_between(from, to).unwrap_or_else(|| panic!("the lane runs {from:?} -> {to:?}"));
        assert_eq!(arc.dest_only, Some(DestGate::Cell(presentation)), "{from:?} -> {to:?} is gated to its cell");
    }
    assert!(reaches(&world.map, cell_of(world, APRON[1]), presentation), "{presentation:?} is reached from the apron");
    let out: Vec<CellId> = graph.neighbors(presentation).iter().map(|(n, _)| *n).collect();
    assert_eq!(out, vec![egress], "{presentation:?} is left down its egress alone");
}

/// The station presents at both cells, in cell order, each fed by its own
/// lane, and each lane is the one-way corridor a station's approach always is.
/// The flows deliver to the first.
#[test]
fn a_station_with_two_presentation_points_presents_at_both_through_their_lanes() {
    let world = load("two_lane_station", two_lane_station()).expect("the two-lane station loads");
    let map = &world.map;
    let station = &map.stations[0];
    let cells = |ids: [&str; 2]| ids.map(|id| cell_of(&world, id)).to_vec();
    let (west, east) = (cell_of(&world, WEST_PRESENTATION), cell_of(&world, EAST_PRESENTATION));

    assert_eq!(
        station.presentation_points,
        vec![
            PresentationPoint { presentation_cell: west, queue_path: cells(WEST_QUEUE) },
            PresentationPoint { presentation_cell: east, queue_path: cells(EAST_QUEUE) },
        ]
    );
    assert_eq!(station.delivery_cell(), west, "the flows deliver to the first cell");
    assert!(station.handoff_queues.is_empty(), "the lanes are the station's approaches, not handoff queues");

    for (presentation, queue) in [(west, cells(WEST_QUEUE)), (east, cells(EAST_QUEUE))] {
        assert_eq!(map.graph.cell(presentation).map(|c| c.kind), Some(CellKind::Presentation));
        assert_eq!(
            map.graph.cell(presentation).and_then(|c| c.station_id),
            Some(station.id),
            "the cell is the station's"
        );
        for &cell in &queue {
            assert_eq!(map.graph.cell(cell).map(|c| c.kind), Some(CellKind::QueueCell));
        }
    }
    assert_approach(&world, APRON[0], WEST_QUEUE, WEST_PRESENTATION, WEST_EGRESS);
    assert_approach(&world, APRON[3], EAST_QUEUE, EAST_PRESENTATION, EAST_EGRESS);
}

/// A station presenting at one cell states its approach as a lane, as each of
/// a two-cell station's cells does. The lane is the station's presentation
/// lane, and the cell, which has no curved arc out, is left in the station's
/// exit direction.
#[test]
fn a_station_presenting_at_one_cell_through_a_lane_presents_through_it() {
    let world = load("one_lane_station", one_lane_station()).expect("the one-lane station loads");
    let station = &world.map.stations[0];
    let queue = WEST_QUEUE.map(|id| cell_of(&world, id)).to_vec();

    assert_eq!(
        station.presentation_points,
        vec![PresentationPoint { presentation_cell: cell_of(&world, WEST_PRESENTATION), queue_path: queue }]
    );
    assert!(station.handoff_queues.is_empty(), "the lane is the station's approach, not a handoff queue");
    assert_approach(&world, APRON[0], WEST_QUEUE, WEST_PRESENTATION, WEST_EGRESS);
}

/// A point counts once per cell: counting the second point posed on the west
/// cell, the loader would give the station that cell twice.
#[test]
fn a_second_point_on_one_cell_adds_no_cell() {
    let mut entities = one_lane_station();
    entities.push(json!({ "id": SECOND_POINT, "components": {
        "pose": { "frame": SITE, "position": [800, 1600] },
        "presentation_point": { "for": "inventory_tote", "station": STATION },
    }}));
    let world = load("one_lane_station_second_point", entities).expect("the one-lane station loads");
    let cells: Vec<CellId> = world.map.stations[0].presentation_cells().collect();
    assert_eq!(cells, vec![cell_of(&world, WEST_PRESENTATION)]);
}

/// A point that names its station is believed whatever carrier it is for: the
/// east cell's point for an order carrier, naming the one-lane station, is a
/// second cell the station presents at, with no lane.
#[test]
fn a_point_for_an_order_carrier_naming_its_station_is_one_of_its_cells() {
    let mut entities = one_lane_station();
    for entity in entities.iter_mut().filter(|entity| entity["id"] == json!(EAST_PRESENTATION)) {
        entity["components"]["presentation_point"] = json!({ "for": "order_carrier", "station": STATION });
    }
    let world = load("one_lane_station_order_carriers_point", entities).expect("the one-lane station loads");
    let cells: Vec<CellId> = world.map.stations[0].presentation_cells().collect();
    assert_eq!(cells, vec![cell_of(&world, WEST_PRESENTATION), cell_of(&world, EAST_PRESENTATION)]);
}

/// What a presentation cell no lane ends at is: the station's, cut into no
/// corridor. A robot enters it by the arcs `entries` name and leaves it by the
/// arcs `exits` name -- the ones the document authored that no other lane's
/// corridor took -- and none of them is gated, so a robot bound for the cell
/// reaches it from the apron. It is labelled the station's presentation, and a
/// robot standing on it is at the station, so its wait is not counted as
/// traffic.
fn assert_lane_less(world: &WorldMap, id: &str, entries: &[&str], exits: &[&str]) {
    let graph = &world.map.graph;
    let cell = cell_of(world, id);
    let in_cell_order = |mut cells: Vec<CellId>| {
        cells.sort_by_key(|c| c.0);
        cells
    };
    let named = |ids: &[&str]| in_cell_order(ids.iter().map(|id| cell_of(world, id)).collect());
    let into =
        in_cell_order(graph.cells().map(|c| c.id).filter(|&from| graph.edge_between(from, cell).is_some()).collect());
    let out = in_cell_order(graph.neighbors(cell).iter().map(|(next, _)| *next).collect());
    assert_eq!(into, named(entries), "{id} is entered by these arcs");
    assert_eq!(out, named(exits), "{id} is left by these arcs");
    for (from, to) in into.iter().map(|&from| (from, cell)).chain(out.iter().map(|&to| (cell, to))) {
        let gate = graph.edge_between(from, to).and_then(|arc| arc.dest_only);
        assert_eq!(gate, None, "{from:?} -> {to:?} admits every robot");
    }
    assert!(reaches(&world.map, cell_of(world, APRON[1]), cell), "{id} is reached from the apron");
    let station = world.map.stations[0].id;
    assert_eq!(graph.cell(cell).map(|c| (c.kind, c.station_id)), Some((CellKind::Presentation, Some(station))));
    let exclusions = world.map.navigation_wait_exclusions();
    assert_eq!(exclusions.get(&cell.0), Some(&"station presentation / queue cell"), "{id} is the station's");
}

/// A presentation cell need not have a queue. The station below presents at
/// both cells and only the west one has a lane: the east cell is still the
/// station's, with no queue cells before it, entered from its bare queue cell
/// and left down its egress, with no gate either way. Its arc across to the
/// west cell is gone, taken by the west lane's corridor, which is the corridor
/// it always is. The west cell has the lower cell id, so the flows deliver there.
#[test]
fn a_presentation_cell_with_no_lane_has_no_queue() {
    let entities: Vec<Value> = two_lane_station().into_iter().filter(|e| e["id"] != json!(EAST_LANE)).collect();
    let world = load("two_lane_station_one_lane", entities).expect("a cell with no lane loads");
    let station = &world.map.stations[0];
    let (west, east) = (cell_of(&world, WEST_PRESENTATION), cell_of(&world, EAST_PRESENTATION));
    assert_eq!(
        station.presentation_points,
        vec![
            PresentationPoint {
                presentation_cell: west,
                queue_path: WEST_QUEUE.map(|id| cell_of(&world, id)).to_vec()
            },
            PresentationPoint { presentation_cell: east, queue_path: Vec::new() },
        ]
    );
    assert_eq!(station.delivery_cell(), west);
    let graph = &world.map.graph;
    let exclusions = world.map.navigation_wait_exclusions();
    for id in EAST_QUEUE {
        let cell = cell_of(&world, id);
        assert_eq!(graph.cell(cell).map(|c| c.kind), Some(CellKind::Open), "no lane, no queue cells");
        assert_eq!(exclusions.get(&cell.0), None, "{id} is floor, where a wait is traffic");
    }
    assert_lane_less(&world, EAST_PRESENTATION, &[EAST_QUEUE[1]], &[EAST_EGRESS]);
    assert_approach(&world, APRON[0], WEST_QUEUE, WEST_PRESENTATION, WEST_EGRESS);
}

/// The same station with the lane on the other side: now the cell with no lane
/// has the lower cell id, so it is the one the flows deliver to, and robots
/// bound for the station drive to a bare cell while its laned cell waits for a
/// policy that chooses between the two.
#[test]
fn a_presentation_cell_with_no_lane_and_the_lower_cell_id_is_where_the_flows_deliver() {
    let entities: Vec<Value> = two_lane_station().into_iter().filter(|e| e["id"] != json!(WEST_LANE)).collect();
    let world = load("two_lane_station_east_lane", entities).expect("a cell with no lane loads");
    let station = &world.map.stations[0];
    let (west, east) = (cell_of(&world, WEST_PRESENTATION), cell_of(&world, EAST_PRESENTATION));
    assert!(west.0 < east.0, "the bare west cell sorts first");
    assert_eq!(station.delivery_cell(), west, "the flows deliver to the cell with no lane");
    assert_eq!(station.presentation_points[0], PresentationPoint { presentation_cell: west, queue_path: Vec::new() });
    assert_lane_less(&world, WEST_PRESENTATION, &[WEST_QUEUE[1]], &[WEST_EGRESS]);
    assert_approach(&world, APRON[3], EAST_QUEUE, EAST_PRESENTATION, EAST_EGRESS);
}

/// A station presenting at one cell need not have a queue either. With no lane
/// anywhere there is no corridor, so the cell keeps every arc the document
/// drew: in from its bare queue cell and from the east cell, out down its
/// egress and across to the east cell, none gated. The station's exit
/// direction picks an exit only where a corridor is cut.
#[test]
fn a_one_cell_station_with_no_lane_keeps_its_cells_authored_arcs() {
    let entities: Vec<Value> = one_lane_station().into_iter().filter(|e| e["id"] != json!(WEST_LANE)).collect();
    let world = load("one_cell_station_no_lane", entities).expect("a station with no lane loads");
    let station = &world.map.stations[0];
    let west = cell_of(&world, WEST_PRESENTATION);
    assert_eq!(
        station.presentation_points,
        vec![PresentationPoint { presentation_cell: west, queue_path: Vec::new() }]
    );
    assert_eq!(station.delivery_cell(), west);
    assert_lane_less(&world, WEST_PRESENTATION, &[WEST_QUEUE[1], EAST_PRESENTATION], &[WEST_EGRESS, EAST_PRESENTATION]);
}

/// The west lane's queue, for stating on something other than a lane.
fn west_queue() -> Value {
    json!({ "path": [WEST_QUEUE[0], WEST_QUEUE[1], WEST_PRESENTATION], "discipline": "no_reverse" })
}

/// The one-lane station without its lane, with `more` records.
fn without_west_lane(more: Vec<Value>) -> Vec<Value> {
    one_lane_station().into_iter().filter(|e| e["id"] != json!(WEST_LANE)).chain(more).collect()
}

/// The one-lane station with its lane's queue stated on its entity `host`
/// instead of on the lane.
fn west_queue_on(host: &str, more: Vec<Value>) -> Vec<Value> {
    let mut entities = without_west_lane(more);
    let hosts = entities.iter_mut().filter(|entity| entity["id"] == json!(host));
    assert_eq!(hosts.map(|entity| entity["components"]["queue"] = west_queue()).count(), 1, "{host} is stated once");
    entities
}

/// What the loader names in refusing a queue that is not on an entity of its
/// own: the entity, which the message names too, and what else it is.
fn queue_refusal(name: &str, entities: Vec<Value>) -> (String, &'static str) {
    let refusal = refused(name, entities, "a queue on an entity that is something else too");
    let WorldMapError::QueueNotOnItsOwnEntity { entity, also } = &refusal else {
        panic!("the loader names the entity and what else it is: {refusal}");
    };
    assert!(refusal.to_string().contains(&entity.to_string()), "the message names the entity: {refusal}");
    (entity.to_string(), *also)
}

/// A queue is a lane, an entity of its own, so a station carrying one is
/// refused, naming the station, whatever its queue would have meant: here the
/// one-lane station's lane stated on the station.
#[test]
fn a_station_carrying_a_queue_is_refused() {
    let named = queue_refusal("queue_on_station", west_queue_on(STATION, vec![]));
    assert_eq!(named, (STATION.to_string(), "a station"));
}

/// The same for the presentation point the lane fed, even though its path is
/// exactly the lane it replaces. The point is a cell too, and is named as the
/// point it is.
#[test]
fn a_presentation_point_carrying_a_queue_is_refused() {
    let named = queue_refusal("queue_on_point", west_queue_on(WEST_PRESENTATION, vec![]));
    assert_eq!(named, (WEST_PRESENTATION.to_string(), "a presentation point"));
}

/// The same for a cell, declared or derived: here the west egress, and the one
/// cell a tiling derives, through a record overriding it with the queue.
#[test]
fn a_cell_carrying_a_queue_is_refused_declared_or_derived() {
    let named = queue_refusal("queue_on_declared_cell", west_queue_on(WEST_EGRESS, vec![]));
    assert_eq!(named, (WEST_EGRESS.to_string(), "a cell"));

    let pad = json!({ "id": PAD, "components": {
        "region": { "frame": SITE, "origin": [3200, 0], "size": [800, 800] },
        "tiling": { "pitch": [800, 800], "derives": { "cell": { "traversable_by": ["htm"] } } },
    }});
    let queued =
        json!({ "derivation": { "generator": PAD, "key": "cell:0,0" }, "components": { "queue": west_queue() } });
    let named = queue_refusal("queue_on_derived_cell", without_west_lane(vec![pad, queued]));
    assert_eq!(named, (format!("{PAD}/cell:0,0"), "a cell"));
}

/// The same for a port, which is a cell too, and is named as the port it is.
#[test]
fn a_port_carrying_a_queue_is_refused() {
    let port = cell(PORT, 3200, 0, json!({ "port": { "direction": "sink" } }));
    let named = queue_refusal("queue_on_port", west_queue_on(PORT, vec![port]));
    assert_eq!(named, (PORT.to_string(), "a port"));
}

/// The same for a relay: the one shelf of a bay beside the apron, through a
/// record overriding it with the queue.
#[test]
fn a_relay_carrying_a_queue_is_refused() {
    let rack = json!({ "id": RACK, "components": {
        "pose": { "frame": SITE, "position": [0, -1200] },
        "rack": { "slots_wide": 1, "slots_deep": 1, "footprint": [800, 800],
                  "levels": [{ "height": 0, "positions": [[0, 800]], "reach": 600 }] },
    }});
    let queued =
        json!({ "derivation": { "generator": RACK, "key": "relay:0,0" }, "components": { "queue": west_queue() } });
    let named = queue_refusal("queue_on_relay", without_west_lane(vec![rack, queued]));
    assert_eq!(named, (format!("{RACK}/relay:0,0"), "a relay"));
}

/// A lane whose steps run over a cell a station presents at. Its corridor
/// gates every arc onto its steps to the robots bound for its own end, so a
/// robot bound for that cell could never reach it -- and here the cell is the
/// station's lower one, the one the flows deliver to. The one-lane station
/// with a second point, naming the station, on the west lane's last queue cell.
#[test]
fn a_lane_over_another_of_its_stations_cells_is_refused() {
    let mut entities = one_lane_station();
    for step in entities.iter_mut().filter(|entity| entity["id"] == json!(WEST_QUEUE[1])) {
        step["components"]["presentation_point"] = json!({ "for": "inventory_tote", "station": STATION });
    }
    let refusal = refused("one_lane_station_point_on_its_lane", entities, "a lane over its station's cell");
    let WorldMapError::LaneRunsThroughPresentation { lane, station, cell } = &refusal else {
        panic!("the loader names the lane, the station and the cell: {refusal}");
    };
    let intact = load("one_lane_station_cells", one_lane_station()).expect("the one-lane station loads");
    assert_eq!((lane.to_string(), station.to_string()), (WEST_LANE.to_string(), STATION.to_string()));
    assert_eq!(*cell, cell_of(&intact, WEST_QUEUE[1]));
}

/// The same for a cell another station presents at: the lane's corridor would
/// close it to that station's robots just the same.
#[test]
fn a_lane_over_a_cell_another_station_presents_at_is_refused() {
    let mut entities = one_lane_station();
    for step in entities.iter_mut().filter(|entity| entity["id"] == json!(WEST_QUEUE[1])) {
        step["components"]["presentation_point"] = json!({ "for": "inventory_tote", "station": OTHER_STATION });
    }
    entities.push(json!({ "id": OTHER_STATION, "components": {
        "pose": { "frame": SITE, "position": [-800, 1600] },
        "station": { "kind": "put" },
    }}));
    let refusal = refused("one_lane_station_other_point_on_its_lane", entities, "a lane over another station's cell");
    let WorldMapError::LaneRunsThroughPresentation { lane, station, cell } = &refusal else {
        panic!("the loader names the lane, the station and the cell: {refusal}");
    };
    let intact = load("one_lane_station_cells_again", one_lane_station()).expect("the one-lane station loads");
    assert_eq!((lane.to_string(), station.to_string()), (WEST_LANE.to_string(), OTHER_STATION.to_string()));
    assert_eq!(*cell, cell_of(&intact, WEST_QUEUE[1]));
}

/// The second lane's cell, left with no arc out, is refused as the first's
/// would be.
#[test]
fn a_second_lanes_cell_with_no_way_out_is_refused() {
    let entities = two_lane_station_where(LANE_ARCS, |arcs| {
        let pairs = arcs["components"]["stitch"]["pairs"].as_array_mut().expect("the arcs are a list");
        pairs.retain(|pair| pair[0] != json!(EAST_PRESENTATION));
    });
    let refusal = refused("two_lane_station_east_sealed", entities, "a presentation cell with no way out");
    let WorldMapError::StationCellIsolated { station, cell } = &refusal else {
        panic!("the loader names the station and the cell: {refusal}");
    };
    let intact = load("two_lane_station_east_sealed_cells", two_lane_station()).expect("the two-lane station loads");
    assert_eq!((*station, *cell), (intact.map.stations[0].id, cell_of(&intact, EAST_PRESENTATION)));
}

/// A robot waiting on either lane is queueing for the station, not held up by
/// traffic.
#[test]
fn both_lanes_are_left_out_of_navigation_waits() {
    let world = load("two_lane_station_waits", two_lane_station()).expect("the two-lane station loads");
    let exclusions = world.map.navigation_wait_exclusions();
    for cell in world.map.stations[0].presentation_points.iter().flat_map(|point| point.cells()) {
        assert_eq!(exclusions.get(&cell.0), Some(&"station presentation / queue cell"), "{cell:?}");
    }
}

/// A station that stands on a cell of one of its lanes -- here the second
/// lane's queue -- leaves that cell a queue cell rather than marking it the
/// station's standing area.
#[test]
fn a_station_standing_on_its_second_lane_leaves_that_cell_a_queue_cell() {
    let entities = two_lane_station_where(STATION, |station| {
        station["components"]["pose"]["position"] = json!([2400, 1700]);
    });
    let world = load("two_lane_station_on_its_lane", entities).expect("the two-lane station loads");
    let stand = cell_of(&world, EAST_QUEUE[1]);
    assert_eq!(world.map.stations[0].position, stand, "the station stands on the east lane's last queue cell");
    assert_eq!(world.map.graph.cell(stand).map(|c| c.kind), Some(CellKind::QueueCell));
}

/// The change this file exists for: the seeded Cencora NDC2 map states each of
/// its ten pick stations as Dematic draws it, and each presents at two cells,
/// each through a lane of its own gated to that cell.
#[test]
fn the_seeded_cencora_map_presents_each_pick_station_at_two_cells_through_their_lanes() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../data/seed/customer/cencora/ndc2/map.world.json");
    let world = worldmap::load(&path).expect("the seeded Cencora map loads");
    let picks: Vec<_> = world.map.stations.iter().filter(|s| s.station_type == StationType::Pick).collect();
    assert_eq!(picks.len(), 10);
    for station in picks {
        let [first, second] = station.presentation_points.as_slice() else {
            panic!("pick station {:?} presents at {} cells", station.id, station.presentation_points.len());
        };
        assert_ne!(first.presentation_cell, second.presentation_cell);
        for lane in [first, second] {
            let last = *lane.queue_path.last().expect("a lane feeds each cell");
            let into = world.map.graph.edge_between(last, lane.presentation_cell).expect("the lane feeds its cell");
            assert_eq!(into.dest_only, Some(DestGate::Cell(lane.presentation_cell)));
        }
    }
}
