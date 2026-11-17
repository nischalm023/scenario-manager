//! Spec 01 section 3.5's stream through `GET /api/conversations/{c}/stream`: section 8's statement 2 (exactly k+1
//! onward after `Last-Event-ID` k, then live entries; no `id:` on a delta; `stream.live` after the replay; a `ping`
//! within 15 s; the offset-0 delta for a subscriber joining mid-block; `slow-client`), and brief I1.04 section 4's
//! cursors and framing.

mod support;

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use studio_api::journal::{NewEntry, PING_EVERY, SUBSCRIBER_BUFFER};
use studio_api::projects::make_local_default;
use studio_api::{AppState, gate_routes};
use studio_protocol::{
    BlockId, JournalEntry, JournalKind, LiveItem, LiveKind, OpenedConversation, TextDelta, TextOffset, Ulid, WallClock,
};
use support::sse::{EventReader, SseEvent};
use tower::ServiceExt;

const SITE: &str = "customer/demo/site/lshape-6";
const SOON: Duration = Duration::from_secs(5);

fn note(text: &str) -> NewEntry {
    NewEntry::new(JournalKind::SystemNote, json!({ "text": text }))
}

async fn ready(pool: &PgPool) -> (AppState, Router, Ulid) {
    make_local_default(pool).await.expect("the default project");
    let state = support::state(pool.clone(), support::gate_rules());
    let router = gate_routes(support::gate(state.clone()).await);
    let (_, opened) = support::json(&router, "POST", "/api/conversations", "u-1", Some(json!({ "site": SITE }))).await;
    let conversation = serde_json::from_value::<OpenedConversation>(opened).expect("opened").conversation.id;
    (state, router, conversation)
}

/// Open the stream with `last_event_id` as the browser's reconnect sends it, and `query` on the address.
async fn open(
    router: &Router,
    conversation: &Ulid,
    query: &str,
    last_event_id: Option<&str>,
) -> (StatusCode, EventReader) {
    let uri = format!("/api/conversations/{conversation}/stream{query}");
    let mut request = support::from_gate("GET", &uri, "u-1", "").header("accept", "text/event-stream");
    if let Some(id) = last_event_id {
        request = request.header("last-event-id", id);
    }
    let answer = router.clone().oneshot(request.body(Body::empty()).expect("a request")).await.expect("an answer");
    let (parts, body) = answer.into_parts();
    let headers = &parts.headers;
    if parts.status == StatusCode::OK {
        assert_eq!(headers.get("content-type").and_then(|v| v.to_str().ok()), Some("text/event-stream"));
        assert_eq!(headers.get("cache-control").and_then(|v| v.to_str().ok()), Some("no-cache"));
        assert_eq!(headers.get("x-accel-buffering").and_then(|v| v.to_str().ok()), Some("no"));
    }
    (parts.status, EventReader::new(body))
}

/// The journaled entries among `events`, each checked to carry its number as its `id:` and its kind as its `event:`.
fn entries(events: &[SseEvent]) -> Vec<JournalEntry> {
    events
        .iter()
        .filter(|event| event.id.is_some())
        .map(|event| {
            let entry: JournalEntry = serde_json::from_str(&event.data).expect("an envelope");
            assert_eq!(event.id.as_deref(), Some(entry.seq.get().to_string().as_str()));
            assert_eq!(event.kind(), entry.kind.as_str());
            entry
        })
        .collect()
}

fn numbers(entries: &[JournalEntry]) -> Vec<u64> {
    entries.iter().map(|entry| entry.seq.get()).collect()
}

