//! Spec 01 section 3.17, a site's conversation, through the routes of section 3.6 and the binding Create site calls:
//! section 8's statement 16 (conversations by site; Dismiss refused while a written turn waits; the archived
//! conversation closed; the next one numbering on; a run's entry racing a Dismiss), opening, the site's list, and the
//! local default project of brief I1.04.

mod support;

use std::time::Duration;

use axum::Router;
use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use studio_api::conversations::{ConversationError, bind_thread, load};
use studio_api::journal::{Appended, HubItem, JournalError, NewEntry, append, append_to_site};
use studio_api::projects::make_local_default;
use studio_api::{AppState, gate_routes};
use studio_protocol::{
    Conversation, ConversationChanged, ConversationState, JournalKind, JournalSeq, OpenedConversation, Person,
    SitePath, Ulid,
};
use tokio::task::JoinSet;
use tokio::time::sleep;

const SITE: &str = "customer/demo/site/lshape-6";

/// The service with its default project, its world holding every site this file opens.
async fn ready(pool: &PgPool) -> (AppState, Router) {
    make_local_default(pool).await.expect("the default project");
    let state = support::state(pool.clone(), support::gate_rules());
    let gate = support::gate(state.clone()).await;
    let races = (0..5).map(|n| format!("race-{n}")).chain((0..8).map(|n| format!("free-race-{n}")));
    for site in races.chain(["entry-first".to_owned(), "dismiss-first".to_owned()]) {
        support::made_site(&gate.world, &format!("customer/demo/site/{site}"), &site).await;
    }
    (state, gate_routes(gate))
}

fn conversation(answer: &Value) -> Conversation {
    serde_json::from_value::<OpenedConversation>(answer.clone()).expect("an opened conversation").conversation
}

async fn open(router: &Router, subject: &str, site: Option<&str>) -> (StatusCode, Conversation) {
    let body = site.map_or_else(|| json!({}), |site| json!({ "site": site }));
    let (status, answer) = support::json(router, "POST", "/api/conversations", subject, Some(body)).await;
    (status, conversation(&answer))
}

async fn dismiss(router: &Router, subject: &str, conversation: &Ulid) -> (StatusCode, Value) {
    support::json(router, "POST", &format!("/api/conversations/{conversation}/dismiss"), subject, None).await
}

fn note(text: &str) -> NewEntry {
    NewEntry::new(JournalKind::SystemNote, json!({ "text": text }))
}

fn in_turn(kind: JournalKind, turn: &Ulid, body: Value) -> NewEntry {
    let mut entry = NewEntry::new(kind, body);
    entry.turn = Some(turn.clone());
    entry
}

fn started(turn: &Ulid) -> NewEntry {
    let author = json!({ "subject": "u-1", "name": "Person u-1" });
    let body = json!({ "n": 1, "author": author, "message_seq": 1, "attended": true, "browser_tab": "tab-1" });
    in_turn(JournalKind::TurnStarted, turn, body)
}

/// The last entry of `conversation`: its kind and body.
async fn last_entry(pool: &PgPool, conversation: &Ulid) -> (String, Value) {
    sqlx::query_as("SELECT kind, body FROM journal WHERE conversation = $1 ORDER BY seq DESC LIMIT 1")
        .bind(conversation.as_str())
        .fetch_one(pool)
        .await
        .expect("the conversation holds an entry")
}

/// The conversations of the site holding `key`, and the number each holds it under.
async fn holders(pool: &PgPool, site: &str, key: &str) -> Vec<(String, i64)> {
    sqlx::query_as(
        "SELECT j.conversation, j.seq FROM journal j JOIN conversations c ON c.id = j.conversation \
         WHERE c.site = $1 AND j.dedup_key = $2",
    )
    .bind(site)
    .bind(key)
    .fetch_all(pool)
    .await
    .expect("the journal reads")
}

