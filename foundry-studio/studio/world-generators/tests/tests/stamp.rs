//! Original v0.1 stamp contracts, retained across the application ownership move.
use foundry_generators_parity::{stamp_module, stamp_object};
use serde_json::{json, Value};
use worldgen_host::{Budget, Engine, HostError, ImportPolicy, Module, Object, Session};
const FRAME: &str = "edd0316d-a9bc-48f4-bb37-a1594ced8cd1";

fn loaded() -> Module {
    let engine = Engine::new().expect("an engine");
    Module::load(&engine, stamp_module(), ImportPolicy::TrapOnUse).expect("the stamp component loads")
}

#[test]
fn a_stamp_that_is_not_placed_is_refused() {
    let module = loaded();
    let mut session = module.instantiate(Budget::default()).expect("an instance");
    let template = json!({ "template": { "entities": [{ "local": "rack", "components": {} }] } });

    let frameless = stamp_object(
        "a0000000-0000-4000-8000-000000000001",
        json!({ "stamp": { "template": "t" }, "pose": { "position": [0, 0] } }),
        Some(("t", template.clone())),
    );
    assert!(
        refusal(&mut session, &frameless).contains("a stamp must be posed in a frame"),
        "{}",
        refusal(&mut session, &frameless)
    );

    let unplaced = stamp_object(
        "a0000000-0000-4000-8000-000000000002",
        json!({ "stamp": { "template": "t" }, "pose": { "frame": FRAME, "position": [] } }),
        Some(("t", template)),
    );
    assert!(
        refusal(&mut session, &unplaced).contains("a stamp's pose needs at least a planar position"),
        "{}",
        refusal(&mut session, &unplaced)
    );
}

#[test]
fn a_template_that_is_not_there_and_one_that_is_not_a_template_are_both_refused() {
    let module = loaded();
    let mut session = module.instantiate(Budget::default()).expect("an instance");
    let posed = json!({
        "stamp": { "template": "07b4e2ba-0000-4000-8000-00000000000a" },
        "pose": { "frame": FRAME, "position": [0, 0] }
    });

    let dangling = stamp_object("a0000000-0000-4000-8000-000000000003", posed.clone(), None);
    assert!(
        refusal(&mut session, &dangling)
            .contains("`stamp.template` references 07b4e2ba-0000-4000-8000-00000000000a, which does not exist"),
        "{}",
        refusal(&mut session, &dangling)
    );

    let not_a_template = stamp_object(
        "a0000000-0000-4000-8000-000000000004",
        posed,
        Some(("07b4e2ba-0000-4000-8000-00000000000a", json!({ "station": { "kind": "pick" } }))),
    );
    assert!(
        refusal(&mut session, &not_a_template).contains("is missing the `template` component its role requires"),
        "{}",
        refusal(&mut session, &not_a_template)
    );
}

#[test]
fn a_template_the_lens_bound_past_the_stamps_own_reference_is_refused() {
    let module = loaded();
    let mut session = module.instantiate(Budget::default()).expect("an instance");
    let object = stamp_object(
        "a0000000-0000-4000-8000-000000000060",
        json!({ "stamp": { "template": "wanted" }, "pose": { "frame": FRAME, "position": [0, 0] } }),
        Some(("handed", json!({ "template": { "entities": [] } }))),
    );
    assert!(
        refusal(&mut session, &object).contains("the lens bound template handed where the stamp names wanted"),
        "{}",
        refusal(&mut session, &object)
    );
}

#[test]
fn a_template_named_in_capitals_is_the_template_the_lens_bound() {
    let module = loaded();
    let mut session = module.instantiate(Budget::default()).expect("an instance");
    let object = stamp_object(
        "a0000000-0000-4000-8000-000000000061",
        json!({ "stamp": { "template": "B0000000-0000-4000-8000-00000000000B" }, "pose": { "frame": FRAME, "position": [0, 0] } }),
        Some(("b0000000-0000-4000-8000-00000000000b", json!({ "template": { "entities": [] } }))),
    );
    let made = session.generate("stamp", &object).expect("the same template, spelled two ways");
    assert_eq!(made.len(), 1, "one instance of an empty template");
}

