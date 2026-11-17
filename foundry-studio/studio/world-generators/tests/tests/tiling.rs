//! Application behaviour retained from the former WPS samples, run against Foundry-owned modules.
use foundry_generators_parity::{tiling_module, tiling_object};
use serde_json::json;

use worldgen_host::{Budget, Engine, HostError, ImportPolicy, Module};
const FRAME: &str = "edd0316d-a9bc-48f4-bb37-a1594ced8cd1";

#[test]
fn an_extent_that_is_not_a_whole_multiple_of_the_pitch_is_refused() {
    let module = loaded();
    let mut session = module.instantiate(Budget::default()).expect("an instance");
    let object = tiling_object(
        "a0000000-0000-4000-8000-000000000001",
        json!({
            "region": { "frame": FRAME, "origin": [0, 0], "size": [1800, 29650] },
            "tiling": { "pitch": [900, 900] }
        }),
    );
    let refusal = session.generate("tiling", &object).expect_err("a refusal");
    let HostError::Refused { message, .. } = &refusal else {
        panic!("expected a refusal, got {refusal}");
    };
    assert!(
        message.contains(
            "tiling extent 29650 steps on the y axis is not a positive whole \
                          multiple of the pitch 900 steps"
        ),
        "{message}"
    );
}

#[test]
fn a_derives_that_names_pose_is_refused() {
    let module = loaded();
    let mut session = module.instantiate(Budget::default()).expect("an instance");
    let object = tiling_object(
        "a0000000-0000-4000-8000-000000000002",
        json!({
            "region": { "frame": FRAME, "origin": [0, 0], "size": [1800, 1800] },
            "tiling": {
                "pitch": [900, 900],
                "derives": { "pose": { "frame": FRAME, "position": [0, 0] } }
            }
        }),
    );
    let refusal = session.generate("tiling", &object).expect_err("a refusal");
    let HostError::Refused { message, .. } = &refusal else {
        panic!("expected a refusal, got {refusal}");
    };
    assert!(message.contains("a tiling's `derives` may not name `pose`"), "{message}");
}

fn loaded() -> Module {
    let engine = Engine::new().expect("an engine");
    Module::load(&engine, tiling_module(), ImportPolicy::TrapOnUse).expect("the tiling component loads")
}
