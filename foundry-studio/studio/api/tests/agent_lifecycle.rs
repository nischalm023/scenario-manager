//! The agent loop's turn where two acts meet (Astra's agent-loop review as the lead disposed it on 2026-10-20; spec 01
//! sections 3.1, 3.4, 3.10 and 3.14): a message and an answer taking their locks in one order; Stop while an approval
//! card is drawn; the process's permits for running turns, fresh or resumed; the test writer answering a held entry
//! before it changes a turn's row; one approval owning its act; and every resume keeping the conversation's provider.

mod support;

use std::time::{Duration, Instant};

use agent_loop::{Scripted, ScriptedStep, wire_name};
use axum::Router;
use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use studio_api::agent::sweep;
use studio_api::config::{AgentSettings, TurnsAtOnce};
use support::agent::{SITE, create, entries, kinds, message_body, opened, post, question, ready, say, settled};
use support::templates::{draft, send, written};
use tokio::task::JoinSet;
use tokio::time::sleep;

const OTHER: &str = "customer/demo/site/other-6";

/// The review's lock-order finding: a message sent while the turn waits on a question and an answer to that question,
/// posted together, each take the conversation's lock, then the turn's, then the question's, so one of them resumes
/// the turn and the other is refused, and neither fails on a deadlock.
#[tokio::test]
async fn a_message_racing_an_answer_resumes_the_turn_once_and_never_deadlocks() {
    let scratch = scratch_or_skip!();
    let model = Scripted::new(Vec::new());
    let stack = ready(&scratch.pool, AgentSettings::default(), model.clone()).await;
    let conversation = opened(&stack.router, Some(SITE)).await;
    for round in 0..6 {
        model.push(vec![ScriptedStep::call(&wire_name("ask_user"), question()), ScriptedStep::text("Noted.")]);
        let turn = say(&stack.router, &conversation, &format!("ask {round}")).await;
        assert_eq!(settled(&scratch.pool, &turn).await, "waiting");
        let asked: String = sqlx::query_scalar("SELECT id FROM questions WHERE turn = $1")
            .bind(&turn)
            .fetch_one(&scratch.pool)
            .await
            .expect("a question");
        let mut both = JoinSet::new();
        let (router, address) = (stack.router.clone(), format!("/api/questions/{asked}/answers"));
        let answer = json!({ "answers": [{ "item": 0, "selected": ["L"] }] });
        both.spawn(async move { send(&router, "POST", &address, Some(answer), None).await.0 });
        let (router, words, id) = (stack.router.clone(), message_body(&format!("the L {round}")), conversation.clone());
        both.spawn(async move { post(&router, &id, words).await.0 });
        let statuses: Vec<StatusCode> = both.join_all().await;
        assert!(statuses.iter().all(|status| status.as_u16() < 500), "round {round}: {statuses:?}");
        assert!(statuses.iter().any(|status| status.is_success()), "round {round}: {statuses:?}");
        assert_eq!(settled(&scratch.pool, &turn).await, "done", "round {round}");
        let resumed =
            kinds(&entries(&scratch.pool, &conversation).await).iter().filter(|kind| **kind == "turn.resumed").count();
        assert_eq!(resumed, round + 1, "one resumption per round");
    }
    scratch.close().await;
}

