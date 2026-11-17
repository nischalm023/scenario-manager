//! Contracts from Foundry's former embedded lens copy. The preserved inputs
//! retain go.easyloop vocabulary; the shared matcher follows explicit references.
//! Older literal counts had drifted behind these checked-in fixtures. Updated
//! pins were verified with the previous lens implementation, independently of
//! this matcher.
mod fixtures;
use std::path::Path;
use site_model::pipeline::World;
use fixtures::Fixture;
use std::collections::BTreeSet;
use worldlens::{Library, Matcher, Pattern};
use site_model::components::spatial::Pose;
fn library() -> Library {
    Library::parse(foundry_world_lenses::WORLD_LENS).unwrap()
}
fn pattern(text: &str) -> Pattern {
    Library::parse(text).unwrap().patterns().next().unwrap().clone()
}
fn document(name: &str) -> &'static Fixture {
    use std::sync::OnceLock;
    static FIXTURES: [OnceLock<Fixture>; 4] = [const { OnceLock::new() }; 4];
    let (index, name, file) = match name {
        "walmart-dfw6" => (0, "walmart-dfw6", "lenses/legacy-foundry/walmart_dfw6.json"),
        "minimal-easyloop" => (1, "minimal-easyloop", "lenses/legacy-foundry/minimal_easyloop.json"),
        "cencora-v3" => (2, "cencora-v3", "native/fixtures/cencora-v3/cencora_v3.json"),
        "walmart-dallas" => (3, "walmart-dallas", "native/fixtures/walmart-dallas/scenario.json"),
        _ => panic!("unknown legacy fixture {name}"),
    };
    FIXTURES[index].get_or_init(|| {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../data/seed/application-tests")
            .join(file);
        let (layers, world) = site_model::load(&path).unwrap();
        Fixture { name, layers, world }
    })
}

fn corners() -> Fixture {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/seed/application-tests/lenses/legacy-foundry/corner-cases.json");
    let (layers, world) = site_model::load(&path).unwrap();
    Fixture { name: "legacy-corners", layers, world }
}
fn count(world: &World, text: &str) -> usize {
    Matcher::new(&world.entities).matches(&pattern(text)).len()
}

#[test]
fn dfw6_names_the_point_of_every_station_it_has() {
    let Fixture { world, .. } = document("walmart-dfw6");
    let library = library();
    let lens = Matcher::new(&world.entities);
    assert_eq!(lens.matches(library.get("placed_station").expect("the pattern")).len(), 17);
    assert_eq!(lens.matches(library.get("presentation_cell").expect("the pattern")).len(), 17);
    // Ten points ARE cells: the four machines' eight pick places, which name no
    // station, and box induction's two, which name theirs. Other station
    // presentation points remain explicit referenced entities.
    assert_eq!(lens.matches(library.get("presentation_marked_cell").expect("the pattern")).len(), 10);
    assert_eq!(lens.matches(library.get("machine_pick_place").expect("the pattern")).len(), 8);
}

#[test]
fn a_machine_is_a_frame_on_dfw6_and_a_stamp_in_the_catalog() {
    let library = library();
    let Fixture { world, .. } = document("walmart-dfw6");
    let lens = Matcher::new(&world.entities);
    assert_eq!(lens.matches(library.get("easyloop_internal_of").expect("the pattern")).len(), 0);
    assert_eq!(lens.matches(library.get("easyloop_machine").expect("the pattern")).len(), 0);
    let machines = lens.matches(library.get("machine").expect("the pattern"));
    assert_eq!(machines.len(), 4, "four frames, each carrying its own envelope and belt timing");
    for binding in &machines {
        let id = binding.get("m").expect("the anchor bound");
        assert!(world.entities[id].has("frame") && !world.entities[id].has("stamp"));
    }
    assert_eq!(lens.matches(library.get("machine_cell").expect("the pattern")).len(), 4 * 52);
    assert_eq!(lens.matches(library.get("machine_waiting_cell").expect("the pattern")).len(), 4 * 12);

    let Fixture { world: dallas, .. } = document("walmart-dallas");
    let lens = Matcher::new(&dallas.entities);
    let parts = lens.matches(library.get("easyloop_internal_of").expect("the pattern"));
    let stamped = lens.matches(library.get("easyloop_machine").expect("the pattern"));
    assert_eq!((parts.len(), stamped.len()), (1, 1), "nesting still folds a stamp's parts into it");
    for binding in &stamped {
        assert!(dallas.entities[binding.get("m").expect("the anchor bound")].has("stamp"));
    }
}

