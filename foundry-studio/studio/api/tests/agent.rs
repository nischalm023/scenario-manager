//! Spec 01's agent loop on the scripted model (brief I1.03 section 2, "Done when"): statements 1 (one open turn per
//! conversation; the transcript takes inserts only), 3 (a restart interrupts a running turn, a waiting one resumes),
//! 4 (an approved input executes as stored; a denial's reason reaches the model; an unattended turn is denied at once),
//! 6 (the catalogue's names and schemas), 7's budget half, 11 (every spoken entry carries its sentence), 12's message
//! half, 16's waiting half and 17 (compaction); with a reply's text and reasoning, tool calls and their order, a
//! question answered on its card or in words, Stop, and the model's failures.

mod support;

use std::time::{Duration, Instant};

use agent_loop::{ErrorClass, ModelError, Scripted, ScriptedBlock, ScriptedStep, wire_name};
use axum::http::StatusCode;
use serde_json::{Value, json};
use studio_api::agent::sweep;
use studio_api::config::{AgentSettings, ModelCalls, Tokens};
use support::agent::{
    SITE, body_of, call, create, entries, kinds, message_body, opened, post, question, ready, results, say, settled,
    texts,
};
use support::templates::{draft, send};
use tokio::task::JoinSet;
use tokio::time::sleep;
use wps_client::MockWps;

/// A reply's text is journaled as its block, its reasoning as nothing, the turn ends `done` naming its reply, the 202
/// and `message.user` carry the message's `client_message_id` (statement 12's message half), the transcript keeps the
/// user message (the page context's line and the words) and the reply, and the request is kept with its document.
#[tokio::test]
async fn a_reply_is_journaled_block_by_block_and_ends_the_turn_done() {
    let scratch = scratch_or_skip!();
    let steps = vec![ScriptedStep::reply(vec![
        ScriptedBlock::Reasoning("Thinking it over.".into()),
        ScriptedBlock::Text("Hello there.".into()),
    ])];
    let stack = ready(&scratch.pool, AgentSettings::default(), Scripted::new(steps)).await;
    let conversation = opened(&stack.router, Some(SITE)).await;
    let (status, accepted) = post(&stack.router, &conversation, message_body("hello")).await;
    assert_eq!((status, &accepted["client_message_id"]), (StatusCode::ACCEPTED, &json!("m-hello")));
    let turn = accepted["turn"].as_str().expect("a turn");
    assert_eq!(settled(&scratch.pool, turn).await, "done");
    let journal = entries(&scratch.pool, &conversation).await;
    assert_eq!(kinds(&journal), ["message.user", "turn.started", "text.done", "turn.finished"]);
    assert_eq!(body_of(&journal, "message.user")["client_message_id"], json!("m-hello"));
    let blocks: Vec<(&Value, &Value)> = journal
        .iter()
        .filter(|(kind, _, _)| kind == "text.done")
        .map(|(_, body, _)| (&body["kind"], &body["text"]))
        .collect();
    assert_eq!(blocks, [(&json!("text"), &json!("Hello there."))]);
    let finished = body_of(&journal, "turn.finished");
    assert_eq!((&finished["outcome"], &finished["reply_block"]), (&json!("done"), &json!("1.1.1")));
    let sent = stack.model.requests();
    assert_eq!(sent.len(), 1);
    let first = texts(sent[0].history.last().expect("the user message"));
    assert!(first.contains("customer/demo/site/lshape-6") && first.ends_with("hello"), "{first}");
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM model_messages WHERE conversation = $1")
        .bind(&conversation)
        .fetch_one(&scratch.pool)
        .await
        .expect("the transcript");
    assert_eq!(rows, 2);
    let raw: Value = sqlx::query_scalar("SELECT raw FROM model_calls WHERE turn = $1")
        .bind(turn)
        .fetch_one(&scratch.pool)
        .await
        .expect("a row");
    assert_eq!(raw, json!({ "scripted": true }));
    scratch.close().await;
}

/// Brief chore-chat-fixes, the first defect (D117: the chat says each thing once, with no meta-narration): the model's
/// reasoning before a call and before the reply journals no entry, every entry's body read for its words; the
/// transcript keeps it, and the request after the call carries it back, since Gemini's thought signatures go back with
/// what they sign.
#[tokio::test]
async fn the_models_reasoning_journals_no_entry_and_goes_back_to_the_model() {
    let scratch = scratch_or_skip!();
    let (before, after) = ("Listing Site Attributes: I'm focusing on the site list.", "Processing Site Information.");
    let model = Scripted::new(vec![
        ScriptedStep::reply(vec![ScriptedBlock::Reasoning(before.into()), call("site.list", json!({}))]),
        ScriptedStep::reply(vec![ScriptedBlock::Reasoning(after.into()), ScriptedBlock::Text("One site.".into())]),
    ]);
    let stack = ready(&scratch.pool, AgentSettings::default(), model).await;
    let conversation = opened(&stack.router, Some(SITE)).await;
    let turn = say(&stack.router, &conversation, "which sites").await;
    assert_eq!(settled(&scratch.pool, &turn).await, "done");
    let journal = entries(&scratch.pool, &conversation).await;
    for (kind, body, _) in &journal {
        let words = body.to_string();
        assert!(!words.contains("Site Attributes") && !words.contains("Site Information"), "{kind} holds: {words}");
    }
    let said: Vec<&Value> =
        (journal.iter()).filter(|(kind, _, _)| kind == "text.done").map(|(_, body, _)| &body["text"]).collect();
    assert_eq!(said, [&json!("One site.")]);
    let rows: Vec<Value> = sqlx::query_scalar("SELECT content FROM model_messages WHERE conversation = $1 ORDER BY n")
        .bind(&conversation)
        .fetch_all(&scratch.pool)
        .await
        .expect("the transcript");
    let kept = Value::Array(rows).to_string();
    assert!(kept.contains(before) && kept.contains(after), "{kept}");
    let sent = stack.model.requests();
    let carried = serde_json::to_string(&sent[1].history).expect("the history");
    assert!(carried.contains(before), "the request after the call carries the reasoning before it: {carried}");
    scratch.close().await;
}

