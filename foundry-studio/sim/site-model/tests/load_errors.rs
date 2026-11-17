//! What the loader refuses and what it merely remarks on, driven the way a
//! consumer drives it.
//!
//! Most of these are rules the spec states as a refusal rather than a
//! best-effort, and each one exists because the installed base shows what the
//! alternative costs: an override whose target evaporated, a patch that minted a
//! half-formed entity from a typo, a seam an author believed in that was never
//! there, a handoff shelf with no floor in front of it. The rest are the
//! warnings that sit next to those refusals, where the fault is the site's
//! rather than the author's — steel nobody reaches, a floor laid without the
//! relays in mind — because a warning that fires on the wrong case is as wrong
//! as a refusal that does. Documents are built here as values rather than files
//! so the assertion is about the rule and not about a fixture.

use serde_json::{json, Value};
use site_model::pipeline::derive;
use site_model::pipeline::layers::LayerSet;
use site_model::{Document, EntityId, LoadError, Steps, Warning};
use std::path::Path;

const FLOOR: &str = "f0000000-0000-4000-8000-000000000002";
const HIGHWAY: &str = "a0000000-0000-4000-8000-000000000001";
const AISLE: &str = "a0000000-0000-4000-8000-000000000002";

/// A derived entity by the address it was produced at: the generator's UUID and
/// the key beneath it (SPEC §3).
fn at(generator: impl std::fmt::Display, key: impl std::fmt::Display) -> EntityId {
    format!("{generator}/{key}").parse().expect("a valid address")
}

fn document(name: &str, entities: Value) -> Document {
    let value = json!({
        "proto_version": "0.7.0",
        "doc": { "id": format!("da000000-0000-4000-8000-00000000000{name}") },
        "primary_frame": FLOOR,
        "entities": entities,
    });
    Document::from_value(Path::new(name), &value).expect("the document itself parses")
}

/// A world with one tiled highway, which every case below leans on. One step is
/// one millimetre here, so the numbers read as the measurements they came from.
fn highway_layer(extra: Vec<Value>) -> Document {
    let mut entities = vec![
        json!({ "id": FLOOR, "components": {
            "frame": { "dims": 2, "units": "mm", "unit_mm": 1 } } }),
        json!({ "id": HIGHWAY, "name": "north-south highway", "components": {
            "region": { "frame": FLOOR, "origin": [0, 0], "size": [1800, 29700] },
            "tiling": { "pitch": [900, 900], "derives": { "cell": { "traversable_by": ["htm"] } } }
        }}),
    ];
    entities.extend(extra);
    document("1", Value::Array(entities))
}

fn load(layers: Vec<Document>) -> Result<(), LoadError> {
    let scenario = document("9", json!([]));
    derive(&LayerSet { scenario, layers }).map(|_| ())
}

#[test]
fn an_override_of_a_coordinate_no_generator_produces_fails_the_load() {
    // The highway is 2 columns wide and 33 rows tall; column 9 is off the end.
    let err = load(vec![highway_layer(vec![json!({
        "derivation": { "generator": HIGHWAY, "key": "cell:9,7" },
        "components": { "cell": { "blocked": true } }
    })])])
    .unwrap_err();
    assert!(matches!(err, LoadError::DanglingDerivation { kind, .. } if kind == "an override"), "{err}");
}

#[test]
fn a_suppression_of_a_coordinate_no_generator_produces_fails_the_load() {
    let err = load(vec![highway_layer(vec![json!({
        "derivation": { "generator": HIGHWAY, "key": "cell:0,99" },
        "suppress": true
    })])])
    .unwrap_err();
    assert!(matches!(err, LoadError::DanglingDerivation { kind, .. } if kind == "a suppression"), "{err}");
}

#[test]
fn a_patch_of_an_entity_no_layer_declared_fails_the_load() {
    let overlay = document(
        "2",
        json!([{ "id": AISLE, "patch": true,
                 "components": { "flow": { "rule": "two_way" } } }]),
    );
    let err = load(vec![highway_layer(vec![]), overlay]).unwrap_err();
    assert!(matches!(err, LoadError::PatchMissesTarget { .. }), "{err}");
}

#[test]
fn a_tiling_extent_the_pitch_does_not_divide_fails_the_load() {
    let layer = document(
        "1",
        json!([
            { "id": FLOOR, "components": {
                "frame": { "dims": 2, "units": "mm", "unit_mm": 1 } } },
            { "id": HIGHWAY, "components": {
                "region": { "frame": FLOOR, "origin": [0, 0], "size": [1800, 29650] },
                "tiling": { "pitch": [900, 900] }
            }}
        ]),
    );
    let err = load(vec![layer]).unwrap_err();
    assert!(matches!(err, LoadError::TilingNotDivisible { axis: "y", .. }), "{err}");
}

