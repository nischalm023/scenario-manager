//! The shapes expansion invents, pinned where the spec does not name them.
//!
//! SPEC §10 describes the components an *author* writes. Several of the entities
//! a generator mints have no row of their own: `robot` and `carrier` gained one
//! in v0.3 from this crate's reading (SPEC-GAPS 10), while a relay's and a
//! slot's poselessness and what a stamp's `instance:{i}` carries are still this
//! implementation's readings (SPEC-GAPS 8, 9, 11). Pinning the exact shapes here
//! means the next revision of the spec can be checked against something concrete
//! instead of against a recollection, and means a change to them is a deliberate
//! edit rather than a drift.

use std::path::{Path, PathBuf};
use site_model::EntityId;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/seed/application-tests/native/fixtures/walmart-dallas/scenario.json")
}

/// The identity a generator derives at one key: the generator, then the chain of
/// segments beneath it. That address *is* what the derived entity is named by
/// (SPEC §3), so a test writes it exactly as a document would.
fn at(generator: impl std::fmt::Display, key: impl std::fmt::Display) -> EntityId {
    format!("{generator}/{key}")
        .parse()
        .expect("a well-formed derivation address in this test")
}

#[test]
fn generated_entities_carry_the_components_this_reading_gives_them() {
    let (_, world) = site_model::load(&fixture()).expect("the example loads");
    let component = |generator: &str, key: &[&str], name: &str| {
        world.entities[&at(generator, key.join("/"))].components[name].clone()
    };

    let row_a = "d26adfc0-dd23-4cc7-a409-f052e64abfbe";

    // A stamp instance carries only its pose: the stamp's, stepped, unrotated.
    // Instance 1 of rack row A sits one 2750 mm step east of the stamp at
    // [2000, 4000].
    assert_eq!(
        component(row_a, &["instance:1"], "pose"),
        serde_json::json!({
            "frame": "edd0316d-a9bc-48f4-bb37-a1594ced8cd1",
            "position": [4750, 4000],
            "heading_deg": 0.0
        })
    );

    // A relay is a shelf on a bay: which entry of the rack's levels it stands
    // on, how high that is, and where on the bay it sits — 340 along the run and
    // 1300 out, which is the far edge of the bay's depth and so the side it
    // presents on.
    let relay = component(row_a, &["instance:0", "local:rack", "relay:1,0"], "relay");
    assert_eq!(relay["level"], 1);
    assert_eq!(relay["index"], 0);
    assert_eq!(relay["height"], 400);
    assert_eq!(relay["offset"], serde_json::json!([340, 1300]));
    assert_eq!(
        relay["rack"],
        at(row_a, "instance:0/local:rack").to_string()
    );

    // And beside it, the cell an HTM occupies to work it — an ordinary cell of
    // the aisle, named rather than minted. The shelf itself stands at
    // [2340, 5300]; the cell it binds is the aisle tile whose centre is 463
    // away, nearer than any other within the level's 1750 reach.
    let binding = component(
        row_a,
        &["instance:0", "local:rack", "relay:1,0"],
        "access_cell",
    );
    assert_eq!(
        binding,
        serde_json::json!({
            "cell": at("cdda8cce-008d-4c20-b5b7-4d5a816e05c8", "cell:0,0").to_string()
        })
    );

    // A slot is indices plus its rack, and no geometry at all. `level` indexes
    // the rack's whole `levels` list, relay level included, so level 7 is the
    // shelf at 2450 whatever kind of entry sits below it.
    assert_eq!(
        component(row_a, &["instance:0", "local:rack", "slot:7,3,1"], "slot"),
        serde_json::json!({
            "rack": at(row_a, "instance:0/local:rack").to_string(),
            "level": 7, "col": 3, "depth": 1
        })
    );

    // A cohort's robot carries its class, its cohort, its index and the
    // assignment policy it inherits.
    let robot = component(
        "e7ffa31d-5d23-452a-ae2f-9c254ecdea9d",
        &["robot:0"],
        "robot",
    );
    assert_eq!(robot["robot_class"], "5cae603b-9e3e-43e1-b2f3-6d8ce919f110");
    assert_eq!(robot["cohort"], "e7ffa31d-5d23-452a-ae2f-9c254ecdea9d");
    assert_eq!(robot["index"], 0);
    assert_eq!(robot["assignment"]["kind"], "hard");

    // A pool's carrier carries its permanent designation and the pool's
    // placement rule, extra fields of the rule included.
    let carrier = component(
        "700da094-b7a6-4c82-abbe-b7b26bcc3e98",
        &["carrier:41"],
        "carrier",
    );
    assert_eq!(
        carrier["carrier_type"],
        "12cc6d9e-6c7f-4d8e-8443-d1169cd57ddf"
    );
    assert_eq!(carrier["designation"], "inventory");
    assert_eq!(carrier["index"], 41);
    assert_eq!(carrier["placement"]["kind"], "distributed_storage");
    assert_eq!(carrier["placement"]["fill_fraction"], 0.5);
}

