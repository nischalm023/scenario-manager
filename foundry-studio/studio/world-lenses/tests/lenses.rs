//! Within `worldlens`'s tests, `lenses` is THE OBJECTS A PROJECTION MAKES: what
//! a registered lens turns a document's rows into, and the two orders that make
//! one projection the same everywhere.
//!
//! A pattern answers in rows and a row repeats its anchor once per part. A lens
//! is the fold back the other way, and the fold is only worth anything if it
//! comes out the same on both sides of a wire: same objects, same order, same
//! lists, same bytes. So this file pins the shape of a projection on the
//! document small enough to say out loud, folds the same bindings by hand under
//! three different join orders to show the plan does not reach the objects,
//! reads the JSON one object writes, and pins the one string a whole projection
//! compares as.

mod fixtures;

use fixtures::{corners, document, Fixture};
use serde_json::Value;
use std::collections::BTreeMap;
use worldlens::{canonical_projection, Library, Matcher, Object, RESERVED_FIELD_NAMES};
use site_model::EntityId;

/// The minimal EasyLoop map's machine, as one object: the machine entity, and
/// the cells, ports, pick places and waiting spots posed in its frame.
///
/// This is the reassembly a consumer does by hand today. `machine_cell` answers
/// twenty-eight rows that name the machine twenty-eight times; the lens says
/// once that those twenty-eight are one machine's cells.
#[test]
fn the_machine_of_the_minimal_map_projects_as_one_object() {
    let Fixture { world, .. } = document("minimal-easyloop");
    let library = Library::parse(SHIPPED).expect("the patterns and the lens parse");
    let matcher = Matcher::new(&world.entities);

    let lens = library.lens("easyloop_station").expect("registered under its name");
    let objects = matcher.project(lens);
    assert_eq!(objects.len(), 1, "one entity IS the machine, so there is one object");

    let machine = &objects[0];
    assert!(world.entities[&machine.anchor.id].has("easyloop"), "anchored at the machine itself");
    assert_eq!(machine.fields()["cells"].len(), 28);
    assert_eq!(machine.fields()["ports"].len(), 2);
    assert_eq!(machine.fields()["pick_places"].len(), 2);
    assert_eq!(machine.fields()["waiting"].len(), 12);
    assert_eq!(machine.fields().len(), 4, "the object carries the fields the lens declares and no others");

    // Every field's entities are entities of this document, and the narrower
    // fields are drawn from the wider one: a port and a pick place are cells.
    let cells: Vec<&EntityId> = machine.fields()["cells"].iter().map(|view| &view.id).collect();
    for narrower in ["ports", "pick_places", "waiting"] {
        for view in &machine.fields()[narrower] {
            assert!(cells.contains(&&view.id), "{narrower} is drawn from the machine's own cells");
        }
    }

    // A view carries the entity's own components, unchanged, so a consumer
    // that understands one parses it and a consumer that does not passes it on.
    let port = &machine.fields()["ports"][0];
    assert_eq!(port.components, world.entities[&port.id].components);
}

