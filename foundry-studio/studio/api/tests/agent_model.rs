//! The agent loop's model side as Astra's agent-loop review found it and the lead disposed it on 2026-10-20 (spec 01
//! sections 3.4 and 3.7): a message's words kept until their reply's results all reach the model; the budgets checked
//! before each call and bounding a request's stream and a card's drawing, active time counted across waits; a summary
//! kept only when its request stopped usably with text; and the size limit read after an attempted compaction.

mod support;

use std::time::Duration;

use agent_loop::{ModelUsage, Scripted, ScriptedBlock, ScriptedStep, Stop, wire_name};
use axum::http::StatusCode;
use serde_json::{Value, json};
use studio_api::config::{AgentSettings, ModelCalls, Tokens};
use support::agent::{SITE, call, create, message_body, opened, post, question, ready, results, say, settled, texts};
use support::templates::{draft, send};

/// The review's finding on an answer in words: a message that answers a reply's first question while a second waits
/// keeps its page line and words on the turn until the second is answered, and they reach the model after both
/// results.
#[tokio::test]
async fn a_messages_words_wait_until_every_result_of_their_reply_is_sent() {
    let scratch = scratch_or_skip!();
    let model = Scripted::new(vec![
        ScriptedStep::reply(vec![call("ask_user", question()), call("ask_user", question())]),
        ScriptedStep::text("Done."),
    ]);
    let stack = ready(&scratch.pool, AgentSettings::default(), model.clone()).await;
    let conversation = opened(&stack.router, Some(SITE)).await;
    let turn = say(&stack.router, &conversation, "ask twice").await;
    assert_eq!(settled(&scratch.pool, &turn).await, "waiting");
    let mut words = message_body("the L, and keep it small");
    words["context"] = json!({ "route": "/w/customer/demo/site/lshape-6/map@main", "panel": "map", "title": "Map",
                               "seen": [], "selection": null, "highlights": null });
    let (status, accepted) = post(&stack.router, &conversation, words).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    assert_eq!(settled(&scratch.pool, &turn).await, "waiting", "the second question waits");
    let second: String = sqlx::query_scalar("SELECT id FROM questions WHERE turn = $1 AND state = 'open'")
        .bind(&turn)
        .fetch_one(&scratch.pool)
        .await
        .expect("the second question");
    let answer = json!({ "answers": [{ "item": 0, "selected": ["U"] }] });
    let (status, _, _) =
        send(&stack.router, "POST", &format!("/api/questions/{second}/answers"), Some(answer), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(settled(&scratch.pool, &turn).await, "done");
    let sent = model.requests();
    assert_eq!(results(&sent[1]).len(), 2, "both results");
    let after = texts(sent[1].history.last().expect("the user message"));
    assert!(after.contains("Page context") && after.ends_with("the L, and keep it small"), "{after}");
    scratch.close().await;
}

/// The review's budget finding, before each call: a reply whose output passed `STUDIO_TURN_MAX_OUTPUT_TOKENS` has its
/// calls answered `turn-budget` unrun, and the one summary request follows with tools off.
#[tokio::test]
async fn a_reply_past_the_output_budget_runs_none_of_its_calls() {
    let scratch = scratch_or_skip!();
    let calls = ScriptedStep::reply(vec![call("site.list", json!({})), call("site.list", json!({}))])
        .costing(Tokens(100), Tokens(60));
    let model = Scripted::new(vec![calls, ScriptedStep::text("I listed nothing yet.")]);
    let settings = AgentSettings { max_output_tokens: Tokens(50), ..AgentSettings::default() };
    let stack = ready(&scratch.pool, settings, model.clone()).await;
    let conversation = opened(&stack.router, Some(SITE)).await;
    let turn = say(&stack.router, &conversation, "list twice").await;
    assert_eq!(settled(&scratch.pool, &turn).await, "done");
    let rows: Vec<(String, Option<Value>)> =
        sqlx::query_as("SELECT state, problem FROM tool_calls WHERE turn = $1 ORDER BY position")
            .bind(&turn)
            .fetch_all(&scratch.pool)
            .await
            .expect("the calls");
    assert_eq!(rows.len(), 2);
    let unrun = |(state, problem): &(String, Option<Value>)| {
        state == "failed" && problem.as_ref().is_some_and(|problem| problem["type"] == json!("turn-budget"))
    };
    assert!(rows.iter().all(unrun), "{rows:?}");
    let off: Vec<bool> = model.requests().iter().map(|request| request.tools_off).collect();
    assert_eq!(off, [false, true]);
    let budget: Option<String> = sqlx::query_scalar("SELECT budget FROM turns WHERE id = $1")
        .bind(&turn)
        .fetch_one(&scratch.pool)
        .await
        .expect("a turn");
    assert_eq!(budget.as_deref(), Some("output-tokens"));
    scratch.close().await;
}

/// The review's budget finding, on streaming: a request whose stream outlasts what is left of
/// `STUDIO_TURN_MAX_ACTIVE_SECS` is cut there, and the turn ends `done` at that budget after its summary request.
#[tokio::test]
async fn the_active_time_left_cuts_a_streaming_request() {
    let scratch = scratch_or_skip!();
    let model = Scripted::new(vec![ScriptedStep::Hang, ScriptedStep::text("I ran out of time.")]);
    let settings = AgentSettings { max_active: Duration::from_secs(1), ..AgentSettings::default() };
    let stack = ready(&scratch.pool, settings, model.clone()).await;
    let conversation = opened(&stack.router, Some(SITE)).await;
    let turn = say(&stack.router, &conversation, "take long").await;
    assert_eq!(settled(&scratch.pool, &turn).await, "done");
    let budget: Option<String> = sqlx::query_scalar("SELECT budget FROM turns WHERE id = $1")
        .bind(&turn)
        .fetch_one(&scratch.pool)
        .await
        .expect("a turn");
    assert_eq!(budget.as_deref(), Some("active-seconds"));
    assert!(texts(model.requests()[1].history.last().expect("a message")).contains("wall-clock seconds spent running"));
    scratch.close().await;
}

/// The review's budget finding, across waits: active time spent before a wait counts after it, so a card drawn after the
/// turn resumes is cut at what was left (here 100 ms of 2 s), not at the tool's own timeout.
#[tokio::test]
async fn active_time_before_a_wait_bounds_a_card_drawn_after_it() {
    let scratch = scratch_or_skip!();
    let model = Scripted::new(vec![ScriptedStep::call(&wire_name("ask_user"), question())]);
    let settings = AgentSettings { max_active: Duration::from_secs(2), ..AgentSettings::default() };
    let stack = ready(&scratch.pool, settings, model.clone()).await;
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
    let turn = say(&stack.router, &thread, "ask first").await;
    assert_eq!(settled(&scratch.pool, &turn).await, "waiting");
    sqlx::query("UPDATE turns SET active_ms = 1900 WHERE id = $1")
        .bind(&turn)
        .execute(&scratch.pool)
        .await
        .expect("time spent before the wait");
    let mut input = create(&id, "agent-late");
    input["evaluation"] = json!(2);
    model.push(vec![
        ScriptedStep::call(&wire_name("site.create_from_template"), input),
        ScriptedStep::text("Out of time."),
    ]);
    let asked: String = sqlx::query_scalar("SELECT id FROM questions WHERE turn = $1")
        .bind(&turn)
        .fetch_one(&scratch.pool)
        .await
        .expect("a question");
    let answer = json!({ "answers": [{ "item": 0, "selected": ["L"] }] });
    let (status, _, _) =
        send(&stack.router, "POST", &format!("/api/questions/{asked}/answers"), Some(answer), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(settled(&scratch.pool, &turn).await, "done");
    let (state, problem): (String, Value) =
        sqlx::query_as("SELECT state, problem FROM tool_calls WHERE turn = $1 AND tool = 'site.create_from_template'")
            .bind(&turn)
            .fetch_one(&scratch.pool)
            .await
            .expect("the call");
    assert_eq!((state.as_str(), &problem["type"]), ("failed", &json!("turn-budget")));
    scratch.close().await;
}

/// The review's summary finding, for compaction: a summary reply with no text replaces nothing (no summary row, the
/// thinking floor where it was) and ends the turn `failed`, so the history it was asked to shorten stays whole.
#[tokio::test]
async fn a_compaction_with_no_text_replaces_nothing() {
    let scratch = scratch_or_skip!();
    let model = Scripted::new(vec![
        ScriptedStep::text("A long answer.").costing(Tokens(5_000), Tokens(10)),
        ScriptedStep::reply(Vec::new()),
    ]);
    let settings =
        AgentSettings { compact_at: Tokens(1_000), conversation_max: Tokens(100_000), ..AgentSettings::default() };
    let stack = ready(&scratch.pool, settings, model).await;
    let conversation = opened(&stack.router, Some(SITE)).await;
    let first = say(&stack.router, &conversation, "first").await;
    assert_eq!(settled(&scratch.pool, &first).await, "done");
    let second = say(&stack.router, &conversation, "second").await;
    assert_eq!(settled(&scratch.pool, &second).await, "failed");
    let summaries: i64 =
        sqlx::query_scalar("SELECT count(*) FROM model_messages WHERE conversation = $1 AND summary_of IS NOT NULL")
            .bind(&conversation)
            .fetch_one(&scratch.pool)
            .await
            .expect("a count");
    assert_eq!(summaries, 0);
    let floor: Option<i32> = sqlx::query_scalar("SELECT thinking_floor FROM conversations WHERE id = $1")
        .bind(&conversation)
        .fetch_one(&scratch.pool)
        .await
        .expect("the conversation");
    assert_eq!(floor, Some(0), "where it was");
    scratch.close().await;
}

/// The review's summary finding, for a budget: a summary request the model refuses is not kept in the transcript,
/// and the turn still ends `done` naming the budget.
#[tokio::test]
async fn a_refused_budget_summary_is_not_kept() {
    let scratch = scratch_or_skip!();
    let refused = ScriptedStep::Reply {
        blocks: vec![ScriptedBlock::Text("I will not.".into())],
        stop: Some(Stop::Refused(None)),
        usage: ModelUsage::default(),
    };
    let model = Scripted::new(vec![ScriptedStep::call(&wire_name("site.list"), json!({})), refused]);
    let settings = AgentSettings { max_model_calls: ModelCalls(1), ..AgentSettings::default() };
    let stack = ready(&scratch.pool, settings, model).await;
    let conversation = opened(&stack.router, Some(SITE)).await;
    let turn = say(&stack.router, &conversation, "list").await;
    assert_eq!(settled(&scratch.pool, &turn).await, "done");
    let budget: Option<String> = sqlx::query_scalar("SELECT budget FROM turns WHERE id = $1")
        .bind(&turn)
        .fetch_one(&scratch.pool)
        .await
        .expect("a turn");
    assert_eq!(budget.as_deref(), Some("model-calls"));
    let last: String =
        sqlx::query_scalar("SELECT role FROM model_messages WHERE conversation = $1 ORDER BY n DESC LIMIT 1")
            .bind(&conversation)
            .fetch_one(&scratch.pool)
            .await
            .expect("a row");
    assert_eq!(last, "user", "the refusal was not appended");
    scratch.close().await;
}

/// The review's size-limit finding (spec 01 section 3.4, "still past"): a conversation whose last request passed
/// `STUDIO_CONVERSATION_MAX_INPUT_TOKENS` takes its next turn, which compacts first; it refuses a turn only when the
/// request right after a compaction was still past it.
#[tokio::test]
async fn the_size_limit_reads_the_input_left_after_compaction() {
    let scratch = scratch_or_skip!();
    let model = Scripted::new(vec![
        ScriptedStep::text("Long.").costing(Tokens(5_000), Tokens(10)),
        ScriptedStep::text("Earlier: one long answer."),
        ScriptedStep::text("Still long.").costing(Tokens(3_000), Tokens(10)),
    ]);
    let settings =
        AgentSettings { compact_at: Tokens(1_000), conversation_max: Tokens(2_000), ..AgentSettings::default() };
    let stack = ready(&scratch.pool, settings, model.clone()).await;
    let conversation = opened(&stack.router, Some(SITE)).await;
    let first = say(&stack.router, &conversation, "first").await;
    assert_eq!(settled(&scratch.pool, &first).await, "done");
    let second = say(&stack.router, &conversation, "second").await;
    assert_eq!(settled(&scratch.pool, &second).await, "done");
    assert!(model.requests()[1].tools_off, "the second turn compacted first");
    let (status, refused) = post(&stack.router, &conversation, message_body("third")).await;
    assert_eq!((status, &refused["type"]), (StatusCode::CONFLICT, &json!("conversation-too-long")));
    scratch.close().await;
}