#[test]
fn a_stamped_region_composes_into_the_world_frame() {
    let (_, world) = site_model::load(&fixture()).expect("the example loads");
    let easyloop = "b83f97cc-9ff1-49ec-b547-5c1a1ff88411";
    // The EasyLoop template puts its order queue at local [0, 900]; the stamp
    // anchors the template at [23000, 5000] with no rotation. (It sat at 22000
    // until the queue region was found overlapping the station spine, which is
    // what the grid-projection check surfaced.)
    let queue = &world.entities[&at(easyloop, "instance:0/local:order_queue")];
    assert_eq!(
        queue.components["region"]["origin"],
        serde_json::json!([23000, 5900])
    );
    assert_eq!(
        queue.components["region"]["frame"],
        "edd0316d-a9bc-48f4-bb37-a1594ced8cd1"
    );
    // And its first cell sits half a pitch in from that composed origin.
    let cell = &world.entities[&at(easyloop, "instance:0/local:order_queue/cell:0,0")];
    assert_eq!(
        cell.components["pose"]["position"],
        serde_json::json!([23450, 6350])
    );
}

#[test]
fn a_templates_internal_stitch_is_rewritten_to_its_own_instance() {
    let (_, world) = site_model::load(&fixture()).expect("the example loads");
    let easyloop = "b83f97cc-9ff1-49ec-b547-5c1a1ff88411";
    let stitch = &world.entities[&at(easyloop, "instance:0/local:waiting_to_queue")];
    // `{"local_ref": "waiting_block"}` became the UUID that handle got in this
    // instance — the station wires its own organs together.
    assert_eq!(
        stitch.components["stitch"]["a"],
        at(easyloop, "instance:0/local:waiting_block").to_string()
    );
    assert_eq!(
        stitch.components["stitch"]["b"],
        at(easyloop, "instance:0/local:order_queue").to_string()
    );
    // And it produced the two links across the 300 mm internal seam.
    assert_eq!(
        world.edges.iter().filter(|e| e.owner == stitch.id).count(),
        2
    );
}

#[test]
fn a_port_inside_a_template_points_at_its_own_stations_uuid() {
    let (_, world) = site_model::load(&fixture()).expect("the example loads");
    let easyloop = "b83f97cc-9ff1-49ec-b547-5c1a1ff88411";
    let ingress = &world.entities[&at(easyloop, "instance:0/local:inventory_ingress")];
    assert_eq!(
        ingress.components["port"]["into"],
        at(easyloop, "instance:0/local:station").to_string()
    );
    // The product-specific component nobody here understands rode through
    // untouched, parameters substituted.
    let station = &world.entities[&at(easyloop, "instance:0/local:station")];
    assert_eq!(station.components["go.easyloop"]["loop_transit_secs"], 45);
    assert_eq!(station.components["go.easyloop"]["time_per_pick_secs"], 9);
}