fn run_status(key: &str) -> NewEntry {
    let mut entry = NewEntry::new(JournalKind::RunStatus, json!({ "state": "running" }));
    entry.dedup_key = Some(key.to_owned());
    entry
}

/// Spec 01 section 8, statement 16: concurrent opens of one site make one live conversation; the open that made it
/// answers 201 and every other 200 with the same conversation, numbered from 1.
#[tokio::test]
async fn concurrent_opens_of_a_site_make_one_live_conversation() {
    let scratch = scratch_or_skip!();
    let (_, router) = ready(&scratch.pool).await;
    for n in 0..5 {
        let site = format!("customer/demo/site/race-{n}");
        let mut opens = JoinSet::new();
        for subject in ["u-1", "u-2", "u-3", "u-4"] {
            let (router, site) = (router.clone(), site.clone());
            opens.spawn(async move { open(&router, subject, Some(&site)).await });
        }
        let mut answers = Vec::new();
        while let Some(answer) = opens.join_next().await {
            answers.push(answer.expect("an open finishes"));
        }
        let made = answers.iter().filter(|(status, _)| *status == StatusCode::CREATED).count();
        assert_eq!(made, 1, "{site}: one open makes it");
        assert!(answers.iter().all(|(status, _)| matches!(*status, StatusCode::CREATED | StatusCode::OK)));
        let id = &answers[0].1.id;
        assert!(answers.iter().all(|(_, conversation)| &conversation.id == id), "{site}: one conversation");
        let first = &answers[0].1;
        assert_eq!((first.first_seq, first.last_seq), (JournalSeq::new(1), JournalSeq::new(0)));
        assert_eq!((first.state, first.model_label.as_str()), (ConversationState::Live, support::MODEL_LABEL));
    }
    scratch.close().await;
}

/// Spec 01 section 3.17, "Opening": without a site, a person opens their own new-site thread, the same one again
/// until it ends, and another person their own.
#[tokio::test]
async fn each_person_opens_one_new_site_thread() {
    let scratch = scratch_or_skip!();
    let (_, router) = ready(&scratch.pool).await;
    let (status, first) = open(&router, "u-1", None).await;
    assert_eq!((status, first.site.as_ref(), first.first_seq), (StatusCode::CREATED, None, JournalSeq::new(1)));
    let (status, again) = open(&router, "u-1", None).await;
    assert_eq!((status, &again.id), (StatusCode::OK, &first.id));
    let (status, theirs) = open(&router, "u-2", None).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_ne!(theirs.id, first.id);
    assert_eq!(theirs.created_by.as_str(), "u-2");
    scratch.close().await;
}