#[test]
fn a_stamp_is_not_one_of_its_instances() {
    let library = library();
    for (name, stamps, instances) in [("cencora-v3", 182usize, 372usize), ("walmart-dfw6", 448, 892)] {
        let Fixture { world, .. } = document(name);
        let lens = Matcher::new(&world.entities);
        assert_eq!(world.entities.values().filter(|e| e.has("stamp")).count(), stamps);
        assert_eq!(lens.matches(library.get("stamp_instance").expect("the pattern")).len(), instances);
    }
}

#[test]
fn every_relay_stands_on_exactly_one_cell() {
    let library = library();
    for (name, relays) in [("cencora-v3", 1368usize), ("walmart-dfw6", 2132)] {
        let Fixture { world, .. } = document(name);
        let lens = Matcher::new(&world.entities);
        assert_eq!(lens.matches(library.get("rack_bay_with_relays").expect("the pattern")).len(), relays);
        assert!(lens.matches(library.get("unbound_relay").expect("the pattern")).is_empty());
        assert_eq!(world.entities.values().filter(|e| e.has("relay")).count(), relays);
    }
}

#[test]
fn the_minimal_easyloop_map_reads_end_to_end() {
    let Fixture { world, .. } = document("minimal-easyloop");
    let library = library();
    let lens = Matcher::new(&world.entities);
    let count = |name: &str| lens.matches(library.get(name).expect("the pattern")).len();

    assert_eq!(count("machine"), 1, "one entity IS the machine");
    assert_eq!(count("machine_cell"), 52, "fifty-two cells are posed in its frame");
    assert_eq!(count("machine_pick_place"), 2, "two pick places, each a cell carrying the happening");
    assert_eq!(count("machine_port"), 2, "one dropoff and one collection cell");
    assert_eq!(count("machine_waiting_cell"), 12, "twelve shared waiting cells");
    assert_eq!(count("lane"), 4, "the inventory loop and the three stations' lanes");
    assert_eq!(count("lane_cell"), 19, "every step of them names a cell by identity");
    assert_eq!(count("easyloop_machine"), 0, "nothing here is stamped");
    assert_eq!(count("machine_station"), 0, "and nothing here carries `station` but the three that are not machines");

    assert_eq!(count("placed_station"), 3, "inventory induction, box induction, extraction");
    assert_eq!(count("presentation_cell"), 3, "the three that name their station");
    assert_eq!(count("presentation_marked_cell"), 2, "and the machine's two, which name none");
    assert_eq!(count("rack_bay"), 11);
    assert_eq!(count("rack_bay_with_relays"), 11, "one shelf each, each standing on one cell");
    assert_eq!(count("storage_position"), 55, "five levels a bay");

    // Simulator rack anchors are outside the document and therefore outside
    // this predicate's census. It sees only the stated floor and machine cells.
    assert_eq!(count("document_cell"), 266);
}

