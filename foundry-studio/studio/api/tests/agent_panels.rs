//! The agent's panel commands as Astra's agent-loop review found them and the lead disposed them on 2026-10-20 (spec 03
//! section 3.3): `panel.show` carrying the Flow panel's layers and draft, one run's playback instant, the runs compared
//! and the metrics to headline, each on its own panel; and what the turn's last `show` opened, a list included,
//! serving its later commands after a wait.

mod support;

use std::sync::Arc;

use agent_loop::{Scripted, ScriptedStep};
use axum::Router;
use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use studio_api::agent::Agent;
use studio_api::config::AgentSettings;
use studio_api::projects::make_local_default;
use studio_api::{GateState, gate_routes};
use support::agent::{call, entries, opened, question, say, settled};
use support::l_shape::{SITE, made_l_shape};
use support::templates::{registered, seeded, send};
use wps_client::MockWps;

const RUN: &str = "01KN0TJZANP5N0PGJD8F7A3RNA";
const OTHER_RUN: &str = "01KN0TJZANP5N0PGJD8F7A3RNB";

/// The service with the scripted model as its agent over the L-shaped site, its flow included.
async fn l_shaped(pool: &PgPool, model: Scripted) -> Router {
    make_local_default(pool).await.expect("the default project");
    let mock = registered();
    mock.publish("rack", "0.7.0", json!({}), Vec::new());
    let agent = Agent::scripted(AgentSettings::default(), Arc::new(model));
    let app = support::state(pool.clone(), support::gate_rules()).with_agent(agent);
    let gate: GateState<MockWps> = GateState::new(app, seeded(mock).await);
    made_l_shape(&gate.world).await;
    gate_routes(gate)
}

/// A person's changeset draft of the site's flow, made by its first edit at the cut the flow reads now.
async fn drafted(router: &Router) -> String {
    let (status, _, flow) = send(router, "GET", &format!("/api/flow/{SITE}/flow"), None, None).await;
    assert_eq!(status, StatusCode::OK, "{flow}");
    let nodes = flow["nodes"].as_array().expect("nodes");
    let pick = nodes.iter().find(|node| node["name"] == "Pick stations").expect("the pick stations")["ref"].clone();
    let op = json!({ "op": "patch", "target": pick, "pointer": "/flow.node/intake/0/admission_factor", "value": 5.0 });
    let body = json!({ "site": format!("{SITE}@main"), "seqs": flow["seqs"], "summary": "A change", "ops": [op] });
    let (status, _, made) = send(router, "POST", "/api/drafts", Some(body), None).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    made["draft"].as_str().expect("a draft").to_owned()
}

/// The `panel.command` entries the conversation's journal holds, in order.
async fn commands(pool: &PgPool, conversation: &str) -> Vec<Value> {
    (entries(pool, conversation).await.into_iter())
        .filter(|(kind, _, _)| kind == "panel.command")
        .map(|(_, body, _)| body)
        .collect()
}