/// A step becomes physical in exactly one place (SPEC §4), so a frame that is
/// not the primary one may not say what one is worth.
#[test]
fn a_second_frame_claiming_to_size_a_step_fails_the_load() {
    let err = load(vec![highway_layer(vec![json!({
        "id": "f0000000-0000-4000-8000-000000000003",
        "components": { "frame": { "parent": FLOOR, "dims": 2, "units": "mm", "unit_mm": 0.1 } }
    })])])
    .unwrap_err();
    assert!(matches!(err, LoadError::UnitOnNonPrimaryFrame { .. }), "{err}");
}

#[test]
fn a_primary_frame_that_sizes_no_step_fails_the_load() {
    let layer = document("1", json!([{ "id": FLOOR, "components": { "frame": { "dims": 2, "units": "mm" } } }]));
    let err = load(vec![layer]).unwrap_err();
    assert!(matches!(err, LoadError::PrimaryFrameWithoutUnit { .. }), "{err}");
}

#[test]
fn an_expect_pin_that_no_longer_matches_fails_the_load() {
    // cell:1,7 is derived at [1350, 6750]; the pin names a point 900 away with a
    // tolerance of 50, which is what a changed pitch would look like.
    let err = load(vec![highway_layer(vec![json!({
        "derivation": { "generator": HIGHWAY, "key": "cell:1,7" },
        "expect": { "position": [1350, 5850], "tolerance": 50 },
        "components": { "cell": { "blocked": true } }
    })])])
    .unwrap_err();
    match err {
        LoadError::ExpectFailed { distance, tolerance, .. } => {
            assert_eq!(distance, Steps(900));
            assert_eq!(tolerance, Steps(50));
        }
        other => panic!("expected a failed pin, got {other}"),
    }
}

#[test]
fn a_pin_that_still_matches_loads() {
    assert!(load(vec![highway_layer(vec![json!({
        "derivation": { "generator": HIGHWAY, "key": "cell:1,7" },
        "expect": { "position": [1350, 6750], "tolerance": 50 },
        "components": { "cell": { "blocked": true } }
    })])])
    .is_ok());
}

#[test]
fn a_contact_stitch_that_links_nothing_fails_the_load() {
    // The second region starts 5000 past the highway's far edge, well outside
    // the 250 tolerance the stitch claims.
    let err = load(vec![highway_layer(vec![
        json!({ "id": AISLE, "components": {
            "region": { "frame": FLOOR, "origin": [6800, 5300], "size": [1800, 1800] },
            "tiling": { "pitch": [900, 900], "derives": { "cell": { "traversable_by": ["htm"] } } }
        }}),
        json!({ "id": "e0000000-0000-4000-8000-000000000001", "components": {
            "stitch": { "a": HIGHWAY, "b": AISLE, "rule": "contact", "tolerance": 250 }
        }}),
    ])])
    .unwrap_err();
    assert!(matches!(err, LoadError::ZeroLinkStitch { .. }), "{err}");
}

#[test]
fn the_same_seam_within_tolerance_loads() {
    assert!(load(vec![highway_layer(vec![
        json!({ "id": AISLE, "components": {
            "region": { "frame": FLOOR, "origin": [2000, 5300], "size": [1800, 1800] },
            "tiling": { "pitch": [900, 900], "derives": { "cell": { "traversable_by": ["htm"] } } }
        }}),
        json!({ "id": "e0000000-0000-4000-8000-000000000001", "components": {
            "stitch": { "a": HIGHWAY, "b": AISLE, "rule": "contact", "tolerance": 250 }
        }}),
    ])])
    .is_ok());
}

/// A bay whose relays reach floor and another whose relays reach nothing.
///
/// The two bays are one run, so the second one's relay is an authoring mistake
/// rather than a statement about the site: SPEC §7 puts the boundary exactly
/// there, and refusing is how the author hears about it. The run marches 20 000
/// east, well past the highway's two columns.
#[test]
fn a_relay_with_no_floor_in_front_of_it_fails_the_load_when_its_neighbours_bind() {
    let err = load(vec![highway_layer(vec![
        json!({ "id": "c0000000-0000-4000-8000-000000000001", "components": {
            "template": { "params": {}, "entities": [
                { "local": "rack", "components": {
                    "pose": { "position": [0, 0] },
                    "rack": { "slots_wide": 1, "slots_deep": 1, "footprint": [900, 900],
                              "levels": [
                                  { "height": 400, "positions": [[450, 900]],
                                    "reach": 600 },
                                  { "height": 800 }
                              ] } } }
            ] } }}),
        json!({ "id": "b0000000-0000-4000-8000-000000000001", "components": {
            "pose": { "frame": FLOOR, "position": [0, 0] },
            "stamp": { "template": "c0000000-0000-4000-8000-000000000001",
                       "repeat": { "count": 2, "step": [20000, 0] } } }}),
    ])])
    .unwrap_err();
    assert!(matches!(err, LoadError::RelayUnbound { .. }), "{err}");
}