#[test]
fn an_optional_variable_leaves_a_row_rather_than_dropping_it() {
    let Fixture { world, .. } = document("walmart-dfw6");
    let lens = Matcher::new(&world.entities);
    let left_join =
        pattern(r#"pattern p { c: presentation_point; s?: station, pose; ref(c, s, "presentation_point.station"); }"#);
    let bindings = lens.matches(&left_join);
    assert_eq!(bindings.len(), 25, "one row per presentation point");
    assert_eq!(bindings.iter().filter(|b| b.get("s").is_some()).count(), 17);
    assert_eq!(bindings.iter().filter(|b| b.get("s").is_none()).count(), 8);
    // Results are ordered by the bound identities in declaration order, and an
    // absent optional sorts before every present one — so anchoring the same
    // question at the optional variable puts the eight stationless points first.
    let anchored_at_the_station =
        pattern(r#"pattern p { s?: station, pose; c: presentation_point; ref(c, s, "presentation_point.station"); }"#);
    let by_station = lens.matches(&anchored_at_the_station);
    assert_eq!(by_station.len(), 25);
    assert!(by_station[..8].iter().all(|b| b.get("s").is_none()));
    assert!(by_station[8..].iter().all(|b| b.get("s").is_some()));
    // Required, the same pattern drops the eight.
    let inner_join =
        pattern(r#"pattern p { c: presentation_point; s: station, pose; ref(c, s, "presentation_point.station"); }"#);
    assert_eq!(lens.matches(&inner_join).len(), 17);
}

#[test]
fn the_longest_component_name_claims_a_dotted_path() {
    let Fixture { world, .. } = corners();
    let lens = Matcher::new(&world.entities);
    let both = pattern(r#"pattern p { e: go.easyloop; s: any; ref(e, s, "go.easyloop.station"); }"#);
    let bindings = lens.matches(&both);
    assert_eq!(bindings.len(), 1);
    assert_eq!(
        bindings[0].get("s").expect("bound").to_string(),
        "44444444-4444-4444-8444-444444444444",
        "`go.easyloop` is the component, so its own `station` field is what was read"
    );
    // The shorter name is still readable, spelled as its own path.
    let shorter = pattern(r#"pattern p { e: go; s: any; ref(e, s, "go.easyloop"); }"#);
    assert_eq!(lens.matches(&shorter).len(), 0, "`go.easyloop` there is an object, not an identity");
    // And a field test inside a dot-namespaced component is relative to that
    // component, which is the same rule read the other way.
    assert_eq!(count(&world, r#"pattern p { e: go.easyloop(part == "envelope"); }"#), 1);
    assert_eq!(count(&world, r#"pattern p { e: go.easyloop(go.easyloop.part == "envelope"); }"#), 0);
}

#[test]
fn simulator_pattern_names_and_binding_variables_are_preserved() {
    let library = library();
    assert_eq!(library.get("machine").unwrap().variables().collect::<Vec<_>>(), ["m"]);
    for name in ["machine_cell", "machine_pick_place", "machine_port", "machine_waiting_cell"] {
        assert_eq!(library.get(name).unwrap().variables().collect::<Vec<_>>(), ["c", "m"], "{name}");
    }
    for (site, counts) in [("walmart-dfw6", [208, 8, 8, 48]), ("minimal-easyloop", [52, 2, 2, 12])] {
        let fixture = document(site);
        let matcher = Matcher::new(&fixture.world.entities);
        for (name, expected) in
            ["machine_cell", "machine_pick_place", "machine_port", "machine_waiting_cell"].into_iter().zip(counts)
        {
            let rows = matcher.matches(library.get(name).unwrap());
            assert_eq!(rows.len(), expected, "{site}: {name}");
            let actual: BTreeSet<_> = rows
                .iter()
                .map(|row| {
                    (
                        row.get("m").expect("consumer machine binding").clone(),
                        row.get("c").expect("consumer cell binding").clone(),
                    )
                })
                .collect();
            // Independent native frame-graph census: this is the previous
            // consumer's membership calculation, not a second lens query.
            let mut native = BTreeSet::new();
            for (cell, entity) in &fixture.world.entities {
                if !fixture.world.cells.contains_key(cell) {
                    continue;
                }
                let eligible = match name {
                    "machine_pick_place" => entity.has("pick") && entity.has("presentation_point"),
                    "machine_port" => entity.has("port"),
                    "machine_waiting_cell" => entity.has("waiting"),
                    _ => true,
                };
                if !eligible {
                    continue;
                }
                let Some(frame) = entity.component::<Pose>("pose").unwrap().and_then(|p| p.frame) else {
                    continue;
                };
                for ancestor in fixture.world.frames.ancestry(&frame).unwrap() {
                    if fixture.world.entities[&ancestor]
                        .raw("go.easyloop")
                        .and_then(|c| c.get("loop_transit_secs"))
                        .is_some()
                    {
                        native.insert((ancestor, cell.clone()));
                    }
                }
            }
            assert_eq!(actual, native, "{site}: {name} entity pairs match native frame ancestry");
        }
    }
}
#[test]
fn removed_spatial_inference_relations_are_explicitly_refused() {
    // These were tested in the embedded legacy copy. They were removed from
    // the shared language: runtime matching consumes recorded relationships.
    for clause in
        ["nearest(a,b,b)", "nearest(a,b,b,900)", "adjacent(a,b)", "stitched(a,b)", "in_region(a,b)", "in_frame(a,b)"]
    {
        let text = format!("pattern p {{ a: any; b: any; {clause}; }}");
        let error = Library::parse(&text).unwrap_err();
        assert!(error.message.contains("is no relation of this language"), "{clause}: {error}");
    }
}