/// Brief chore-chat-fixes, the second defect (D60: references name entities, never scope paths): a reply's links to
/// templates and a site, by `scope:` references or the bare path a tool gave, list no ref, since the chat links a
/// location to its panel route itself; an entity link the site's map does not hold is listed unresolved.
#[tokio::test]
async fn a_replys_locations_list_no_ref_and_an_unknown_entity_stays_unresolved() {
    let scratch = scratch_or_skip!();
    let missing = "b6a9a763-687c-4067-909b-460e986e2229";
    let words = format!(
        "Start from [Square](ref:template/site/square) or [L-Shape](ref:scope:template/site/l-shape@main) beside \
         [the demo site](ref:scope:{SITE}@main), whose [station](ref:{missing}) is gone."
    );
    let stack = ready(&scratch.pool, AgentSettings::default(), Scripted::new(vec![ScriptedStep::text(&words)])).await;
    let conversation = opened(&stack.router, Some(SITE)).await;
    let turn = say(&stack.router, &conversation, "templates").await;
    assert_eq!(settled(&scratch.pool, &turn).await, "done");
    let journal = entries(&scratch.pool, &conversation).await;
    let refs = &body_of(&journal, "text.done")["refs"];
    assert_eq!(refs, &json!([{ "key": missing, "kind": null, "label": "station", "route": null }]));
    scratch.close().await;
}

/// Brief chore-chat-fixes, the second defect's tool half: `template.list` names each template by its location at
/// `main`, the form a reply links it by, and by no `ref` the model could take for an entity's reference.
#[tokio::test]
async fn template_list_names_each_template_by_its_location() {
    let scratch = scratch_or_skip!();
    let model =
        Scripted::new(vec![ScriptedStep::reply(vec![call("template.list", json!({}))]), ScriptedStep::text("Listed.")]);
    let stack = ready(&scratch.pool, AgentSettings::default(), model).await;
    let conversation = opened(&stack.router, None).await;
    let turn = say(&stack.router, &conversation, "which templates").await;
    assert_eq!(settled(&scratch.pool, &turn).await, "done");
    let answered = results(&stack.model.requests()[1]);
    let items = answered[0]["content"][0]["value"]["items"].as_array().expect("the templates").clone();
    assert!(!items.is_empty());
    for item in &items {
        let location = item["location"].as_str().unwrap_or_default();
        let slug = location.strip_prefix("template/site/").and_then(|rest| rest.strip_suffix("@main"));
        assert!(slug.is_some_and(|slug| !slug.contains('/')) && item.get("ref").is_none(), "{item}");
    }
    scratch.close().await;
}

/// Spec 01 section 8, statement 1: two messages at once give one 202 and one `turn-running`; the partial unique index
/// refuses a second open turn whatever the code does; the transcript takes inserts only. Stop ends the running turn
/// `stopped` within 1 s of wall clock (brief I1.03, "Done when").
#[tokio::test]
async fn two_messages_at_once_start_one_turn_and_stop_ends_it_within_a_second() {
    let scratch = scratch_or_skip!();
    let model = Scripted::new(Vec::new()).then_forever(ScriptedStep::Hang);
    let stack = ready(&scratch.pool, AgentSettings::default(), model).await;
    let conversation = opened(&stack.router, Some(SITE)).await;
    let mut posts = JoinSet::new();
    for text in ["one", "two"] {
        let (router, conversation) = (stack.router.clone(), conversation.clone());
        posts.spawn(async move { post(&router, &conversation, message_body(text)).await });
    }
    let mut answers = posts.join_all().await;
    answers.sort_by_key(|(status, _)| *status);
    assert_eq!((answers[0].0, answers[1].0), (StatusCode::ACCEPTED, StatusCode::CONFLICT));
    assert_eq!(answers[1].1["type"], json!("turn-running"));
    let turn = answers[0].1["turn"].as_str().expect("a turn").to_owned();

    let second = sqlx::query(
        "INSERT INTO turns (id, conversation, n, started_by, attended, state, message_seq) \
         VALUES ('01KN0TJZANP5N0PGJD8F7A3ZZZ', $1, 9, 'dev', true, 'running', 1)",
    )
    .bind(&conversation)
    .execute(&scratch.pool)
    .await;
    let refused =
        second.expect_err("one open turn").as_database_error().and_then(|fault| fault.constraint()).map(str::to_owned);
    assert_eq!(refused.as_deref(), Some("turns_one_open_per_conversation"));
    let changed = sqlx::query("UPDATE model_messages SET role = 'assistant'").execute(&scratch.pool).await;
    assert!(changed.expect_err("inserts only").to_string().contains("takes inserts only"));

    sleep(Duration::from_millis(200)).await;
    let pressed = Instant::now();
    let (status, _, stopped) = send(&stack.router, "POST", &format!("/api/turns/{turn}/stop"), None, None).await;
    assert_eq!((status, &stopped["turn"]), (StatusCode::ACCEPTED, &json!(turn)));
    assert_eq!(settled(&scratch.pool, &turn).await, "stopped");
    assert!(pressed.elapsed() < Duration::from_secs(1), "stopped in {:?}", pressed.elapsed());
    let (status, _, again) = send(&stack.router, "POST", &format!("/api/turns/{turn}/stop"), None, None).await;
    assert_eq!((status, &again["turn"]), (StatusCode::OK, &json!(turn)));
    scratch.close().await;
}