/// The same run with nothing at all in front of it warns rather than refusing.
///
/// That is the ordinary shape of a converted legacy site, where relay zones
/// stand shoulder to shoulder and the floor outside one run's face is the next
/// run's steel. Refusing those would refuse the installed base (SPEC §7).
#[test]
fn a_run_none_of_whose_relays_binds_warns_instead() {
    let scenario = document("9", json!([]));
    let world = derive(&LayerSet {
        scenario,
        layers: vec![highway_layer(vec![
            json!({ "id": "c0000000-0000-4000-8000-000000000001", "components": {
                "template": { "params": {}, "entities": [
                    { "local": "rack", "components": {
                        "pose": { "position": [0, 0] },
                        "rack": { "slots_wide": 1, "slots_deep": 1, "footprint": [900, 900],
                                  "levels": [
                                      { "height": 400, "positions": [[450, 900]],
                                        "reach": 600 },
                                      { "height": 800 }
                                  ] } } }
                ] } }}),
            json!({ "id": "b0000000-0000-4000-8000-000000000001", "components": {
                "pose": { "frame": FLOOR, "position": [20000, 0] },
                "stamp": { "template": "c0000000-0000-4000-8000-000000000001",
                           "repeat": { "count": 2, "step": [3000, 0] } } }}),
        ])],
    })
    .expect("steel nobody reaches is a statement about the site, not a defect");
    assert_eq!(
        world.warnings.iter().filter(|w| matches!(w, Warning::SteelNobodyReaches { relays: 2, .. })).count(),
        1,
        "{:?}",
        world.warnings
    );
}

/// Two shelves at different places on their bays, one cell between them.
///
/// The bay's boards sit 450 apart and the highway's tiles are 900, so both
/// boards' nearest tile is the same one. SPEC §5 expects a cell under each relay
/// stack, so this is the floor and the steel out of step — a warning naming both
/// shelves, and not a refusal, because which of the two is in the wrong place is
/// not something the document can say.
#[test]
fn two_relays_at_different_places_sharing_one_cell_warn_once_and_name_both() {
    const RACK: &str = "b0000000-0000-4000-8000-000000000003";
    let scenario = document("9", json!([]));
    let world = derive(&LayerSet {
        scenario,
        layers: vec![highway_layer(vec![json!({ "id": RACK, "components": {
            "pose": { "frame": FLOOR, "position": [0, 900] },
            "rack": { "slots_wide": 1, "slots_deep": 1, "footprint": [900, 900],
                      "levels": [{ "height": 400,
                                   "positions": [[225, 0], [675, 0]],
                                   "reach": 900 }] } }})])],
    })
    .expect("a floor out of step with the steel still loads");

    // Both shelves take the highway tile centred [450, 450]: 493 and 508 away,
    // and the next tile up is 1146 and 1075, past the 900 reach.
    let sharing: Vec<&Warning> =
        world.warnings.iter().filter(|warning| matches!(warning, Warning::RelaysApartShareOneCell { .. })).collect();
    assert_eq!(sharing.len(), 1, "{:?}", world.warnings);
    let spoken = sharing[0].to_string();
    assert!(spoken.contains(&format!("{RACK}/relay:0,0")), "{spoken}");
    assert!(spoken.contains(&format!("{RACK}/relay:0,1")), "{spoken}");
    assert!(spoken.contains(&format!("{HIGHWAY}/cell:0,0")), "{spoken}");
}

/// Two shelves stacked over one place on one bay, one cell between them, and
/// nothing said about it.
///
/// That is what a stack is: two heights, one place to stand, and SPEC §5 expects
/// them to share their cell without being told to. The document below is the one
/// above with the second shelf moved to the first one's offset and given a
/// height of its own — the same two relays binding the same one cell, and the
/// difference is only whether they stand at the same place on the steel.
#[test]
fn relays_stacked_at_one_place_share_their_cell_in_silence() {
    const RACK: &str = "b0000000-0000-4000-8000-000000000004";
    let scenario = document("9", json!([]));
    let world = derive(&LayerSet {
        scenario,
        layers: vec![highway_layer(vec![json!({ "id": RACK, "components": {
            "pose": { "frame": FLOOR, "position": [0, 900] },
            "rack": { "slots_wide": 1, "slots_deep": 1, "footprint": [900, 900],
                      "levels": [
                          { "height": 400, "positions": [[225, 0]], "reach": 900 },
                          { "height": 900, "positions": [[225, 0]], "reach": 900 }
                      ] } }})])],
    })
    .expect("a stack over one cell is the expected shape");

    let bound = |key: &str| {
        world.entities[&at(RACK, key)].components["access_cell"]["cell"]
            .as_str()
            .expect("a bound relay names its cell")
            .to_string()
    };
    assert_eq!(bound("relay:0,0"), bound("relay:1,0"));
    assert_eq!(bound("relay:0,0"), at(HIGHWAY, "cell:0,0").to_string());
    assert!(
        !world.warnings.iter().any(|warning| matches!(warning, Warning::RelaysApartShareOneCell { .. })),
        "{:?}",
        world.warnings
    );
}

