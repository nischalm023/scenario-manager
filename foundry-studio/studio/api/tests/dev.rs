//! Brief I1.04 section 5, the test writer: the journal fixtures replay through it into fresh conversations, a stream
//! hearing them as the agent loop's entries would be heard; it refuses a body the projection cannot read, answers a
//! repeated dedup key with the entry that holds it, and is mounted only where the service runs locally.

mod support;

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use studio_api::config::Deployment;
use studio_api::projects::make_local_default;
use studio_api::{AppState, gate_routes};
use studio_protocol::{EntriesPage, JournalEntry, OpenedConversation, Ulid};
use support::sse::EventReader;
use tower::ServiceExt;
use wps_client::BlockingCompute;

const SOON: Duration = Duration::from_secs(5);

async fn ready(pool: &PgPool) -> (AppState, Router) {
    make_local_default(pool).await.expect("the default project");
    let state = support::state(pool.clone(), support::gate_rules());
    (state.clone(), gate_routes(support::gate(state).await))
}

async fn open(router: &Router, site: Option<&str>) -> Ulid {
    let body = site.map_or_else(|| json!({}), |site| json!({ "site": site }));
    let (_, opened) = support::json(router, "POST", "/api/conversations", "dev", Some(body)).await;
    serde_json::from_value::<OpenedConversation>(opened).expect("opened").conversation.id
}

async fn post(router: &Router, conversation: &Ulid, body: Value) -> (StatusCode, Value) {
    support::json(router, "POST", &format!("/api/dev/conversations/{conversation}/entries"), "dev", Some(body)).await
}

/// A fixture's lines: each journaled entry, or a live item as the test writer takes it.
fn fixture(name: &str) -> Vec<Value> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../studio/protocol/tests/journal").join(name);
    let text = fs::read_to_string(&path).unwrap_or_else(|fault| panic!("{}: {fault}", path.display()));
    text.lines().map(|line| serde_json::from_str(line).expect("a JSON line")).collect()
}

/// Brief I1.04 section 5: each fixture replays through the test writer into a fresh conversation, every line taken;
/// the journal then holds the fixture's entries under the fixture's numbers, with its kinds, turns, causes and
/// bodies; and a stream open meanwhile hears every entry under its `id:` and every live item without one.
#[tokio::test]
async fn the_fixtures_replay_through_the_test_writer() {
    let scratch = scratch_or_skip!();
    let (_, router) = ready(&scratch.pool).await;
    let replays = [
        ("new-site-thread.jsonl", None),
        ("site-conversation.jsonl", Some("customer/demo/site/lshape-6")),
        ("ended-turns.jsonl", Some("customer/demo/site/ushape-4")),
    ];
    for (name, site) in replays {
        let conversation = open(&router, site).await;
        let uri = format!("/api/conversations/{conversation}/stream");
        let request = support::from_gate("GET", &uri, "dev", "").body(Body::empty()).expect("a request");
        let mut stream = EventReader::new(router.clone().oneshot(request).await.expect("an answer").into_body());
        stream.until("stream.live", SOON).await;

        let lines = fixture(name);
        let mut expected: Vec<JournalEntry> = Vec::new();
        let mut live_kinds = Vec::new();
        for line in &lines {
            let (status, answer) = if let Some(live) = line.get("live") {
                live_kinds.push(live["kind"].as_str().expect("a kind").to_owned());
                post(&router, &conversation, json!({ "live": live })).await
            } else {
                let entry: JournalEntry = serde_json::from_value(line.clone()).expect("an entry");
                let written =
                    json!({ "kind": entry.kind, "body": entry.body, "turn": entry.turn, "caused_by": entry.caused_by });
                expected.push(entry);
                post(&router, &conversation, written).await
            };
            let wanted = if line.get("live").is_some() { StatusCode::ACCEPTED } else { StatusCode::CREATED };
            assert_eq!(status, wanted, "{name}: {line} answered {answer}");
        }

        let uri = format!("/api/conversations/{conversation}/entries?after=0&limit=500");
        let (_, page) = support::json(&router, "GET", &uri, "dev", None).await;
        let page: EntriesPage = serde_json::from_value(page).expect("a page");
        let shape = |entry: &JournalEntry| {
            (entry.seq, entry.kind, entry.turn.clone(), entry.caused_by.clone(), entry.body.clone())
        };
        let replayed: Vec<_> = page.entries.iter().map(shape).collect();
        assert_eq!(replayed, expected.iter().map(shape).collect::<Vec<_>>(), "{name}");

        let last = expected.last().expect("entries").seq.get().to_string();
        let mut heard_live = Vec::new();
        loop {
            let event = stream.next(SOON).await.expect("the stream goes on");
            match &event.id {
                None if event.kind() != "ping" => heard_live.push(event.kind().to_owned()),
                Some(id) if *id == last => break,
                _ => {}
            }
        }
        live_kinds.dedup();
        heard_live.dedup();
        assert_eq!(heard_live, live_kinds, "{name}: the live items, without ids, in order");
    }
    scratch.close().await;
}

