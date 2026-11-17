//! The structure report, held to the two things it promises.
//!
//! The first is that its numbers are the text output's numbers — a report that
//! disagreed with the tool that wrote it would be worse than no report, because
//! the disagreement would be invisible to whoever was sent only the file. The
//! second is that a document cannot write markup into it: names, paths and
//! component contents are the one part of the page that comes from outside, and
//! they arrive as text or they arrive as elements, and only one of those is a
//! report.
//!
//! The counts below are the ones `site-model-check` prints for the Dallas
//! fixture, copied from its output rather than from the report's own.

use std::collections::BTreeMap;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use site_model::pipeline::derive;
use site_model::pipeline::layers::LayerSet;
use site_model::report::model;
use site_model::report::model::Node;
use site_model::{report, Document, EntityId, World};

/// The identity a generator derives at one key: the generator, then the chain of
/// segments beneath it. That address *is* what the derived entity is named by
/// (SPEC §3), so a test writes it exactly as a document would.
fn at(generator: impl std::fmt::Display, key: impl std::fmt::Display) -> EntityId {
    format!("{generator}/{key}")
        .parse()
        .expect("a well-formed derivation address in this test")
}

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/seed/application-tests/native/fixtures/walmart-dallas/scenario.json")
}

fn dallas() -> (LayerSet, World) {
    site_model::load(&example()).expect("the example loads")
}

/// The JSON the page embeds for its script, pulled back out of the file the
/// way the browser pulls it out: everything between the data element's tag and
/// the first `</script>` after it. Nothing a document says can end that element
/// early, which is half of what this file is here to check.
fn embedded(html: &str) -> Value {
    let opened = html
        .find("<script type=\"application/json\" id=\"world-data\">")
        .expect("the page embeds its data");
    let start = opened + html[opened..].find('>').expect("the tag closes") + 1;
    let end = start + html[start..].find("</script>").expect("the element ends");
    serde_json::from_str(&html[start..end]).expect("the embedded data is JSON")
}

#[test]
fn the_reports_numbers_are_the_numbers_the_text_output_prints() {
    let (set, world) = dallas();
    let page = model::build(&example(), &set, &world);
    let s = &page.summary;

    assert_eq!(s.errors, 0);
    // The sixteen tiles of aisle that take two diving boards apiece; `dallas.rs`
    // does the arithmetic.
    assert_eq!(s.warnings, 16);
    assert_eq!(s.layers, 5);
    assert_eq!(s.entities, 4370);
    assert_eq!(s.declared, 51);
    assert_eq!(s.derived, 4319);
    assert_eq!(s.edges, 364);
    assert_eq!(s.flow_arcs, 349);
    assert_eq!(s.stitch_links, 15);
    assert_eq!(s.suppressed, 7);
    assert_eq!(s.generators, 20);
    // One name fewer than v0.7 counted: `exclusion` is not a component any
    // more, and the carve-out it stood for rides inside `tiling`.
    assert_eq!(s.components, 30);

    // And the same numbers read off the world itself, so the two cannot drift
    // apart even if the example changes.
    assert_eq!(s.entities, world.entities.len());
    assert_eq!(s.derived, world.derived_count());
    assert_eq!(s.declared, world.entities.len() - world.derived_count());
    assert_eq!(s.edges, world.edges.len());
    assert_eq!(s.flow_arcs + s.stitch_links, world.edges.len());
    assert_eq!(s.suppressed, world.suppressions.len());
    assert_eq!(s.warnings, world.warnings.len());
    assert_eq!(s.generators, world.counts.len());
    assert_eq!(s.layers, set.layers.len());
}

/// A warning reaches the page as the loader's own sentence, not as a summary of
/// one — which is the whole reason to send somebody the file.
#[test]
fn the_warnings_are_the_loaders_own_words() {
    let (set, world) = dallas();
    let spoken: Vec<String> = world.warnings.iter().map(ToString::to_string).collect();
    assert_eq!(model::build(&example(), &set, &world).warnings, spoken);

    let (set, world) = folded_lattice();
    let path = Path::new("<folded>");
    let spoken: Vec<String> = world.warnings.iter().map(ToString::to_string).collect();
    assert_eq!(spoken.len(), 1, "{spoken:?}");
    assert_eq!(model::build(path, &set, &world).warnings, spoken);

    let html = report::render(path, &set, &world);
    for warning in &spoken {
        let escaped = warning.replace('&', "&amp;").replace('<', "&lt;");
        assert!(
            html.contains(&escaped),
            "the page drops a warning: {warning}"
        );
    }
}