#[test]
fn a_parameter_left_unbound_and_one_nobody_declared_are_both_refused() {
    let module = loaded();
    let mut session = module.instantiate(Budget::default()).expect("an instance");
    let template = json!({
        "template": { "params": { "induct_secs": {} }, "entities": [] }
    });

    let unbound = stamp_object(
        "a0000000-0000-4000-8000-000000000005",
        json!({ "stamp": { "template": "t" }, "pose": { "frame": FRAME, "position": [0, 0] } }),
        Some(("t", template.clone())),
    );
    assert!(
        refusal(&mut session, &unbound).contains("leaves the parameter `induct_secs` of template t unbound"),
        "{}",
        refusal(&mut session, &unbound)
    );

    let surplus = stamp_object(
        "a0000000-0000-4000-8000-000000000006",
        json!({
            "stamp": { "template": "t", "params": { "induct_secs": 6, "induct_sec": 6 } },
            "pose": { "frame": FRAME, "position": [0, 0] }
        }),
        Some(("t", template)),
    );
    assert!(
        refusal(&mut session, &surplus).contains("binds `induct_sec`, which template t does not declare"),
        "{}",
        refusal(&mut session, &surplus)
    );

    // And a template that reads a parameter it never declared, which is the
    // same mistake written on the other side of the pair.
    let undeclared = stamp_object(
        "a0000000-0000-4000-8000-000000000007",
        json!({ "stamp": { "template": "t" }, "pose": { "frame": FRAME, "position": [0, 0] } }),
        Some((
            "t",
            json!({ "template": { "entities": [{
                "local": "station",
                "components": { "easyloop": { "loop_transit_secs": { "$param": "transit" } } }
            }] } }),
        )),
    );
    assert!(
        refusal(&mut session, &undeclared)
            .contains("template t references the parameter `transit`, which it does not declare"),
        "{}",
        refusal(&mut session, &undeclared)
    );
}

#[test]
fn a_handle_that_is_not_an_identity_is_refused() {
    let module = loaded();
    let mut session = module.instantiate(Budget::default()).expect("an instance");
    let posed = json!({ "stamp": { "template": "t" }, "pose": { "frame": FRAME, "position": [0, 0] } });
    let of = |entities: Value| json!({ "template": { "entities": entities } });

    let cases = [
        (
            of(json!([{ "local": "port", "components": { "port": { "into": { "local_ref": "statoin" } } } }])),
            "template t references the local handle `statoin`, which it does not define",
        ),
        (
            of(json!([{ "local": "rack", "components": {} }, { "local": "rack", "components": {} }])),
            "template t defines the local handle `rack` twice",
        ),
        (
            of(json!([{ "local": "bay/1", "components": {} }])),
            "template t: the local handle `bay/1` contains `/`, which SPEC §3 reserves",
        ),
        (
            of(json!([{ "local": "q", "components": { "queue": { "path": [
                { "local_ref": "q", "key": "cell:0,0", "depth": 2 }
            ] } } }])),
            "a `local_ref` stands alone or carries exactly a `key` beside it, each holding a \
             string; this one sits among depth, key, local_ref",
        ),
    ];
    for (index, (template, expected)) in cases.into_iter().enumerate() {
        let object =
            stamp_object(&format!("a0000000-0000-4000-8000-00000000001{index}"), posed.clone(), Some(("t", template)));
        let said = refusal(&mut session, &object);
        assert!(said.contains(expected), "expected {expected:?}, got {said:?}");
    }
}

#[test]
fn a_key_parameter_bound_to_something_other_than_a_string_is_refused() {
    let module = loaded();
    let mut session = module.instantiate(Budget::default()).expect("an instance");
    let object = stamp_object(
        "a0000000-0000-4000-8000-000000000080",
        json!({
            "stamp": { "template": "t", "params": { "tile": [1, 0] } },
            "pose": { "frame": FRAME, "position": [0, 0] }
        }),
        Some((
            "t",
            json!({ "template": { "params": { "tile": {} }, "entities": [{ "local": "q", "components": {
                "region": { "origin": [0, 0], "size": [1800, 900] },
                "tiling": { "pitch": [900, 900] },
                "queue": { "path": [{ "local_ref": "q", "key": { "$param": "tile" } }] }
            } }] } }),
        )),
    );
    let said = refusal(&mut session, &object);
    assert!(
        said.contains(
            "template t: a `local_ref` stands alone or carries exactly a `key` beside it, each holding a \
             string; this one sits among key, local_ref"
        ),
        "{said}"
    );
}

#[test]
fn a_repeat_of_zero_is_refused() {
    let module = loaded();
    let mut session = module.instantiate(Budget::default()).expect("an instance");
    let object = stamp_object(
        "a0000000-0000-4000-8000-000000000020",
        json!({
            "stamp": { "template": "t", "repeat": { "count": 0, "step": [2750, 0] } },
            "pose": { "frame": FRAME, "position": [0, 0] }
        }),
        Some(("t", json!({ "template": { "entities": [] } }))),
    );
    assert!(
        refusal(&mut session, &object).contains("`repeat.count` must be a positive integer"),
        "{}",
        refusal(&mut session, &object)
    );
}

