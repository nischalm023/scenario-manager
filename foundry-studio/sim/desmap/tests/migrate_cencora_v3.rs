//! Migration of the cencora_v3 fixture's rack prototype from one full-run
//! definition to the physical bay unit (the four-legged frame) tiled by
//! placements, with the equivalence proof that both compile to the same
//! `Zone.slots` content.
//!
//! (An earlier revision of this file hoisted the pre-prototype inline
//! per-zone `slots` into the full-run prototype; see git history.)
//!
//! `migrate` is a `serde_json::Value`-level transform. The non-ignored test
//! is the proof; the `#[ignore]` test rewrites the fixture file in place
//! (run once, commit the result).

use desmap::WarehouseDesign;
use desmap::design::SlotDef;
use serde_json::{Value, json};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/cencora_v3.json");
const RUN_NAME: &str = "cencora_a5_row";
const BAY_NAME: &str = "cencora_a5_bay";
const BAY_MM: f64 = 2140.0;
const BAYS_PER_RUN: usize = 6;

/// Split the full-run prototype into the bay unit and re-point every zone's
/// placements at 6 tilings per side. Returns false if the fixture no longer
/// carries the full-run prototype (already migrated).
fn migrate(root: &mut Value) -> bool {
    let racks = root["racks"].as_array().expect("racks array");
    let Some(run) = racks.iter().find(|r| r["name"] == RUN_NAME) else {
        return false;
    };
    let slots = run["slots"].as_array().expect("run slots");
    assert_eq!(slots.len(), 396, "unexpected full-run slot count");

    // Rebase each slot into its bay's bottom-left frame and check every bay
    // is the same physical unit. Filtering preserves the prototype's
    // level-major order within each bay.
    let mut bays: Vec<Vec<Value>> = vec![Vec::new(); BAYS_PER_RUN];
    for slot in slots {
        let x = slot["x"].as_f64().expect("slot x");
        let bay = ((x / BAY_MM).floor() as usize).min(BAYS_PER_RUN - 1);
        let mut local = slot.clone();
        local["x"] = json!(x - bay as f64 * BAY_MM);
        bays[bay].push(local);
    }
    for (bay, local) in bays.iter().enumerate() {
        assert_eq!(local, &bays[0], "bay {bay} differs from bay 0; not a uniform tiling");
    }

    root["racks"] = json!([{ "name": BAY_NAME, "length_mm": BAY_MM, "slots": bays[0] }]);
    for zone in root["zones"].as_array_mut().unwrap() {
        if zone["type"] != "relay" {
            continue;
        }
        let mut placements = Vec::new();
        for side in ["Left", "Right"] {
            for bay in 0..BAYS_PER_RUN {
                let mut p = json!({ "rack": BAY_NAME, "side": side });
                if bay > 0 {
                    p["x_offset_mm"] = json!(bay as f64 * BAY_MM);
                }
                placements.push(p);
            }
        }
        zone["rack_placements"] = Value::Array(placements);
    }
    true
}

fn compiled_zone_slots(design: &WarehouseDesign) -> Vec<(u32, Vec<SlotDef>)> {
    let map = design.compile().expect("compile");
    let mut zones: Vec<_> = map.zones.iter().filter_map(|z| z.slots.as_ref().map(|s| (z.id.0, s.clone()))).collect();
    zones.sort_by_key(|(id, _)| *id);
    zones
}

/// Sort a compiled slot list into a canonical order for content comparison.
/// Tiling by bay changes the materialization ORDER within a rack column
/// (bay-major instead of level-major) — deliberately — so equivalence is
/// per-zone multiset equality, not list equality.
fn canonical(mut slots: Vec<SlotDef>) -> Vec<SlotDef> {
    slots.sort_by(|a, b| {
        (a.cell, a.diving_board, a.diving_board_slot)
            .cmp(&(b.cell, b.diving_board, b.diving_board_slot))
            .then(a.x.total_cmp(&b.x))
            .then(a.y.total_cmp(&b.y))
    });
    slots
}