/// Brief I1.04 section 5: the test writer refuses what the projection could not read (a turn's entry naming no
/// turn, a call's end that ends nothing, a decision that is neither, a panel command off spec 03's shape, an unknown
/// kind, a delta that is not one, and a panel event of a kind no spec names or carrying a field beyond its three, as
/// `POST …/panel-events` refuses them, the two on one type) with `validation-failed`, and writes none of it.
#[tokio::test]
async fn the_test_writer_refuses_what_the_projection_cannot_read() {
    let scratch = scratch_or_skip!();
    let (_, router) = ready(&scratch.pool).await;
    let conversation = open(&router, None).await;
    let turn = support::ulid();
    let refused = [
        json!({ "kind": "turn.started", "body": { "n": 1, "author": { "subject": "dev", "name": "Developer" }, "message_seq": 1, "attended": true } }),
        json!({ "kind": "tool.finished", "turn": turn, "body": { "call": "toolu_1", "outcome": "running", "summary": "Still going", "refs": [] } }),
        json!({ "kind": "approval.decided", "turn": turn, "body": { "approval": "01KN0TJZANP5N0PGJD8F7A3A01", "decision": "maybe" } }),
        json!({ "kind": "panel.command", "body": { "action": "dance" } }),
        json!({ "kind": "stream.live", "body": { "last_seq": 1 } }),
        json!({ "live": { "kind": "text.delta", "body": { "block": "b1", "offset": 3, "delta": "x" } } }),
        json!({ "kind": "system.note", "body": { "text": "Hi" }, "colour": "red" }),
        json!({ "kind": "panel.event", "body": { "kind": "panel.danced", "text": "You danced.", "body": {} } }),
        json!({ "kind": "panel.event", "body": { "kind": "knob.changed", "text": "Changed.", "body": {}, "extra": 1 } }),
    ];
    for body in refused {
        let (status, problem) = post(&router, &conversation, body.clone()).await;
        assert_eq!(
            (status, problem["type"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("validation-failed")),
            "{body}"
        );
    }
    let uri = format!("/api/conversations/{conversation}/entries");
    let (_, page) = support::json(&router, "GET", &uri, "dev", None).await;
    assert_eq!(page["entries"], json!([]), "nothing was written");
    scratch.close().await;
}

/// Brief I1.04 section 5 and spec 01 section 3.5's dedup keys through the test writer: a repeated key answers
/// `200 { held }` naming the entry that holds it; an archived conversation takes neither entries nor live items
/// (`conversation-closed`); an unknown one is not found.
#[tokio::test]
async fn a_repeated_key_is_held_and_a_closed_conversation_refuses() {
    let scratch = scratch_or_skip!();
    let (_, router) = ready(&scratch.pool).await;
    let conversation = open(&router, Some("customer/demo/site/lshape-6")).await;
    let keyed = json!({ "kind": "run.status", "body": { "run": "01KN0TJZANP5N0PGJD8F7A3R01", "state": "queued" }, "dedup_key": "run.status:r:1:queued" });
    let (status, wrote) = post(&router, &conversation, keyed.clone()).await;
    assert_eq!((status, wrote["entry"]["seq"].as_u64()), (StatusCode::CREATED, Some(1)));
    let (status, held) = post(&router, &conversation, keyed).await;
    assert_eq!((status, held), (StatusCode::OK, json!({ "held": 1 })));

    let (_, _) =
        support::json(&router, "POST", &format!("/api/conversations/{conversation}/dismiss"), "dev", None).await;
    let note = json!({ "kind": "system.note", "body": { "text": "Too late" } });
    let (status, problem) = post(&router, &conversation, note.clone()).await;
    assert_eq!((status, problem["type"].as_str()), (StatusCode::CONFLICT, Some("conversation-closed")));
    let live =
        json!({ "live": { "kind": "run.progress", "body": { "run": "r", "attempt": 1, "sim_ms": 0, "values": [] } } });
    let (status, problem) = post(&router, &conversation, live).await;
    assert_eq!((status, problem["type"].as_str()), (StatusCode::CONFLICT, Some("conversation-closed")));
    let (status, problem) = post(&router, &support::ulid(), note).await;
    assert_eq!((status, problem["type"].as_str()), (StatusCode::NOT_FOUND, Some("not-found")));
    scratch.close().await;
}

/// Brief I1.04 section 5: a cluster never mounts the test writer; its path is not found there.
#[tokio::test]
async fn a_cluster_never_mounts_the_test_writer() {
    let scratch = scratch_or_skip!();
    let (compute, runstore) = (Arc::new(BlockingCompute), support::runstore());
    let state = AppState::new(
        scratch.pool.clone(),
        support::gate_rules(),
        Deployment::Cluster,
        support::MODEL_LABEL,
        compute,
        runstore,
    );
    let router = gate_routes(support::gate(state).await);
    let uri = format!("/api/dev/conversations/{}/entries", support::ulid());
    let request = support::from_gate("POST", &uri, "dev", "").header("content-type", "application/json");
    let (status, _, body) = support::call(&router, request.body(Body::from("{}")).expect("a request")).await;
    let problem: Value = serde_json::from_slice(&body).expect("a problem");
    assert_eq!((status, problem["type"].as_str()), (StatusCode::NOT_FOUND, Some("not-found")));
    scratch.close().await;
}