/// A binding stated by hand has to name floor that exists. The search can only
/// ever pick a live cell; an override is the half of SPEC §5's parity where a
/// person can be wrong.
#[test]
fn a_hand_stated_binding_naming_no_cell_fails_the_load() {
    let err = load(vec![highway_layer(vec![
        json!({ "id": "b0000000-0000-4000-8000-000000000002", "components": {
            "pose": { "frame": FLOOR, "position": [0, 0] },
            "rack": { "slots_wide": 1, "slots_deep": 1, "footprint": [900, 900],
                      "levels": [{ "height": 400, "positions": [[450, 900]],
                                   "reach": 600 }] } }}),
        json!({ "derivation": { "generator": "b0000000-0000-4000-8000-000000000002",
                                "key": "relay:0,0" },
                "components": { "access_cell": {
                    "cell": "e0000000-0000-4000-8000-0000000000ff" } } }),
    ])])
    .unwrap_err();
    assert!(matches!(err, LoadError::AccessCellNotALiveCell { .. }), "{err}");
}

#[test]
fn two_records_addressing_one_target_in_one_document_fail_the_load() {
    let value = json!({
        "proto_version": "0.7.0",
        "doc": { "id": "da000000-0000-4000-8000-000000000001" },
        "entities": [
            { "derivation": { "generator": HIGHWAY, "key": "cell:1,7" },
              "components": { "cell": { "blocked": true } } },
            { "derivation": { "generator": HIGHWAY, "key": "cell:1,7" },
              "components": { "cell": { "blocked": false } } }
        ]
    });
    let err = Document::from_value(Path::new("<test>"), &value).unwrap_err();
    assert!(matches!(err, LoadError::DuplicateTarget { .. }), "{err}");
}

#[test]
fn a_reference_to_an_entity_nobody_declared_fails_the_load() {
    let missing = "e0000000-0000-4000-8000-0000000000ff";
    let err = load(vec![highway_layer(vec![json!({
        "id": "e0000000-0000-4000-8000-000000000003",
        "components": { "sector": { "members": [missing] } }
    })])])
    .unwrap_err();
    assert!(matches!(err, LoadError::DanglingReference { ref field, .. } if field == "members"), "{err}");
}

#[test]
fn a_stamp_of_a_template_nobody_declared_fails_the_load() {
    let err = load(vec![highway_layer(vec![json!({
        "id": "b0000000-0000-4000-8000-000000000001",
        "components": {
            "pose": { "frame": FLOOR, "position": [0, 0] },
            "stamp": { "template": "c0000000-0000-4000-8000-0000000000ff" }
        }
    })])])
    .unwrap_err();
    assert!(matches!(err, LoadError::DanglingReference { ref field, .. } if field == "template"), "{err}");
}

/// A declaration's `id` mints an identity, so it is a UUID; the address of a
/// derived entity says a generator made it, which is the one thing a declaration
/// cannot claim. Writing one there is refused rather than read as a UUID with an
/// unusual spelling (SPEC §3).
///
/// This case used to read the other way round: while a derived id was a UUIDv5
/// hashed down its chain, an authored UUID could land on one, and the loader
/// caught the collision. Identity is now the address itself, so a UUID and an
/// address can never be the same string and that collision is unconstructible.
/// What is left to refuse is the confusion of the two.
#[test]
fn a_declaration_that_states_an_address_as_its_identity_fails_the_load() {
    // Refused where the record is read, since the shape of a record is what says
    // which kind it is — no later step has to carry the confusion along.
    let value = json!({
        "proto_version": "0.7.0",
        "doc": { "id": "da000000-0000-4000-8000-000000000001" },
        "entities": [{
            "id": format!("{HIGHWAY}/cell:1,7"),
            "components": { "cell": { "traversable_by": ["htm"] } }
        }],
    });
    let err = Document::from_value(Path::new("declaration"), &value).unwrap_err();
    assert!(matches!(err, LoadError::DeclaredIdIsAnAddress { .. }), "{err}");
}

/// An override may state the identity it addresses as well as the address it
/// addresses it by; they have to be the same entity.
#[test]
fn a_stated_id_that_disagrees_with_its_derivation_address_fails_the_load() {
    let err = load(vec![highway_layer(vec![json!({
        "id": format!("{HIGHWAY}/cell:1,8"),
        "derivation": { "generator": HIGHWAY, "key": "cell:1,7" },
        "components": { "cell": { "blocked": true } }
    })])])
    .unwrap_err();
    assert!(matches!(err, LoadError::DerivationIdMismatch { .. }), "{err}");
}