/// One floor, one hand-declared cell standing on it, and no stitch to say the
/// two are one place: the lattice folds them together and the loader says so.
fn folded_lattice() -> (LayerSet, World) {
    let site = "44d924cd-e36e-4da9-b7ac-401b932bf445";
    let floor = "edd0316d-a9bc-48f4-bb37-a1594ced8cd1";
    let value = json!({
        "proto_version": "0.7.0",
        "doc": { "id": "3cbd200e-580a-40d8-b5ee-edf406aa1c88", "name": "folded lattice" },
        "primary_frame": site,
        "required_components": ["frame", "region", "tiling"],
        "entities": [
            { "id": site, "components": {
                "frame": { "dims": 3, "units": "mm", "unit_mm": 1 } } },
            { "id": floor, "components": { "frame": {
                "parent": site, "dims": 2, "units": "index", "integer_only": true,
                "transform_to_parent": { "translation": [0, 0], "scale": [750, 750] } } } },
            { "id": "272542c8-2ea2-441f-8807-8a37aafc8f10", "components": {
                "region": { "frame": site, "origin": [0, 0], "size": [3000, 3000] },
                "tiling": { "pitch": [1000, 1000],
                            "derives": { "cell": { "traversable_by": ["htm"] } } } } },
            { "id": "fed8af20-b454-4764-bea8-123b0a780724", "components": {
                "pose": { "frame": site, "position": [1500, 1500] },
                "cell": { "traversable_by": ["htm"] } } }
        ]
    });
    let scenario = Document::from_value(Path::new("<folded>"), &value).expect("it parses");
    let set = LayerSet {
        scenario,
        layers: Vec::new(),
    };
    let world = derive(&set).expect("it loads");
    (set, world)
}

#[test]
fn every_entity_is_in_the_tree_exactly_once() {
    let (set, world) = dallas();
    let page = model::build(&example(), &set, &world);

    let mut seen: Vec<usize> = page.declared.clone();
    for generator in &page.generators {
        collect(&generator.children, &mut seen);
    }
    seen.sort_unstable();
    let expected: Vec<usize> = (0..page.entities.len()).collect();
    assert_eq!(seen, expected, "the tree is not the entity set");
}

fn collect(nodes: &[Node], out: &mut Vec<usize>) {
    for node in nodes {
        if let Some(entity) = node.entity {
            out.push(entity);
        }
        collect(&node.children, out);
    }
}

#[test]
fn every_generators_subtree_holds_the_number_of_entities_it_claims() {
    let (set, world) = dallas();
    let page = model::build(&example(), &set, &world);
    assert_eq!(page.generators.len(), world.counts.len());

    let mut total = 0;
    for generator in &page.generators {
        let mut entities = Vec::new();
        collect(&generator.children, &mut entities);
        assert_eq!(
            entities.len(),
            generator.count,
            "generator {} claims {} derived entities and its subtree holds {}",
            generator.id,
            generator.count,
            entities.len()
        );
        let mut expected = BTreeMap::new();
        for entity in
            world.entities.values().filter(|entity| entity.is_derived() && entity.id.root().to_string() == generator.id)
        {
            for name in entity.components.keys() {
                *expected.entry(name.clone()).or_insert(0usize) += 1;
            }
        }
        assert_eq!(generator.components, expected.into_iter().collect::<Vec<_>>());
        total += generator.count;
    }
    assert_eq!(total, page.summary.derived);
}

#[test]
fn a_slot_points_at_its_rack_and_the_rack_knows_the_way_back() {
    let (set, world) = dallas();
    let page = model::build(&example(), &set, &world);

    // Rack row A's first bay, its rack internal, and that rack's first slot.
    let rack = at(
        "d26adfc0-dd23-4cc7-a409-f052e64abfbe",
        "instance:0/local:rack",
    );
    let slot = at(&rack, "slot:0,0,0");

    let position_of = |id: EntityId| {
        page.entities
            .iter()
            .position(|entity| entity.id == id.to_string())
            .unwrap_or_else(|| panic!("{id} is in the report"))
    };
    let references = &page.entities[position_of(slot)].references;
    let kind = &page.reference_kinds[references[0]];
    assert_eq!(kind, "slot.rack");
    assert_eq!(references[1], position_of(rack));
}

