//! What the loader refuses when a document's LANES do not say unambiguously
//! which place each one serves.
//!
//! A lane is a `queue` on an entity that is neither a station nor a generator
//! of the floor it runs over. It BELONGS to the machine whose cells it runs
//! over, and within that owner it binds to the place its path TERMINATES at.
//! That rule is only a rule if the loader refuses everything it cannot resolve:
//! the older reading read the machine off the terminus instead of off the
//! cells, so a lane over one machine's floor ending at an ordinary station came
//! out owned by nobody; it dropped an unclaimed lane on the floor, let another
//! machine claim a mistyped terminus, installed two corridors where two lanes
//! ended at one cell while the collect staging map kept one of them, and let a
//! bare position put a nearest-cell search on the binding rule itself.
//!
//! Every document here is hand-built and tiny -- one machine, five or six cells,
//! one lane -- so what each test states is one defect and not a fixture. The
//! first test is the POSITIVE control: the same document with nothing wrong with
//! it loads, so a refusal below means the defect and not the scaffolding.
//!
//! The last test is not a refusal at all. Binding has two ends -- the place a
//! lane serves, which is where it ENDS, and the cell it is entered from, which
//! is the one arc into its head -- and the second is only resolvable from the
//! graph. It belongs beside the refusals because it is the same question asked
//! of a document the site fixtures do not contain: one whose lane is entered
//! somewhere other than the machine's dropoff.

use std::path::PathBuf;

use desgraph::CellId;
use dessim::worldmap::{self, WorldMap, WorldMapError};
use native_modules::migration::flows::ttp::demand::loop_collect::approach_to_collection;
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// the scaffolding
// ---------------------------------------------------------------------------

const SITE: &str = "10000000-0000-4000-8000-000000000001";
const MACHINE: &str = "10000000-0000-4000-8000-000000000002";
const DROPOFF: &str = "10000000-0000-4000-8000-000000000003";
const STEP: &str = "10000000-0000-4000-8000-000000000004";
const COLLECT: &str = "10000000-0000-4000-8000-000000000005";
const SEAT: &str = "10000000-0000-4000-8000-000000000006";
const LANE: &str = "10000000-0000-4000-8000-000000000007";
const OUTSIDE: &str = "10000000-0000-4000-8000-000000000008";
const SECOND_LANE: &str = "10000000-0000-4000-8000-000000000009";
const OTHER_MACHINE: &str = "10000000-0000-4000-8000-00000000000a";
const OTHER_SEAT: &str = "10000000-0000-4000-8000-00000000000b";
const ARCS: &str = "10000000-0000-4000-8000-00000000000c";
const SECOND_SEAT: &str = "10000000-0000-4000-8000-00000000000e";
const FLOOR_ARCS: &str = "10000000-0000-4000-8000-00000000000f";

/// Write a document out and read it back the way any consumer would.
///
/// `WorldMap` carries the loaded world and has no `Debug`, so what comes back
/// on the happy path is the map itself and what comes back on a refusal is the
/// refusal -- a test that wants both never needs both at once.
fn load(name: &str, entities: Vec<Value>) -> Result<WorldMap, WorldMapError> {
    let document = json!({
        "proto_version": "0.7.0",
        "doc": { "id": "10000000-0000-4000-8000-0000000000ff", "kind": "world", "name": name },
        "primary_frame": SITE,
        "required_components": ["frame", "port", "presentation_point", "queue", "region", "station", "stitch"],
        "entities": entities,
    });
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.json"));
    std::fs::write(&path, serde_json::to_vec_pretty(&document).expect("the document serializes"))
        .expect("the scratch document is writable");
    worldmap::load(&path)
}

/// The refusal a load came back with, or a failure naming what should have been
/// refused.
fn refused(loaded: Result<WorldMap, WorldMapError>, what: &str) -> WorldMapError {
    match loaded {
        Ok(_) => panic!("{what} loaded without a word"),
        Err(refusal) => refusal,
    }
}

/// The compiled cell a loaded document gives the entity `id`.
fn cell_of(map: &WorldMap, id: &str) -> CellId {
    let found = map.index.cells_by_entity.iter().find(|(entity, _)| entity.to_string() == id);
    found.map(|(_, cell)| *cell).unwrap_or_else(|| panic!("{id} is a cell of the loaded map"))
}

