//! Spec 08 section 8's statements 3 and 4, the run halves, on the local invoker and simrunner, what a real runner's
//! manifest records: each patch's artifact hashed as the runner used it, and a head moved after the plan read at the
//! plan's changeset and recorded inexact (statement 14's `read-differs`). What the tests need is in
//! `support::containers`; any of it missing, they say they are skipped.

mod support;

use std::fs;

use axum::http::StatusCode;
use scenario_spec::request_bytes;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use studio_protocol::{DeclaredId, GenerateRequest, Ulid};
use wps_client::draft::Op;
use wps_client::wps::Wps;

use support::containers::{RemovedAtEnd, SIM_MINUTES, local, loops, needed, run_of, state_is};
use support::runs::{experiment, plan, send, short_scenario, submit, until};
use support::sites::{below, components};

/// Statement 3's manifest half: each attempt's manifest names the artifact its variant patched, hashed either side,
/// `sha256_after` the hash of that artifact exactly as the runner used it, holding the run's own value; two runs of one
/// sweep share the artifact before their variants and differ after. A generator field patches the generator request,
/// which the manifest keeps as `workload.spec`, hashed as the canonical bytes sent; a run control patches the run's
/// `spec.json` as loaded. This build sweeps only the scenario's own fields, so a world target's export is not exercised
/// here.
#[tokio::test]
async fn each_manifest_hashes_the_artifact_its_variant_patched_as_the_runner_used_it() {
    let Some(needed) = needed() else { return };
    let scratch = scratch_or_skip!();
    let local = local(&scratch.pool, &needed, &needed.image, &[], 5).await;
    let stack = &local.stack;
    let running = loops(stack).await;
    let scenario = short_scenario(stack, "Peak hour", SIM_MINUTES).await;
    let out = stack.gate.app.runs.container_setup().expect("a setup").out.clone();
    let sweeps = [
        ("/workload/generator/units_per_stored_tote", json!([1, 2]), "workload.spec"),
        ("/run_controls/floor_traffic", json!(["reserved", "ghost"]), "spec.json"),
    ];
    for (pointer, values, artifact) in sweeps {
        let varies = json!({ "target": format!("scenario:{scenario}"), "pointer": pointer, "values": values });
        let body = json!({ "scenario": scenario, "version": 1, "revision": 1, "varies": varies });
        let (status, made) = send(&stack.router, "POST", "/api/experiments", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        let experiment: Ulid = made["id"].as_str().expect("its id").parse().expect("a ULID");
        let (plan, hash) = plan(stack, &experiment).await;
        submit(stack, &plan, &hash).await;
        let _removed = RemovedAtEnd::of(&needed.engine, plan.runs.iter().map(|planned| &planned.run));
        let mut sides = Vec::new();
        for planned in &plan.runs {
            until("the run to end", || state_is(stack, &planned.run, "finished")).await;
            let (status, manifest) =
                send(&stack.router, "GET", &format!("/api/runs/{}/manifest", planned.run), None).await;
            assert_eq!(status, StatusCode::OK, "{manifest}");
            let patches = manifest["variant"].as_array().expect("the variant's patches");
            assert_eq!(patches.len(), 1, "{manifest}");
            let patched = &patches[0];
            assert_eq!((&patched["file"], &patched["value"]), (&json!(artifact), &planned.variant[0].value));
            let used = if artifact == "spec.json" {
                let job = run_of(stack, &planned.run).await["attempts"][0]["job"].as_str().expect("its job").to_owned();
                fs::read(out.join(job).join("inputs/spec.json")).expect("the spec the runner wrote")
            } else {
                let sent: GenerateRequest =
                    serde_json::from_value(manifest["workload"]["spec"].clone()).expect("a request");
                assert_eq!(json!(sent.generator.units_per_stored_tote), planned.variant[0].value, "its own value");
                request_bytes(&sent)
            };
            assert_eq!(
                patched["sha256_after"],
                format!("{:x}", Sha256::digest(&used)),
                "{artifact} as the runner used it"
            );
            sides.push((patched["sha256_before"].clone(), patched["sha256_after"].clone()));
        }
        assert_eq!(sides[0].0, sides[1].0, "one {artifact} before either variant");
        assert_ne!(sides[0].1, sides[1].1, "each run's own variant after");
    }
    drop(running);
    scratch.close().await;
}

/// Statement 4's run half and statement 14's read half on the stub: the flow's head moves after the plan, and the run
/// approved then reads the plan's flow changeset, as its manifest names it; that read below the moved head is not
/// exact, the flow's export alone saying so, recorded as `read-differs` with its reason, and never as `plan-differs`,
/// since the flow it read is the one the check read.
#[tokio::test]
async fn a_head_moved_after_the_plan_is_read_at_the_plans_changeset_and_recorded_inexact() {
    let Some(needed) = needed() else { return };
    let scratch = scratch_or_skip!();
    let local = local(&scratch.pool, &needed, &needed.image, &[], 5).await;
    let stack = &local.stack;
    let running = loops(stack).await;
    let scenario = short_scenario(stack, "Peak hour", SIM_MINUTES).await;
    let made = experiment(stack, &scenario, 1, &[]).await;
    let (plan, hash) = plan(stack, &made.id).await;
    let flow = below(&stack.site, "flow");
    let planned = plan.reads[&flow];
    // A flow edit lands after the plan: the pick node's pick time, as an applied draft sets it.
    let raw = stack.gate.world.wps().export_scope(&flow, None).await.expect("the flow's export");
    let document: Value = serde_json::from_slice(&raw.bytes).expect("the export reads as JSON");
    let entities = document["entities"].as_array().expect("the flow's entities");
    let pick = entities
        .iter()
        .find(|entity| entity["components"]["flow.node"]["process"]["operation"] == "pick")
        .expect("a pick node");
    let mut node = pick["components"]["flow.node"].clone();
    node["work"]["pick_sim_ms"] = json!(7_500);
    let id: DeclaredId = pick["id"].as_str().expect("its id").parse().expect("a declared id");
    let patch = Op::Patch { id, name: None, components: components(json!({ "flow.node": node })) };
    let moved = support::stage(&stack.gate.world, &flow, vec![patch], None).await;
    assert!(moved > planned, "the flow's head moved past the plan");
    submit(stack, &plan, &hash).await;
    let _removed = RemovedAtEnd::of(&needed.engine, plan.runs.iter().map(|planned| &planned.run));
    let run = &plan.runs[0].run;
    until("the run to end", || state_is(stack, run, "finished")).await;
    let (status, manifest) = send(&stack.router, "GET", &format!("/api/runs/{run}/manifest"), None).await;
    assert_eq!(status, StatusCode::OK, "{manifest}");
    assert_eq!(manifest["reads"][flow.as_str()], json!(planned.get()), "the plan's changeset, not the head");
    assert_eq!(manifest["world"]["exact"], false, "read below the moved head");
    // Each export says its own read's exactness: the flow's alone is below its head.
    for export in manifest["exports"].as_array().expect("the exports") {
        let read_below = export["scope"] == flow.as_str();
        assert_eq!(export["exact"], !read_below, "{export}");
    }
    let read = run_of(stack, run).await;
    assert_eq!(read["findings"], json!(["read-differs"]), "no plan-differs: {read}");
    let finding = &read["attempts"][0]["findings"][0];
    assert_eq!(finding["differs"], json!(["exact"]));
    let sentence = finding["sentence"].as_str().expect("its sentence");
    assert!(sentence.contains("read below a moved head, so its labels and prologue are the head's"), "{sentence}");
    drop(running);
    scratch.close().await;
}