/// Spec 01 section 3.8: a reply's read calls run under ids the API mints, each row journaled `tool.started` before it
/// runs and `tool.finished` after, the results going back in the calls' order in one user message; the turn's calls
/// read back whole; a call whose input fails the full schema never runs and fails `validation-failed` (statement 6).
#[tokio::test]
async fn a_replys_calls_run_and_answer_in_order_and_an_input_the_schema_refuses_never_runs() {
    let scratch = scratch_or_skip!();
    let model = Scripted::new(vec![
        ScriptedStep::reply(vec![
            call("site.list", json!({})),
            call("site.list", json!({ "customer": "nobody" })),
            call("ask_user", json!({ "items": [] })),
        ]),
        ScriptedStep::text("Done."),
    ]);
    let stack = ready(&scratch.pool, AgentSettings::default(), model).await;
    let conversation = opened(&stack.router, Some(SITE)).await;
    let turn = say(&stack.router, &conversation, "list them").await;
    assert_eq!(settled(&scratch.pool, &turn).await, "done");
    let journal = entries(&scratch.pool, &conversation).await;
    let tools: Vec<(&str, &Value)> = (journal.iter())
        .filter(|(kind, _, _)| kind.starts_with("tool."))
        .map(|(kind, body, _)| (kind.as_str(), &body["outcome"]))
        .collect();
    let (ok, failed, none) = (json!("ok"), json!("failed"), Value::Null);
    assert_eq!(
        tools,
        [
            ("tool.started", &none),
            ("tool.started", &none),
            ("tool.finished", &ok),
            ("tool.finished", &ok),
            ("tool.started", &none),
            ("tool.finished", &failed)
        ]
    );
    let refused = &journal.iter().rev().find(|(kind, _, _)| kind == "tool.finished").expect("a call").1;
    assert_eq!(refused["problem"]["type"], json!("validation-failed"));
    let questions: i64 =
        sqlx::query_scalar("SELECT count(*) FROM questions").fetch_one(&scratch.pool).await.expect("a count");
    assert_eq!(questions, 0, "the refused ask_user never ran");
    let sent = stack.model.requests();
    let answered = results(&sent[1]);
    assert_eq!(
        answered.iter().map(|result| result["is_error"].as_bool()).collect::<Vec<_>>(),
        [Some(false), Some(false), Some(true)]
    );
    assert_eq!(
        answered[0]["content"][0]["value"]["items"][0]["site"],
        json!(SITE),
        "the first call's own result first"
    );
    let (status, _, calls) = send(&stack.router, "GET", &format!("/api/turns/{turn}/calls"), None, None).await;
    assert_eq!((status, calls["calls"].as_array().map(Vec::len)), (StatusCode::OK, Some(3)));
    assert_eq!(calls["calls"][1]["input"], json!({ "customer": "nobody" }));
    scratch.close().await;
}