/// Spec 01 section 3.14's `project-required`: with no default project, a new conversation has none to belong to.
#[tokio::test]
async fn opening_without_a_default_project_is_refused() {
    let scratch = scratch_or_skip!();
    let router = gate_routes(support::gate(support::state(scratch.pool.clone(), support::gate_rules())).await);
    let body = json!({ "site": SITE });
    let (status, problem) = support::json(&router, "POST", "/api/conversations", "u-1", Some(body)).await;
    assert_eq!((status, problem["type"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("project-required")));
    scratch.close().await;
}

/// Brief I1.04 section 2: a site path is read by spec 03's grammar, `customer/{c}/site/{s}`, in a body and in the
/// list's query; anything else, or a field the body does not have, fails the check.
#[tokio::test]
async fn a_site_off_the_grammar_is_refused() {
    let scratch = scratch_or_skip!();
    let (_, router) = ready(&scratch.pool).await;
    for body in [json!({ "site": "customer/demo/lshape-6" }), json!({ "site": 6 }), json!({ "sight": SITE })] {
        let (status, problem) = support::json(&router, "POST", "/api/conversations", "u-1", Some(body)).await;
        assert_eq!((status, problem["type"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("validation-failed")));
    }
    for query in ["site=customer/demo/site", "site=template/site/l-shape", "before=3"] {
        let (status, problem) =
            support::json(&router, "GET", &format!("/api/conversations?{query}"), "u-1", None).await;
        assert_eq!((status, problem["type"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("validation-failed")));
    }
    scratch.close().await;
}

/// Spec 01 section 8, statement 16, and section 3.17, "Dismissing": Dismiss archives a site's conversation with
/// `conversation.changed` as its last entry, naming the next, whose first number is one past the archived one's last;
/// a repeat answers the first outcome; opening the site answers the next; the archived conversation takes nothing
/// more (`conversation-closed`); every stream of it hears the entry.
#[tokio::test]
async fn dismiss_archives_a_site_conversation_and_the_next_numbers_on() {
    let scratch = scratch_or_skip!();
    let (state, router) = ready(&scratch.pool).await;
    let (_, first) = open(&router, "u-1", Some(SITE)).await;
    for text in ["one", "two", "three"] {
        support::write(&state, &first.id, note(text)).await;
    }
    let mut subscription = state.hub.subscribe(&first.id);
    let (status, dismissed) = dismiss(&router, "u-2", &first.id).await;
    assert_eq!(status, StatusCode::OK);
    let (archived, live) = (
        conversation(&json!({ "conversation": dismissed["archived"] })),
        conversation(&json!({ "conversation": dismissed["live"] })),
    );
    assert_eq!(
        (archived.id.clone(), archived.state, archived.last_seq),
        (first.id.clone(), ConversationState::Archived, JournalSeq::new(4))
    );
    assert_eq!(archived.ended_by.as_ref().map(|by| by.as_str()), Some("u-2"));
    assert_eq!(
        (live.state, live.first_seq, live.last_seq),
        (ConversationState::Live, JournalSeq::new(5), JournalSeq::new(4))
    );
    assert_eq!(live.site.as_ref().map(SitePath::as_str), Some(SITE));
    assert_eq!(subscription.recv().await.ok(), Some(HubItem::Wake(JournalSeq::new(4))));

    let (kind, body) = last_entry(&scratch.pool, &first.id).await;
    let changed: ConversationChanged = serde_json::from_value(body).expect("a conversation.changed body");
    assert_eq!(kind, "conversation.changed");
    let by = Person { subject: "u-2".parse().expect("a subject"), name: "Person u-2".to_owned() };
    let expected = ConversationChanged {
        state: ConversationState::Archived,
        site: Some(SITE.parse().expect("a site")),
        title: None,
        next: Some(live.id.clone()),
        by,
    };
    assert_eq!(changed, expected);

    let (status, repeat) = dismiss(&router, "u-3", &first.id).await;
    assert_eq!((status, &repeat), (StatusCode::OK, &dismissed), "a repeat answers the first outcome");
    let (status, opened) = open(&router, "u-1", Some(SITE)).await;
    assert_eq!((status, opened.id), (StatusCode::OK, live.id));
    let mut tx = scratch.pool.begin().await.expect("a transaction");
    let refused = append(&mut tx, &first.id, note("an answer, too late")).await.expect_err("closed");
    assert!(matches!(refused, JournalError::Closed { state: ConversationState::Archived, .. }), "{refused}");
    tx.rollback().await.expect("a rollback");
    scratch.close().await;
}

/// Spec 01 section 8, statement 16: Dismiss answers `turn-running` while a written turn waits, and while it runs;
/// once `turn.finished` is written the conversation dismisses.
#[tokio::test]
async fn dismiss_is_refused_while_a_written_turn_waits_or_runs() {
    let scratch = scratch_or_skip!();
    let (state, router) = ready(&scratch.pool).await;
    let (_, live) = open(&router, "u-1", Some(SITE)).await;
    let turn = support::ulid();
    support::write(&state, &live.id, started(&turn)).await;
    let waiting = json!({ "on": "approval", "id": "01KN0TJZANP5N0PGJD8F7A3FAP" });
    support::write(&state, &live.id, in_turn(JournalKind::TurnWaiting, &turn, waiting)).await;
    let (status, problem) = dismiss(&router, "u-2", &live.id).await;
    assert_eq!((status, problem["type"].as_str()), (StatusCode::CONFLICT, Some("turn-running")));
    support::write(&state, &live.id, in_turn(JournalKind::TurnResumed, &turn, json!({ "by": "decision" }))).await;
    assert_eq!(dismiss(&router, "u-2", &live.id).await.0, StatusCode::CONFLICT, "a running turn too");
    let usage = json!({ "model_calls": 2, "input_tokens": 900, "output_tokens": 120, "cache_read_input_tokens": 0 });
    let finished = json!({ "outcome": "done", "reply_block": "b2", "usage": usage });
    support::write(&state, &live.id, in_turn(JournalKind::TurnFinished, &turn, finished)).await;
    assert_eq!(dismiss(&router, "u-2", &live.id).await.0, StatusCode::OK);
    scratch.close().await;
}

/// Spec 01 sections 3.2 and 3.17: a new-site thread is its maker's alone (`not-conversation-owner`); Dismiss discards
/// it, a repeat answering the same, and its maker's next open makes a new thread.
#[tokio::test]
async fn a_new_site_thread_is_discarded_by_its_maker_alone() {
    let scratch = scratch_or_skip!();
    let (_, router) = ready(&scratch.pool).await;
    let (_, thread) = open(&router, "u-1", None).await;
    let (status, problem) = dismiss(&router, "u-2", &thread.id).await;
    assert_eq!((status, problem["type"].as_str()), (StatusCode::FORBIDDEN, Some("not-conversation-owner")));
    let (status, discarded) = dismiss(&router, "u-1", &thread.id).await;
    assert_eq!(status, StatusCode::OK);
    let ended = conversation(&json!({ "conversation": discarded["discarded"] }));
    assert_eq!((ended.id.clone(), ended.state), (thread.id.clone(), ConversationState::Discarded));
    assert_eq!(dismiss(&router, "u-1", &thread.id).await, (StatusCode::OK, discarded));
    let (kind, body) = last_entry(&scratch.pool, &thread.id).await;
    assert_eq!(
        (kind.as_str(), &body["state"], &body["site"], &body["next"]),
        ("conversation.changed", &json!("discarded"), &Value::Null, &Value::Null)
    );
    let (status, next) = open(&router, "u-1", None).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_ne!(next.id, thread.id);
    scratch.close().await;
}

/// Spec 01 section 8, statement 16: a run's status entry racing a Dismiss lands in exactly one conversation, the one
/// live when it commits, and its repeat after the Dismiss writes nothing. Both orders are forced: the entry commits
/// first and Dismiss archives after it; Dismiss commits first and the site form, which waited on the row it
/// archived, reads again and lands in the next conversation. Then free races.
#[tokio::test]
async fn a_run_entry_racing_a_dismiss_lands_once_where_the_site_lives() {
    let scratch = scratch_or_skip!();
    let (state, router) = ready(&scratch.pool).await;
    let pool = &scratch.pool;

    // The entry first: it holds the live row's lock while Dismiss waits for it.
    let site_a: SitePath = "customer/demo/site/entry-first".parse().expect("a site");
    let (_, first) = open(&router, "u-1", Some(site_a.as_str())).await;
    let mut tx = pool.begin().await.expect("a transaction");
    let Appended::Wrote(written) = append_to_site(&mut tx, &site_a, run_status("k")).await.expect("an append") else {
        panic!("written");
    };
    let racing = tokio::spawn({
        let (router, id) = (router.clone(), first.id.clone());
        async move { dismiss(&router, "u-2", &id).await }
    });
    sleep(Duration::from_millis(200)).await;
    tx.commit().await.expect("a commit");
    state.hub.announce([written]);
    assert_eq!(racing.await.expect("Dismiss finishes").0, StatusCode::OK);
    let landed = holders(pool, site_a.as_str(), "k").await;
    assert_eq!(
        landed,
        [(first.id.to_string(), 1)],
        "in the conversation live when it committed, before the archival entry"
    );

    // Dismiss first: it queues on the live row before the site form does, and archives it.
    let site_b: SitePath = "customer/demo/site/dismiss-first".parse().expect("a site");
    let (_, second) = open(&router, "u-1", Some(site_b.as_str())).await;
    let mut hold = pool.begin().await.expect("a transaction");
    sqlx::query("SELECT 1 FROM conversations WHERE id = $1 FOR UPDATE")
        .bind(second.id.as_str())
        .execute(&mut *hold)
        .await
        .expect("the row locked");
    let dismissing = tokio::spawn({
        let (router, id) = (router.clone(), second.id.clone());
        async move { dismiss(&router, "u-2", &id).await }
    });
    sleep(Duration::from_millis(200)).await;
    let appending = tokio::spawn({
        let (pool, site) = (pool.clone(), site_b.clone());
        async move {
            let mut tx = pool.begin().await.expect("a transaction");
            let appended = append_to_site(&mut tx, &site, run_status("k")).await.expect("an append");
            tx.commit().await.expect("a commit");
            appended
        }
    });
    sleep(Duration::from_millis(200)).await;
    hold.rollback().await.expect("the row released");
    let (_, dismissed) = dismissing.await.expect("Dismiss finishes");
    let next: Ulid = serde_json::from_value(dismissed["live"]["id"].clone()).expect("the next conversation");
    assert!(matches!(appending.await.expect("the append finishes"), Appended::Wrote(_)));
    assert_eq!(holders(pool, site_b.as_str(), "k").await, [(next.to_string(), 2)], "in the next conversation");

    for (site, archived) in [(&site_a, &first.id), (&site_b, &second.id)] {
        let mut tx = pool.begin().await.expect("a transaction");
        assert!(matches!(append_to_site(&mut tx, site, run_status("k")).await, Ok(Appended::Held(_))), "{archived}");
        tx.commit().await.expect("a commit");
        assert_eq!(holders(pool, site.as_str(), "k").await.len(), 1, "the repeat writes nothing");
    }

    for n in 0..8 {
        let site: SitePath = format!("customer/demo/site/free-race-{n}").parse().expect("a site");
        let (_, live) = open(&router, "u-1", Some(site.as_str())).await;
        let racing = tokio::spawn({
            let (router, id) = (router.clone(), live.id.clone());
            async move { dismiss(&router, "u-2", &id).await }
        });
        let mut tx = pool.begin().await.expect("a transaction");
        let appended = append_to_site(&mut tx, &site, run_status("k")).await.expect("an append");
        tx.commit().await.expect("a commit");
        let (_, dismissed) = racing.await.expect("Dismiss finishes");
        let landed = holders(pool, site.as_str(), "k").await;
        assert_eq!(landed.len(), 1, "{site}: one entry");
        let Appended::Wrote(written) = appended else { panic!("written") };
        let archival: i64 = sqlx::query_scalar("SELECT last_seq FROM conversations WHERE id = $1")
            .bind(live.id.as_str())
            .fetch_one(pool)
            .await
            .expect("a row");
        let next = dismissed["live"]["id"].as_str().expect("the next conversation");
        let (holder, seq) = &landed[0];
        assert!(holder == next || (holder == live.id.as_str() && *seq < archival), "{site}: {holder} at {seq}");
        assert_eq!(&written.entry().conversation.to_string(), holder);
    }
    scratch.close().await;
}

/// Spec 01 section 8, statement 16, and section 3.17, "Binding": Create site's transaction binds the new-site thread,
/// which keeps its numbers and journals `conversation.changed` with the site and its name; a rollback leaves it
/// unbound; a bound thread is the site's live conversation; a site that already has a conversation refuses another
/// thread, and a bound thread binds no more.
#[tokio::test]
async fn create_sites_transaction_binds_the_new_site_thread() {
    let scratch = scratch_or_skip!();
    let (state, router) = ready(&scratch.pool).await;
    let site: SitePath = SITE.parse().expect("a site");
    let by = Person { subject: "u-1".parse().expect("a subject"), name: "Person u-1".to_owned() };
    let (_, thread) = open(&router, "u-1", None).await;
    support::write(&state, &thread.id, note("Which template?")).await;
    support::write(&state, &thread.id, note("The L-shape.")).await;

    let mut tx = scratch.pool.begin().await.expect("Create site's transaction");
    let written = bind_thread(&mut tx, &thread.id, &site, "Small L-shape, 6 aisles", &by).await.expect("bound");
    assert_eq!(written.entry().seq, JournalSeq::new(3));
    tx.rollback().await.expect("Create site fails");
    let mut conn = scratch.pool.acquire().await.expect("a connection");
    let unbound = load(&mut conn, &thread.id, support::MODEL_LABEL).await.expect("a read").expect("the thread");
    assert_eq!((unbound.site, unbound.last_seq), (None, JournalSeq::new(2)));

    let mut tx = scratch.pool.begin().await.expect("Create site's transaction");
    let written = bind_thread(&mut tx, &thread.id, &site, "Small L-shape, 6 aisles", &by).await.expect("bound");
    tx.commit().await.expect("Create site succeeds");
    state.hub.announce([written]);
    let bound = load(&mut conn, &thread.id, support::MODEL_LABEL).await.expect("a read").expect("the thread");
    assert_eq!((bound.site.as_ref(), bound.title.as_deref()), (Some(&site), Some("Small L-shape, 6 aisles")));
    assert_eq!(
        (bound.first_seq, bound.last_seq, bound.state),
        (JournalSeq::new(1), JournalSeq::new(3), ConversationState::Live)
    );
    assert!(bound.bound_at.is_some());
    let (kind, body) = last_entry(&scratch.pool, &thread.id).await;
    let changed: ConversationChanged = serde_json::from_value(body).expect("a conversation.changed body");
    assert_eq!(kind, "conversation.changed");
    assert_eq!(
        (changed.state, changed.site.as_ref(), changed.title.as_deref(), changed.next, &changed.by),
        (ConversationState::Live, Some(&site), Some("Small L-shape, 6 aisles"), None, &by)
    );

    let (status, opened) = open(&router, "u-2", Some(SITE)).await;
    assert_eq!((status, &opened.id), (StatusCode::OK, &thread.id), "the bound thread is the site's conversation");
    let (status, another) = open(&router, "u-1", None).await;
    assert_eq!(status, StatusCode::CREATED);
    let mut tx = scratch.pool.begin().await.expect("a transaction");
    let refused = bind_thread(&mut tx, &another.id, &site, "Again", &by).await.expect_err("the site has one");
    assert!(matches!(refused, ConversationError::SiteHasConversation(_)), "{refused}");
    let refused = bind_thread(&mut tx, &thread.id, &"customer/demo/site/other".parse().expect("a site"), "Other", &by)
        .await
        .expect_err("bound");
    assert!(matches!(refused, ConversationError::NotAThread(_)), "{refused}");
    tx.rollback().await.expect("a rollback");
    drop(conn);
    scratch.close().await;
}

/// Spec 01 section 3.6's list: a site's live conversation, then its archived ones newest first, `before` paging by
/// the first number each holds; the live one says what its open turn waits on; each carries its first and last
/// entries' wall-clock times.
#[tokio::test]
async fn a_sites_list_holds_its_live_conversation_then_its_archived_ones() {
    let scratch = scratch_or_skip!();
    let (state, router) = ready(&scratch.pool).await;
    let (_, oldest) = open(&router, "u-1", Some(SITE)).await;
    support::write(&state, &oldest.id, note("one")).await;
    let (_, dismissed) = dismiss(&router, "u-1", &oldest.id).await;
    let middle: Ulid = serde_json::from_value(dismissed["live"]["id"].clone()).expect("an id");
    let (_, dismissed) = dismiss(&router, "u-1", &middle).await;
    let newest: Ulid = serde_json::from_value(dismissed["live"]["id"].clone()).expect("an id");
    let turn = support::ulid();
    support::write(&state, &newest, started(&turn)).await;
    let waiting = json!({ "on": "question", "id": "01KN0TJZANP5N0PGJD8F7A3FQU" });
    support::write(&state, &newest, in_turn(JournalKind::TurnWaiting, &turn, waiting)).await;

    let uri = format!("/api/conversations?site={SITE}");
    let (status, list) = support::json(&router, "GET", &uri, "u-2", None).await;
    assert_eq!(status, StatusCode::OK);
    let ids: Vec<&str> =
        list["items"].as_array().expect("items").iter().map(|item| item["id"].as_str().expect("id")).collect();
    assert_eq!(ids, [newest.as_str(), middle.as_str(), oldest.id.as_str()]);
    let waiting: Vec<&Value> = list["items"].as_array().expect("items").iter().map(|item| &item["waiting"]).collect();
    assert_eq!(waiting, [&json!("question"), &Value::Null, &Value::Null]);
    let middle_item = &list["items"][1];
    assert!(middle_item["first_at"].is_string() && middle_item["first_at"] == middle_item["last_at"], "its one entry");
    assert_eq!(list.get("next"), None);

    let (_, page) = support::json(&router, "GET", &format!("{uri}&limit=2"), "u-2", None).await;
    assert_eq!(page["items"].as_array().map(Vec::len), Some(2));
    let before = page["next"].as_u64().expect("a next page");
    let (_, rest) = support::json(&router, "GET", &format!("{uri}&limit=2&before={before}"), "u-2", None).await;
    assert_eq!(rest["items"][0]["id"].as_str(), Some(oldest.id.as_str()));
    assert_eq!((rest["items"].as_array().map(Vec::len), rest.get("next")), (Some(1), None));
    let (_, none) = support::json(&router, "GET", "/api/conversations?site=customer/demo/site/none", "u-2", None).await;
    assert_eq!(none, json!({ "items": [] }));
    scratch.close().await;
}

/// Brief I1.04 section 2 (lead decided): locally the service makes the default project, "Demo" for the customer
/// `demo`, once however often it starts; the table's index keeps a single default.
#[tokio::test]
async fn the_local_default_project_is_made_once() {
    let scratch = scratch_or_skip!();
    let mut makes = JoinSet::new();
    for _ in 0..4 {
        let pool = scratch.pool.clone();
        makes.spawn(async move { make_local_default(&pool).await.expect("a make") });
    }
    let mut made = 0;
    while let Some(outcome) = makes.join_next().await {
        made += usize::from(outcome.expect("a make finishes"));
    }
    assert_eq!(made, 1);
    let rows: Vec<(String, Option<String>, bool)> = sqlx::query_as("SELECT name, customer, is_default FROM projects")
        .fetch_all(&scratch.pool)
        .await
        .expect("the projects read");
    assert_eq!(rows, [("Demo".to_owned(), Some("demo".to_owned()), true)]);
    let second =
        sqlx::query("INSERT INTO projects (id, name, is_default) VALUES ('01KN0TJZANP5N0PGJD8F7A3FP2', 'Two', true)")
            .execute(&scratch.pool)
            .await
            .expect_err("one default");
    assert_eq!(second.as_database_error().and_then(|fault| fault.constraint()), Some("projects_one_default"));
    scratch.close().await;
}