/// The review's first panel finding: `show` takes `layers` and `draft` on the Flow panel (the draft checked against the
/// cut the command reads, the announcement saying "with its draft"), `sim_ms` on one run, and `compare` and `metrics`
/// on the Runs panel, each in the entry and its route in spec 03's canonical order; on another panel each is refused
/// `panel-mismatch`.
#[tokio::test]
async fn panel_show_carries_its_panel_specific_inputs() {
    let scratch = scratch_or_skip!();
    let model = Scripted::new(Vec::new());
    let router = l_shaped(&scratch.pool, model.clone()).await;
    let draft = drafted(&router).await;
    let flow =
        json!({ "panel": "flow", "location": format!("{SITE}/flow@main"), "layers": ["material"], "draft": draft });
    let run = json!({ "panel": "runs", "location": format!("run:{RUN}"), "sim_ms": 5000 });
    let compared = json!({ "panel": "runs", "compare": [RUN, OTHER_RUN], "metrics": ["picks per hour"] });
    let astray = json!({ "panel": "map", "location": format!("{SITE}/map@main"), "layers": ["agent"] });
    model.push(vec![
        ScriptedStep::reply(vec![
            call("panel.show", flow),
            call("panel.show", run),
            call("panel.show", compared),
            call("panel.show", astray),
        ]),
        ScriptedStep::text("Shown."),
    ]);
    let conversation = opened(&router, Some(SITE)).await;
    let turn = say(&router, &conversation, "show me").await;
    assert_eq!(settled(&scratch.pool, &turn).await, "done");
    let shown = commands(&scratch.pool, &conversation).await;
    let by_panel = |panel: &str, field: &str| {
        shown.iter().find(|entry| entry["panel"] == panel && !entry[field].is_null()).cloned().unwrap_or_default()
    };
    let flow = by_panel("flow", "draft");
    assert_eq!((&flow["layers"], &flow["draft"]), (&json!(["material"]), &json!(draft)), "{flow}");
    assert_eq!(flow["route"], json!(format!("/w/{SITE}/flow@main?layers=material&draft={draft}")));
    assert!(flow["announce"].as_str().is_some_and(|said| said.contains("with its draft")), "{flow}");
    let run = by_panel("runs", "sim_ms");
    assert_eq!((&run["sim_ms"], &run["route"]), (&json!(5000), &json!(format!("/runs/{RUN}?sim_ms=5000"))));
    let compared = by_panel("runs", "compare");
    assert_eq!(compared["route"], json!(format!("/runs/compare?ids={RUN},{OTHER_RUN}")));
    assert_eq!((&compared["compare"], &compared["metrics"]), (&json!([RUN, OTHER_RUN]), &json!(["picks per hour"])));
    let refused: (String, Value) = sqlx::query_as(
        "SELECT state, problem FROM tool_calls WHERE turn = $1 AND tool = 'panel.show' AND state <> 'ok'",
    )
    .bind(&turn)
    .fetch_one(&scratch.pool)
    .await
    .expect("one refused");
    assert_eq!((refused.0.as_str(), &refused.1["type"]), ("failed", &json!("panel-mismatch")));
    scratch.close().await;
}

/// The review's panel-context finding (spec 03 section 3.3, the default location): a `select` with no panel and no
/// location after the turn waited on a question acts on what the turn's last `show` opened before the wait, here the
/// Runs list.
#[tokio::test]
async fn the_last_show_serves_a_command_after_a_wait() {
    let scratch = scratch_or_skip!();
    let model = Scripted::new(vec![
        ScriptedStep::reply(vec![call("panel.show", json!({ "panel": "runs" })), call("ask_user", question())]),
        ScriptedStep::reply(vec![call("panel.select", json!({ "select": format!("run:{RUN}") }))]),
        ScriptedStep::text("Selected."),
    ]);
    let router = l_shaped(&scratch.pool, model).await;
    let conversation = opened(&router, Some(SITE)).await;
    let turn = say(&router, &conversation, "show the runs").await;
    assert_eq!(settled(&scratch.pool, &turn).await, "waiting");
    let asked: String = sqlx::query_scalar("SELECT id FROM questions WHERE turn = $1")
        .bind(&turn)
        .fetch_one(&scratch.pool)
        .await
        .expect("a question");
    let answer = json!({ "answers": [{ "item": 0, "selected": ["L"] }] });
    let (status, _, _) = send(&router, "POST", &format!("/api/questions/{asked}/answers"), Some(answer), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(settled(&scratch.pool, &turn).await, "done");
    let shown = commands(&scratch.pool, &conversation).await;
    let selected = shown.iter().find(|entry| entry["action"] == "select").expect("the select's entry");
    assert_eq!((&selected["panel"], &selected["route"]), (&json!("runs"), &json!(format!("/runs?select=run:{RUN}"))));
    scratch.close().await;
}