/// The objects come back in identity order and each list in its pattern's
/// binding order, whatever route the join took to them.
///
/// The three routes are the planner's own and the two `tests/determinism.rs`
/// uses — the pattern's variables in the order it declares them, and the
/// reverse of that. The fold from bindings to objects is written out here a
/// second time rather than borrowed, so a projection that agreed with itself
/// but not with its own bindings would still be caught.
#[test]
fn the_join_order_does_not_reach_the_objects() {
    let Fixture { world, .. } = document("minimal-putaway2");
    let library = Library::parse(SHIPPED).expect("the patterns and the lens parse");
    let matcher = Matcher::new(&world.entities);
    let lens = library.lens("rack_bay").expect("registered under its name");

    let projected = matcher.project(lens);
    assert_eq!(projected.len(), 6, "minimal-putaway2 states six bays");
    assert_eq!(projected.iter().map(|o| o.fields()["positions"].len()).collect::<Vec<_>>(), [10; 6]);

    // Sorted and without repeats, which is the claim the identity order makes.
    let ids: Vec<&EntityId> = projected.iter().map(|o| &o.anchor.id).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(ids, sorted);

    for order in [["b", "r"], ["r", "b"]] {
        let bays = matcher.matches_with_join_order(library.get("rack_bay").expect("the pattern"), &order);
        let positions =
            matcher.matches_with_join_order(library.get("storage_position").expect("the pattern"), &["s", "b"]);

        let mut anchors: Vec<&EntityId> = bays.iter().filter_map(|binding| binding.get("b")).collect();
        anchors.sort();
        anchors.dedup();
        let mut grouped: BTreeMap<&EntityId, Vec<&EntityId>> = BTreeMap::new();
        for binding in &positions {
            let (Some(bay), Some(slot)) = (binding.get("b"), binding.get("s")) else {
                continue;
            };
            let list = grouped.entry(bay).or_default();
            if !list.contains(&slot) {
                list.push(slot);
            }
        }

        let by_hand: Vec<(&EntityId, Vec<&EntityId>)> =
            anchors.into_iter().map(|bay| (bay, grouped.get(bay).cloned().unwrap_or_default())).collect();
        let by_lens: Vec<(&EntityId, Vec<&EntityId>)> = projected
            .iter()
            .map(|object| (&object.anchor.id, object.fields()["positions"].iter().map(|v| &v.id).collect()))
            .collect();
        assert_eq!(by_hand, by_lens, "the objects are the bindings folded, however the join reached them");
    }
}

/// A field's list says what belongs to an object, not how many ways the pattern
/// reached it: an entity bound through two rows is in the list once, and the
/// list keeps the order the rows first reached each entity rather than sorting
/// them.
///
/// The corner-case document states the shape that tells those two rules from
/// their opposites, because no surveyed one does: one hub, three routes and two
/// stops, where the first and the third route reach the same stop and the
/// middle route reaches a stop of a lower identity. Rows come back ordered by
/// the bound identities in the pattern's declaration order — hub, then route,
/// then stop — so the stops arrive as the higher one, the lower one, and the
/// higher one over again. A fold that kept the repeat would answer three; one
/// that sorted the list would answer the lower stop first.
#[test]
fn a_field_holds_a_twice_reached_entity_once_and_in_the_order_it_was_reached() {
    let Fixture { world, .. } = corners();
    let library = Library::parse(
        "pattern served_hub { h: hub; }\n\
         pattern hub_stop { h: hub; r: route; s: stop; ref(r, h, \"route.hub\"); ref(r, s, \"route.to\"); }\n\
         lens served { anchor h: served_hub; stops: hub_stop by h -> s; }",
    )
    .expect("parses");
    let matcher = Matcher::new(&world.entities);
    let (lower, higher) = ("a1000000-0000-4000-8000-000000000001", "a2000000-0000-4000-8000-000000000002");

    // The rows really do name one stop twice, so the fold is what drops it.
    let rows = matcher.matches(library.get("hub_stop").expect("the pattern"));
    let reached: Vec<String> = rows.iter().map(|row| row.get("s").expect("bound").to_string()).collect();
    assert_eq!(reached, [higher, lower, higher]);

    let objects = matcher.project(library.lens("served").expect("registered"));
    assert_eq!(objects.len(), 1, "one entity carries `hub`");
    let stops: Vec<String> = objects[0].fields()["stops"].iter().map(|view| view.id.to_string()).collect();
    assert_eq!(stops, [higher, lower]);
}

