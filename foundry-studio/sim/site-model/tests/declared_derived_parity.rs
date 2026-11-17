//! Whether a hand-declared entity is usable everywhere a derived one is.
//!
//! SPEC §1 commits to derivation being a convenience and never a second class of
//! existence, and SPEC §3 gives the two identities different spellings — a UUID
//! somebody minted, an address a generator produced. Different spellings are
//! exactly where a parity claim quietly rots: a lookup that only handles one
//! shape, a check that only runs on the other, a field a document can only fill
//! one way. So this file takes the places a derived cell normally sits and puts
//! a hand-declared one there instead: a queue's path, a relay's `access_cell`
//! binding, and both ends of an explicit stitch. Nothing here is about how the
//! cell was named; every assertion is that naming it the other way changed
//! nothing.

use serde_json::{json, Value};
use std::path::Path;
use site_model::components::product::Queue;
use site_model::pipeline::layers::LayerSet;
use site_model::pipeline::{derive, World};
use site_model::{Document, EntityId};

const FLOOR: &str = "f0000000-0000-4000-8000-000000000002";
const HIGHWAY: &str = "a0000000-0000-4000-8000-000000000001";
const RACK: &str = "b0000000-0000-4000-8000-000000000001";
const LANE: &str = "c0000000-0000-4000-8000-000000000001";
const STITCH: &str = "c0000000-0000-4000-8000-000000000002";
/// A cell nobody derived: written out by hand, one tile east of the highway's
/// own column, on the tile the rack's relay reaches for.
const BY_HAND: &str = "e0000000-0000-4000-8000-000000000001";

fn id(text: &str) -> EntityId {
    text.parse().expect("a literal identity in this test")
}

/// The identity a generator derives at one key, spelled the way a document
/// spells it.
fn at(generator: &str, key: &str) -> EntityId {
    id(&format!("{generator}/{key}"))
}

/// One tiled highway, one hand-declared cell beside it, a rack whose relay
/// reaches that hand-declared cell, a lane queueing over one of each, and a
/// stitch joining the two kinds. One step is one millimetre.
fn world() -> World {
    let value = json!({
        "proto_version": "0.7.0",
        "doc": { "id": "da000000-0000-4000-8000-000000000001" },
        "primary_frame": FLOOR,
        "entities": [
            { "id": FLOOR, "components": {
                "frame": { "dims": 2, "units": "mm", "unit_mm": 1 } } },

            // The generator: a 1 x 3 column of cells at x = 450.
            { "id": HIGHWAY, "name": "highway", "components": {
                "region": { "frame": FLOOR, "origin": [0, 0], "size": [900, 2700] },
                "tiling": { "pitch": [900, 900],
                            "derives": { "cell": { "traversable_by": ["htm"] } } }
            }},

            // The same thing, written out: a cell centred at [1350, 450], which
            // is the tile the highway would have had at column 1 if it had one.
            { "id": BY_HAND, "name": "a cell somebody wrote", "components": {
                "pose": { "frame": FLOOR, "position": [1350, 450] },
                "cell": { "size": [900, 900], "traversable_by": ["htm"] }
            }},

            // A bay with one relay level, its shelf standing at the near-y edge
            // of the bay and reaching 900: from its point at [1350, 900] the
            // nearest cell is the declared one.
            { "id": RACK, "components": {
                "pose": { "frame": FLOOR, "position": [900, 900] },
                "rack": { "slots_wide": 1, "slots_deep": 1, "footprint": [900, 900],
                          "levels": [ { "height": 400, "positions": [[450, 0]],
                                        "reach": 900 } ] }
            }},

            // A queue whose two steps are one of each: a derived cell by its
            // address, a declared cell by its UUID.
            { "id": LANE, "components": {
                "queue": { "path": [
                    { "generator": HIGHWAY, "key": "cell:0,0" },
                    BY_HAND
                ]}
            }},

            // And a seam stated the same mixed way.
            { "id": STITCH, "components": {
                "stitch": { "a": HIGHWAY, "b": BY_HAND, "rule": "explicit",
                            "pairs": [[
                                { "generator": HIGHWAY, "key": "cell:0,0" },
                                BY_HAND
                            ]] }
            }}
        ],
    });
    let scenario = Document::from_value(Path::new("parity"), &value).expect("the document parses");
    derive(&LayerSet {
        scenario,
        layers: Vec::new(),
    })
    .expect("a hand-declared cell is as loadable as a derived one")
}

