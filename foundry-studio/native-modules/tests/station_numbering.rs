//! Station IDs survive equivalent frame authoring and retain the shipped
//! Cencora/DFW6 identity mappings used by workbooks and scenario configuration.

use std::path::Path;

use dessim::worldmap::{self, WorldMap};
use serde_json::{Value, json};

fn document(path: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn load(document: &Value) -> WorldMap {
    worldmap::load_from_bytes(&serde_json::to_vec(document).unwrap()).unwrap()
}

#[test]
fn station_ids_survive_equivalent_local_units_and_mirrors() {
    let original = document("../data/seed/worlds/minimal-putaway2/minimal_putaway2.json");
    let before = load(&original);
    for scale in [[100, 100], [100, 10], [-100, 100], [100, -100]] {
        let mut changed = original.clone();
        let new_frame = "d1e5e17a-3fc2-4bdd-940c-e27bec53f543";
        let entities = changed["entities"].as_array_mut().unwrap();
        let station = entities.iter_mut().find(|e| e["components"]["station"]["kind"] == "pick").unwrap();
        let old_frame = station["components"]["pose"]["frame"].clone();
        station["components"]["pose"]["frame"] = json!(new_frame);
        for (coordinate, factor) in
            station["components"]["pose"]["position"].as_array_mut().unwrap().iter_mut().zip(scale)
        {
            *coordinate = json!(coordinate.as_i64().unwrap() / factor);
        }
        // Preserve the physical exit heading as well as the position.
        if scale[1] < 0 {
            station["components"]["station"]["exit_direction"] = json!("-y");
        }
        entities.push(json!({
            "id": new_frame, "name": "equivalent local frame", "components": {"frame": {
                "dims": 2, "units": "local", "parent": old_frame,
                "transform_to_parent": {"scale": scale, "translation": [0, 0]}
            }}
        }));
        let after = load(&changed);
        for (id, entity) in &before.index.station_entities {
            let new_id = after.index.station_entities.iter().find(|(_, e)| *e == entity).unwrap().0;
            let old_station = before.map.station(*id).unwrap();
            let new_station = after.map.station(*new_id).unwrap();
            assert_eq!(before.map.cell_world_pos(old_station.position), after.map.cell_world_pos(new_station.position));
            assert_eq!(old_station.exit_direction, new_station.exit_direction);
        }
        assert_eq!(before.index.station_entities, after.index.station_entities, "equivalent frame scale {scale:?}");
    }
}

#[test]
fn mixed_endpoints_keep_ids_when_the_floor_is_mirrored_and_translated() {
    let original = document("tests/fixtures/minimal_easyloop.json");
    let before = load(&original);
    assert_eq!(before.index.station_entities.len(), 5, "three declared stations and two machine pick places");
    for scale in [[1, -1], [-1, 1], [-1, -1]] {
        for translation in [[0, 150_000, 0], [0, -150_000, 0]] {
            let mut changed = original.clone();
            let floor = changed["entities"].as_array_mut().unwrap().iter_mut().find(|e| e["name"] == "floor").unwrap();
            floor["components"]["frame"]["transform_to_parent"] = json!({"scale": scale, "translation": translation});
            let after = load(&changed);
            assert_eq!(
                before.index.station_entities, after.index.station_entities,
                "floor scale {scale:?}, translation {translation:?}"
            );
        }
    }
}

// Captured from PR #563 at 0844b36f4, before correcting the shared sort. Pin
// every ID to its entity, rather than only checking counts or station types.
fn assert_station_ids(fixture: &str, expected: &[&str]) {
    let world = load(&document(&format!("tests/fixtures/{fixture}.json")));
    let actual: Vec<_> = world.index.station_entities.iter().map(|(id, entity)| (id.0, entity.to_string())).collect();
    let expected: Vec<_> =
        expected.iter().enumerate().map(|(index, entity)| (index as u32 + 1, entity.to_string())).collect();
    assert_eq!(actual, expected, "{fixture} station identities");
}

#[test]
fn cencora_v3_station_ids_are_unchanged() {
    assert_station_ids(
        "cencora_v3",
        &[
            "d199b054-b8d6-5a47-8d59-9c9dcecc311f", // 1
            "e4657fb5-083a-558d-bc08-c2756ba2ef29", // 2
            "0c62e4b0-b71f-5a94-9f3d-c850c18fcc06", // 3
            "49461b89-77be-57fd-a94b-7b7da2af70a6", // 4
            "b30c6172-e99a-518a-9f58-36c25b90e155", // 5
            "b64977b0-37ed-5ca0-b041-9a05a08948e4", // 6
            "a1d946c9-c532-5fab-8595-a3ea11ce40c9", // 7
            "90bd4889-745b-565b-8547-bcdf558248a4", // 8
            "7a054a89-f699-5a17-ad78-96ea76bab8a8", // 9
            "f7d19015-d8ea-50c1-af03-e87febc3b631", // 10
            "eb4689b9-880c-5dbb-8480-834f71ecc10b", // 11
            "6acc012e-d45b-5582-bf11-ebf283c0ac6a", // 12
            "890f1fb1-92e6-56e9-a7ad-140a9d9cfa2e", // 13
            "50e764e9-600a-5e4e-927b-c7a90cadc8dd", // 14
            "d61eb514-0980-5d4e-9393-0a4dfe41f37e", // 15
            "86424a96-2f47-5a13-8e5f-410e5c18511a", // 16
            "080effb6-2da5-51cc-b888-dfd7f5d14ed6", // 17
            "b43a85be-e98e-5de1-89f5-20486013c7fa", // 18
            "b3bc65ce-8463-5834-bdf5-1f3b6c79a0ab", // 19
            "2bc95c16-ae78-598d-b935-76acfbb072d3", // 20
            "a6dc0a21-c5b2-52c9-a8d8-b138b2032eb0", // 21
            "5e12e73e-dbbd-5cd7-a1a8-bd0ad00eafe7", // 22
            "aedc2b39-de4d-531d-beb5-f841719f067b", // 23
            "19f9f2aa-1dab-58a6-8646-584c73797106", // 24
            "185383d5-b100-5412-a29d-76509e1fa318", // 25
            "13a4a8bc-f0f4-56d6-adf9-4208d6860cc3", // 26
        ],
    );
}

/// The Dematic copies of the Cencora world, the two DA50 cycle variants and
/// the 21-aisle cut, carry the document of record's stations, so their runs
/// name each station by the same id.
#[test]
fn cencora_v3_da50_station_ids_match_the_document_of_record() {
    let record = load(&document("tests/fixtures/cencora_v3.json")).index.station_entities;
    for fixture in ["cencora_v3_da50_baseline", "cencora_v3_da50_sens6", "cencora_v3_da50_aisles21"] {
        let world = load(&document(&format!("tests/fixtures/{fixture}.json")));
        assert_eq!(world.index.station_entities, record, "{fixture} station identities");
    }
}

#[test]
fn walmart_dfw6_station_ids_are_unchanged() {
    assert_station_ids(
        "walmart_dfw6",
        &[
            "005dd14c-e90a-5947-a57b-94fbce646821", // 1
            "0b72adae-4a62-54ac-a99c-d82acbd0f7fb", // 2
            "5d306357-bafd-5289-a1d7-082a76d5e45a", // 3
            "9bc221fd-b3cf-5034-82c3-71bf17fab587", // 4
            "c0c3d36f-1fd3-57fd-b981-f5a2950a2ad6", // 5
            "b39e8417-a522-5c42-82a6-3f5a01edf3f5", // 6
            "f141da51-8b84-5b50-8830-2463d4216e70", // 7
            "16c6e9cb-ce91-5c47-9d53-e36bcb3eea84", // 8
            "7212b2a4-f507-583d-ae13-d3896463ba87", // 9
            "aacad52c-0690-5730-b7d7-0b1d850cf701", // 10
            "0a353c67-8d70-5fdb-8496-4c7c855354b9", // 11
            "fc5692b9-872d-5c4f-9024-9272cc56bda5", // 12
            "ded41cd0-76fe-5b68-a1b8-2c721816428a", // 13
            "8b693aa6-a755-5ab6-85e4-bab884dc3e63", // 14
            "88757463-0260-5fad-91fb-22d1b3a75b59", // 15
            "86fbfa73-7a5b-5c0d-8da7-7abcf710c7b5", // 16
            "0648873e-bba3-509f-9b36-a942d7dca438", // 17
            "3d9f26ef-23d2-58a0-ab3e-abbd071e1237", // 18
            "caa78a2e-1371-5b78-93b2-be50e696bd11", // 19
            "f945e51e-d1f5-5027-8d78-3d6314b6a69a", // 20
            "48928043-7d54-5fac-8afb-fff6c357bcab", // 21
            "b48f1742-975d-5887-889f-cb4c7fc6eb3b", // 22
            "5be8e84d-50aa-5aa2-84e9-9bc7ac305fec", // 23
            "d0d45430-f9ae-5793-ab33-f45c7d389bc6", // 24
            "5604be5d-6f4d-5e52-9042-93d6fc734d66", // 25
        ],
    );
}