/// One cell of the machine, posed in the machine's own frame.
fn cell(id: &str, frame: &str, x: i64, y: i64, extra: Value) -> Value {
    let mut components = json!({
        "pose": { "frame": frame, "position": [x, y] },
        "cell": { "traversable_by": ["htm"], "size": [700, 700] },
    });
    for (key, value) in extra.as_object().expect("the extra components are an object") {
        components[key] = value.clone();
    }
    json!({ "id": id, "components": components })
}

/// The site frame, and one machine posed in it: a frame of its own, the belt
/// timing, and the envelope that says how much floor it stands on.
fn machine_frames(machine: &str, x: i64) -> Vec<Value> {
    vec![
        json!({ "id": SITE, "components": { "frame": { "dims": 2, "units": "mm", "unit_mm": 1 } } }),
        json!({ "id": machine, "components": {
            "frame": { "dims": 2, "units": "mm", "parent": SITE,
                       "transform_to_parent": { "translation": [x, 0], "scale": [1, 1] } },
            "region": { "frame": machine, "origin": [-500, -3000], "size": [3000, 4000] },
            "go.easyloop": { "part": "envelope", "loop_transit_secs": 45 },
        }}),
    ]
}

/// One machine, its two belt ends, the cell between them, and the seat an
/// operator works: everything a lane can legitimately end at.
fn machine_cells(machine: &str, dropoff: &str, step: &str, collect: &str, seat: &str) -> Vec<Value> {
    vec![
        cell(dropoff, machine, 0, 0, json!({ "port": { "direction": "sink", "of": [], "into": machine } })),
        cell(step, machine, 0, -800, json!({})),
        cell(collect, machine, 800, 0, json!({ "port": { "direction": "source", "of": [], "from": machine } })),
        // `pick` is the happening that makes a cell a pick place. The reader
        // does not know the name yet, so a document carries it rather than
        // listing it in `required_components`.
        cell(seat, machine, 1600, -1600, json!({ "presentation_point": { "for": "order_carrier" }, "pick": {} })),
    ]
}

/// The machine's inventory lane: onto the dropoff, across, off at the
/// collection cell, which is the place it serves.
fn inventory_lane(id: &str, path: Vec<&str>) -> Value {
    json!({ "id": id, "components": { "queue": { "path": path, "discipline": "no_reverse" } } })
}

/// Every arc of the little machine, named pair by pair. A cell with no arc is
/// scenery -- the loader refuses a station standing on one -- so the positive
/// control needs these even though no test is about them.
fn machine_arcs() -> Value {
    json!({ "id": ARCS, "components": { "stitch": {
        "a": DROPOFF, "b": COLLECT, "rule": "explicit",
        "pairs": [[DROPOFF, STEP], [STEP, COLLECT], [COLLECT, SEAT], [SEAT, DROPOFF]],
    }}})
}

/// Arcs between cells that are not a machine's, named pair by pair. Same
/// purpose as `machine_arcs`: a cell with no arc is scenery, and a document
/// whose lane runs over ordinary floor still needs the floor joined up.
fn floor_arcs(pairs: &[[&str; 2]]) -> Value {
    json!({ "id": FLOOR_ARCS, "components": { "stitch": {
        "a": pairs[0][0], "b": pairs[0][1], "rule": "explicit", "pairs": pairs,
    }}})
}

fn one_machine() -> Vec<Value> {
    let mut entities = machine_frames(MACHINE, 0);
    entities.extend(machine_cells(MACHINE, DROPOFF, STEP, COLLECT, SEAT));
    entities.push(machine_arcs());
    entities
}

// ---------------------------------------------------------------------------
// the positive control
// ---------------------------------------------------------------------------

/// The same shape every test below spoils: a lane that runs over one machine's
/// cells and ends at that machine's collection cell binds, and the document
/// loads.
#[test]
fn a_lane_over_one_machines_cells_binds_to_the_cell_it_ends_at() {
    let mut entities = one_machine();
    entities.push(inventory_lane(LANE, vec![DROPOFF, STEP, COLLECT]));
    let map = load("lane_binds", entities).expect("the document loads");
    let served: Vec<_> = map
        .map
        .stations
        .iter()
        .flat_map(|s| s.handoff_queues.iter())
        .map(|q| (q.into, q.path.len(), q.approach))
        .collect();
    assert!(
        served.iter().any(|(_, steps, approach)| *steps == 2 && !*approach),
        "the machine's lane is a two-step through route, not an approach: {served:?}"
    );
}

