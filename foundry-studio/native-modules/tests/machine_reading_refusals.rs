//! What the loader refuses when a document's MACHINE does not say
//! unambiguously which cells belong to it, where its belt hands carriers over,
//! or which way an HTM leaves it.
//!
//! A machine is a frame and its members are the cells posed in that frame. That
//! reading is only single-valued if a machine frame is never nested under
//! another machine's -- `in_frame` walks every ancestor, so nesting would put
//! one cell in two machines and let the outer machine's ports overwrite the
//! inner one's. Its ports are declared cells, not poses to be matched to the
//! nearest floor. And its pick places' exit headings are stated in the
//! machine's OWN frame, so they cross the frame chain before they become
//! compass points: a mirrored frame turns a heading round.
//!
//! Every document here is hand-built and tiny. The first test is the POSITIVE
//! control -- the unspoiled shape loads and its exit reads East -- so a refusal
//! below means the defect and not the scaffolding.

use std::path::PathBuf;

use desgraph::CardinalDirection;
use dessim::worldmap::{self, WorldMap, WorldMapError};
use serde_json::{Value, json};

const SITE: &str = "20000000-0000-4000-8000-000000000001";
const MACHINE: &str = "20000000-0000-4000-8000-000000000002";
const DROPOFF: &str = "20000000-0000-4000-8000-000000000003";
const STEP: &str = "20000000-0000-4000-8000-000000000004";
const COLLECT: &str = "20000000-0000-4000-8000-000000000005";
const SEAT: &str = "20000000-0000-4000-8000-000000000006";
const ARCS: &str = "20000000-0000-4000-8000-000000000007";
const INNER: &str = "20000000-0000-4000-8000-000000000008";
const SECOND_SINK: &str = "20000000-0000-4000-8000-000000000009";
const POSED_PORT: &str = "20000000-0000-4000-8000-00000000000a";
const SECOND_SOURCE: &str = "20000000-0000-4000-8000-00000000000b";
const OUTSIDE_PORT: &str = "20000000-0000-4000-8000-00000000000c";
const PORT_STATION: &str = "20000000-0000-4000-8000-00000000000d";