/// Both cells exist, and each is named the way its own provenance names it.
#[test]
fn a_declared_cell_and_a_derived_one_stand_side_by_side() {
    let world = world();
    let derived = at(HIGHWAY, "cell:0,0");
    assert!(world.entities[&derived].has("cell"));
    assert!(world.entities[&id(BY_HAND)].has("cell"));
    assert!(derived.is_derived());
    assert!(!id(BY_HAND).is_derived());
    // Geometry is geometry: both are cells of the graph with a centre and an
    // extent, and the cell table holds them on the same terms.
    assert_eq!(
        world.cells[&derived].footprint,
        world.cells[&id(BY_HAND)].footprint
    );
}

/// A queue path takes either arm in either position, and the loader resolves
/// both — SPEC §3's three spellings of a cell address, two of which are
/// identities.
#[test]
fn a_queue_path_walks_from_a_derived_cell_onto_a_declared_one() {
    let world = world();
    let queue: Queue = world.entities[&id(LANE)]
        .require("queue")
        .expect("the lane's queue");
    assert_eq!(queue.path.len(), 2);
    let steps: Vec<EntityId> = site_model::references::cells_addressed(&world.entities[&id(LANE)])
        .expect("the addresses parse")
        .into_iter()
        .map(|reference| reference.target)
        .collect();
    assert_eq!(steps, vec![at(HIGHWAY, "cell:0,0"), id(BY_HAND)]);
}

/// The relay's search returns whichever cell is nearest, and it writes that cell
/// into `access_cell` the same way whichever it is. Here the nearest cell is one
/// nobody derived.
#[test]
fn a_relay_binds_a_hand_declared_cell_the_same_way_it_binds_a_derived_one() {
    let world = world();
    let relay = at(RACK, "relay:0,0");
    let binding = world.entities[&relay].components["access_cell"].clone();
    assert_eq!(binding, json!({ "cell": BY_HAND }));
}

/// An explicit stitch links the pair it names, and the arc it compiles reaches
/// across the two kinds of identity without noticing them.
#[test]
fn a_stitch_joins_a_derived_cell_to_a_declared_one() {
    let world = world();
    let derived = at(HIGHWAY, "cell:0,0");
    let declared = id(BY_HAND);
    let link = world
        .edges
        .iter()
        .find(|edge| edge.owner == id(STITCH))
        .expect("the stitch compiled its link");
    assert_eq!((&link.from, &link.to), (&derived, &declared));
    assert!(link.bidirectional);
}

/// The expanded form writes each identity in the one place its kind belongs: a
/// declared cell stays a record of the document, a derived one hangs under the
/// generator that produced it, keyed by the segment it was produced at.
#[test]
fn the_expanded_form_carries_both_without_flattening_either() {
    let value = json!({
        "proto_version": "0.7.0",
        "doc": { "id": "da000000-0000-4000-8000-000000000001" },
        "primary_frame": FLOOR,
        "entities": [
            { "id": FLOOR, "components": {
                "frame": { "dims": 2, "units": "mm", "unit_mm": 1 } } },
            { "id": HIGHWAY, "components": {
                "region": { "frame": FLOOR, "origin": [0, 0], "size": [900, 900] },
                "tiling": { "pitch": [900, 900] } }},
            { "id": BY_HAND, "components": {
                "pose": { "frame": FLOOR, "position": [1350, 450] },
                "cell": { "size": [900, 900] } }}
        ],
    });
    let scenario = Document::from_value(Path::new("parity"), &value).expect("the document parses");
    let set = LayerSet {
        scenario,
        layers: Vec::new(),
    };
    let world = derive(&set).expect("the world derives");
    let document = site_model::expanded::emit(&set, &world);
    let records = document["entities"].as_array().expect("the record array");

    let declared: &Value = records
        .iter()
        .find(|record| record["id"] == BY_HAND)
        .expect("the hand-declared cell is a record of its own");
    assert!(declared.get("derived").is_none(), "it derives nothing");

    let generator = records
        .iter()
        .find(|record| record["id"] == HIGHWAY)
        .expect("the generator is a record");
    assert_eq!(
        generator["derived"]["cell:0,0"]["id"],
        at(HIGHWAY, "cell:0,0").to_string(),
        "a derived node states the address it sits at"
    );
}