/// One object as JSON: the anchor's own keys at the top, each field beside them
/// under its own name. This is the form `site_model::canonical` turns into
/// the bytes a server and a client compare, so its shape is a promise.
#[test]
fn an_object_writes_its_anchor_and_its_fields_as_one_json_object() {
    let Fixture { world, .. } = document("minimal-easyloop");
    let library = Library::parse(SHIPPED).expect("the patterns and the lens parse");
    let machine = &Matcher::new(&world.entities).project(library.lens("easyloop_station").expect("registered"))[0];

    let json = machine.to_json();
    let object = json.as_object().expect("an object");
    assert_eq!(object["id"].as_str(), Some(machine.anchor.id.to_string().as_str()));
    assert_eq!(object["name"].as_str(), Some("easyloop 1"));
    assert_eq!(object["components"]["easyloop"], world.entities[&machine.anchor.id].components["easyloop"]);
    assert_eq!(object["cells"].as_array().expect("a list").len(), 28);
    assert_eq!(
        object.keys().collect::<Vec<_>>(),
        ["cells", "components", "id", "name", "pick_places", "ports", "waiting"]
    );

    // Each entry of a list is one entity written the same way.
    let port = object["ports"][0].as_object().expect("an object");
    assert_eq!(port.keys().collect::<Vec<_>>(), ["components", "id", "name"]);

    // The canonical projection of it is bytes, and two runs give one answer.
    let bytes = site_model::canonical::to_canonical_string(&json);
    assert_eq!(bytes, site_model::canonical::to_canonical_string(&machine.to_json()));
    assert!(bytes.starts_with('{') && bytes.contains("\"cells\":["));
}

/// A whole projection is ONE string, and it is the array of the objects.
///
/// Object by object is the wrong granularity to compare two answers by: two
/// runs can agree on every object they both hold and still differ in how many
/// there are or in what order they came, and a per-object comparison calls that
/// agreement. So the form a server and a client compare is the array, and this
/// pins it against the array built by hand from the same objects.
#[test]
fn a_whole_projection_is_one_canonical_array() {
    let Fixture { world, .. } = document("minimal-putaway2");
    let library = Library::parse(SHIPPED).expect("the patterns and the lens parse");
    let objects = Matcher::new(&world.entities).project(library.lens("rack_bay").expect("registered"));
    assert_eq!(objects.len(), 6, "minimal-putaway2 states six bays");

    let by_hand = Value::Array(objects.iter().map(Object::to_json).collect());
    let line = canonical_projection(&objects);
    assert_eq!(line, site_model::canonical::to_canonical_string(&by_hand));

    // One line holding six objects, not six lines holding one each.
    assert!(line.starts_with("[{") && line.ends_with("}]"));
    assert!(!line.contains('\n'));
    let read_back: Value = serde_json::from_str(&line).expect("the line is one JSON array");
    assert_eq!(read_back.as_array().expect("an array").len(), 6);

    assert_eq!(canonical_projection(&[]), "[]", "a projection of nothing is an empty array");
}

/// A field whose pattern binds nothing for an object is an empty list rather
/// than a missing key: the shape of an object is the lens's, not the
/// document's, so a consumer reading `cells` gets a list either way.
#[test]
fn a_field_that_binds_nothing_is_an_empty_list() {
    let Fixture { world, .. } = document("minimal-putaway2");
    let library = Library::parse(
        "pattern any_station { s: station; }\n\
         pattern queue_into_station { q: queue; s: station; ref(q, s, \"queue.into\"); }\n\
         lens station_feeds { anchor s: any_station; fed_by: queue_into_station by s -> q; }",
    )
    .expect("parses");
    let objects = Matcher::new(&world.entities).project(library.lens("station_feeds").expect("registered"));
    assert_eq!(objects.len(), 2, "minimal-putaway2 states two stations");
    for object in &objects {
        assert!(object.fields()["fed_by"].is_empty(), "nobody writes `queue.into`");
        assert_eq!(object.to_json()["fed_by"], serde_json::json!([]));
    }
}