#[test]
fn a_queue_path_step_that_no_generator_produces_fails_the_load() {
    // The highway is 2 columns wide; column 9 is off the end, so the third step
    // of this queue addresses floor that was never tiled.
    let err = load(vec![highway_layer(vec![json!({
        "id": "d0000000-0000-4000-8000-000000000001",
        "components": { "queue": { "path": [
            { "generator": HIGHWAY, "key": "cell:0,0" },
            { "generator": HIGHWAY, "key": "cell:0,1" },
            { "generator": HIGHWAY, "key": "cell:9,1" }
        ] } }
    })])])
    .unwrap_err();
    assert!(matches!(err, LoadError::QueuePathDangling { index: 2, .. }), "{err}");
}

#[test]
fn a_queue_path_through_a_suppressed_cell_fails_the_load() {
    // The address was right when it was written; a suppression later took the
    // floor out from under it. A queue with a hole in the middle is an
    // authoring mistake to fix now, not a surprise for a simulator to meet.
    let err = load(vec![highway_layer(vec![
        json!({ "derivation": { "generator": HIGHWAY, "key": "cell:0,1" },
                "expect": { "position": [450, 1350], "tolerance": 50 },
                "suppress": true }),
        json!({ "id": "d0000000-0000-4000-8000-000000000001",
                "components": { "queue": { "path": [
                    { "generator": HIGHWAY, "key": "cell:0,0" },
                    { "generator": HIGHWAY, "key": "cell:0,1" },
                    { "generator": HIGHWAY, "key": "cell:0,2" }
                ] } } }),
    ])])
    .unwrap_err();
    assert!(matches!(err, LoadError::QueuePathThroughRemovedCell { index: 1, .. }), "{err}");
}

#[test]
fn a_queue_path_step_that_is_not_a_cell_fails_the_load() {
    // `relay:0,0` is a rack's handoff shelf, not the floor cell beside it.
    let err = load(vec![highway_layer(vec![
        json!({ "id": "b0000000-0000-4000-8000-000000000001", "components": {
            "pose": { "frame": FLOOR, "position": [0, 0] },
            "rack": { "slots_wide": 1, "slots_deep": 1, "footprint": [1000, 1000],
                      "levels": [{ "height": 400, "positions": [[500, 1000]],
                                   "reach": 600 },
                                 { "height": 800 }] } } }),
        json!({ "id": "d0000000-0000-4000-8000-000000000001", "components": {
            "queue": { "path": [
                { "generator": "b0000000-0000-4000-8000-000000000001", "key": "relay:0,0" }
            ] } } }),
    ])])
    .unwrap_err();
    assert!(matches!(err, LoadError::QueuePathNotACell { index: 0, .. }), "{err}");
}

/// What the load says of a placeholder left outside every template: the
/// entity, the component and field it stands in, and what it is, in one
/// sentence. Asserted whole, because naming the placeholder is the point.
fn left_outside(entity: impl std::fmt::Display, field: &str, placeholder: &str) -> String {
    format!("entity {entity}: `{field}` still holds {placeholder}, which only a stamp can resolve")
}

const QUEUE: &str = "d0000000-0000-4000-8000-000000000001";

#[test]
fn a_queue_path_naming_a_local_handle_outside_a_template_fails_the_load() {
    let err = load(vec![highway_layer(vec![json!({
        "id": QUEUE,
        "components": { "queue": { "path": [
            { "local_ref": "order_queue", "key": "cell:0,0" }
        ] } }
    })])])
    .unwrap_err();
    assert_eq!(
        err.to_string(),
        left_outside(QUEUE, "queue.path[0]", "the local handle `order_queue` with the key `cell:0,0`")
    );
}

/// The schema admits a bare handle wherever a cell address goes, since it cannot
/// see whether a value stands inside a template (`proto/schema/README.md`), so
/// the reader is what keeps one there: met in a queue path or an explicit
/// stitch pair outside any template, the handle is refused as the handle it is,
/// exactly as the handle beside a key is above.
#[test]
fn a_bare_local_handle_outside_a_template_fails_the_load() {
    let queue = load(vec![highway_layer(vec![json!({
        "id": QUEUE,
        "components": { "queue": { "path": [{ "generator": HIGHWAY, "key": "cell:0,0" }, { "local_ref": "stand" }] } }
    })])])
    .unwrap_err();
    assert_eq!(queue.to_string(), left_outside(QUEUE, "queue.path[1]", "the local handle `stand`"));

    let pair = load(vec![highway_layer(vec![json!({
        "id": QUEUE,
        "components": { "stitch": { "a": HIGHWAY, "b": HIGHWAY, "rule": "explicit", "pairs": [
            [{ "generator": HIGHWAY, "key": "cell:0,0" }, { "local_ref": "stand", "$comment": "no template here" }]
        ] } }
    })])])
    .unwrap_err();
    assert_eq!(pair.to_string(), left_outside(QUEUE, "stitch.pairs[0][1]", "the local handle `stand`"));
}

