//! Application inputs continue to demonstrate the language contracts after
//! moving ownership; generic corner graphs also cover these in WPS.
mod fixtures;
use fixtures::{corners, document, Fixture};
use serde_json::from_value;
use std::collections::BTreeSet;
use worldlens::{Library, Matcher, Pattern};
use site_model::pipeline::World;
use site_model::EntityId;
fn pattern(text: &str) -> Pattern {
    Library::parse(text).unwrap().patterns().next().unwrap().clone()
}
fn count(world: &World, text: &str) -> usize {
    Matcher::new(&world.entities).matches(&pattern(text)).len()
}

#[test]
fn a_walk_of_one_field_answers_what_a_reference_answers() {
    let Fixture { world, .. } = document("cencora-v3");
    let one_step = r#"pattern p { c: presentation_point; s: station; ref(c, s, "presentation_point.station"); }"#;
    let walked = r#"pattern p { c: presentation_point; s: station; ref_walk(c, s, "presentation_point.station"); }"#;
    let matcher = Matcher::new(&world.entities);
    let printed = |text: &str| -> Vec<String> {
        matcher.matches(&pattern(text)).iter().map(|binding| format!("{binding}")).collect()
    };
    assert_eq!(printed(one_step), printed(walked));
    assert_eq!(printed(walked).len(), 26);
}