/// The review's first finding: Stop while Create site's card is still drawn (its evaluation still running, so the card
/// waits on it) ends the turn `stopped` within a second of wall clock, and no card is left pending or waited on.
#[tokio::test]
async fn stop_while_an_approval_card_is_drawn_ends_the_turn_and_leaves_no_card() {
    let scratch = scratch_or_skip!();
    let model = Scripted::new(Vec::new());
    let stack = ready(&scratch.pool, AgentSettings::default(), model.clone()).await;
    let thread = opened(&stack.router, None).await;
    let (id, _) = draft(&stack.router, "rectangle", Some(&thread)).await;
    sqlx::query("INSERT INTO template_evaluations (draft, n, revision, knobs) VALUES ($1, 2, 1, '{}')")
        .bind(&id)
        .execute(&scratch.pool)
        .await
        .expect("an evaluation still running");
    sqlx::query("UPDATE template_drafts SET latest_n = 2 WHERE id = $1")
        .bind(&id)
        .execute(&scratch.pool)
        .await
        .expect("the draft's latest");
    let mut input = create(&id, "agent-stop");
    input["evaluation"] = json!(2);
    model.push(vec![ScriptedStep::call(&wire_name("site.create_from_template"), input)]);
    let turn = say(&stack.router, &thread, "make it").await;
    sleep(Duration::from_millis(400)).await;
    let pressed = Instant::now();
    let (status, _, _) = send(&stack.router, "POST", &format!("/api/turns/{turn}/stop"), None, None).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(settled(&scratch.pool, &turn).await, "stopped");
    assert!(pressed.elapsed() < Duration::from_secs(1), "stopped in {:?}", pressed.elapsed());
    let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM approvals WHERE turn = $1 AND state = 'pending'")
        .bind(&turn)
        .fetch_one(&scratch.pool)
        .await
        .expect("a count");
    assert_eq!(pending, 0);
    let journal = entries(&scratch.pool, &thread).await;
    assert!(!kinds(&journal).contains(&"turn.waiting"), "{:?}", kinds(&journal));
    scratch.close().await;
}

/// The review's permit finding (spec 01 section 3.1's `STUDIO_TURNS_AT_ONCE`): with one permit, two messages to two
/// conversations at once start one turn and answer the other `agent-busy`; and an answer that would resume a waiting
/// turn while another runs is refused `agent-busy` with nothing changed, then resumes it once the other has ended.
#[tokio::test]
async fn one_permit_per_running_turn_bounds_fresh_and_resumed_turns() {
    let scratch = scratch_or_skip!();
    let model = Scripted::new(vec![ScriptedStep::call(&wire_name("ask_user"), question())]);
    let settings = AgentSettings { turns_at_once: TurnsAtOnce(1), ..AgentSettings::default() };
    let stack = ready(&scratch.pool, settings, model.clone()).await;
    let waits = opened(&stack.router, None).await;
    let waiting = say(&stack.router, &waits, "ask").await;
    assert_eq!(settled(&scratch.pool, &waiting).await, "waiting");

    model.push(vec![ScriptedStep::Hang]);
    support::made_site(&stack.gate.world, OTHER, "Other 6").await;
    let (one, two) = (opened(&stack.router, Some(SITE)).await, opened(&stack.router, Some(OTHER)).await);
    let mut both = JoinSet::new();
    for conversation in [one, two] {
        let router = stack.router.clone();
        both.spawn(async move { post(&router, &conversation, message_body("hang")).await });
    }
    let answers: Vec<(StatusCode, Value)> = both.join_all().await;
    let mut statuses: Vec<u16> = answers.iter().map(|(status, _)| status.as_u16()).collect();
    statuses.sort_unstable();
    assert_eq!(statuses, [202, 503], "{answers:?}");
    let busy = answers.iter().find(|(status, _)| status.as_u16() == 503).expect("one refused");
    assert_eq!(busy.1["type"], json!("agent-busy"));
    let running = answers.iter().find_map(|(_, body)| body["turn"].as_str()).expect("one started").to_owned();

    let asked: String = sqlx::query_scalar("SELECT id FROM questions WHERE turn = $1")
        .bind(&waiting)
        .fetch_one(&scratch.pool)
        .await
        .expect("a question");
    let address = format!("/api/questions/{asked}/answers");
    let answer = json!({ "answers": [{ "item": 0, "selected": ["U"] }] });
    let (status, _, refused) = send(&stack.router, "POST", &address, Some(answer.clone()), None).await;
    assert_eq!((status, &refused["type"]), (StatusCode::SERVICE_UNAVAILABLE, &json!("agent-busy")));
    let (state, open): (String, String) =
        sqlx::query_as("SELECT t.state, q.state FROM turns t JOIN questions q ON q.turn = t.id WHERE t.id = $1")
            .bind(&waiting)
            .fetch_one(&scratch.pool)
            .await
            .expect("the turn");
    assert_eq!((state.as_str(), open.as_str()), ("waiting", "open"), "a refusal changes nothing");

    let (status, _, _) = send(&stack.router, "POST", &format!("/api/turns/{running}/stop"), None, None).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(settled(&scratch.pool, &running).await, "stopped");
    model.push(vec![ScriptedStep::text("Done.")]);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let (status, _, _) = send(&stack.router, "POST", &address, Some(answer.clone()), None).await;
        if status == StatusCode::OK {
            break;
        }
        assert!(Instant::now() < deadline && status == StatusCode::SERVICE_UNAVAILABLE, "{status}");
        sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(settled(&scratch.pool, &waiting).await, "done");
    scratch.close().await;
}