/// A contact stitch matches its cells by geometry and never reads `pairs`, and
/// SPEC §5 does not forbid one stating them, so they are carried unread; a
/// placeholder in them is refused all the same, in each of its forms and
/// whatever the rule. The same seam stating two real cells loads.
#[test]
fn a_contact_stitch_carrying_a_placeholder_in_its_pairs_fails_the_load() {
    let stitch = "e0000000-0000-4000-8000-000000000001";
    let seam = |end: Value| {
        highway_layer(vec![
            json!({ "id": AISLE, "components": {
                "region": { "frame": FLOOR, "origin": [2000, 5300], "size": [1800, 1800] },
                "tiling": { "pitch": [900, 900], "derives": { "cell": { "traversable_by": ["htm"] } } }
            }}),
            json!({ "id": stitch, "components": {
                "stitch": { "a": HIGHWAY, "b": AISLE, "rule": "contact", "tolerance": 250,
                            "pairs": [[end, { "generator": AISLE, "key": "cell:0,0" }]] }
            }}),
        ])
    };
    for (end, placeholder) in [
        (json!({ "local_ref": "stand" }), "the local handle `stand`"),
        (json!({ "local_ref": "patch", "key": "cell:1,0" }), "the local handle `patch` with the key `cell:1,0`"),
        (
            json!({ "local_ref": "patch", "key": { "$param": "queue" } }),
            "the local handle `patch` with the parameter `queue` for its key",
        ),
        (json!({ "$param": "exit" }), "the parameter `exit`"),
        (json!({ "generator": HIGHWAY, "key": { "$param": "tile" } }), "an address whose key is the parameter `tile`"),
    ] {
        let err = load(vec![seam(end.clone())]).unwrap_err();
        assert_eq!(err.to_string(), left_outside(stitch, "stitch.pairs[0][0]", placeholder), "{end}");
    }
    assert!(load(vec![seam(json!({ "generator": HIGHWAY, "key": "cell:1,5" }))]).is_ok());
}

/// Outside a template nothing binds a parameter, so one standing for a
/// reference, a cell address or the key of a derivation address names nothing.
/// The load refuses it by name before any stage reads the component, so a
/// region's frame, which expansion reads first, is refused in the same words as
/// a queue path, which the last checks read. A record is never inside a
/// template, and the schema shares one key definition between a record's
/// address and a template's, so an override or a suppression whose key is a
/// parameter passes the schema too.
#[test]
fn a_parameter_outside_a_template_fails_the_load() {
    for (component, value, field, placeholder) in [
        (
            "stitch",
            json!({ "a": HIGHWAY, "b": { "$param": "floor" }, "rule": "contact" }),
            "stitch.b",
            "the parameter `floor`",
        ),
        ("queue", json!({ "path": [{ "$param": "exit" }] }), "queue.path[0]", "the parameter `exit`"),
        (
            "queue",
            json!({ "path": [{ "generator": HIGHWAY, "key": { "$param": "queue" } }] }),
            "queue.path[0]",
            "an address whose key is the parameter `queue`",
        ),
        (
            "region",
            json!({ "frame": { "$param": "floor" }, "origin": [0, 0], "size": [900, 900] }),
            "region.frame",
            "the parameter `floor`",
        ),
    ] {
        let err = load(vec![highway_layer(vec![json!({
            "id": QUEUE,
            "components": { component: value.clone() }
        })])])
        .unwrap_err();
        assert_eq!(err.to_string(), left_outside(QUEUE, field, placeholder), "{component} = {value}");
    }

    for record in [
        json!({ "derivation": { "generator": HIGHWAY, "key": { "$param": "tile" } },
                "components": { "cell": { "blocked": true } } }),
        json!({ "derivation": { "generator": HIGHWAY, "key": { "$param": "tile" } }, "suppress": true }),
    ] {
        let value = json!({
            "proto_version": "0.7.0",
            "doc": { "id": "da000000-0000-4000-8000-000000000003" },
            "primary_frame": FLOOR,
            "entities": [record.clone()],
        });
        let err = Document::from_value(Path::new("3"), &value).unwrap_err();
        assert_eq!(
            err.to_string(),
            "3: record #0: `derivation.key` still holds the parameter `tile`, which only a stamp can resolve",
            "{record}"
        );
    }
}

