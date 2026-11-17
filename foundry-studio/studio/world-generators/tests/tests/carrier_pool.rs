//! Application behaviour retained from the former WPS samples, run against Foundry-owned modules.
use foundry_generators_parity::{anchor_object, carrier_pool_module, expanded_form};
use serde_json::{json, Value};
use std::path::PathBuf;
use worldgen_host::{Budget, Engine, HostError, ImportPolicy, Module, Session};
const POOL: &str = "d0000000-0000-4000-8000-0000000000ff";
const TOTE: &str = "b1000000-0000-4000-8000-0000000000c1";
const SECTOR: &str = "b1000000-0000-4000-8000-0000000000e1";

#[test]
fn one_pool_shows_the_keys_and_the_bytes_of_its_carriers() {
    let module = loaded();
    let manifest = module.describe(Budget::default()).expect("a manifest");
    assert_eq!((manifest.name.as_str(), manifest.version.as_str()), ("carrier_pool", "0.1.0"));
    assert_eq!(manifest.writes, ["carrier"]);

    let mut session = module.instantiate(Budget::default()).expect("an instance");
    let pool = json!({
        "carrier_type": TOTE,
        "designation": "order",
        "count": 2,
        "placement": { "kind": "sector", "sector": SECTOR, "fill_fraction": 0.25 },
    });
    let made =
        session.generate("carrier_pool", &anchor_object(POOL, json!({ "carrier_pool": pool }))).expect("two carriers");
    let carrier = |index: usize| {
        format!(
            r#"{{"carrier":{{"carrier_type":"{TOTE}","designation":"order","index":{index},"placement":{{"fill_fraction":0.25,"kind":"sector","sector":"{SECTOR}"}},"pool":"{POOL}"}}}}"#
        )
    };
    assert_eq!(
        made.iter().map(|one| (one.key.clone(), one.components.clone())).collect::<Vec<_>>(),
        [("carrier:0".to_string(), carrier(0)), ("carrier:1".to_string(), carrier(1))],
    );
    assert!(made.iter().all(|one| one.name.is_none()), "a carrier has nothing to call itself");
}

#[test]
fn a_pool_the_reader_refuses_and_native_reader_both_refuse() {
    let module = loaded();
    let mut session = module.instantiate(Budget::default()).expect("an instance");
    let cases = [
        (
            json!({ "carrier_type": "a tote", "designation": "order", "count": 1 }),
            "`a tote` is not an entity identity: a declared identity is a UUID",
        ),
        (
            json!({ "carrier_type": format!("{TOTE}/Opaque/"), "designation": "order", "count": 1 }),
            "derivation key `Opaque/` is malformed at segment ``: a key segment must not be empty",
        ),
        (json!({ "carrier_type": TOTE, "count": 1 }), "missing field `designation`"),
        (json!({ "carrier_type": TOTE, "designation": 7, "count": 1 }), "invalid type: integer `7`, expected a string"),
        (json!({ "carrier_type": TOTE, "designation": "order" }), "missing field `count`"),
        (
            json!({ "carrier_type": TOTE, "designation": "order", "count": 1, "placement": { "sector": SECTOR } }),
            "missing field `kind`",
        ),
        (
            json!({
                "carrier_type": TOTE, "designation": "order", "count": 1,
                "placement": { "kind": "sector", "sector": "a sector" },
            }),
            "`a sector` is not an entity identity: a declared identity is a UUID",
        ),
        (
            json!({ "carrier_type": TOTE, "designation": "order", "count": 1, "placement": "anywhere" }),
            "invalid type: string \"anywhere\", expected struct Placement",
        ),
    ];
    for (index, (pool, wrong)) in cases.into_iter().enumerate() {
        let said = refusal(&mut session, &pool);
        assert_eq!(said, format!("entity {POOL}: component `carrier_pool` does not parse: {wrong}"), "case {index}");
        let reader = reader_refusal(&format!("pool-refused-{index}"), &pool);
        assert!(reader.contains(POOL) && reader.contains("component `carrier_pool`"),
            "case {index}: native must refuse this component; reader said {reader:?}, generator {said:?}");
    }
}

#[test]
fn a_count_that_is_no_count_is_refused_by_both_naming_each_one_s_type() {
    let module = loaded();
    let mut session = module.instantiate(Budget::default()).expect("an instance");
    let pool = json!({ "carrier_type": TOTE, "designation": "order", "count": -1 });
    let said = refusal(&mut session, &pool);
    let common =
        format!("entity {POOL}: component `carrier_pool` does not parse: invalid value: integer `-1`, expected ");
    assert_eq!(said, format!("{common}u64"));
    let reader = reader_refusal("pool-negative-count", &pool);
    assert!(reader.ends_with(&format!("{common}usize")), "{reader}");
}

#[test]
fn a_count_past_thirty_two_bits_is_read_as_the_reader_reads_it() {
    let module = loaded();
    let mut session = module.instantiate(Budget::default()).expect("an instance");
    let pool = json!({ "carrier_type": TOTE, "designation": "order", "count": 4_294_967_296u64 });
    match session.generate("carrier_pool", &anchor_object(POOL, json!({ "carrier_pool": pool }))) {
        Err(HostError::OutOfFuel { .. } | HostError::OutOfMemory { .. }) => {}
        other => panic!("expected the budget to stop the loop, got {other:?}"),
    }
}

#[test]
fn opaque_derived_catalog_references_are_preserved_verbatim() {
    let module = loaded();
    let mut session = module.instantiate(Budget::default()).unwrap();
    let target = format!("{TOTE}/DeckA/42/north-port/Δ");
    let made = session.generate("carrier_pool", &anchor_object(POOL, json!({"carrier_pool":{
        "carrier_type":target, "count":1, "designation":"order"
    }}))).unwrap();
    let value: Value = serde_json::from_str(&made[0].components).unwrap();
    assert_eq!(value["carrier"]["carrier_type"], target);
}

fn loaded() -> Module {
    let engine = Engine::new().expect("an engine");
    Module::load(&engine, carrier_pool_module(), ImportPolicy::TrapOnUse).expect("the carrier-pool component loads")
}

fn refusal(session: &mut Session, pool: &Value) -> String {
    match session
        .generate("carrier_pool", &anchor_object(POOL, json!({ "carrier_pool": pool })))
        .expect_err("a refusal")
    {
        HostError::Refused { message, .. } => message,
        other => panic!("expected a refusal, got {other}"),
    }
}

fn reader_refusal(name: &str, pool: &Value) -> String {
    let frame = "6a2f8c41-0d93-4e57-b1c8-2f7e9a3d5c06";
    let document = json!({
        "proto_version": "0.7.0",
        "doc": { "id": "4c7e1a90-3b25-4d68-9f0e-8a21c6d5b3f9", "name": name, "kind": "world" },
        "primary_frame": frame,
        "entities": [
            { "id": frame, "components": { "frame": { "dims": 3, "units": "mm", "unit_mm": 1 } } },
            { "id": TOTE, "components": { "carrier_type": { "kind": "tote" } } },
            { "id": SECTOR, "components": { "sector": { "members": [] } } },
            { "id": POOL, "components": { "carrier_pool": pool } },
        ],
    });
    let path = written(name, &document);
    expanded_form(&path).expect_err("the reader refuses this pool").to_string()
}

fn written(name: &str, document: &Value) -> PathBuf {
    let directory = std::env::temp_dir().join("seed-generators-fleet");
    std::fs::create_dir_all(&directory).expect("somewhere to write a document");
    let path = directory.join(format!("{name}.world.json"));
    std::fs::write(&path, document.to_string()).expect("a document on disk");
    path
}