#[test]
fn geometry_a_quarter_turn_cannot_carry_is_refused() {
    let module = loaded();
    let mut session = module.instantiate(Budget::default()).expect("an instance");
    let at = |heading: f64, components: Value| {
        stamp_object(
            "a0000000-0000-4000-8000-000000000030",
            json!({
                "stamp": { "template": "t" },
                "pose": { "frame": FRAME, "position": [0, 0], "heading_deg": heading }
            }),
            Some((
                "t",
                json!({ "template": { "entities": [
                { "local": "block", "components": components }
            ] } }),
            )),
        )
    };

    let skew = at(30.0, json!({ "region": { "origin": [0, 0], "size": [1800, 900] } }));
    assert!(
        refusal(&mut session, &skew)
            .contains("a template region cannot be stamped at 30°: v0 regions are axis-aligned rectangles"),
        "{}",
        refusal(&mut session, &skew)
    );

    let carved = at(
        180.0,
        json!({ "tiling": { "pitch": [900, 900], "exclusions": [{ "origin": [0, 0], "size": [900, 900] }] } }),
    );
    assert!(
        refusal(&mut session, &carved).contains("a template tiling carrying `exclusions` cannot be stamped at 180°"),
        "{}",
        refusal(&mut session, &carved)
    );
}

#[test]
fn a_template_cells_size_turns_with_the_stamp_as_a_pitch_does() {
    let module = loaded();
    let mut session = module.instantiate(Budget::default()).expect("an instance");
    let stamped = |heading: f64, entities: Value| {
        stamp_object(
            "a0000000-0000-4000-8000-000000000070",
            json!({
                "stamp": { "template": "t" },
                "pose": { "frame": FRAME, "position": [0, 0], "heading_deg": heading }
            }),
            Some(("t", json!({ "template": { "entities": entities } }))),
        )
    };
    let stand_and_lane = json!([
        { "local": "stand", "components": {
            "pose": { "position": [1500, 500] },
            "cell": { "size": [3000, 1000], "traversable_by": ["htm"] } } },
        { "local": "lane", "components": {
            "region": { "origin": [0, 1000], "size": [3000, 1000] },
            "tiling": { "pitch": [1500, 1000], "derives": { "cell": { "size": [1500, 500] } } } } },
        { "local": "fixed", "components": {
            "pose": { "frame": FRAME, "position": [9000, 9000] },
            "cell": { "size": [3000, 1000] } } }
    ]);
    for (heading, size, pitch, tile) in [
        (0.0, [3000, 1000], [1500, 1000], [1500, 500]),
        (90.0, [1000, 3000], [1000, 1500], [500, 1500]),
        (180.0, [3000, 1000], [1500, 1000], [1500, 500]),
        (270.0, [1000, 3000], [1000, 1500], [500, 1500]),
    ] {
        let made = session.generate("stamp", &stamped(heading, stand_and_lane.clone())).expect("a quarter turn");
        let component = |key: &str, name: &str| -> Value {
            let one = made.iter().find(|one| one.key == key).expect("the internal");
            serde_json::from_str::<Value>(&one.components).expect("a component map")[name].clone()
        };
        assert_eq!(component("instance:0/local:stand", "cell")["size"], json!(size), "the cell at {heading}°");
        let tiling = component("instance:0/local:lane", "tiling");
        assert_eq!(tiling["pitch"], json!(pitch), "the pitch at {heading}°");
        assert_eq!(tiling["derives"]["cell"]["size"], json!(tile), "the derived size at {heading}°");
        assert_eq!(component("instance:0/local:fixed", "cell")["size"], json!([3000, 1000]), "fixed at {heading}°");
    }

    let skewed = |cell: Value| {
        stamped(
            30.0,
            json!([{ "local": "stand", "components": { "pose": { "position": [1500, 500] }, "cell": cell } }]),
        )
    };
    let square = skewed(json!({ "size": [1000, 1000] }));
    assert!(
        refusal(&mut session, &square).contains("a template cell stating a `size` cannot be stamped at 30°"),
        "{}",
        refusal(&mut session, &square)
    );
    session.generate("stamp", &skewed(json!({ "traversable_by": ["htm"] }))).expect("a cell stating no size");
}