/// Spec 01 sections 3.4 and 3.9, `ask_user`: the turn waits on the question with the reply before it named; Dismiss
/// answers `turn-running` while it waits and Home's list says it waits (statement 16's waiting half); an answer with
/// a label the item offers resumes it, the same answer again answers as the first, another `question-closed`; the
/// model reads the answers as the call's result.
#[tokio::test]
async fn a_question_waits_on_its_card_and_an_answer_resumes_the_turn() {
    let scratch = scratch_or_skip!();
    let model = Scripted::new(vec![
        ScriptedStep::reply(vec![ScriptedBlock::Text("One question first.".into()), call("ask_user", question())]),
        ScriptedStep::text("L it is."),
    ]);
    let stack = ready(&scratch.pool, AgentSettings::default(), model).await;
    let conversation = opened(&stack.router, Some(SITE)).await;
    let turn = say(&stack.router, &conversation, "ask me first").await;
    assert_eq!(settled(&scratch.pool, &turn).await, "waiting");
    let journal = entries(&scratch.pool, &conversation).await;
    assert!(kinds(&journal).ends_with(&["tool.started", "question.asked", "turn.waiting"]), "{:?}", kinds(&journal));
    let waiting = body_of(&journal, "turn.waiting");
    assert_eq!((&waiting["on"], &waiting["reply_block"]), (&json!("question"), &json!("1.1.0")));
    let question = body_of(&journal, "question.asked")["question"].as_str().expect("a question").to_owned();
    let (status, _, refused) =
        send(&stack.router, "POST", &format!("/api/conversations/{conversation}/dismiss"), None, None).await;
    assert_eq!((status, &refused["type"]), (StatusCode::CONFLICT, &json!("turn-running")));
    let (_, _, sites) = send(&stack.router, "GET", "/api/sites", None, None).await;
    let listed =
        sites["items"].as_array().expect("sites").iter().find(|item| item["site"] == json!(SITE)).expect("the site");
    assert_eq!(
        (&listed["conversation"]["waiting"], &listed["conversation"]["working"]),
        (&json!("question"), &json!(false))
    );

    let answers = |label: &str| json!({ "answers": [{ "item": 0, "selected": [label] }] });
    let address = format!("/api/questions/{question}/answers");
    let (status, _, wrong) = send(&stack.router, "POST", &address, Some(answers("V")), None).await;
    assert_eq!((status, &wrong["type"]), (StatusCode::UNPROCESSABLE_ENTITY, &json!("validation-failed")));
    let (status, _, taken) = send(&stack.router, "POST", &address, Some(answers("L")), None).await;
    assert_eq!((status, &taken), (StatusCode::OK, &json!({ "question": { "id": question, "state": "answered" } })));
    let (status, _, _) = send(&stack.router, "POST", &address, Some(answers("L")), None).await;
    assert_eq!(status, StatusCode::OK, "the same answer again answers as the first");
    let (status, _, closed) = send(&stack.router, "POST", &address, Some(answers("U")), None).await;
    assert_eq!((status, &closed["type"]), (StatusCode::CONFLICT, &json!("question-closed")));
    assert_eq!(settled(&scratch.pool, &turn).await, "done");
    let sent = stack.model.requests();
    let result = &results(&sent[1])[0];
    assert_eq!(
        result["content"][0]["value"],
        json!({ "answers": [{ "header": "Shape", "selected": ["L"], "other": null }] })
    );
    scratch.close().await;
}

/// Spec 01 section 3.4: a message while the turn waits on a question answers it in words; the model reads
/// `answered_in_words` as the call's result and the words after it.
#[tokio::test]
async fn a_message_while_a_question_waits_answers_it_in_words() {
    let scratch = scratch_or_skip!();
    let model =
        Scripted::new(vec![ScriptedStep::call(&wire_name("ask_user"), question()), ScriptedStep::text("Noted.")]);
    let stack = ready(&scratch.pool, AgentSettings::default(), model).await;
    let conversation = opened(&stack.router, Some(SITE)).await;
    let turn = say(&stack.router, &conversation, "ask me").await;
    assert_eq!(settled(&scratch.pool, &turn).await, "waiting");
    let (status, accepted) = post(&stack.router, &conversation, message_body("a T shape, please")).await;
    assert_eq!((status, &accepted["turn"]), (StatusCode::ACCEPTED, &json!(turn)));
    assert!(accepted["answered"].is_string(), "{accepted}");
    assert_eq!(settled(&scratch.pool, &turn).await, "done");
    let sent = stack.model.requests();
    assert_eq!(results(&sent[1])[0]["content"][0]["value"], json!({ "answered_in_words": true }));
    assert!(texts(sent[1].history.last().expect("a message")).ends_with("a T shape, please"));
    let journal = entries(&scratch.pool, &conversation).await;
    let answered = journal.iter().find(|(kind, _, _)| kind == "question.answered").expect("answered");
    assert_eq!(answered.1["in_words"], json!(true));
    scratch.close().await;
}