/// The review's test-writer finding (brief I1.03 section 2): an entry whose dedup key the conversation already holds
/// is answered `200` with the holder's number before the turn's row changes, so replaying an old `turn.waiting` after
/// the turn resumed leaves the row running, as the journal says.
#[tokio::test]
async fn the_test_writer_answers_a_held_entry_before_it_changes_the_turns_row() {
    let scratch = scratch_or_skip!();
    let stack = ready(&scratch.pool, AgentSettings::default(), Scripted::new(Vec::new())).await;
    let conversation = opened(&stack.router, Some(SITE)).await;
    let turn = "01KN0TJZANP5N0PGJD8F7A3HXD";
    let write = |kind: &str, body: Value, key: Option<&str>| {
        let (router, address) = (stack.router.clone(), format!("/api/dev/conversations/{conversation}/entries"));
        let entry = json!({ "kind": kind, "body": body, "turn": turn, "dedup_key": key });
        async move { send(&router, "POST", &address, Some(entry), None).await }
    };
    let author = json!({ "subject": "dev", "name": "Developer" });
    let message = json!({ "author": author, "text": "hi", "context": null, "source": "composer", "browser_tab": null });
    assert_eq!(write("message.user", message, None).await.0, StatusCode::CREATED);
    let waits = json!({ "on": "question", "id": "01KN0TJZANP5N0PGJD8F7A3QHX" });
    let (status, _, first) = write("turn.waiting", waits.clone(), Some("wait-1")).await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    assert_eq!(write("turn.resumed", json!({ "by": "answer" }), None).await.0, StatusCode::CREATED);
    let (status, _, again) = write("turn.waiting", waits, Some("wait-1")).await;
    assert_eq!((status, &again["held"]), (StatusCode::OK, &first["entry"]["seq"]), "{again}");
    let state: String = sqlx::query_scalar("SELECT state FROM turns WHERE id = $1")
        .bind(turn)
        .fetch_one(&scratch.pool)
        .await
        .expect("the turn");
    assert_eq!(state, "running");
    scratch.close().await;
}

/// A New site turn waiting on the agent's Create site card at `slug`: its thread, draft, turn and approval.
async fn waiting_on_create(
    pool: &PgPool,
    model: &Scripted,
    router: &Router,
    slug: &str,
) -> (String, String, String, String) {
    let thread = opened(router, None).await;
    let (id, _) = draft(router, "rectangle", Some(&thread)).await;
    model.push(vec![
        ScriptedStep::call(&wire_name("site.create_from_template"), create(&id, slug)),
        ScriptedStep::text("Noted."),
    ]);
    let turn = say(router, &thread, "make it").await;
    assert_eq!(settled(pool, &turn).await, "waiting");
    let approval: String = sqlx::query_scalar("SELECT id FROM approvals WHERE turn = $1")
        .bind(&turn)
        .fetch_one(pool)
        .await
        .expect("a card");
    (thread, id, turn, approval)
}

async fn execution(pool: &PgPool, approval: &str) -> String {
    sqlx::query_scalar("SELECT execution FROM approvals WHERE id = $1")
        .bind(approval)
        .fetch_one(pool)
        .await
        .expect("an approval")
}