// ---------------------------------------------------------------------------
// the refusals
// ---------------------------------------------------------------------------

/// A machine's lane ending on that machine's ordinary floor serves nothing: the
/// step between the belt's two ends is somewhere to drive over, not somewhere a
/// carrier changes hands. It used to be dropped without a word, so the document
/// loaded and the corridor the author drew was simply not there.
#[test]
fn a_lane_ending_on_its_machines_ordinary_floor_is_refused() {
    let mut entities = one_machine();
    entities.push(inventory_lane(LANE, vec![DROPOFF, COLLECT, STEP]));
    let refusal = refused(load("lane_unclaimed", entities), "a lane serving nothing");
    assert!(
        matches!(refusal, WorldMapError::LaneEndsOutsideItsMachine { .. }),
        "the loader names the lane, its machine and the cell: {refusal}"
    );
}

/// A lane over ordinary floor that ends where nothing presents. No machine owns
/// it, so the only place it could serve is a cell a station presents at, and
/// this one is bare floor.
#[test]
fn a_lane_over_floor_ending_at_a_cell_nothing_presents_at_is_refused() {
    let entities = vec![
        json!({ "id": SITE, "components": { "frame": { "dims": 2, "units": "mm", "unit_mm": 1 } } }),
        cell(OUTSIDE, SITE, 0, 1600, json!({})),
        cell(STEP, SITE, 0, 800, json!({})),
        cell(DROPOFF, SITE, 0, 0, json!({})),
        floor_arcs(&[[OUTSIDE, STEP], [STEP, DROPOFF]]),
        inventory_lane(LANE, vec![OUTSIDE, STEP, DROPOFF]),
    ];
    let refusal = refused(load("lane_floor_unclaimed", entities), "a lane over floor serving nothing");
    assert!(
        matches!(refusal, WorldMapError::LaneEndsAtUnclaimedCell { .. }),
        "the loader names the lane and the cell: {refusal}"
    );
}

/// The lane whose OWNER and whose TERMINUS disagreed. It runs over machine A's
/// cells, so it is A's lane, and it ends at an ordinary station's presentation
/// cell. Reading the machine off the terminus made it a lane nobody owned, the
/// ordinary station adopted it as its approach, and every robot driving to that
/// station was routed across A's floor.
#[test]
fn a_lane_over_a_machines_cells_may_not_end_at_an_ordinary_stations_presentation() {
    let mut entities = one_machine();
    entities.push(cell(OUTSIDE, SITE, 12_000, 0, json!({ "presentation_point": {} })));
    entities.push(json!({ "id": SECOND_SEAT, "components": {
        "pose": { "frame": SITE, "position": [12_000, 0] },
        "station": { "kind": "put" },
    }}));
    entities.push(inventory_lane(LANE, vec![DROPOFF, STEP, OUTSIDE]));
    let refusal = refused(load("lane_leaves_machine", entities), "a machine's lane ending outside it");
    let WorldMapError::LaneEndsOutsideItsMachine { lane, machine, .. } = &refusal else {
        panic!("the loader names the lane, its machine and the cell: {refusal}");
    };
    assert_eq!(lane.to_string(), LANE, "the lane named is the one that was declared");
    assert_eq!(machine.to_string(), MACHINE, "the machine named is the one whose cells the lane runs over");
}