/// Spec 01 section 8, statement 2: a reconnect sending `Last-Event-ID` k receives exactly the entries after k, in
/// order, then `stream.live` naming the last, then live entries as they commit; the header wins over the stale
/// `?after=` the browser's reconnect reuses, and `?after=` alone is the cursor otherwise. The stream opens with
/// `retry: 2000`.
#[tokio::test]
async fn a_reconnect_gets_exactly_the_later_entries_then_live_ones() {
    let scratch = scratch_or_skip!();
    let (state, router, conversation) = ready(&scratch.pool).await;
    for n in 1..=5 {
        support::write(&state, &conversation, note(&format!("note {n}"))).await;
    }
    let (status, mut stream) = open(&router, &conversation, "?after=1", Some("3")).await;
    assert_eq!(status, StatusCode::OK);
    let first = stream.next(SOON).await.expect("the first event");
    assert_eq!((first.retry.as_deref(), first.data.as_str()), (Some("2000"), ""));
    let caught_up = stream.until("stream.live", SOON).await;
    assert_eq!(numbers(&entries(&caught_up)), [4, 5]);
    let live = caught_up.last().expect("stream.live");
    assert_eq!((live.id.as_ref(), live.json()), (None, json!({ "last_seq": 5 })));
    support::write(&state, &conversation, note("note 6")).await;
    let next = stream.next(SOON).await.expect("the live entry");
    assert_eq!(numbers(&entries(&[next])), [6]);

    let (_, mut stale) = open(&router, &conversation, "?after=2", None).await;
    assert_eq!(numbers(&entries(&stale.until("stream.live", SOON).await)), [3, 4, 5, 6]);
    scratch.close().await;
}

/// Spec 01 section 8, statement 2: a subscriber joining while a block streams first gets one delta at offset 0
/// holding the text so far; deltas and other live items carry no `id:` line, so a browser's last event id stays at
/// the last journaled entry; a block whose `text.done` the replay sent is not sent again as a delta.
#[tokio::test]
async fn a_subscriber_joining_mid_block_starts_from_offset_0_and_live_items_carry_no_id() {
    let scratch = scratch_or_skip!();
    let (state, router, conversation) = ready(&scratch.pool).await;
    support::write(&state, &conversation, note("before the block")).await;
    let delta = |block: &str, offset, text: &str| TextDelta {
        block: BlockId::new(block),
        offset: TextOffset::new(offset),
        delta: text.to_owned(),
    };
    state.hub.text_delta(&conversation, delta("b1", 0, "Aisle 3 ")).expect("placed");
    state.hub.text_delta(&conversation, delta("b1", 8, "holds the slow picks")).expect("placed");
    let (_, mut stream) = open(&router, &conversation, "", None).await;
    let caught_up = stream.until("stream.live", SOON).await;
    let deltas: Vec<&SseEvent> = caught_up.iter().filter(|event| event.kind() == "text.delta").collect();
    assert_eq!(deltas.len(), 1);
    assert_eq!(deltas[0].id, None);
    assert_eq!(deltas[0].json(), json!({ "block": "b1", "offset": 0, "delta": "Aisle 3 holds the slow picks" }));

    // Placed as a page places them, the deltas that follow, a coalesced one the subscriber already had included,
    // leave the block's text whole.
    let mut text: Vec<u16> = "Aisle 3 holds the slow picks".encode_utf16().collect();
    state.hub.text_delta(&conversation, delta("b1", 28, ".")).expect("placed");
    while text.len() < 29 {
        let next = stream.next(SOON).await.expect("a delta");
        assert_eq!((next.kind(), next.id.as_ref()), ("text.delta", None));
        let offset = next.json()["offset"].as_u64().expect("an offset") as usize;
        let placed: Vec<u16> = next.json()["delta"].as_str().expect("text").encode_utf16().collect();
        let end = (offset + placed.len()).min(text.len());
        text.splice(offset..end, placed);
    }
    assert_eq!(String::from_utf16(&text).expect("text"), "Aisle 3 holds the slow picks.");
    let progress =
        LiveItem { kind: LiveKind::RunProgress, body: json!({ "run": "01KN0TJZANP5N0PGJD8F7A3R01", "sim_ms": 60000 }) };
    state.hub.live(&conversation, progress).expect("delivered");
    let next = stream.next(SOON).await.expect("the progress");
    assert_eq!((next.kind(), next.id.as_ref(), next.json()["sim_ms"].as_u64()), ("run.progress", None, Some(60000)));

    // A block's entry names a turn the conversation has started (migration 0018's journal.turn reference).
    let turn = support::ulid();
    let author = json!({ "subject": "dev", "name": "Developer" });
    let started = json!({ "n": 1, "author": author, "message_seq": 1, "attended": true, "browser_tab": null });
    let mut started = NewEntry::new(JournalKind::TurnStarted, started);
    started.turn = Some(turn.clone());
    support::write(&state, &conversation, started).await;
    let done = json!({ "block": "b1", "kind": "text", "text": "Aisle 3 holds the slow picks." });
    let mut done = NewEntry::new(JournalKind::TextDone, done);
    done.turn = Some(turn);
    support::write(&state, &conversation, done).await;
    let (_, mut later) = open(&router, &conversation, "", None).await;
    let caught_up = later.until("stream.live", SOON).await;
    assert!(caught_up.iter().all(|event| event.kind() != "text.delta"), "the block is done: {caught_up:?}");
    scratch.close().await;
}