/// The identity proof: the full-run prototype and the tiled bay prototype
/// compile to the same slot content for every zone.
#[test]
fn tiled_bays_compile_to_the_same_slot_content() {
    let text = std::fs::read_to_string(FIXTURE).expect("read fixture");

    let mut old_root: Value = serde_json::from_str(&text).expect("parse fixture");
    let mut root = old_root.clone();
    if !migrate(&mut root) {
        // Fixture already migrated; the golden structural test covers it.
        return;
    }

    // The run-form fixture predates `length_mm`; declare the run envelope
    // (6 touching bays) so it parses under the current schema.
    old_root["racks"][0]["length_mm"] = json!(BAY_MM * BAYS_PER_RUN as f64);
    let old: WarehouseDesign = serde_json::from_value(old_root).expect("run-form design");
    let new: WarehouseDesign = serde_json::from_value(root).expect("bay-form design");

    let old_slots = compiled_zone_slots(&old);
    let new_slots = compiled_zone_slots(&new);
    assert_eq!(old_slots.len(), 19, "expected 19 relay zones with slots");
    assert_eq!(new_slots.len(), 19);
    for ((old_id, old), (new_id, new)) in old_slots.into_iter().zip(new_slots) {
        assert_eq!(old_id, new_id);
        assert_eq!(old.len(), new.len(), "zone {old_id}: slot count differs");
        assert_eq!(canonical(old), canonical(new), "zone {old_id}: slot content differs from run form");
    }
}

/// Re-render slot objects one per line, the fixture's established style
/// (every other structure is plain `to_string_pretty`, which the fixture's
/// non-slot regions reproduce byte-for-byte). Slot objects hold only scalar
/// values, so each pretty-printed object is a `{` line, key lines, `}` line;
/// joining them with single spaces yields `{"x": 425.0, "y": 394.0}`.
fn compact_slot_objects(pretty: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_slots = false;
    let mut object: Option<String> = None;
    for line in pretty.lines() {
        let trimmed = line.trim();
        if !in_slots {
            if trimmed == "\"slots\": [" {
                in_slots = true;
            }
            out.push(line.to_string());
            continue;
        }
        match (&mut object, trimmed) {
            (None, "]" | "],") => {
                in_slots = false;
                out.push(line.to_string());
            }
            (None, "{") => object = Some(format!("{}{{", &line[..line.len() - 1])),
            (Some(buffer), "}" | "},") => {
                buffer.push('}');
                if trimmed == "}," {
                    buffer.push(',');
                }
                out.push(object.take().unwrap());
            }
            (Some(buffer), field) => {
                if !buffer.ends_with('{') {
                    buffer.push(' ');
                }
                buffer.push_str(field);
            }
            (None, other) => panic!("unexpected line in slots array: {other:?}"),
        }
    }
    out.join("\n") + "\n"
}

/// Rewrite the fixture file to the tiled-bay form. Run explicitly:
/// `cargo test -p desmap --test migrate_cencora_v3 -- --ignored`
#[test]
#[ignore = "rewrites the fixture in place; run once and commit"]
fn rewrite_fixture_to_tiled_bays() {
    let text = std::fs::read_to_string(FIXTURE).expect("read fixture");
    let mut root: Value = serde_json::from_str(&text).expect("parse fixture");
    if !migrate(&mut root) {
        return; // already migrated
    }
    // The migrated design must still compile before we overwrite anything.
    let design: WarehouseDesign = serde_json::from_value(root.clone()).expect("bay-form design");
    design.compile().expect("bay-form design compiles");

    let out = compact_slot_objects(&(serde_json::to_string_pretty(&root).expect("serialize") + "\n"));
    // The re-render is formatting only.
    let reparsed: Value = serde_json::from_str(&out).expect("reparse output");
    assert_eq!(reparsed, root, "compact rendering changed content");
    std::fs::write(FIXTURE, out).expect("write fixture");
}