/// A lane whose terminus carries a `presentation_point` that no station selects.
/// `lanes` admits the terminus on the strength of the point alone -- which
/// station presents there is not known until the stations are resolved -- and
/// the lane then binds to nothing and disappears.
#[test]
fn a_lane_ending_at_a_presentation_point_no_station_selects_is_refused() {
    let entities = vec![
        json!({ "id": SITE, "components": { "frame": { "dims": 2, "units": "mm", "unit_mm": 1 } } }),
        // The point the station is bound to by name, and the point nothing
        // selects: the lane ends at the second.
        cell(SEAT, SITE, 0, 0, json!({ "presentation_point": { "station": SECOND_SEAT } })),
        cell(DROPOFF, SITE, 4000, 0, json!({ "presentation_point": {} })),
        cell(STEP, SITE, 4000, 800, json!({})),
        cell(OUTSIDE, SITE, 4000, 1600, json!({})),
        floor_arcs(&[[OUTSIDE, STEP], [STEP, DROPOFF], [SEAT, STEP]]),
        json!({ "id": SECOND_SEAT, "components": {
            "pose": { "frame": SITE, "position": [0, 0] },
            "station": { "kind": "pick" },
        }}),
        inventory_lane(LANE, vec![OUTSIDE, STEP, DROPOFF]),
    ];
    let refusal = refused(load("lane_unselected_point", entities), "a lane into a point no station presents at");
    assert!(
        matches!(refusal, WorldMapError::LaneServesNoStation { .. }),
        "the loader names the lane and the cell: {refusal}"
    );
}

/// Two stations presenting at ONE cell both adopt the lane that ends there, so
/// its corridor is installed twice under two owners and nothing says which
/// station the document meant.
#[test]
fn a_lane_ending_where_two_stations_present_is_refused() {
    let entities = vec![
        json!({ "id": SITE, "components": { "frame": { "dims": 2, "units": "mm", "unit_mm": 1 } } }),
        cell(DROPOFF, SITE, 0, 0, json!({ "presentation_point": {} })),
        cell(STEP, SITE, 0, 800, json!({})),
        cell(OUTSIDE, SITE, 0, 1600, json!({})),
        floor_arcs(&[[OUTSIDE, STEP], [STEP, DROPOFF]]),
        json!({ "id": SEAT, "components": {
            "pose": { "frame": SITE, "position": [0, 0] },
            "station": { "kind": "pick" },
        }}),
        json!({ "id": SECOND_SEAT, "components": {
            "pose": { "frame": SITE, "position": [0, 100] },
            "station": { "kind": "put" },
        }}),
        inventory_lane(LANE, vec![OUTSIDE, STEP, DROPOFF]),
    ];
    let refusal = refused(load("lane_two_stations", entities), "a lane two stations present behind");
    assert!(
        matches!(refusal, WorldMapError::LaneServesTwoStations { .. }),
        "the loader names the lane, both stations and the cell: {refusal}"
    );
}

/// Machine A's lane, mistyped to end at machine B's seat. B used to claim it —
/// moving the approach onto the wrong machine and, because the lane still ran
/// over A's port, telling A not to infer an approach for its own handover.
#[test]
fn a_lane_that_crosses_from_one_machine_into_another_is_refused() {
    let mut entities = one_machine();
    entities.extend(machine_frames(OTHER_MACHINE, 10_000).into_iter().skip(1));
    entities.push(cell(
        OTHER_SEAT,
        OTHER_MACHINE,
        1600,
        -1600,
        json!({ "presentation_point": { "for": "order_carrier" }, "pick": {} }),
    ));
    entities.push(inventory_lane(LANE, vec![DROPOFF, STEP, OTHER_SEAT]));
    let refusal = refused(load("lane_crosses", entities), "a lane across two machines");
    assert!(
        matches!(refusal, WorldMapError::LaneCrossesMachines { .. }),
        "the loader names the lane and both machines: {refusal}"
    );
}

/// Two lanes ending at one collection cell used to install two corridors in the
/// graph while the collect staging map kept whichever the scan reached last, so
/// routing and reservations disagreed about which cells a robot queues on.
#[test]
fn two_lanes_ending_at_one_cell_are_refused() {
    let mut entities = one_machine();
    entities.push(inventory_lane(LANE, vec![DROPOFF, STEP, COLLECT]));
    entities.push(inventory_lane(SECOND_LANE, vec![SEAT, STEP, COLLECT]));
    let refusal = refused(load("lane_twice", entities), "two lanes into one cell");
    assert!(
        matches!(refusal, WorldMapError::LaneTerminusClaimedTwice { .. }),
        "the loader names both lanes and the cell: {refusal}"
    );
}