#[test]
fn a_queue_path_is_a_chain_of_links_in_the_order_it_is_walked() {
    let (set, world) = dallas();
    let page = model::build(&example(), &set, &world);

    // The EasyLoop's order queue runs four cells this scenario -- the policy
    // overlay admits fewer than the template's six by naming fewer -- and the
    // report has to name them in that order: a queue is a path and its order
    // is the data.
    let queue = page
        .entities
        .iter()
        .find(|entity| entity.components.contains_key("queue"))
        .expect("the example has a queue");
    let steps: Vec<&String> = queue
        .references
        .chunks(2)
        .map(|pair| &page.reference_kinds[pair[0]])
        .filter(|kind| *kind == "queue.path")
        .collect();
    assert_eq!(steps.len(), 4);

    let path = queue.components["queue"]["path"]
        .as_array()
        .expect("a queue path is an array");
    assert_eq!(path.len(), steps.len());
}

#[test]
fn two_runs_over_one_scenario_write_the_same_bytes() {
    let (set, world) = dallas();
    assert_eq!(
        report::render(&example(), &set, &world),
        report::render(&example(), &set, &world)
    );
}

#[test]
fn a_name_carrying_markup_arrives_as_text() {
    let hostile = "</script><script>alert(1)</script>";
    let value = json!({
        "proto_version": "0.7.0",
        "doc": { "id": "da000000-0000-4000-8000-000000000001", "name": hostile },
        "primary_frame": "f0000000-0000-4000-8000-000000000002",
        "entities": [
            { "id": "f0000000-0000-4000-8000-000000000002",
              "components": { "frame": { "dims": 2, "units": "mm", "unit_mm": 1 } } },
            { "id": "a0000000-0000-4000-8000-000000000001",
              "name": hostile,
              "components": {
                  "region": { "frame": "f0000000-0000-4000-8000-000000000002",
                              "origin": [0, 0], "size": [900, 900] },
                  "tiling": { "pitch": [900, 900],
                              "derives": { "cell": { "traversable_by": ["htm"] } } },
                  "external_ids": { "desmap_zone": "<img src=x onerror=alert(2)>" }
              } }
        ]
    });
    let scenario = Document::from_value(Path::new(hostile), &value).expect("the document parses");
    let set = LayerSet {
        scenario,
        layers: Vec::new(),
    };
    let world = derive(&set).expect("the world derives");
    let html = report::render(Path::new(hostile), &set, &world);

    // The page opens exactly two elements of its own — the stylesheet and the
    // script — and a document's text opens none: the only `<` that survive from
    // it are the ones the browser will show as `<`, and inside the data element
    // they are JSON escapes rather than characters at all.
    assert!(
        !html.contains("<script>alert(1)"),
        "a name became an element"
    );
    assert!(!html.contains("<img"), "a component became an element");
    assert!(html.contains("&lt;/script&gt;&lt;script&gt;alert(1)"));
    assert_eq!(
        html.matches("<script").count(),
        2,
        "the page holds only its own two script elements"
    );

    // The data element ends where the generator ended it, and the name is
    // still the name once a JSON parser has read it back.
    let data = embedded(&html);
    assert_eq!(data["scenario"]["name"], hostile);
    let region = data["entities"]
        .as_array()
        .expect("the entity array")
        .iter()
        .find(|entity| entity["c"].get("external_ids").is_some())
        .expect("the one entity carrying legacy ids");
    assert_eq!(region["n"], hostile);
    assert_eq!(
        region["c"]["external_ids"]["desmap_zone"],
        "<img src=x onerror=alert(2)>"
    );
}

#[test]
fn a_scenario_that_will_not_load_still_gets_a_report() {
    let missing = Path::new("no/such/scenario.json");
    let error = site_model::load(missing).expect_err("there is no such file");
    let html = report::render_failure(missing, &error);
    let data = embedded(&html);

    // One error, the way the text output counts them, and the loader's own
    // sentence — which is the whole reason to send somebody this file.
    assert_eq!(data["summary"]["errors"], 1);
    assert_eq!(data["errors"][0], error.to_string());
    assert_eq!(data["summary"]["entities"], 0);
    assert!(html.contains(&error.to_string().replace('<', "&lt;")));
}
