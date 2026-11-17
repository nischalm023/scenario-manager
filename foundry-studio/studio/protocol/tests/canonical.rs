//! The JSON form Agent Studio hashes, and spec 06 section 3.3's flow plan digest over it: every object's keys sorted
//! whatever order its map was built in, numbers as serde_json writes them, and the digest pinned to one plan's bytes,
//! so the check's digest and a runner's cannot drift apart unnoticed.

use serde_json::{Map, Value, json};
use studio_protocol::{flow_plan_digest, write_canonical};

fn canonical(value: &Value) -> String {
    let mut out = Vec::new();
    write_canonical(value, &mut out);
    String::from_utf8(out).expect("JSON text is UTF-8")
}

/// One object built in two orders, which a build with serde_json's `preserve_order` iterates differently, writes as
/// one text: keys sorted at every depth, arrays in their order, no whitespace.
#[test]
fn a_value_writes_with_every_objects_keys_sorted_and_no_whitespace() {
    let built = |names: [&str; 3]| {
        let mut fields = Map::new();
        for name in names {
            fields.insert(name.to_owned(), json!({ "z": [3, 1], "a": null }));
        }
        Value::Object(fields)
    };
    let text = r#"{"after":{"a":null,"z":[3,1]},"config":{"a":null,"z":[3,1]},"type":{"a":null,"z":[3,1]}}"#;
    assert_eq!(canonical(&built(["type", "config", "after"])), text);
    assert_eq!(canonical(&built(["after", "type", "config"])), text);
}

/// A float keeps its fraction (`10.0`, where RFC 8785 writes `10`).
#[test]
fn a_number_is_written_as_serde_json_writes_it() {
    assert_eq!(canonical(&json!([10.0, 0.4, 3, -2])), "[10.0,0.4,3,-2]");
}

/// `sha256:` and 64 hex digits over `{"composition":…,"settings":…}` written canonically, and nothing else.
#[test]
fn the_flow_plan_digest_is_the_sha256_of_the_canonical_composition_and_settings() {
    let composition = json!({ "tasks": [
        { "type": "FillInventory", "config": { "stock_at_start": 0.4 }, "after": ["InitializeFleet"] },
        { "type": "InitializeFleet" },
    ] });
    let settings = json!({ "operator": { "time_per_pick_secs": 10.0 } });
    let digest = flow_plan_digest(&composition, &settings);
    assert_eq!(digest, "sha256:794e9df55d8f3e0c1a26c8d43c2eb7c00dd66a3e11bc0b8dde93f6770b4bbd12");
    let slower = json!({ "operator": { "time_per_pick_secs": 11.0 } });
    assert_ne!(flow_plan_digest(&composition, &slower), digest, "a setting's value moves it");
}