/// A lane of one cell declares no approach at all — and still told the loader
/// not to infer one for the handover it named, so the handover was left with no
/// way in.
#[test]
fn a_lane_that_states_fewer_than_two_cells_is_refused() {
    for (name, path) in [("lane_empty", vec![]), ("lane_one_step", vec![COLLECT])] {
        let mut entities = one_machine();
        entities.push(inventory_lane(LANE, path));
        let refusal = refused(load(name, entities), "a lane with no approach");
        assert!(
            matches!(refusal, WorldMapError::LaneWithoutPath { .. }),
            "the loader names the lane and its step count: {refusal}"
        );
    }
}

/// A lane names its cells by identity. A step written as a bare position binds
/// to whichever way cell lies nearest, and on the LAST step that is the binding
/// rule itself: a terminus mistyped by a metre would quietly serve another
/// machine's port.
#[test]
fn a_lane_step_written_as_a_position_is_refused() {
    let mut entities = one_machine();
    entities.push(json!({ "id": LANE, "components": {
        "queue": { "path": [DROPOFF, [0, -800], COLLECT], "discipline": "no_reverse" },
    }}));
    let refusal = refused(load("lane_position", entities), "a lane step written as a position");
    assert!(
        matches!(refusal, WorldMapError::LaneStepIsAPosition { .. }),
        "the loader names the lane and the position: {refusal}"
    );
}

/// Two lanes of one machine that share an arc and admit different robots. The
/// inventory lane is a THROUGH ROUTE, whose forward chain is open because it is
/// also the way out; the lane into the seat is an APPROACH, whose every forward
/// edge is gated to the one robot that claimed the seat. Both run
/// `dropoff -> step`, and the older reading let whichever corridor was written
/// last decide what that arc admits -- an ungated claim silently erasing a
/// gated one.
#[test]
fn two_lanes_claiming_one_arc_with_different_gates_are_refused() {
    let mut entities = one_machine();
    entities.push(inventory_lane(LANE, vec![DROPOFF, STEP, COLLECT]));
    entities.push(inventory_lane(SECOND_LANE, vec![DROPOFF, STEP, SEAT]));
    let refusal = refused(load("lane_arcs_conflict", entities), "two lanes disagreeing about one arc");
    assert!(
        matches!(refusal, WorldMapError::LaneArcsConflict { .. }),
        "the loader names the arc the two lanes disagree about: {refusal}"
    );
}

/// A lane into a port, at a station that presents on that port and has no belt
/// of its own: the port's `into` names the site rather than the station, so the
/// station gets the presentation and none of the belt. A lane ending at a port
/// is a belt's through route, gated to the cells the belt hands carriers over
/// at, and this station states none. The refusal names the lane, the station
/// and the port's cell.
#[test]
fn a_lane_into_a_port_a_station_without_a_belt_presents_at_is_refused() {
    let entities = |with_lane: bool| {
        let mut entities = vec![
            json!({ "id": SITE, "components": { "frame": { "dims": 2, "units": "mm", "unit_mm": 1 } } }),
            cell(
                DROPOFF,
                SITE,
                0,
                0,
                json!({
                    "presentation_point": {},
                    "port": { "direction": "sink", "of": [], "into": SITE },
                }),
            ),
            cell(STEP, SITE, 0, 800, json!({})),
            cell(OUTSIDE, SITE, 0, 1600, json!({})),
            floor_arcs(&[[OUTSIDE, STEP], [STEP, DROPOFF]]),
            json!({ "id": SEAT, "components": {
                "pose": { "frame": SITE, "position": [0, 0] },
                "station": { "kind": "pick" },
            }}),
        ];
        if with_lane {
            entities.push(inventory_lane(LANE, vec![OUTSIDE, STEP, DROPOFF]));
        }
        entities
    };
    let refusal = refused(load("lane_no_handover", entities(true)), "a through route with no handover to admit");
    let WorldMapError::LaneEndsAtPortWithoutBelt { lane, station, cell } = &refusal else {
        panic!("the loader names the lane, the station and the port's cell: {refusal}");
    };
    // The same floor without the lane loads, and says which cell the port is.
    let floor = load("lane_no_handover_floor", entities(false)).expect("the station presents at the port, laneless");
    assert_eq!((lane.to_string(), station.to_string()), (LANE.to_string(), SEAT.to_string()));
    assert_eq!(*cell, cell_of(&floor, DROPOFF), "the port's cell: {refusal}");
}