/// The names an object's own JSON spells cannot become field names, and the
/// parser is the only place that can be said.
///
/// `Object::to_json` writes the anchor's keys and then spreads each field
/// beside them, so a field called `id` or `components` would overwrite one of
/// the anchor's. The parser refuses those names when it reads a lens; what
/// makes that refusal the whole story is that an object's field map is private
/// and projection is the one thing that fills it, so there is no second way for
/// a name to reach `to_json`. This asserts both halves: that the list the
/// parser refuses is exactly the keys the anchor writes, and that a projected
/// object hands its lists back to be read and not to be written.
#[test]
fn the_keys_an_object_writes_cannot_become_fields() {
    let Fixture { world, .. } = document("minimal-easyloop");
    let library = Library::parse(SHIPPED).expect("the patterns and the lens parse");
    let lens = library.lens("easyloop_station").expect("registered under its name");
    let objects = Matcher::new(&world.entities).project(lens);
    let json = objects[0].to_json();

    // Every key of the object that is not one of the lens's own fields is a key
    // the anchor wrote, and each of those is a name the parser turns away.
    let declared: Vec<&String> = objects[0].fields().keys().collect();
    let from_the_anchor: Vec<&String> =
        json.as_object().expect("an object").keys().filter(|key| !declared.contains(key)).collect();
    assert_eq!(from_the_anchor, ["components", "id", "name"]);
    for key in &from_the_anchor {
        assert!(RESERVED_FIELD_NAMES.contains(&key.as_str()), "`{key}` is written by the anchor and left unreserved");
    }
    // `anchor` is reserved for the fourth reason: `anchor m: machine;` states
    // the anchor, and a field of that name would be the same words twice.
    assert_eq!(RESERVED_FIELD_NAMES.len(), from_the_anchor.len() + 1);
    assert!(RESERVED_FIELD_NAMES.contains(&"anchor"));

    for name in RESERVED_FIELD_NAMES {
        let text = format!(
            "pattern machine {{ m: frame, easyloop(loop_transit_secs exists), region; }}\n\
             pattern machine_cell {{ c: cell, pose; m: frame, easyloop(loop_transit_secs exists); \
             ref_walk(c, m, \"pose.frame\", \"frame.parent\"); }}\n\
             lens taken {{ anchor m: machine; {name}: machine_cell by m -> c; }}"
        );
        let refusal = Library::parse(&text).expect_err("a field may not be called that");
        assert!(refusal.to_string().contains(name), "the refusal names `{name}`: {refusal}");
    }
}

/// The patterns and lenses these tests read. Written out here rather than taken
/// from `data/seed/application-tests/lenses/legacy-wps/world.lens` so that a change to the shipped library shows up
/// as a change to the counts it is pinned by, and not as a change to what a
/// projection is.
const SHIPPED: &str = "\
pattern machine { m: frame, easyloop(loop_transit_secs exists), region; }
pattern machine_cell { c: cell, pose; m: frame, easyloop(loop_transit_secs exists); ref_walk(c, m, \"pose.frame\", \"frame.parent\"); }
pattern machine_port { c: cell, port; m: frame, easyloop(loop_transit_secs exists); ref_walk(c, m, \"pose.frame\", \"frame.parent\"); }
pattern machine_pick_place {
  c: cell, pick, presentation_point;
  m: frame, easyloop(loop_transit_secs exists);
  ref_walk(c, m, \"pose.frame\", \"frame.parent\");
}
pattern machine_waiting_cell { c: cell, waiting; m: frame, easyloop(loop_transit_secs exists); ref_walk(c, m, \"pose.frame\", \"frame.parent\"); }
pattern rack_bay { b: rack, pose; r: stamp; descends_from(b, r); }
pattern storage_position { s: slot; b: rack; ref(s, b, \"slot.rack\"); }

lens easyloop_station {
  anchor m: machine;
  cells: machine_cell by m -> c;
  ports: machine_port by m -> c;
  pick_places: machine_pick_place by m -> c;
  waiting: machine_waiting_cell by m -> c;
}

lens rack_bay { anchor b: rack_bay; positions: storage_position by b -> s; }
";