/// Spec 01 section 8, statement 2: a `ping` event, without an `id:` line, arrives within 15 s of wall clock and names
/// the wall-clock time it was sent.
#[tokio::test]
async fn a_ping_arrives_within_15_s() {
    let scratch = scratch_or_skip!();
    let (_, router, conversation) = ready(&scratch.pool).await;
    let (_, mut stream) = open(&router, &conversation, "", None).await;
    stream.until("stream.live", SOON).await;
    let ping = stream.next(PING_EVERY + Duration::from_secs(1)).await.expect("a ping");
    assert_eq!((ping.kind(), ping.id.as_ref()), ("ping", None));
    let at: WallClock = serde_json::from_value(ping.json()["at"].clone()).expect("a wall-clock time");
    assert!(at <= WallClock::now());
    scratch.close().await;
}

/// Spec 01 section 8, statement 2: a client that stops reading while more than 1,000 items arrive gets
/// `stream.reset` with `slow-client`, after what was already on its way, and the stream ends.
#[tokio::test]
async fn a_slow_client_is_reset() {
    let scratch = scratch_or_skip!();
    let (state, router, conversation) = ready(&scratch.pool).await;
    let (_, mut stream) = open(&router, &conversation, "", None).await;
    stream.until("stream.live", SOON).await;
    for n in 0..=2 * SUBSCRIBER_BUFFER {
        let item = LiveItem { kind: LiveKind::TemplateEvaluation, body: json!({ "n": n }) };
        state.hub.live(&conversation, item).expect("delivered");
    }
    let mut last = None;
    let mut heard = 0;
    while let Some(event) = stream.next(SOON).await {
        heard += 1;
        last = Some(event);
    }
    let last = last.expect("events before the end");
    assert_eq!((last.kind(), last.json()), ("stream.reset", json!({ "reason": "slow-client" })));
    assert!(heard < SUBSCRIBER_BUFFER, "the reset came long before the items it dropped: {heard}");
    scratch.close().await;
}

/// Brief I1.04 section 4: one sequence spans a site's conversations, so on the site's next conversation a cursor
/// past its last entry is reset as `unknown-cursor`, and one below its first entry replays from that entry; an
/// unknown conversation is not found, and a cursor that is no number fails the check.
#[tokio::test]
async fn cursors_past_or_before_a_conversations_numbers() {
    let scratch = scratch_or_skip!();
    let (state, router, conversation) = ready(&scratch.pool).await;
    for n in 1..=3 {
        support::write(&state, &conversation, note(&format!("note {n}"))).await;
    }
    let path = format!("/api/conversations/{conversation}/dismiss");
    let (_, dismissed) = support::json(&router, "POST", &path, "u-1", None).await;
    let next: Ulid = serde_json::from_value(dismissed["live"]["id"].clone()).expect("the next conversation");
    support::write(&state, &next, note("note 5")).await;

    let (_, mut past) = open(&router, &next, "", Some("9")).await;
    let reset = past.until("stream.reset", SOON).await;
    assert_eq!(reset.last().map(SseEvent::json), Some(json!({ "reason": "unknown-cursor" })));
    assert_eq!(past.next(SOON).await, None, "the stream ends");
    let (_, mut before) = open(&router, &next, "?after=1", None).await;
    assert_eq!(numbers(&entries(&before.until("stream.live", SOON).await)), [5]);

    let (status, _) = open(&router, &support::ulid(), "", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = open(&router, &next, "", Some("five")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let request = support::from_gate("GET", &format!("/api/conversations/{next}/stream?after=-1"), "u-1", "");
    let (status, _, body) = support::call(&router, request.body(Body::empty()).expect("a request")).await;
    let problem: Value = serde_json::from_slice(&body).expect("a problem");
    assert_eq!((status, problem["type"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("validation-failed")));
    scratch.close().await;
}
