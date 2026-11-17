//! Application behaviour retained from the former WPS samples, run against Foundry-owned modules.
use foundry_generators_parity::{anchor_object, rack_module};
use serde_json::{json, Value};

use worldgen_host::{Budget, Engine, HostError, ImportPolicy, Module, Object, Session};
const BAY: &str = "a0000000-0000-4000-8000-000000000002/instance:0/local:rack";

#[test]
fn one_bay_shows_the_keys_and_the_bytes_of_both_kinds_of_shelf() {
    let module = loaded();
    let manifest = module.describe(Budget::default()).expect("a manifest");
    assert_eq!(manifest.name, "rack");
    assert_eq!(manifest.writes, ["relay", "slot"]);

    let mut session = module.instantiate(Budget::default()).expect("an instance");
    let anchor = "a0000000-0000-4000-8000-000000000001/instance:0/local:rack";
    let object = anchor_object(
        anchor,
        json!({ "rack": { "slots_wide": 2, "slots_deep": 1, "footprint": [1100, 650], "levels": [
            { "height": 350 },
            { "height": 400, "reach": 1750, "positions": [[340, 1300], [1030, 1300]] },
            { "height": 900 }
        ] } }),
    );
    let made = session.generate("rack", &object).expect("two relays and four slots");

    let relay = |index: usize, along: i64| {
        format!(r#"{{"relay":{{"height":400,"index":{index},"level":1,"offset":[{along},1300],"rack":"{anchor}"}}}}"#)
    };
    let slot =
        |level: usize, col: usize| format!(r#"{{"slot":{{"col":{col},"depth":0,"level":{level},"rack":"{anchor}"}}}}"#);
    assert_eq!(
        made.iter().map(|one| (one.key.clone(), one.components.clone())).collect::<Vec<_>>(),
        [
            ("relay:1,0", relay(0, 340)),
            ("relay:1,1", relay(1, 1030)),
            ("slot:0,0,0", slot(0, 0)),
            ("slot:0,1,0", slot(0, 1)),
            ("slot:2,0,0", slot(2, 0)),
            ("slot:2,1,0", slot(2, 1)),
        ]
        .map(|(key, components)| (key.to_string(), components)),
        "a relay per position of the one relay level, and a slot per position of the two storage levels"
    );
    assert!(made.iter().all(|one| one.name.is_none()), "a shelf has nothing to call itself");
}

#[test]
fn levels_that_do_not_rise_are_refused() {
    let module = loaded();
    let mut session = module.instantiate(Budget::default()).expect("an instance");

    let level = refusal(&mut session, &bay(json!([{ "height": 400 }, { "height": 400 }])));
    assert_eq!(
        level,
        format!(
            "rack {BAY}: level 1 sits at 400 steps, not above level 0 at 400 steps — a rack's \
             levels run bottom-up and strictly increasing"
        ),
        "the reader's whole sentence, entity and all"
    );

    let sunken = refusal(&mut session, &bay(json!([{ "height": 400 }, { "height": 900 }, { "height": 850 }])));
    assert_eq!(
        sunken,
        format!(
            "rack {BAY}: level 2 sits at 850 steps, not above level 1 at 900 steps — a rack's \
             levels run bottom-up and strictly increasing"
        )
    );
}

#[test]
fn a_relay_level_with_no_reach_is_refused() {
    let module = loaded();
    let mut session = module.instantiate(Budget::default()).expect("an instance");
    let said = refusal(&mut session, &bay(json!([{ "height": 350 }, { "height": 400, "positions": [[340, 1300]] }])));
    assert_eq!(
        said,
        format!(
            "rack {BAY}: level 1 states `positions` and so is a relay level, but states no \
             `reach`, without which its relays cannot be bound to a cell"
        ),
        "the reader's whole sentence, entity and all"
    );
}

#[test]
fn a_rack_whose_shape_the_reader_refuses_is_refused_here_too() {
    let module = loaded();
    let mut session = module.instantiate(Budget::default()).expect("an instance");
    let levels = json!([{ "height": 350 }]);

    let absent = refusal(&mut session, &malformed(json!({ "slots_wide": 1, "slots_deep": 1, "levels": levels })));
    assert_eq!(absent, format!("entity {BAY}: component `rack` does not parse: missing field `footprint`"));

    for (rack, expected) in [
        (
            json!({ "slots_wide": 1, "slots_deep": 1, "footprint": "bad", "levels": levels }),
            "invalid type: string \"bad\", expected an array of length 2",
        ),
        (json!({ "slots_wide": 1, "slots_deep": 1, "footprint": [550], "levels": levels }), "invalid length 1"),
        (json!({ "slots_wide": 1, "slots_deep": 1, "footprint": null, "levels": levels }), "invalid type: null"),
        (
            json!({ "slots_wide": 1, "slots_deep": 1, "footprint": [550, 650], "slot_pitch": 7, "levels": levels }),
            "invalid type: integer `7`",
        ),
    ] {
        let said = refusal(&mut session, &malformed(rack));
        assert!(said.starts_with(&format!("entity {BAY}: component `rack` does not parse: ")), "{said}");
        assert!(said.contains(expected), "expected {expected:?} in {said:?}");
    }
}

#[test]
fn a_slot_pitch_that_is_absent_or_null_is_accepted() {
    let module = loaded();
    let mut session = module.instantiate(Budget::default()).expect("an instance");
    let of = |pitch: Value| {
        let mut rack =
            json!({ "slots_wide": 1, "slots_deep": 1, "footprint": [550, 650], "levels": [{ "height": 350 }] });
        if !pitch.is_null() {
            rack["slot_pitch"] = pitch;
        }
        malformed(rack)
    };
    for (what, object) in [("absent", of(Value::Null)), ("null", of(json!(null)))] {
        let made = session.generate("rack", &object).unwrap_or_else(|why| panic!("{what}: {why}"));
        assert_eq!(made.len(), 1, "{what}: one storage level one position wide and one deep");
    }
    let stated = of(json!([550, 650]));
    assert_eq!(session.generate("rack", &stated).expect("a stated pitch").len(), 1);
}

#[test]
fn a_count_of_positions_above_thirty_two_bits_is_read_as_the_reader_reads_it() {
    let module = loaded();
    let mut session = module.instantiate(Budget::default()).expect("an instance");
    let object = malformed(json!({
        "slots_wide": 0,
        "slots_deep": 4_294_967_296u64,
        "footprint": [0, 650],
        "levels": [{ "height": 350 }]
    }));
    let made = session.generate("rack", &object).expect("a bay no positions wide derives nothing, and is not refused");
    assert!(made.is_empty(), "no positions wide is no slots, whatever the depth says");
}

fn loaded() -> Module {
    let engine = Engine::new().expect("an engine");
    Module::load(&engine, rack_module(), ImportPolicy::TrapOnUse).expect("the rack component loads")
}

fn bay(levels: Value) -> Object {
    anchor_object(
        BAY,
        json!({ "rack": { "slots_wide": 1, "slots_deep": 1, "footprint": [550, 650], "levels": levels } }),
    )
}

fn malformed(rack: Value) -> Object {
    anchor_object(BAY, json!({ "rack": rack }))
}

fn refusal(session: &mut Session, object: &Object) -> String {
    match session.generate("rack", object).expect_err("a refusal") {
        HostError::Refused { message, .. } => message,
        other => panic!("expected a refusal, got {other}"),
    }
}