// ---------------------------------------------------------------------------
// what a lane resolves to at its OTHER end
// ---------------------------------------------------------------------------

/// A lane carries the cell it is ENTERED from, and that cell is the lane's own
/// -- not whatever handover the station happens to name.
///
/// The document here is the awkward one every EasyLoop in the tree is not: its
/// collection lane's head is `STEP`, and the one arc into `STEP` comes off
/// `OUTSIDE`, a floor cell west of the machine. The station's `dropoff_cell` is
/// `DROPOFF`, which stands on the other side of the lane's head and which no
/// robot bound for the collection cell drives through at all.
///
/// `LoopCollect` skips a belt whose whole way in is held open-ended, because a
/// search that has to cross a held cell cannot succeed at any horizon
/// (`loop_collect.rs`, and the 614 failing five-million-iteration searches it
/// was measured at). "The whole way in" was the lane's steps plus the STATION'S
/// dropoff, which on this document reads the wrong two cells both ways round:
/// a hold on `OUTSIDE` -- the cell that really is in the way -- would go unseen
/// and the search would run and fail, and a hold on `DROPOFF` would skip a belt
/// whose way in is clear. It is the LANE's entry cell now, so this document
/// reads `[STEP, OUTSIDE]`.
///
/// Nothing about DFW6 or the minimal map moves: on both, the arc into the
/// lane's head comes off the dropoff and the two answers are the same cell
/// (`dfw6_easyloop_floor.rs`,
/// `each_machines_lane_is_entered_at_its_own_dropoff`).
#[test]
fn a_lane_entered_from_a_cell_that_is_not_the_dropoff_carries_that_cell() {
    let mut entities = machine_frames(MACHINE, 0);
    entities.extend(vec![
        cell(DROPOFF, MACHINE, 0, 0, json!({ "port": { "direction": "sink", "of": [], "into": MACHINE } })),
        cell(STEP, MACHINE, 800, 0, json!({})),
        cell(COLLECT, MACHINE, 1600, 0, json!({ "port": { "direction": "source", "of": [], "from": MACHINE } })),
        cell(SEAT, MACHINE, 1600, -1600, json!({ "presentation_point": { "for": "order_carrier" }, "pick": {} })),
        // West of the machine's envelope, so it is ordinary floor: the lane is
        // entered from OUTSIDE the machine and the dropoff is off to one side.
        cell(OUTSIDE, MACHINE, -800, 0, json!({})),
        json!({ "id": ARCS, "components": { "stitch": {
            "a": DROPOFF, "b": COLLECT, "rule": "explicit",
            "pairs": [[DROPOFF, OUTSIDE], [OUTSIDE, STEP], [STEP, COLLECT], [COLLECT, SEAT], [SEAT, DROPOFF]],
        }}}),
        inventory_lane(LANE, vec![STEP, COLLECT]),
    ]);
    let map = load("lane_entered_off_the_dropoff", entities).expect("the document loads");

    let station = map
        .map
        .stations
        .iter()
        .find(|s| s.collection_cell.is_some())
        .expect("the machine's station names a collection cell");
    let collect = station.collection_cell.expect("a collection cell");
    let lane = station.handoff_queues.iter().find(|q| q.into == collect).expect("a lane serves the collection cell");

    let by_id = |id: &str| {
        map.index
            .cell_entities
            .iter()
            .find(|(_, entity)| entity.to_string() == id)
            .map(|(cell, _)| *cell)
            .unwrap_or_else(|| panic!("{id} is a cell of the loaded map"))
    };
    let (outside, dropoff) = (by_id(OUTSIDE), by_id(DROPOFF));
    assert_eq!(station.dropoff_cell, Some(dropoff), "the station's dropoff is the sink port, as always");
    assert_eq!(lane.entry_from, Some(outside), "the lane is entered from the floor cell with the arc into its head");
    assert_ne!(lane.entry_from, station.dropoff_cell, "which on this document is not the dropoff");

    let approach = approach_to_collection(station).expect("the machine has a way in");
    assert!(approach.contains(&outside), "the way in crosses the cell the lane is entered from: {approach:?}");
    assert!(
        !approach.contains(&dropoff),
        "and not the dropoff, which no robot bound for the collection cell drives through: {approach:?}"
    );
}