/// The review's first approval finding (spec 01 section 3.10, rules 2 and 6): once the agent's Create site approval is
/// approved, before its act takes the draft, a person's press answers with that approval, whatever its values, and
/// records none of its own; a turn that ends before that act started (here a restart's sweep) ends the approval's
/// execution `interrupted`, after which a press records and runs its own.
#[tokio::test]
async fn a_press_after_the_agents_approval_was_approved_answers_with_it() {
    let scratch = scratch_or_skip!();
    let model = Scripted::new(Vec::new());
    let stack = ready(&scratch.pool, AgentSettings::default(), model.clone()).await;
    let (_, id, turn, approval) = waiting_on_create(&scratch.pool, &model, &stack.router, "agent-own").await;
    sqlx::query(
        "UPDATE approvals SET state = 'approved', execution = 'not_started', decided_by = 'dev', decided_at = now() \
         WHERE id = $1",
    )
    .bind(&approval)
    .execute(&scratch.pool)
    .await
    .expect("approved, as a decision leaves it before its act starts");
    sqlx::query(
        "UPDATE turns SET state = 'running', waiting_on = NULL, waiting_id = NULL, waiting_since = NULL WHERE id = $1",
    )
    .bind(&turn)
    .execute(&scratch.pool)
    .await
    .expect("running again");
    let press = format!("/api/template-drafts/{id}/site");
    let other = json!({ "customer": "demo", "slug": "person-own", "name": "Person's own", "evaluation": 1 });
    let (status, _, pressed) = send(&stack.router, "POST", &press, Some(other.clone()), None).await;
    assert_eq!((status, &pressed["approval"]["id"]), (StatusCode::ACCEPTED, &json!(approval)), "{pressed}");
    let buttons: i64 = sqlx::query_scalar("SELECT count(*) FROM approvals WHERE via = 'button'")
        .fetch_one(&scratch.pool)
        .await
        .expect("a count");
    assert_eq!(buttons, 0);

    sweep(&stack.gate.app).await.expect("the sweep");
    assert_eq!(settled(&scratch.pool, &turn).await, "interrupted");
    assert_eq!(execution(&scratch.pool, &approval).await, "interrupted");
    let (status, _, pressed) = send(&stack.router, "POST", &press, Some(other), None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{pressed}");
    assert_ne!(pressed["approval"]["id"], json!(approval), "the press's own approval");
    assert_eq!(written(&stack.router, &id).await["state"], json!("created"));
    scratch.close().await;
}

/// The review's second approval finding (spec 01 section 3.10's execution states): an approved Create site whose
/// evaluation went stale before its act started ends its approval's execution `failed` with its `approval.execution`
/// entries, the call failing `evaluation-stale` for the model, and the turn goes on.
#[tokio::test]
async fn an_approved_act_refused_before_it_starts_records_its_approval_failed() {
    let scratch = scratch_or_skip!();
    let model = Scripted::new(Vec::new());
    let stack = ready(&scratch.pool, AgentSettings::default(), model.clone()).await;
    let (thread, id, turn, approval) = waiting_on_create(&scratch.pool, &model, &stack.router, "agent-stale").await;
    sqlx::query("INSERT INTO template_evaluations (draft, n, revision, knobs) VALUES ($1, 2, 1, '{}')")
        .bind(&id)
        .execute(&scratch.pool)
        .await
        .expect("a newer evaluation");
    sqlx::query("UPDATE template_drafts SET latest_n = 2 WHERE id = $1")
        .bind(&id)
        .execute(&scratch.pool)
        .await
        .expect("the draft's latest");
    let decide = format!("/api/approvals/{approval}/decision");
    let (status, _, _) = send(&stack.router, "POST", &decide, Some(json!({ "decision": "approve" })), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(settled(&scratch.pool, &turn).await, "done");
    assert_eq!(execution(&scratch.pool, &approval).await, "failed");
    let journal = entries(&scratch.pool, &thread).await;
    let executions: Vec<&Value> = (journal.iter())
        .filter(|(kind, body, _)| kind == "approval.execution" && body["approval"] == json!(approval))
        .map(|(_, body, _)| &body["execution"])
        .collect();
    assert_eq!(executions, [&json!("running"), &json!("failed")]);
    let result = &support::agent::results(&model.requests()[1])[0];
    assert_eq!(
        (&result["is_error"], &result["content"][0]["value"]["type"]),
        (&json!(true), &json!("evaluation-stale"))
    );
    scratch.close().await;
}

/// The review's approval-ownership finding (spec 01 section 3.10, rules 2 and 6): an approved Create site whose draft
/// another approval already writes is refused and its own execution ends `failed`, never answered with the other
/// approval, which keeps the draft.
#[tokio::test]
async fn an_approved_act_never_runs_under_another_approval() {
    let scratch = scratch_or_skip!();
    let model = Scripted::new(Vec::new());
    let stack = ready(&scratch.pool, AgentSettings::default(), model.clone()).await;
    let (thread, id, turn, approval) = waiting_on_create(&scratch.pool, &model, &stack.router, "agent-two").await;
    let other = "01KN0TJZANP5N0PGJD8F7A3BBB";
    sqlx::query(
        "INSERT INTO approvals (id, conversation, tool, class, input, input_digest, summary, facts, via, state, \
             decided_by, decided_at, execution) \
         VALUES ($1, $2, 'site.create_from_template', 'apply', '{}', 'x', 'A press', '{}', 'button', 'approved', \
             'dev', now(), 'not_started')",
    )
    .bind(other)
    .bind(&thread)
    .execute(&scratch.pool)
    .await
    .expect("a press's approval");
    sqlx::query(
        "UPDATE template_drafts SET state = 'writing', write_approval = $2, write_evaluation = 1 WHERE id = $1",
    )
    .bind(&id)
    .bind(other)
    .execute(&scratch.pool)
    .await
    .expect("the press writing");
    let decide = format!("/api/approvals/{approval}/decision");
    let (status, _, _) = send(&stack.router, "POST", &decide, Some(json!({ "decision": "approve" })), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(settled(&scratch.pool, &turn).await, "done");
    assert_eq!(execution(&scratch.pool, &approval).await, "failed");
    let writing: String = sqlx::query_scalar("SELECT write_approval FROM template_drafts WHERE id = $1")
        .bind(&id)
        .fetch_one(&scratch.pool)
        .await
        .expect("the draft");
    assert_eq!(writing, other);
    scratch.close().await;
}

/// The review's provider finding (brief I1.03 section 2, a conversation keeps its provider; spec 01 section 3.4's
/// surviving waits): once the agent runs on another provider than the conversation began with, an answer to its
/// question and a decision on its approval are refused `provider-changed`, and nothing resumes.
#[tokio::test]
async fn every_resume_keeps_the_provider_the_conversation_began_with() {
    let scratch = scratch_or_skip!();
    let model = Scripted::new(vec![ScriptedStep::call(&wire_name("ask_user"), question())]);
    let stack = ready(&scratch.pool, AgentSettings::default(), model.clone()).await;
    let asks = opened(&stack.router, Some(SITE)).await;
    let asking = say(&stack.router, &asks, "ask").await;
    assert_eq!(settled(&scratch.pool, &asking).await, "waiting");
    let (thread, _, approving, approval) = waiting_on_create(&scratch.pool, &model, &stack.router, "agent-was").await;
    sqlx::query("UPDATE conversations SET provider = 'anthropic' WHERE id = $1 OR id = $2")
        .bind(&asks)
        .bind(&thread)
        .execute(&scratch.pool)
        .await
        .expect("begun on another provider");
    let asked: String = sqlx::query_scalar("SELECT id FROM questions WHERE turn = $1")
        .bind(&asking)
        .fetch_one(&scratch.pool)
        .await
        .expect("a question");
    let answer = json!({ "answers": [{ "item": 0, "selected": ["L"] }] });
    let (status, _, refused) =
        send(&stack.router, "POST", &format!("/api/questions/{asked}/answers"), Some(answer), None).await;
    assert_eq!((status, &refused["type"]), (StatusCode::CONFLICT, &json!("provider-changed")));
    let decide = format!("/api/approvals/{approval}/decision");
    let (status, _, refused) = send(&stack.router, "POST", &decide, Some(json!({ "decision": "approve" })), None).await;
    assert_eq!((status, &refused["type"]), (StatusCode::CONFLICT, &json!("provider-changed")));
    for turn in [&asking, &approving] {
        assert_eq!(settled(&scratch.pool, turn).await, "waiting");
    }
    assert_eq!(approval_state(&scratch.pool, &approval).await, "pending");
    scratch.close().await;
}

async fn approval_state(pool: &PgPool, approval: &str) -> String {
    sqlx::query_scalar("SELECT state FROM approvals WHERE id = $1").bind(approval).fetch_one(pool).await.expect("a row")
}