/// Spec 01 section 8, statement 4, and section 3.17: Create site asks; approved, the stored input executes once under
/// the approval's id, as the approver, whatever the model says after the card; the made site binds the New site
/// thread, the act's own worker journals its execution, and the turn ends `done` with its reply.
#[tokio::test]
async fn an_approved_create_site_executes_its_stored_input_and_binds_the_thread() {
    let scratch = scratch_or_skip!();
    let model = Scripted::new(Vec::new());
    let stack = ready(&scratch.pool, AgentSettings::default(), model.clone()).await;
    let thread = opened(&stack.router, None).await;
    let (id, _) = draft(&stack.router, "rectangle", Some(&thread)).await;
    model.push(vec![
        ScriptedStep::call(&wire_name("site.create_from_template"), create(&id, "agent-1")),
        ScriptedStep::text("The site is made."),
    ]);
    let turn = say(&stack.router, &thread, "make it").await;
    assert_eq!(settled(&scratch.pool, &turn).await, "waiting");
    let journal = entries(&scratch.pool, &thread).await;
    let requested = body_of(&journal, "approval.requested").clone();
    assert_eq!(
        (&requested["tool"], &requested["input"]),
        (&json!("site.create_from_template"), &create(&id, "agent-1"))
    );
    assert_eq!(requested["facts"]["action_label"], json!("Create site"));
    let approval = requested["approval"].as_str().expect("an approval").to_owned();
    let decide = format!("/api/approvals/{approval}/decision");
    let (status, _, decided) = send(&stack.router, "POST", &decide, Some(json!({ "decision": "approve" })), None).await;
    assert_eq!((status, &decided["approval"]["state"]), (StatusCode::OK, &json!("approved")));
    assert_eq!(settled(&scratch.pool, &turn).await, "done");
    let (stored, via, execution): (Value, String, String) =
        sqlx::query_as("SELECT input, via, execution FROM approvals WHERE id = $1")
            .bind(&approval)
            .fetch_one(&scratch.pool)
            .await
            .expect("a row");
    assert_eq!((stored, via.as_str(), execution.as_str()), (create(&id, "agent-1"), "card", "ok"));
    let written: String = sqlx::query_scalar("SELECT write_approval FROM template_drafts WHERE id = $1")
        .bind(&id)
        .fetch_one(&scratch.pool)
        .await
        .expect("a draft");
    assert_eq!(written, approval, "executed under the approval's id");
    let journal = entries(&scratch.pool, &thread).await;
    let bound = body_of(&journal, "conversation.changed");
    assert_eq!(bound["site"], json!("customer/demo/site/agent-1"));
    let order = kinds(&journal);
    let at = |kind: &str| order.iter().position(|held| *held == kind).unwrap_or_else(|| panic!("no {kind}"));
    assert!(
        at("approval.decided") < at("turn.resumed") && at("turn.resumed") < at("conversation.changed"),
        "{order:?}"
    );
    assert_eq!(order.last(), Some(&"turn.finished"));
    let result = &results(&stack.model.requests()[1])[0];
    assert_eq!(result["content"][0]["value"]["site"], json!("customer/demo/site/agent-1"));
    scratch.close().await;
}

