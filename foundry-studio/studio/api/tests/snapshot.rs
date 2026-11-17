//! Spec 01 section 3.6's snapshot and entries pages: section 8's statement 12 (a snapshot whose 200 entries begin
//! inside a turn draws what a replay from the first entry draws, and paging back changes no id the tab holds), the
//! active turn with its streaming blocks, and the pages' bounds.

mod support;

use std::fmt::Debug;

use axum::Router;
use axum::http::StatusCode;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sqlx::PgPool;
use studio_api::journal::NewEntry;
use studio_api::projects::make_local_default;
use studio_api::{AppState, gate_routes};
use studio_protocol::{
    BlockId, CallState, DraftState, EntriesPage, JournalKind, JournalSeq, OpenedConversation, Projection, Snapshot,
    TextDelta, TextOffset, TurnState, Ulid, WaitingOn,
};

const SITE: &str = "customer/demo/site/lshape-6";

fn in_turn(kind: JournalKind, turn: &Ulid, body: Value) -> NewEntry {
    let mut entry = NewEntry::new(kind, body);
    entry.turn = Some(turn.clone());
    entry
}

fn turn_start(turn: &Ulid, n: u32, message_seq: u64) -> Vec<NewEntry> {
    let author = json!({ "subject": "u-1", "name": "Person u-1" });
    let message =
        json!({ "author": author, "text": "Go on", "context": null, "source": "composer", "browser_tab": "tab-1" });
    let started =
        json!({ "n": n, "author": author, "message_seq": message_seq, "attended": true, "browser_tab": "tab-1" });
    vec![in_turn(JournalKind::MessageUser, turn, message), in_turn(JournalKind::TurnStarted, turn, started)]
}

fn call(turn: &Ulid, call: &str) -> Vec<NewEntry> {
    let started = json!({ "call": call, "tool": "flow.read", "class": "read", "summary": format!("Read {call}") });
    let finished = json!({ "call": call, "outcome": "ok", "summary": format!("Read {call}: 3 nodes"), "refs": [] });
    vec![in_turn(JournalKind::ToolStarted, turn, started), in_turn(JournalKind::ToolFinished, turn, finished)]
}

struct Built {
    state: AppState,
    router: Router,
    conversation: Ulid,
    second: Ulid,
}

/// A site's conversation of 260 entries: turn 1 done (1 to 6); turn 2 (from 7) asks an approval that is approved
/// and executed, drafts, makes 120 calls, watches a run, and ends waiting on a question (260).
async fn built(pool: &PgPool) -> Built {
    make_local_default(pool).await.expect("the default project");
    let state = support::state(pool.clone(), support::gate_rules());
    let router = gate_routes(support::gate(state.clone()).await);
    let (_, opened) = support::json(&router, "POST", "/api/conversations", "u-1", Some(json!({ "site": SITE }))).await;
    let conversation = serde_json::from_value::<OpenedConversation>(opened).expect("opened").conversation.id;
    let (first, second) = (support::ulid(), support::ulid());
    let (approval, draft, run, question) = (support::ulid(), support::ulid(), support::ulid(), support::ulid());
    let mut entries = turn_start(&first, 1, 1);
    entries.push(in_turn(JournalKind::TextDone, &first, json!({ "block": "b1", "kind": "text", "text": "Reading." })));
    entries.extend(call(&first, "toolu_first"));
    entries.push(in_turn(
        JournalKind::TurnFinished,
        &first,
        json!({ "outcome": "done", "reply_block": "b1", "usage": {} }),
    ));
    entries.extend(turn_start(&second, 2, 7));
    let asking = json!({ "call": "toolu_apply", "tool": "flow.apply", "class": "asking", "summary": "Apply the flow" });
    entries.push(in_turn(JournalKind::ToolStarted, &second, asking));
    let requested = json!({ "approval": approval, "call": "toolu_apply", "tool": "flow.apply", "class": "asking", "summary": "Apply the flow", "input": {}, "facts": {} });
    entries.push(in_turn(JournalKind::ApprovalRequested, &second, requested));
    entries.push(in_turn(JournalKind::TurnWaiting, &second, json!({ "on": "approval", "id": approval })));
    let author = json!({ "subject": "u-1", "name": "Person u-1" });
    entries.push(in_turn(
        JournalKind::ApprovalDecided,
        &second,
        json!({ "approval": approval, "decision": "approve", "author": author }),
    ));
    entries.push(in_turn(JournalKind::TurnResumed, &second, json!({ "by": "decision" })));
    entries.push(in_turn(JournalKind::ApprovalExecution, &second, json!({ "approval": approval, "execution": "ok" })));
    let applied =
        json!({ "call": "toolu_apply", "outcome": "ok", "summary": "Applied as flow changeset 2", "refs": [] });
    entries.push(in_turn(JournalKind::ToolFinished, &second, applied));
    entries.push(in_turn(
        JournalKind::DraftUpdated,
        &second,
        json!({ "draft": draft, "state": "ready", "ops_total": 1 }),
    ));
    for n in 0..120 {
        entries.extend(call(&second, &format!("toolu_{n:03}")));
    }
    entries.push(NewEntry::new(JournalKind::RunStatus, json!({ "run": run, "state": "queued" })));
    let ask = json!({ "call": "toolu_ask", "tool": "ask_user", "class": "read", "summary": "Ask which aisle" });
    entries.push(in_turn(JournalKind::ToolStarted, &second, ask));
    entries.push(in_turn(
        JournalKind::QuestionAsked,
        &second,
        json!({ "question": question, "call": "toolu_ask", "items": [] }),
    ));
    entries.push(in_turn(
        JournalKind::TurnWaiting,
        &second,
        json!({ "on": "question", "id": question, "reply_block": "b2" }),
    ));
    support::write_all(&state, &conversation, entries).await;
    Built { state, router, conversation, second }
}