/// The four fields the reader carries without reading are references all the
/// same, so a placeholder left in one is refused as surely as one in a field
/// the reader resolves, `stitch.b` here, and in the same words. The cell loads
/// once nothing is left in it.
#[test]
fn a_placeholder_in_a_field_carried_unread_fails_the_load_as_one_in_a_field_read_does() {
    let marker = "e0000000-0000-4000-8000-000000000005";
    let cell = |cell: Value, point: Value| {
        json!({ "id": marker, "components": {
            "pose": { "frame": FLOOR, "position": [19000, 2000] }, "cell": cell, "presentation_point": point } })
    };
    for (entity, field, placeholder) in [
        (
            cell(json!({ "location_profile": { "$param": "profile" } }), json!({})),
            "cell.location_profile",
            "the parameter `profile`",
        ),
        (cell(json!({ "zone": { "local_ref": "zone" } }), json!({})), "cell.zone", "the local handle `zone`"),
        (
            cell(json!({}), json!({ "station": { "$param": "station" } })),
            "presentation_point.station",
            "the parameter `station`",
        ),
        (
            cell(
                json!({}),
                json!({ "attributes": { "cell": { "local_ref": "patch", "key": { "$param": "queue" } } } }),
            ),
            "presentation_point.attributes.cell",
            "the local handle `patch` with the parameter `queue` for its key",
        ),
        (
            json!({ "id": marker, "components": { "stitch": { "a": HIGHWAY, "b": { "$param": "floor" }, "rule": "contact" } } }),
            "stitch.b",
            "the parameter `floor`",
        ),
    ] {
        let err = load(vec![highway_layer(vec![entity.clone()])]).unwrap_err();
        assert_eq!(err.to_string(), left_outside(marker, field, placeholder), "{entity}");
    }
    assert!(load(vec![highway_layer(vec![cell(json!({}), json!({}))])]).is_ok());
}

/// Outside a template nothing binds a parameter, and the schema lets one stand
/// for a number, a numeric array or one of its elements, a flag or a rack's
/// levels as well as for a reference, so the load refuses one there in the same
/// words: in a field the reader reads, in a component it carries unread, and in
/// a tiling's `derives`. A component the reader does not know is its owner's,
/// and a handle is a placeholder only where a reference goes, so the same
/// parameter in an extension component, and a handle where only a parameter may
/// stand, load.
#[test]
fn a_parameter_left_where_a_value_goes_fails_the_load_as_one_where_a_reference_goes() {
    let marker = "e0000000-0000-4000-8000-000000000006";
    let cell = |pose: Value| json!({ "id": marker, "components": { "pose": pose, "cell": {} } });
    let carried = |components: Value| json!({ "id": marker, "components": components });
    for (entity, id, field, placeholder) in [
        (
            cell(json!({ "frame": FLOOR, "position": [19000, 2000], "heading_deg": { "$param": "heading" } })),
            marker,
            "pose.heading_deg",
            "the parameter `heading`",
        ),
        (
            cell(json!({ "frame": FLOOR, "position": [19000, { "$param": "y" }] })),
            marker,
            "pose.position[1]",
            "the parameter `y`",
        ),
        (
            carried(json!({ "carrier_type": { "kind": "tote", "size": { "$param": "size" } } })),
            marker,
            "carrier_type.size",
            "the parameter `size`",
        ),
        (
            json!({ "id": AISLE, "components": {
                "region": { "frame": FLOOR, "origin": [2000, 5300], "size": [1800, 1800] },
                "tiling": { "pitch": [900, 900], "derives": { "cell": { "size": [{ "$param": "side" }, 900] } } }
            }}),
            AISLE,
            "tiling.derives.cell.size[0]",
            "the parameter `side`",
        ),
    ] {
        let err = load(vec![highway_layer(vec![entity.clone()])]).unwrap_err();
        assert_eq!(err.to_string(), left_outside(id, field, placeholder), "{entity}");
    }
    for entity in [
        carried(json!({ "acme.tote": { "kind": "tote", "size": { "$param": "size" } } })),
        carried(json!({ "carrier_type": { "kind": "tote", "size": { "local_ref": "size" } } })),
    ] {
        assert!(load(vec![highway_layer(vec![entity.clone()])]).is_ok(), "{entity}");
    }
}

/// A template whose seam joins whichever floor its stamp names, and the stamp
/// that places it over the highway's first row.
fn seam_template(floor: Value) -> Vec<Value> {
    vec![
        json!({ "id": "c0000000-0000-4000-8000-000000000001", "components": { "template": {
            "params": { "floor": {} },
            "entities": [
                { "local": "patch", "components": {
                    "region": { "origin": [0, 0], "size": [1800, 900] },
                    "tiling": { "pitch": [900, 900], "derives": { "cell": { "traversable_by": ["htm"] } } } } },
                { "local": "seam", "components": {
                    "stitch": { "a": { "local_ref": "patch" }, "b": { "$param": "floor" }, "rule": "contact" } } }
            ] } } }),
        json!({ "id": "b0000000-0000-4000-8000-000000000001", "components": {
            "pose": { "frame": FLOOR, "position": [0, 0] },
            "stamp": { "template": "c0000000-0000-4000-8000-000000000001", "params": { "floor": floor } } } }),
    ]
}