#[test]
fn a_walk_of_two_fields_climbs_the_chain_they_name() {
    let Fixture { world, .. } = document("minimal-putaway2");
    let parent_of = |id: &EntityId| -> Option<EntityId> {
        let parent = world.entities.get(id)?.raw("frame")?.get("parent")?.clone();
        from_value(parent).ok()
    };
    let mut expected = 0usize;
    let mut two_steps_up = 0usize;
    for entity in world.entities.values() {
        if !entity.has("cell") {
            continue;
        }
        let Some(stated) = entity.raw("pose").and_then(|p| p.get("frame")).cloned() else {
            continue;
        };
        let Ok(stated) = from_value::<EntityId>(stated) else {
            continue;
        };
        let mut reached: BTreeSet<EntityId> = BTreeSet::new();
        let mut cursor = Some(stated);
        let mut steps = 0usize;
        while let Some(current) = cursor {
            if !reached.insert(current.clone()) {
                break;
            }
            steps += 1;
            if steps > 1 && world.entities.get(&current).is_some_and(|e| e.has("frame")) {
                two_steps_up += 1;
            }
            cursor = parent_of(&current);
        }
        expected += reached.iter().filter(|id| world.entities.get(*id).is_some_and(|e| e.has("frame"))).count();
    }
    assert_eq!(
        count(&world, r#"pattern p { c: cell; f: frame; ref_walk(c, f, "pose.frame", "frame.parent"); }"#),
        expected
    );
    assert_eq!(expected, 264, "132 cells, each in its floor frame and in the site frame above it");
    assert!(two_steps_up > 0, "and the second frame is only reached by taking a second step");
}

#[test]
fn a_walk_over_a_field_nothing_states_reaches_nothing() {
    let Fixture { world, .. } = corners();
    assert_eq!(count(&world, r#"pattern p { c: cell; f: any; ref_walk(c, f, "no_such.field"); }"#), 0);
    // The chain is only as long as the fields it is given: without the second
    // field there is no second step, so nothing above the pose's own frame is
    // reached.
    let one = count(&world, r#"pattern p { c: cell; f: frame; ref_walk(c, f, "pose.frame"); }"#);
    let two = count(&world, r#"pattern p { c: cell; f: frame; ref_walk(c, f, "pose.frame", "frame.parent"); }"#);
    assert!(one > 0 && two > one, "one field reaches {one} frames and two reach {two}");
}

#[test]
fn an_optional_variable_leaves_a_row_rather_than_dropping_it() {
    let Fixture { world, .. } = corners();
    let matcher = Matcher::new(&world.entities);
    let left_join = pattern(r#"pattern p { t: sector; m?: presentation_point; ref(t, m, "sector.members"); }"#);
    let bindings = matcher.matches(&left_join);
    assert_eq!(bindings.len(), 2, "one row for the named point, and one for the sector that names none");
    assert_eq!(bindings.iter().filter(|b| b.get("m").is_some()).count(), 1);
    assert_eq!(bindings.iter().filter(|b| b.get("m").is_none()).count(), 1);
    // Results are ordered by the bound identities in declaration order, and an
    // absent optional sorts before every present one — so anchoring the same
    // question at the optional variable puts the memberless sector first.
    let anchored_at_the_member =
        pattern(r#"pattern p { m?: presentation_point; t: sector; ref(t, m, "sector.members"); }"#);
    let by_member = matcher.matches(&anchored_at_the_member);
    assert_eq!(by_member.len(), 2);
    assert!(by_member[0].get("m").is_none());
    assert!(by_member[1].get("m").is_some());
    // Required, the same pattern drops the empty sector.
    let inner_join = pattern(r#"pattern p { t: sector; m: presentation_point; ref(t, m, "sector.members"); }"#);
    assert_eq!(matcher.matches(&inner_join).len(), 1);

    // And where every sector names its runs, the two joins agree.
    let Fixture { world, .. } = document("cencora-v3");
    let matcher = Matcher::new(&world.entities);
    let left = pattern(r#"pattern p { t: sector; m?: stamp; ref(t, m, "sector.members"); }"#);
    let inner = pattern(r#"pattern p { t: sector; m: stamp; ref(t, m, "sector.members"); }"#);
    let rows = |q| matcher.matches(q).len();
    assert_eq!((rows(&left), rows(&inner)), (38, 38));
}

#[test]
fn field_tests_read_the_components_own_json() {
    let Fixture { world, .. } = document("cencora-v3");
    let kinds = |want: &str| {
        world
            .entities
            .values()
            .filter(|e| e.raw("station").and_then(|s| s.get("kind")).and_then(|k| k.as_str()) == Some(want))
            .count()
    };
    assert_eq!(count(&world, r#"pattern p { s: station(kind == "pick"); }"#), kinds("pick"));
    assert_eq!(
        count(&world, r#"pattern p { s: station(kind != "pick"); }"#),
        world.entities.values().filter(|e| e.has("station")).count() - kinds("pick")
    );
    assert_eq!(count(&world, r#"pattern p { s: station(kind in ["pick", "put"]); }"#), kinds("pick") + kinds("put"));
    assert_eq!(
        count(&world, "pattern p { s: station(exit_direction exists); }"),
        world.entities.values().filter(|e| e.raw("station").is_some_and(|s| s.get("exit_direction").is_some())).count()
    );
    // A numeric field, read at an index inside an array. cencora tiles its
    // aisles at 645 mm down y and its cross-aisles at 850, so the y pitch is
    // what splits its eight grids and the x pitch, 850 throughout, is not.
    let tall = world
        .entities
        .values()
        .filter_map(|e| e.raw("tiling").and_then(|t| t.get("pitch")).and_then(|p| p.get(1)).and_then(|v| v.as_f64()))
        .filter(|pitch| *pitch > 700.0)
        .count();
    assert_eq!(count(&world, "pattern p { g: tiling(pitch.1 > 700); }"), tall);
    assert_eq!(tall, 2, "two of the eight grids step further than 700 down y");
    // A name no document uses is not an error; it matches nothing.
    assert_eq!(count(&world, "pattern p { s: no_such_component; }"), 0);
    assert_eq!(count(&world, "pattern p { s: station(no_such_field exists); }"), 0);
    // Two JSON kinds that cannot be ordered are neither equal nor unequal.
    assert_eq!(count(&world, "pattern p { s: station(kind == 3); }"), 0);
    assert_eq!(count(&world, "pattern p { s: station(kind != 3); }"), 0);
}

#[test]
fn negation_and_provenance_split_the_world() {
    let Fixture { world, .. } = document("cencora-v3");
    let stations = world.entities.values().filter(|e| e.has("station")).count();
    let queued = world.entities.values().filter(|e| e.has("station") && e.has("queue")).count();
    assert_eq!(count(&world, "pattern p { s: station, !queue; }"), stations - queued);
    assert_eq!(count(&world, "pattern p { e: cell, derived; }"), world.cells.len());
    assert_eq!(count(&world, "pattern p { e: cell, declared; }"), 0);
    assert_eq!(
        count(&world, "pattern p { e: derived; }") + count(&world, "pattern p { e: declared; }"),
        world.entities.len()
    );
}

#[test]
fn a_negated_field_test_holds_where_the_component_says_nothing() {
    let Fixture { world, .. } = document("cencora-v3");
    let frames = world.entities.values().filter(|e| e.has("frame")).count();
    let parented =
        world.entities.values().filter(|e| e.raw("frame").is_some_and(|f| f.get("parent").is_some())).count();
    assert_eq!(count(&world, "pattern p { f: frame(!(parent exists)); }"), frames - parented);
    assert_eq!(count(&world, "pattern p { f: frame(!(parent exists)); }"), 1, "one frame has no parent: the primary");
    // The comparison reversed is a different question, and on a field nobody
    // writes it answers nothing at all.
    assert_eq!(count(&world, r#"pattern p { s: station(role != "nothing_writes_this"); }"#), 0);
    assert_eq!(
        count(&world, r#"pattern p { s: station(!(role == "nothing_writes_this")); }"#),
        world.entities.values().filter(|e| e.has("station")).count()
    );
}

#[test]
fn exists_means_the_document_states_something() {
    let Fixture { world, .. } = corners();
    assert_eq!(count(&world, "pattern p { t: sector; }"), 2, "two sectors, one of them empty");
    assert_eq!(count(&world, "pattern p { t: sector(members exists); }"), 1);
    assert_eq!(count(&world, "pattern p { t: sector(!(members exists)); }"), 1);
    // A list with something in it is a statement, and `ref` reaches every
    // element of it.
    let named = pattern(r#"pattern p { t: sector; m: presentation_point; ref(t, m, "sector.members"); }"#);
    assert_eq!(Matcher::new(&world.entities).matches(&named).len(), 1);
}

#[test]
fn the_longest_component_name_claims_a_dotted_path() {
    let Fixture { world, .. } = corners();
    let matcher = Matcher::new(&world.entities);
    let both = pattern(r#"pattern p { e: go.marker; s: any; ref(e, s, "go.marker.station"); }"#);
    let bindings = matcher.matches(&both);
    assert_eq!(bindings.len(), 1);
    assert_eq!(
        bindings[0].get("s").expect("bound").to_string(),
        "44444444-4444-4444-8444-444444444444",
        "`go.marker` is the component, so its own `station` field is what was read"
    );
    // The shorter name is still readable, spelled as its own path.
    let shorter = pattern(r#"pattern p { e: go; s: any; ref(e, s, "go.marker"); }"#);
    assert_eq!(matcher.matches(&shorter).len(), 0, "`go.marker` there is an object, not an identity");
    // And a field test inside a dotted component is relative to that
    // component, which is the same rule read the other way.
    assert_eq!(count(&world, r#"pattern p { e: go.marker(part == "envelope"); }"#), 1);
    assert_eq!(count(&world, r#"pattern p { e: go.marker(go.marker.part == "envelope"); }"#), 0);
}

#[test]
fn any_stands_for_every_entity() {
    let Fixture { world, .. } = corners();
    assert_eq!(count(&world, "pattern p { e: any; }"), world.entities.len());
    assert_eq!(
        count(&world, "pattern p { e: any, !cell; }"),
        world.entities.values().filter(|e| !e.has("cell")).count()
    );
}

#[test]
fn ref_reads_the_formats_reference_forms_and_nothing_else() {
    let Fixture { world, .. } = corners();
    let matcher = Matcher::new(&world.entities);
    let named = |field: &str| {
        let text = format!(r#"pattern p {{ e: reference_forms; y: any; ref(e, y, "reference_forms.{field}"); }}"#);
        matcher.matches(&pattern(&text)).len()
    };
    assert_eq!(named("bare"), 1, "a bare identity string");
    assert_eq!(named("derived"), 1, "a derivation address, written as a generator and a key");
    assert_eq!(named("several"), 2, "one flat list holding one of each");
    assert_eq!(named("nested"), 0, "an array inside an array is no reference form");
    assert_eq!(named("local"), 0, "a template-local handle names nothing after expansion");
    assert_eq!(named("position"), 0, "a position is geometry, not an identity");
}