async fn read<T: DeserializeOwned>(router: &Router, uri: &str) -> T {
    let (status, answer) = support::json(router, "GET", uri, "u-2", None).await;
    assert_eq!(status, StatusCode::OK, "{uri}: {answer}");
    serde_json::from_value(answer).expect("the answer reads")
}

/// Every object of `part` is the same object, under the same id, in `whole`; with `only_shared`, only those `whole`
/// holds too need be.
fn same_objects(part: &Projection, whole: &Projection, only_shared: bool) {
    fn check<T: PartialEq + Debug, K: PartialEq>(part: &[T], whole: &[T], key: impl Fn(&T) -> K, only_shared: bool) {
        for object in part {
            match whole.iter().find(|other| key(other) == key(object)) {
                Some(other) => assert_eq!(object, other),
                None => assert!(only_shared, "{object:?} is missing"),
            }
        }
    }
    check(&part.turns, &whole.turns, |turn| turn.id.clone(), only_shared);
    check(&part.calls, &whole.calls, |call| call.call.clone(), only_shared);
    check(&part.approvals, &whole.approvals, |approval| approval.id.clone(), only_shared);
    check(&part.questions, &whole.questions, |question| question.id.clone(), only_shared);
    check(&part.drafts, &whole.drafts, |draft| draft.id.clone(), only_shared);
    check(&part.runs, &whole.runs, |run| run.run.clone(), only_shared);
}

/// Spec 01 section 8, statement 12: a snapshot's 200 entries begin inside turn 2, and its projection holds what a
/// replay from the first entry draws for them: the turn's header and state, every call of the turn with its final
/// state though most started before the window, the approval decided before it, the open draft and question, and the
/// run its window names; each the very object a page of every entry projects. Paging back to the entries before the
/// first held changes no object the tab holds.
#[tokio::test]
async fn a_snapshot_starting_inside_a_turn_draws_what_a_replay_draws() {
    let scratch = scratch_or_skip!();
    let built = built(&scratch.pool).await;
    let snapshot: Snapshot = read(&built.router, &format!("/api/conversations/{}", built.conversation)).await;
    let numbers: Vec<u64> = snapshot.entries.iter().map(|entry| entry.seq.get()).collect();
    assert_eq!(numbers, (61..=260).collect::<Vec<_>>(), "the last 200, oldest first");
    assert_eq!((snapshot.last_seq, snapshot.conversation.last_seq), (JournalSeq::new(260), JournalSeq::new(260)));
    assert_eq!(snapshot.conversation.model_label, support::MODEL_LABEL);

    let projection = &snapshot.projection;
    let turns: Vec<(u32, TurnState, Option<WaitingOn>)> =
        projection.turns.iter().map(|turn| (turn.n.get(), turn.state, turn.waiting_on)).collect();
    assert_eq!(turns, [(2, TurnState::Waiting, Some(WaitingOn::Question))], "turn 1 lies before the window");
    assert_eq!(projection.calls.len(), 122, "the asking call, 120 reads and the question's call");
    assert!(projection.calls.iter().all(|call| call.turn.as_ref() == Some(&built.second)));
    assert_eq!(projection.calls[0].state, CallState::Ok, "the approved call, done before the window");
    assert_eq!(projection.approvals[0].as_of_seq, JournalSeq::new(14));
    assert_eq!(projection.drafts[0].state, DraftState::Ready);
    assert_eq!((projection.questions.len(), projection.runs.len()), (1, 1));
    assert_eq!(snapshot.pending.questions, [projection.questions[0].id.clone()]);
    assert_eq!(snapshot.pending.drafts, [projection.drafts[0].id.clone()]);
    assert!(snapshot.pending.approvals.is_empty());
    let active = snapshot.active_turn.as_ref().expect("turn 2 waits");
    assert_eq!((&active.id, active.n.get(), active.state), (&built.second, 2, TurnState::Waiting));
    assert!(active.streaming.is_empty());

    let everything: EntriesPage =
        read(&built.router, &format!("/api/conversations/{}/entries?after=0&limit=500", built.conversation)).await;
    assert_eq!((everything.entries.len(), everything.next), (260, None));
    same_objects(projection, &everything.projection, false);
    assert_eq!(everything.projection.turns.len(), 2, "the replay draws turn 1 too");

    // Spec 02 section 3.3's page back, `after={first held - 201}&limit=200`, held to the entries before the first.
    let first_held = snapshot.entries[0].seq.get();
    let after = first_held.saturating_sub(201);
    let limit = (first_held - 1 - after).min(200);
    let uri = format!("/api/conversations/{}/entries?after={after}&limit={limit}", built.conversation);
    let earlier: EntriesPage = read(&built.router, &uri).await;
    let numbers: Vec<u64> = earlier.entries.iter().map(|entry| entry.seq.get()).collect();
    assert_eq!(numbers, (1..first_held).collect::<Vec<_>>());
    assert_eq!(earlier.next, Some(JournalSeq::new(first_held - 1)), "more follow it");
    same_objects(&earlier.projection, &everything.projection, false);
    same_objects(&earlier.projection, projection, true);
    scratch.close().await;
}