/// Spec 01 section 8, statement 4: a denial's reason reaches the next request as the call's error result; a decision
/// again answers as the first, another `approval-closed`; and in an unattended turn the asking call is denied at once
/// with `approval-required`, no approval row made.
#[tokio::test]
async fn a_denial_reaches_the_model_and_an_unattended_turn_is_denied_at_once() {
    let scratch = scratch_or_skip!();
    let model = Scripted::new(Vec::new());
    let stack = ready(&scratch.pool, AgentSettings::default(), model.clone()).await;
    let thread = opened(&stack.router, None).await;
    let (id, _) = draft(&stack.router, "rectangle", Some(&thread)).await;
    model.push(vec![
        ScriptedStep::call(&wire_name("site.create_from_template"), create(&id, "agent-2")),
        ScriptedStep::text("Understood."),
    ]);
    let turn = say(&stack.router, &thread, "make it").await;
    assert_eq!(settled(&scratch.pool, &turn).await, "waiting");
    let approval: String = sqlx::query_scalar("SELECT id FROM approvals WHERE turn = $1")
        .bind(&turn)
        .fetch_one(&scratch.pool)
        .await
        .expect("a card");
    let decide = format!("/api/approvals/{approval}/decision");
    let deny = json!({ "decision": "deny", "reason": "Not yet: pick a wider shell" });
    let (status, _, _) = send(&stack.router, "POST", &decide, Some(deny.clone()), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(settled(&scratch.pool, &turn).await, "done");
    let (status, _, _) = send(&stack.router, "POST", &decide, Some(deny), None).await;
    assert_eq!(status, StatusCode::OK, "the same decision again");
    let (status, _, closed) = send(&stack.router, "POST", &decide, Some(json!({ "decision": "approve" })), None).await;
    assert_eq!((status, &closed["type"]), (StatusCode::CONFLICT, &json!("approval-closed")));
    let result = &results(&model.requests()[1])[0];
    assert_eq!(result["is_error"], json!(true));
    assert_eq!(result["content"][0]["value"]["reason"], json!("Not yet: pick a wider shell"));

    model.push(vec![
        ScriptedStep::call(&wire_name("site.create_from_template"), create(&id, "agent-3")),
        ScriptedStep::text("Fine."),
    ]);
    let mut unattended = message_body("make it unattended");
    unattended["unattended"] = json!(true);
    let (status, accepted) = post(&stack.router, &thread, unattended).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    let turn = accepted["turn"].as_str().expect("a turn");
    assert_eq!(settled(&scratch.pool, turn).await, "done");
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM approvals WHERE turn = $1")
        .bind(turn)
        .fetch_one(&scratch.pool)
        .await
        .expect("a count");
    assert_eq!(rows, 0);
    let (state, problem): (String, Value) = sqlx::query_as("SELECT state, problem FROM tool_calls WHERE turn = $1")
        .bind(turn)
        .fetch_one(&scratch.pool)
        .await
        .expect("a call");
    assert_eq!((state.as_str(), &problem["type"]), ("denied", &json!("approval-required")));
    scratch.close().await;
}

/// Spec 01 section 3.10, rule 6: a person's press of Create site with the values the agent's waiting card holds
/// decides that card, answering with the agent's approval, and the turn carries it out.
#[tokio::test]
async fn a_press_decides_the_agents_pending_approval_of_the_same_act() {
    let scratch = scratch_or_skip!();
    let model = Scripted::new(Vec::new());
    let stack = ready(&scratch.pool, AgentSettings::default(), model.clone()).await;
    let thread = opened(&stack.router, None).await;
    let (id, _) = draft(&stack.router, "rectangle", Some(&thread)).await;
    model.push(vec![
        ScriptedStep::call(&wire_name("site.create_from_template"), create(&id, "agent-4")),
        ScriptedStep::text("Made."),
    ]);
    let turn = say(&stack.router, &thread, "make it").await;
    assert_eq!(settled(&scratch.pool, &turn).await, "waiting");
    let approval: String = sqlx::query_scalar("SELECT id FROM approvals WHERE turn = $1")
        .bind(&turn)
        .fetch_one(&scratch.pool)
        .await
        .expect("a card");
    let body = json!({ "customer": "demo", "slug": "agent-4", "name": "Agent agent-4", "evaluation": 1 });
    let (status, _, pressed) =
        send(&stack.router, "POST", &format!("/api/template-drafts/{id}/site"), Some(body), None).await;
    assert_eq!((status, &pressed["approval"]["id"]), (StatusCode::ACCEPTED, &json!(approval)));
    assert_eq!(settled(&scratch.pool, &turn).await, "done");
    let execution: String = sqlx::query_scalar("SELECT execution FROM approvals WHERE id = $1")
        .bind(&approval)
        .fetch_one(&scratch.pool)
        .await
        .expect("a row");
    assert_eq!(execution, "ok");
    scratch.close().await;
}

/// Spec 01 section 8, statement 7's budget half: a model that never stops calling tools ends the turn `done` once the
/// call budget is spent, after one summary request sent with tools off, `turn.finished` naming the budget.
#[tokio::test]
async fn a_turn_that_never_stops_calling_tools_ends_done_at_its_budget() {
    let scratch = scratch_or_skip!();
    let model = Scripted::new(Vec::new()).then_forever(ScriptedStep::call(&wire_name("site.list"), json!({})));
    let settings = AgentSettings { max_model_calls: ModelCalls(3), ..AgentSettings::default() };
    let stack = ready(&scratch.pool, settings, model).await;
    let conversation = opened(&stack.router, Some(SITE)).await;
    let turn = say(&stack.router, &conversation, "keep going").await;
    assert_eq!(settled(&scratch.pool, &turn).await, "done");
    let sent = stack.model.requests();
    let off: Vec<bool> = sent.iter().map(|request| request.tools_off).collect();
    assert_eq!(off, [false, false, false, true], "three steps, then one summary request with tools off");
    assert!(texts(sent[3].history.last().expect("a message")).contains("budget of 3 model requests"));
    let journal = entries(&scratch.pool, &conversation).await;
    assert_eq!(body_of(&journal, "turn.finished")["budget"], json!("model-calls"));
    scratch.close().await;
}

/// Spec 01 section 8, statement 17: a conversation past `STUDIO_COMPACT_AT_INPUT_TOKENS` sends one summary request with
/// tools off at the next turn's boundary, then requests carrying the summary and none of the messages it replaced, no
/// call without its result; the journal is unchanged but for the note.
#[tokio::test]
async fn a_long_conversation_is_summarised_at_a_turn_boundary() {
    let scratch = scratch_or_skip!();
    let model = Scripted::new(vec![
        ScriptedStep::reply(vec![call("site.list", json!({}))]).costing(Tokens(900), Tokens(10)),
        ScriptedStep::text("The first answer.").costing(Tokens(1_000), Tokens(10)),
        ScriptedStep::text("SUMMARY: the person asked about sites.").costing(Tokens(1_100), Tokens(20)),
        ScriptedStep::text("The second answer.").costing(Tokens(200), Tokens(10)),
    ]);
    let settings =
        AgentSettings { compact_at: Tokens(500), conversation_max: Tokens(50_000), ..AgentSettings::default() };
    let stack = ready(&scratch.pool, settings, model).await;
    let conversation = opened(&stack.router, Some(SITE)).await;
    let first = say(&stack.router, &conversation, "first question").await;
    assert_eq!(settled(&scratch.pool, &first).await, "done");
    let before = entries(&scratch.pool, &conversation).await.len();
    let second = say(&stack.router, &conversation, "second question").await;
    assert_eq!(settled(&scratch.pool, &second).await, "done");
    let sent = stack.model.requests();
    assert_eq!(sent.iter().map(|request| request.tools_off).collect::<Vec<_>>(), [false, false, true, false]);
    let summary = &sent[2];
    assert!(texts(summary.history.last().expect("a message")).contains("Summarise"));
    let after = &sent[3].history;
    assert_eq!(after.len(), 1, "the summary and the new message, one user message, nothing before it");
    let carried = texts(&after[0]);
    assert!(
        carried.starts_with("Summary of the conversation before this turn") && carried.ends_with("second question"),
        "{carried}"
    );
    assert!(!carried.contains("first question") && !carried.contains("The first answer."), "{carried}");
    let journal = entries(&scratch.pool, &conversation).await;
    let added: Vec<&str> = kinds(&journal[before..]).into_iter().collect();
    assert_eq!(added, ["message.user", "turn.started", "system.note", "text.done", "turn.finished"]);
    assert_eq!(
        journal[before + 2].1["text"],
        json!("Earlier turns were summarised for the agent; the log keeps them in full.")
    );
    scratch.close().await;
}

fn unavailable() -> ScriptedStep {
    ScriptedStep::Fail(ModelError {
        class: ErrorClass::Unavailable,
        status: Some(429),
        words: "Resource exhausted".into(),
        retry_after: None,
    })
}

/// Spec 01 section 3.7: a 429 is asked again and the turn goes on; one that stays unavailable past three retries
/// fails the turn `model-unavailable`, a 400 `model-request-rejected`, and a failure the section does not name (a
/// refused credential) `agent-provider-failed` (brief I1.03 section 2), each with a note naming the status and the
/// provider's words (statement 11: the note is a sentence).
#[tokio::test]
async fn the_models_failures_are_retried_or_end_the_turn_with_a_note() {
    let scratch = scratch_or_skip!();
    let rejected = ScriptedStep::Fail(ModelError {
        class: ErrorClass::Rejected,
        status: Some(400),
        words: "bad schema".into(),
        retry_after: None,
    });
    let unnamed = ScriptedStep::Fail(ModelError {
        class: ErrorClass::Failed,
        status: Some(401),
        words: "API key not valid".into(),
        retry_after: None,
    });
    let model = Scripted::new(vec![
        unavailable(),
        ScriptedStep::text("Back again."),
        unavailable(),
        unavailable(),
        unavailable(),
        unavailable(),
        rejected,
        unnamed,
    ]);
    let stack = ready(&scratch.pool, AgentSettings::default(), model).await;
    let conversation = opened(&stack.router, Some(SITE)).await;
    let outcomes = [
        ("done", None),
        ("failed", Some("model-unavailable")),
        ("failed", Some("model-request-rejected")),
        ("failed", Some("agent-provider-failed")),
    ];
    for (n, (state, reason)) in outcomes.into_iter().enumerate() {
        let turn = say(&stack.router, &conversation, &format!("try {n}")).await;
        assert_eq!(settled(&scratch.pool, &turn).await, state);
        let (end, calls): (Option<String>, i32) =
            sqlx::query_as("SELECT end_reason, model_calls FROM turns WHERE id = $1")
                .bind(&turn)
                .fetch_one(&scratch.pool)
                .await
                .expect("a turn");
        assert_eq!(end.as_deref(), reason);
        assert_eq!(calls, [2, 4, 1, 1][n], "requests sent by turn {n}");
    }
    let journal = entries(&scratch.pool, &conversation).await;
    let notes: Vec<&str> = (journal.iter())
        .filter(|(kind, _, _)| kind == "system.note")
        .filter_map(|(_, body, _)| body["text"].as_str())
        .collect();
    assert_eq!(notes.len(), 3);
    assert!(
        notes[0].starts_with("The model stayed unavailable after 3 retries (429).")
            && notes[0].ends_with("Resource exhausted"),
        "{}",
        notes[0]
    );
    assert!(notes[1].contains("(400)") && notes[1].ends_with("bad schema"), "{}", notes[1]);
    assert!(notes[2].contains("(401)") && notes[2].ends_with("API key not valid"), "{}", notes[2]);
    scratch.close().await;
}

/// Spec 01 section 8, statement 3: a restart's sweep interrupts a running turn and its executing call; a waiting turn
/// keeps waiting and resumes on its answer.
#[tokio::test]
async fn a_restart_interrupts_a_running_turn_and_a_waiting_one_resumes() {
    let scratch = scratch_or_skip!();
    let model = Scripted::new(vec![
        ScriptedStep::Hang,
        ScriptedStep::call(&wire_name("ask_user"), question()),
        ScriptedStep::text("Done."),
    ]);
    let stack = ready(&scratch.pool, AgentSettings::default(), model).await;
    let running = opened(&stack.router, Some(SITE)).await;
    let turn = say(&stack.router, &running, "hang").await;
    sleep(Duration::from_millis(200)).await;
    sweep(&stack.gate.app).await.expect("the sweep");
    assert_eq!(settled(&scratch.pool, &turn).await, "interrupted");
    let journal = entries(&scratch.pool, &running).await;
    let finished = body_of(&journal, "turn.finished");
    assert_eq!((&finished["outcome"], &finished["end_reason"]), (&json!("interrupted"), &json!("interrupted")));

    let thread = opened(&stack.router, None).await;
    let waiting = say(&stack.router, &thread, "ask").await;
    assert_eq!(settled(&scratch.pool, &waiting).await, "waiting");
    sweep(&stack.gate.app).await.expect("the sweep");
    assert_eq!(settled(&scratch.pool, &waiting).await, "waiting", "a waiting turn keeps waiting");
    let question: String = sqlx::query_scalar("SELECT id FROM questions WHERE turn = $1")
        .bind(&waiting)
        .fetch_one(&scratch.pool)
        .await
        .expect("a question");
    let answer = json!({ "answers": [{ "item": 0, "selected": ["U"] }] });
    let (status, _, _) =
        send(&stack.router, "POST", &format!("/api/questions/{question}/answers"), Some(answer), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(settled(&scratch.pool, &waiting).await, "done");
    scratch.close().await;
}

/// Spec 01 section 8, statement 6: every catalogue name maps to one wire name a model takes, and ask_user's schema
/// takes its documented example and refuses five items.
#[test]
fn every_tool_has_one_legal_wire_name_and_a_schema_that_takes_its_examples() {
    let registry = studio_api::tools::registry::<MockWps>();
    let names: Vec<(&str, String)> = registry.names().map(|(name, wire)| (name, wire.to_owned())).collect();
    assert_eq!(names.len(), registry.definitions().len());
    for (name, wire) in &names {
        assert_eq!(*wire, name.replace('.', "_"));
        assert!(
            !wire.is_empty()
                && wire.len() <= 128
                && wire.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte)),
            "{wire}"
        );
        assert_eq!(names.iter().filter(|(_, other)| other == wire).count(), 1, "{wire} is one tool's");
    }
    assert!(registry.check("ask_user", &question()).is_ok());
    let five = json!({ "items": vec![question()["items"][0].clone(); 5] });
    let refused = registry.check("ask_user", &five).expect_err("at most four items");
    assert!(refused.iter().any(|why| why.starts_with("`/items`")), "{refused:?}");
    let create = json!({ "draft": "01KN0TJZANP5N0PGJD8F7A3FD1", "evaluation": 1, "customer": "demo", "slug": "x-1", "name": "X" });
    assert!(registry.check("site.create_from_template", &create).is_ok());
    assert!(registry.check("site.create_from_template", &json!({ "draft": "d" })).is_err());
}