#[test]
fn one_instance_shows_every_rewrite_at_once() {
    let module = loaded();
    let mut session = module.instantiate(Budget::default()).expect("an instance");
    let object = stamp_object(
        "a0000000-0000-4000-8000-000000000040",
        json!({
            "stamp": { "template": "t", "params": {
                "transit": 45,
                "feeds": { "local_ref": "station" },
                "head": { "local_ref": "q", "key": "cell:0,0" }
            } },
            "pose": { "frame": FRAME, "position": [6200, 300], "heading_deg": 90 }
        }),
        Some((
            "t",
            json!({ "template": {
                "params": { "transit": {}, "feeds": {}, "head": {} },
                "entities": [
                    { "local": "station", "name": "the picker", "components": {
                        "pose": { "position": [900, 0], "heading_deg": 180 },
                        "easyloop": { "$comment": "the station's own note",
                                         "loop_transit_secs": { "$param": "transit" } }
                    } },
                    { "local": "q", "components": {
                        "region": { "origin": [0, 900], "size": [2700, 1800] },
                        "tiling": { "pitch": [900, 450] },
                        "queue": { "path": [{ "local_ref": "q", "key": "cell:0,0",
                                              "$comment": "the tile this queue starts at" }],
                                   "into": { "local_ref": "station",
                                             "$comment": "the picker this queue feeds" },
                                   "feeds": { "$param": "feeds" },
                                   "head": { "$param": "head" } }
                    } }
                ]
            } }),
        )),
    );
    let made = session.generate("stamp", &object).expect("three entities");
    let by_key: Vec<(&str, &str)> = made.iter().map(|one| (one.key.as_str(), one.components.as_str())).collect();
    let root = "a0000000-0000-4000-8000-000000000040";
    let station = format!("{root}/instance:0/local:station");
    let first_tile = format!(r#"{{"generator":"{root}","key":"instance:0/local:q/cell:0,0"}}"#);

    assert_eq!(made[1].name.as_deref(), Some("the picker"), "a template names an internal");
    assert_eq!(
        by_key[1],
        (
            "instance:0/local:station",
            format!(
                r#"{{"easyloop":{{"loop_transit_secs":45}},"pose":{{"frame":"{FRAME}","heading_deg":270,"position":[6200,1200]}}}}"#
            )
            .as_str()
        ),
        "the parameter is bound, (900,0) turns to (0,900) and lands on the anchor, 90+180=270"
    );
    assert_eq!(
        by_key[2],
        (
            "instance:0/local:q",
            format!(
                r#"{{"queue":{{"feeds":"{station}","head":{first_tile},"into":"{station}","path":[{first_tile}]}},"region":{{"frame":"{FRAME}","origin":[3500,300],"size":[1800,2700]}},"tiling":{{"pitch":[450,900]}}}}"#
            )
            .as_str()
        ),
        "a parameter bound to a handle reference is wired like one written in the template, \
         the rectangle turns onto its far corner and is re-cornered, and the pitch swaps axes"
    );
    assert!(
        !by_key[1].1.contains("$comment") && !by_key[2].1.contains("$comment"),
        "the canonical projection is where an annotation stops, on this side as on the reader's"
    );
}

#[test]
fn a_keyed_reference_under_a_derived_anchor_is_rooted_where_the_reader_roots_it() {
    let module = loaded();
    let mut session = module.instantiate(Budget::default()).expect("an instance");
    let declared = "a0000000-0000-4000-8000-000000000050";
    let anchor = format!("{declared}/instance:0/local:child");
    let object = stamp_object(
        &anchor,
        json!({ "stamp": { "template": "t" }, "pose": { "frame": FRAME, "position": [0, 0] } }),
        Some((
            "t",
            json!({ "template": { "entities": [
                { "local": "q", "components": { "queue": {
                    "into": { "local_ref": "q" },
                    "path": [{ "local_ref": "q", "key": "cell:0,0" }]
                } } }
            ] } }),
        )),
    );
    let made = session.generate("stamp", &object).expect("an instance and the internal inside it");

    assert_eq!(made[1].key, "instance:0/local:q", "the key is still stated under the anchor the host ran");
    assert_eq!(
        made[1].components,
        format!(
            r#"{{"queue":{{"into":"{anchor}/instance:0/local:q","path":[{{"generator":"{declared}","key":"instance:0/local:child/instance:0/local:q/cell:0,0"}}]}}}}"#
        ),
        "the generator is the declared entity at the head, and the key is every segment under it"
    );
}

fn refusal(session: &mut Session, object: &Object) -> String {
    match session.generate("stamp", object).expect_err("a refusal") {
        HostError::Refused { message, .. } => message,
        other => panic!("expected a refusal, got {other}"),
    }
}
