//! What the agent loop's tests share (spec 01 sections 3.4 to 3.10): the service over the development seed with the
//! scripted model as its agent, a conversation opened and a message sent to it, a turn awaited past `running`, the
//! journal read back, and what a request to the model carried.

use std::sync::Arc;
use std::time::{Duration, Instant};

use agent_loop::rig::{Message, UserContent};
use agent_loop::{ModelRequest, Scripted, ScriptedBlock, wire_name};
use axum::Router;
use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use studio_api::GateState;
use studio_api::agent::Agent;
use studio_api::config::AgentSettings;
use studio_api::gate_routes;
use studio_api::projects::make_local_default;
use tokio::time::sleep;

use super::templates::{Held, registered, seeded, send};

pub const SITE: &str = "customer/demo/site/lshape-6";

pub struct Stack {
    pub gate: GateState<Held>,
    pub router: Router,
    pub model: Scripted,
}

/// The service over the development seed, the demo site made, its agent the scripted model with `settings`.
pub async fn ready(pool: &PgPool, settings: AgentSettings, model: Scripted) -> Stack {
    make_local_default(pool).await.expect("the default project");
    let agent = Agent::scripted(settings, Arc::new(model.clone()));
    let app = super::state(pool.clone(), super::gate_rules()).with_agent(agent);
    let gate = GateState::new(app, seeded(Held::new(registered())).await);
    super::made_site(&gate.world, SITE, "L-Shape 6").await;
    let router = gate_routes(gate.clone());
    Stack { gate, router, model }
}

pub async fn opened(router: &Router, site: Option<&str>) -> String {
    let body = site.map_or_else(|| json!({}), |site| json!({ "site": site }));
    let (status, _, answer) = send(router, "POST", "/api/conversations", Some(body), None).await;
    assert!(status.is_success(), "{answer}");
    answer["conversation"]["id"].as_str().expect("an id").to_owned()
}

pub fn message_body(text: &str) -> Value {
    json!({ "text": text, "context": null, "source": "composer", "browser_tab": "tab-1",
            "client_message_id": format!("m-{text}") })
}

pub async fn post(router: &Router, conversation: &str, body: Value) -> (StatusCode, Value) {
    let (status, _, answer) =
        send(router, "POST", &format!("/api/conversations/{conversation}/messages"), Some(body), None).await;
    (status, answer)
}

pub async fn say(router: &Router, conversation: &str, text: &str) -> String {
    let (status, answer) = post(router, conversation, message_body(text)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{answer}");
    answer["turn"].as_str().expect("a turn").to_owned()
}

/// The turn's state once it is no longer `running`, within 20 s of wall clock.
pub async fn settled(pool: &PgPool, turn: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let state: String = sqlx::query_scalar("SELECT state FROM turns WHERE id = $1")
            .bind(turn)
            .fetch_one(pool)
            .await
            .expect("the turn");
        if state != "running" {
            return state;
        }
        assert!(Instant::now() < deadline, "turn {turn} still runs");
        sleep(Duration::from_millis(20)).await;
    }
}

pub async fn entries(pool: &PgPool, conversation: &str) -> Vec<(String, Value, Option<String>)> {
    sqlx::query_as("SELECT kind, body, turn FROM journal WHERE conversation = $1 ORDER BY seq")
        .bind(conversation)
        .fetch_all(pool)
        .await
        .expect("the journal")
}

pub fn kinds(entries: &[(String, Value, Option<String>)]) -> Vec<&str> {
    entries.iter().map(|(kind, _, _)| kind.as_str()).collect()
}

pub fn body_of<'a>(entries: &'a [(String, Value, Option<String>)], kind: &str) -> &'a Value {
    &entries.iter().find(|(held, _, _)| held == kind).unwrap_or_else(|| panic!("no {kind}")).1
}

/// The tool results the last user message of `request` carries, as JSON.
pub fn results(request: &ModelRequest) -> Vec<Value> {
    let Some(Message::User { content }) = request.history.last() else { return Vec::new() };
    (content.iter())
        .filter_map(|part| match part {
            UserContent::ToolResult(result) => Some(json!({ "is_error": result.is_error, "content": result.content })),
            _ => None,
        })
        .collect()
}

pub fn texts(message: &Message) -> String {
    match message {
        Message::User { content } => (content.iter())
            .filter_map(|part| match part {
                UserContent::Text(text) => Some(text.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Message::Assistant(said) => said.text(),
        Message::System { content } => content.clone(),
    }
}

pub fn call(tool: &str, input: Value) -> ScriptedBlock {
    ScriptedBlock::Call { tool: wire_name(tool), input }
}

pub fn question() -> Value {
    json!({ "items": [{ "header": "Shape", "text": "Which shape?", "options": [
        { "label": "L", "description": "two walls" }, { "label": "U", "description": "three walls" } ] }] })
}

/// The input of Create site the scripted model asks for on `draft`.
pub fn create(draft: &str, slug: &str) -> Value {
    json!({ "draft": draft, "evaluation": 1, "customer": "demo", "slug": slug, "name": format!("Agent {slug}") })
}