/// Brief I1.03 section 2: the test writer makes the loop's change to `turns` for each `turn.*` entry, refuses a turn
/// another conversation holds, and a card it wrote has no row, so acting on it answers `not-found`.
#[tokio::test]
async fn the_test_writer_follows_turns_and_its_cards_have_no_rows() {
    let scratch = scratch_or_skip!();
    let stack = ready(&scratch.pool, AgentSettings::default(), Scripted::new(Vec::new())).await;
    let (one, two) = (opened(&stack.router, Some(SITE)).await, opened(&stack.router, None).await);
    let turn = "01KN0TJZANP5N0PGJD8F7A3TRN";
    let author = json!({ "subject": "dev", "name": "Developer" });
    let write = |conversation: &str, kind: &str, body: Value| {
        let (router, address) = (stack.router.clone(), format!("/api/dev/conversations/{conversation}/entries"));
        let entry = json!({ "kind": kind, "body": body, "turn": turn });
        async move { send(&router, "POST", &address, Some(entry), None).await }
    };
    let message = json!({ "author": author, "text": "hi", "context": null, "source": "composer", "browser_tab": null });
    assert_eq!(write(&one, "message.user", message.clone()).await.0, StatusCode::CREATED);
    let started = json!({ "n": 1, "author": author, "message_seq": 1, "attended": true, "browser_tab": null });
    assert_eq!(write(&one, "turn.started", started).await.0, StatusCode::CREATED);
    let question = "01KN0TJZANP5N0PGJD8F7A3QST";
    assert_eq!(write(&one, "turn.waiting", json!({ "on": "question", "id": question })).await.0, StatusCode::CREATED);
    let state: (String, Option<String>) = sqlx::query_as("SELECT state, waiting_on FROM turns WHERE id = $1")
        .bind(turn)
        .fetch_one(&scratch.pool)
        .await
        .expect("a row");
    assert_eq!((state.0.as_str(), state.1.as_deref()), ("waiting", Some("question")));
    let (status, _, refused) = write(&two, "message.user", message).await;
    assert_eq!(
        (status, refused["type"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("validation-failed")),
        "{refused}"
    );
    let answers = json!({ "answers": [{ "item": 0, "selected": ["L"] }] });
    let (status, _, missing) =
        send(&stack.router, "POST", &format!("/api/questions/{question}/answers"), Some(answers), None).await;
    assert_eq!((status, missing["type"].as_str()), (StatusCode::NOT_FOUND, Some("not-found")));
    let finished = json!({ "outcome": "stopped", "usage": { "model_calls": 0, "input_tokens": 0, "output_tokens": 0, "cache_read_input_tokens": 0 } });
    assert_eq!(write(&one, "turn.finished", finished).await.0, StatusCode::CREATED);
    let state: String = sqlx::query_scalar("SELECT state FROM turns WHERE id = $1")
        .bind(turn)
        .fetch_one(&scratch.pool)
        .await
        .expect("a row");
    assert_eq!(state, "stopped");
    scratch.close().await;
}