/// Spec 01 section 3.6: a snapshot's active turn carries each block still streaming with its text so far, from the
/// hub, as a reload while the agent writes needs.
#[tokio::test]
async fn a_snapshot_shows_the_blocks_still_streaming() {
    let scratch = scratch_or_skip!();
    let built = built(&scratch.pool).await;
    let delta = TextDelta { block: BlockId::new("b9"), offset: TextOffset::new(0), delta: "Aisle 3 is".to_owned() };
    built.state.hub.text_delta(&built.conversation, delta).expect("placed");
    let snapshot: Snapshot = read(&built.router, &format!("/api/conversations/{}", built.conversation)).await;
    let streaming = &snapshot.active_turn.expect("turn 2 waits").streaming;
    assert_eq!((streaming[0].block.as_str(), streaming[0].text.as_str()), ("b9", "Aisle 3 is"));
    scratch.close().await;
}

/// Spec 01 section 3.6: a page reads from the conversation's first entry when `after` is below it, as for a site's
/// next conversation, whose numbers start past 1, and finds nothing past its last, however large; an empty
/// conversation's snapshot holds nothing; an unknown or malformed id is not found.
#[tokio::test]
async fn pages_start_at_the_conversations_first_number() {
    let scratch = scratch_or_skip!();
    let built = built(&scratch.pool).await;
    let path = format!("/api/conversations/{}/dismiss", built.conversation);
    let (status, problem) = support::json(&built.router, "POST", &path, "u-1", None).await;
    assert_eq!(status, StatusCode::CONFLICT, "turn 2 still waits: {problem}");
    support::write_all(
        &built.state,
        &built.conversation,
        vec![in_turn(JournalKind::TurnFinished, &built.second, json!({ "outcome": "stopped", "usage": {} }))],
    )
    .await;
    let (_, dismissed) = support::json(&built.router, "POST", &path, "u-1", None).await;
    let next: Ulid = serde_json::from_value(dismissed["live"]["id"].clone()).expect("the next conversation");
    let empty: Snapshot = read(&built.router, &format!("/api/conversations/{next}")).await;
    assert_eq!(
        (empty.entries.len(), empty.last_seq, empty.conversation.first_seq),
        (0, JournalSeq::new(262), JournalSeq::new(263))
    );
    assert_eq!((empty.projection, empty.active_turn), (Projection::default(), None));
    support::write_all(&built.state, &next, vec![NewEntry::new(JournalKind::SystemNote, json!({ "text": "Hello" }))])
        .await;
    let page: EntriesPage = read(&built.router, &format!("/api/conversations/{next}/entries?after=0")).await;
    assert_eq!(page.entries.iter().map(|entry| entry.seq.get()).collect::<Vec<_>>(), [263]);
    let past_every_number = format!("/api/conversations/{next}/entries?after={}", u64::MAX);
    let empty: EntriesPage = read(&built.router, &past_every_number).await;
    assert!(empty.entries.is_empty() && empty.next.is_none(), "a cursor past BIGINT reads past every entry");
    for uri in ["/api/conversations/01KN0TJZANP5N0PGJD8F7A3ZZZ", "/api/conversations/nope/entries"] {
        let (status, problem) = support::json(&built.router, "GET", uri, "u-1", None).await;
        assert_eq!((status, problem["type"].as_str()), (StatusCode::NOT_FOUND, Some("not-found")), "{uri}");
    }
    scratch.close().await;
}