fn load(name: &str, entities: Vec<Value>) -> Result<WorldMap, WorldMapError> {
    let document = json!({
        "proto_version": "0.7.0",
        "doc": { "id": "20000000-0000-4000-8000-0000000000ff", "kind": "world", "name": name },
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
/// refused. `WorldMap` carries the loaded world and has no `Debug`.
fn refused(loaded: Result<WorldMap, WorldMapError>, what: &str) -> WorldMapError {
    match loaded {
        Ok(_) => panic!("{what} loaded without a word"),
        Err(refusal) => refusal,
    }
}

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

/// One machine posed in the site frame, with a transform the caller chooses:
/// `scale [-1, 1]` is the mirrored install a heading has to survive.
fn machine(id: &str, parent: &str, translation: [i64; 2], scale: [i64; 2]) -> Value {
    json!({ "id": id, "components": {
        "frame": { "dims": 2, "units": "mm", "parent": parent,
                   "transform_to_parent": { "translation": translation, "scale": scale } },
        "region": { "frame": id, "origin": [-500, -3000], "size": [3000, 4000] },
        "go.easyloop": { "part": "envelope", "loop_transit_secs": 45 },
    }})
}

fn site() -> Value {
    json!({ "id": SITE, "components": { "frame": { "dims": 2, "units": "mm", "unit_mm": 1 } } })
}

/// The machine's cells and arcs. `heading` is the pick place's own
/// `heading_deg`, which is what these tests vary.
fn machine_body(frame: &str, heading: f64) -> Vec<Value> {
    vec![
        cell(DROPOFF, frame, 0, 0, json!({ "port": { "direction": "sink", "of": [], "into": frame } })),
        cell(STEP, frame, 0, -800, json!({})),
        cell(COLLECT, frame, 800, 0, json!({ "port": { "direction": "source", "of": [], "from": frame } })),
        json!({ "id": SEAT, "components": {
            "pose": { "frame": frame, "position": [1600, -1600], "heading_deg": heading },
            "cell": { "traversable_by": ["htm"], "size": [700, 700] },
            "presentation_point": { "for": "order_carrier" },
            // The happening that makes the cell a pick place; the reader does
            // not know the name yet, so it is carried rather than required.
            "pick": {},
        }}),
        json!({ "id": ARCS, "components": { "stitch": {
            "a": DROPOFF, "b": COLLECT, "rule": "explicit",
            "pairs": [[DROPOFF, STEP], [STEP, COLLECT], [COLLECT, SEAT], [SEAT, DROPOFF]],
        }}}),
        json!({ "id": "20000000-0000-4000-8000-0000000000aa", "components": {
            "queue": { "path": [DROPOFF, STEP, COLLECT], "discipline": "no_reverse" },
        }}),
    ]
}

/// One machine, unmirrored, whose pick place states heading 0.
fn plain_machine(heading: f64) -> Vec<Value> {
    let mut entities = vec![site(), machine(MACHINE, SITE, [0, 0], [1, 1])];
    entities.extend(machine_body(MACHINE, heading));
    entities
}

/// Which way the document's one pick station says an HTM leaves.
fn exit_of(map: &WorldMap) -> CardinalDirection {
    map.map.stations.iter().find(|s| s.conveyor_group.is_some()).expect("the machine's pick station").exit_direction
}

// ---------------------------------------------------------------------------
// the positive control, and the mirror
// ---------------------------------------------------------------------------

/// Heading 0 is `+x`, and in an unmirrored frame `+x` is East.
#[test]
fn an_unmirrored_pick_place_leaves_along_plus_x() {
    let map = load("machine_plain", plain_machine(0.0)).expect("the document loads");
    assert_eq!(exit_of(&map), CardinalDirection::East);
}

/// The same heading in a MIRRORED frame points the other way. A heading is a
/// direction, and `scale [-1, 1]` turns a direction round: local `+x` reaches
/// the primary frame pointing along `-x`, which is West. Reading the number as
/// though it had been stated in the primary frame sends the robot out of the
/// machine on the wrong side.
#[test]
fn a_mirrored_frame_turns_a_pick_places_exit_west() {
    let mut entities = vec![site(), machine(MACHINE, SITE, [8000, 0], [-1, 1])];
    entities.extend(machine_body(MACHINE, 0.0));
    let map = load("machine_mirrored", entities).expect("the document loads");
    assert_eq!(exit_of(&map), CardinalDirection::West, "local +x under scale [-1, 1] is primary -x");
}

/// A quarter turn is still a quarter turn: 90 degrees is `+y`, and row centres
/// increase along `+y`, so it is South.
#[test]
fn a_quarter_turn_leaves_south() {
    let map = load("machine_quarter", plain_machine(90.0)).expect("the document loads");
    assert_eq!(exit_of(&map), CardinalDirection::South);
}

// ---------------------------------------------------------------------------
// the refusals
// ---------------------------------------------------------------------------

/// A heading between the axes has no cardinal to round to, and rounding it to
/// East silently sends the robot out the wrong side of the machine.
#[test]
fn a_heading_that_is_not_a_cardinal_is_refused() {
    let refusal = refused(load("machine_diagonal", plain_machine(45.0)), "a 45-degree exit heading");
    assert!(
        matches!(refusal, WorldMapError::ExitHeadingIsNotCardinal { .. }),
        "the loader names the pick place, the heading and where the frame chain sent it: {refusal}"
    );
}

/// And a heading a HAIR off an axis is refused too. A twentieth of a degree
/// leaves the along-axis component within 1e-6 of 1 -- a cosine barely moves
/// near an axis -- so a tolerance read on that component called this due East.
/// Read on the component ACROSS the axis it is a sine, which at 0.05° is
/// 8.7e-4: three orders of magnitude outside, and refused.
#[test]
fn a_heading_a_twentieth_of_a_degree_off_an_axis_is_refused() {
    let refusal = refused(load("machine_near_axis", plain_machine(0.05)), "a 0.05-degree exit heading");
    assert!(
        matches!(refusal, WorldMapError::ExitHeadingIsNotCardinal { .. }),
        "the loader names the pick place, the heading and where the frame chain sent it: {refusal}"
    );
}

/// A machine frame nested under another machine's frame puts every cell of the
/// inner machine in both, because membership walks the whole frame chain. The
/// outer machine's ports then overwrite the inner machine's without a word.
#[test]
fn a_machine_nested_under_another_machine_is_refused() {
    let mut entities = vec![site(), machine(MACHINE, SITE, [0, 0], [1, 1]), machine(INNER, MACHINE, [4000, 0], [1, 1])];
    entities.extend(machine_body(INNER, 0.0));
    let refusal = refused(load("machine_nested", entities), "a machine inside a machine");
    assert!(
        matches!(refusal, WorldMapError::NestedMachines { .. }),
        "the loader names the inner machine and the outer one: {refusal}"
    );
}

/// One belt has one way in. A second sink used to leave whichever port the scan
/// reached last as the machine's dropoff.
#[test]
fn a_machine_with_two_sink_ports_is_refused() {
    let mut entities = plain_machine(0.0);
    entities.push(cell(
        SECOND_SINK,
        MACHINE,
        1600,
        0,
        json!({ "port": { "direction": "sink", "of": [], "into": MACHINE } }),
    ));
    let refusal = refused(load("machine_two_sinks", entities), "a machine with two sinks");
    assert!(
        matches!(refusal, WorldMapError::MachineWithTwoPorts { .. }),
        "the loader names the machine, the direction and both ports: {refusal}"
    );
}

/// One belt has one way out, as it has one way in. Two sources on one machine
/// used to leave whichever port the scan reached last as the machine's
/// collection cell -- the same silent overwrite the sink pair had, on the half
/// of the belt no test covered.
#[test]
fn a_machine_with_two_source_ports_is_refused() {
    let mut entities = plain_machine(0.0);
    entities.push(cell(
        SECOND_SOURCE,
        MACHINE,
        1600,
        0,
        json!({ "port": { "direction": "source", "of": [], "from": MACHINE } }),
    ));
    let refusal = refused(load("machine_two_sources", entities), "a machine with two sources");
    let WorldMapError::MachineWithTwoPorts { machine, direction, .. } = &refusal else {
        panic!("the loader names the machine, the direction and both ports: {refusal}");
    };
    assert_eq!(machine.to_string(), MACHINE);
    assert_eq!(direction, "source", "the direction named is the one stated twice");
}

/// A port is where an HTM stands to hand a carrier over, so it IS a declared
/// cell. Resolving a posed port to the floor nearest it could put a machine's
/// handover on another machine's lane, and the two readings that did and did
/// not resolve it disagreed about whether a lane ending there was a through
/// route or an approach.
///
/// The refusal names WHOSE port it is, and for a port posed inside a machine
/// that is the machine: the census cannot answer, because the pattern that
/// finds a machine's ports only returns ports that are cells, and the refusal
/// used to name the port as its own owner.
#[test]
fn a_port_that_is_not_a_cell_is_refused() {
    let mut entities = plain_machine(0.0);
    entities.push(json!({ "id": POSED_PORT, "components": {
        "pose": { "frame": MACHINE, "position": [800, -800] },
        "port": { "direction": "source", "of": [], "from": MACHINE },
    }}));
    let refusal = refused(load("machine_posed_port", entities), "a port that is not a cell");
    let WorldMapError::PortIsNotACell { port, owner } = &refusal else {
        panic!("the loader names the port and its owner: {refusal}");
    };
    assert_eq!(port.to_string(), POSED_PORT, "the port named is the one that is not a cell");
    assert_eq!(owner.to_string(), MACHINE, "the owner named is the machine whose frame the port is posed in");
}

/// A port posed on ordinary floor belongs to no machine, so the owner the
/// refusal names is whatever its `into` says it feeds.
#[test]
fn a_port_outside_every_machine_is_named_for_the_station_it_feeds() {
    let mut entities = plain_machine(0.0);
    entities.push(json!({ "id": PORT_STATION, "components": {
        "pose": { "frame": SITE, "position": [12000, 0] },
        "station": { "kind": "put" },
    }}));
    entities.push(json!({ "id": OUTSIDE_PORT, "components": {
        "pose": { "frame": SITE, "position": [12000, 800] },
        "port": { "direction": "sink", "of": [], "into": PORT_STATION },
    }}));
    let refusal = refused(load("floor_posed_port", entities), "a port on the floor that is not a cell");
    let WorldMapError::PortIsNotACell { port, owner } = &refusal else {
        panic!("the loader names the port and its owner: {refusal}");
    };
    assert_eq!(port.to_string(), OUTSIDE_PORT, "the port named is the one that is not a cell");
    assert_eq!(owner.to_string(), PORT_STATION, "the owner named is the station the port's `into` feeds");
}