/// A placeholder enters the world four ways, and each is refused before any
/// stage reads it: in a declaration, as above; in a tiling's `derives`, which
/// every cell the tiling derives would copy; in an override; and in a stamped
/// internal, where a stamp that binds a parameter to another parameter leaves
/// one. A template's own internals are where placeholders stand, and load.
#[test]
fn a_placeholder_is_refused_wherever_it_enters_the_world() {
    let derives = load(vec![highway_layer(vec![json!({ "id": AISLE, "components": {
        "region": { "frame": FLOOR, "origin": [2000, 5300], "size": [1800, 1800] },
        "tiling": { "pitch": [900, 900], "derives": { "cell": { "zone": { "$param": "zone" } } } }
    }})])])
    .unwrap_err();
    assert_eq!(derives.to_string(), left_outside(AISLE, "tiling.derives.cell.zone", "the parameter `zone`"));

    let overridden = load(vec![highway_layer(vec![json!({
        "derivation": { "generator": HIGHWAY, "key": "cell:0,0" },
        "expect": { "position": [450, 450], "tolerance": 50 },
        "components": { "cell": { "zone": { "local_ref": "zone" } } }
    })])])
    .unwrap_err();
    assert_eq!(overridden.to_string(), left_outside(at(HIGHWAY, "cell:0,0"), "cell.zone", "the local handle `zone`"));

    let stamp = "b0000000-0000-4000-8000-000000000001";
    let left = load(vec![highway_layer(seam_template(json!({ "$param": "elsewhere" })))]).unwrap_err();
    assert_eq!(
        left.to_string(),
        left_outside(at(stamp, "instance:0/local:seam"), "stitch.b", "the parameter `elsewhere`")
    );
    assert!(load(vec![highway_layer(seam_template(json!(HIGHWAY)))]).is_ok(), "bound to the highway, it loads");
}

/// A component the reader does not know is carried and never walked: what its
/// fields mean is its owner's (SPEC §9), so a value shaped like a placeholder
/// in one is that owner's data.
#[test]
fn a_component_the_reader_does_not_know_is_never_walked() {
    assert!(load(vec![highway_layer(vec![json!({ "id": QUEUE, "components": {
        "mapcreator.station": { "zone": { "$param": "zone" }, "cell": { "local_ref": "patch" } },
        "easyloop": { "station": { "local_ref": "station", "key": { "$param": "queue" } } }
    }})])])
    .is_ok());
}

#[test]
fn a_queue_over_cells_that_all_exist_loads() {
    assert!(load(vec![highway_layer(vec![json!({
        "id": "d0000000-0000-4000-8000-000000000001",
        "components": { "queue": { "path": [
            { "generator": HIGHWAY, "key": "cell:0,0" },
            { "generator": HIGHWAY, "key": "cell:0,1" },
            { "generator": HIGHWAY, "key": "cell:0,2" }
        ], "discipline": "no_reverse" } }
    })])])
    .is_ok());
}

#[test]
fn a_queue_path_may_name_a_hand_declared_cell_by_its_own_uuid() {
    // A cell authored directly has no generator and so no derivation address;
    // the bare UUID form is the only way to queue over one, and SPEC §3 promises
    // anything derivable is also declarable. It states no `size`, so its extent
    // is the 900 mm default read through the primary frame's `unit_mm`.
    let marker = "e0000000-0000-4000-8000-000000000005";
    assert!(load(vec![highway_layer(vec![
        json!({ "id": marker, "name": "dock-face keep-clear marker", "components": {
            "pose": { "frame": FLOOR, "position": [19000, 2000] },
            "cell": { "traversable_by": ["htm"], "blocked": false } } }),
        json!({ "id": "d0000000-0000-4000-8000-000000000001", "components": {
            "queue": { "path": [
                { "generator": HIGHWAY, "key": "cell:0,0" },
                marker
            ] } } }),
    ])])
    .is_ok());
}

#[test]
fn a_bare_uuid_naming_nothing_fails_the_load() {
    let err = load(vec![highway_layer(vec![json!({
        "id": "d0000000-0000-4000-8000-000000000001",
        "components": { "queue": { "path": ["e0000000-0000-4000-8000-0000000000ff"] } }
    })])])
    .unwrap_err();
    assert!(matches!(err, LoadError::QueuePathDangling { index: 0, .. }), "{err}");
}

#[test]
fn a_path_string_that_is_not_a_uuid_is_a_parse_error() {
    // The bare form is an identity, not a key. `"cell:0,0"` was the v0.2
    // spelling and must fail loudly rather than resolve to nothing.
    let err = load(vec![highway_layer(vec![json!({
        "id": "d0000000-0000-4000-8000-000000000001",
        "components": { "queue": { "path": ["cell:0,0"] } }
    })])])
    .unwrap_err();
    assert!(matches!(err, LoadError::ComponentShape { ref component, .. } if component == "queue"), "{err}");
}
