//! Within `agent`, `calls` are one reply's tool calls run through the registry (spec 01 sections 3.8 and 3.10): each
//! under an id the API mints, its row and `tool.started` written before it runs, its input checked against the full
//! schema, consecutive read and page calls run together four at a time, a draft call alone; an asking call pauses the
//! turn on an approval (denied at once in an unattended turn) and `ask_user` on a question, the lease let go, its card
//! drawn while Stop can still end the turn and its wait written under the turn's locks only while the turn runs; and on
//! resumption the waiting call settles first, an approved input executing once under the approval's id with the
//! approver as who acts, before the reply's remaining calls run. Before each call the turn's budgets are checked: past
//! one, the reply's calls left fail unrun; and the wall-clock time the turn has left to run bounds each call and card.
//! When every call has its row's outcome, the results go back in the calls' order, a failure as an error result.

use std::time::Duration;

use agent_loop::registry::RESULT_LIMIT;
use agent_loop::rig::{ToolCall, ToolResult, ToolResultContent};
use agent_loop::{Card, Class, Tool};
use futures::future::join_all;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{Error as SqlxError, Postgres, Row, Transaction};
use studio_protocol::{
    ApprovalRequested, CallId, CallState, Execution, ItemAnswer, JournalKind, Problem, ProblemType, QuestionAsked,
    QuestionItem, ToolFinished, ToolStarted, TurnState, TurnWaiting, Ulid, WaitingOn,
};
use tokio::select;
use tokio::time::{sleep, timeout};
use wps_client::wps::Wps;

use super::end::{Budget, Ending, Next, over_budget, reply_block};
use super::transcript;
use super::turn::Turn;
use super::{journal, lock_turn};
use crate::approvals::{Moved, executed};
use crate::db::{decode, mint_ulid, word};
use crate::idempotency::write_canonical;
use crate::identity::Identity;
use crate::journal::Written;
use crate::tools::ToolContext;

/// How many read and page calls run at once (spec 01 section 3.8, "Order within one response").
const TOGETHER: usize = 4;
pub(super) const ASK_USER: &str = "ask_user";

pub(super) enum Ran {
    Paused,
    Ended(Ending),
    Done(Vec<ToolResult>),
}

/// What an asking call came to: the turn waits on its card; the call settled without a wait (denied in an unattended
/// turn, or its card refused); or the turn was stopped before it could wait.
enum Asked {
    Paused,
    Settled,
    Stopped,
}

/// One call as the registry reads it: its tool when the wire name names one, and why its input fails the schema.
struct Planned<'a, S: Wps> {
    call: &'a ToolCall,
    tool: Option<&'a dyn Tool<ToolContext<S>>>,
    name: String,
    input: Value,
    invalid: Option<Vec<String>>,
}

impl<S: Wps> Planned<'_, S> {
    fn class(&self) -> Class {
        self.tool.map_or(Class::Read, |tool| tool.class())
    }

    fn together(&self) -> bool {
        self.invalid.is_none()
            && self.tool.is_some()
            && matches!(self.class(), Class::Read | Class::Page)
            && self.name != ASK_USER
    }
}

enum Outcome {
    Ok(Value),
    Failed(Problem),
    Cancelled,
}

fn plan<'a, S: Wps>(turn: &'a Turn<S>, call: &'a ToolCall) -> Planned<'a, S> {
    let input = Value::Object(call.function.arguments.clone());
    let tool = turn.registry.by_wire(call.function.name.as_str());
    let name = tool.map_or_else(|| call.function.name.to_string(), |tool| tool.name().to_owned());
    let invalid = match tool {
        None => Some(vec![format!("there is no tool `{}`", call.function.name)]),
        Some(_) if call.function.invalid_arguments.is_some() => {
            Some(vec!["the arguments are not a JSON object".into()])
        }
        Some(tool) => turn.registry.check(tool.name(), &input).err(),
    };
    Planned { call, tool, name, input, invalid }
}

/// Run the reply's `calls` from the first.
pub(super) async fn run<S: Wps>(turn: &Turn<S>, model_call: i32, calls: Vec<ToolCall>) -> Result<Ran, SqlxError> {
    run_from(turn, model_call, &calls, 0).await
}

async fn run_from<S: Wps>(turn: &Turn<S>, model_call: i32, calls: &[ToolCall], from: usize) -> Result<Ran, SqlxError> {
    let mut at = from;
    while at < calls.len() {
        if turn.cancel.is_cancelled() {
            return Ok(Ran::Ended(Ending::stopped()));
        }
        if let Some(budget) = over_budget(turn, Next::Call).await? {
            unrun(turn, model_call, &calls[at..], at, &budget).await?;
            break;
        }
        let first = plan(turn, &calls[at]);
        if first.invalid.is_none() && (first.class().asks() || first.name == ASK_USER) {
            let id = start(turn, model_call, at, &first).await?;
            let asked = match first.name.as_str() {
                ASK_USER => ask(turn, &id, &first).await?,
                _ => request_approval(turn, &id, &first).await?,
            };
            match asked {
                Asked::Paused => return Ok(Ran::Paused),
                Asked::Stopped => return Ok(Ran::Ended(Ending::stopped())),
                Asked::Settled => at += 1,
            }
            continue;
        }
        let mut batch = vec![first];
        while batch[0].together() && batch.len() < TOGETHER && at + batch.len() < calls.len() {
            let next = plan(turn, &calls[at + batch.len()]);
            if !next.together() {
                break;
            }
            batch.push(next);
        }
        let mut ids = Vec::new();
        for (offset, planned) in batch.iter().enumerate() {
            ids.push(start(turn, model_call, at + offset, planned).await?);
        }
        let identity = turn.identity.clone();
        let outcomes =
            join_all(batch.iter().zip(&ids).map(|(planned, id)| execute(turn, planned, id, &identity, None))).await;
        for ((planned, id), outcome) in batch.iter().zip(&ids).zip(outcomes) {
            match outcome {
                Outcome::Cancelled => return Ok(Ran::Ended(Ending::stopped())),
                outcome => settle(turn, id, planned, outcome).await?,
            }
        }
        at += batch.len();
    }
    Ok(Ran::Done(results(turn, model_call).await?))
}

/// The reply's calls from position `from` on, which `budget` stopped before they ran: each gets its row and
/// `tool.started`, then fails saying so, so every call of the reply has its result.
async fn unrun<S: Wps>(
    turn: &Turn<S>,
    model_call: i32,
    calls: &[ToolCall],
    from: usize,
    budget: &Budget,
) -> Result<(), SqlxError> {
    for (offset, call) in calls.iter().enumerate() {
        let planned = plan(turn, call);
        let id = start(turn, model_call, from + offset, &planned).await?;
        settle(turn, &id, &planned, Outcome::Failed(budget.stopped_call(false))).await?;
    }
    Ok(())
}

/// The call's row and `tool.started`, before it runs.
async fn start<S: Wps>(
    turn: &Turn<S>,
    model_call: i32,
    position: usize,
    planned: &Planned<'_, S>,
) -> Result<CallId, SqlxError> {
    let id = CallId::new(mint_ulid().to_string());
    let summary = match (planned.tool, &planned.invalid) {
        (Some(tool), None) => tool.describe(&planned.input),
        _ => format!("Calling {}", planned.name),
    };
    let class = planned.class().as_str();
    let mut tx = turn.gate.app.pool.begin().await?;
    sqlx::query(
        "INSERT INTO tool_calls (id, conversation, turn, model_call, position, wire_call, tool, class, input, state, \
             summary) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'running', $10)",
    )
    .bind(id.as_str())
    .bind(turn.conversation.as_str())
    .bind(turn.id.as_str())
    .bind(model_call)
    .bind(position as i32)
    .bind(json!(planned.call))
    .bind(&planned.name)
    .bind(class)
    .bind(&planned.input)
    .bind(&summary)
    .execute(&mut *tx)
    .await?;
    let body = ToolStarted { call: id.clone(), tool: planned.name.clone(), class: class.to_owned(), summary };
    let written =
        journal(&mut tx, &turn.conversation, Some(&turn.id), JournalKind::ToolStarted, body, Some(id.to_string()))
            .await?;
    tx.commit().await?;
    turn.gate.app.hub.announce([written]);
    Ok(id)
}

/// Run one call as `identity`, under its timeout and the turn's active-time budget, `approval` naming the approved
/// input it executes; an input the schema refuses never runs.
async fn execute<S: Wps>(
    turn: &Turn<S>,
    planned: &Planned<'_, S>,
    id: &CallId,
    identity: &Identity,
    approval: Option<Ulid>,
) -> Outcome {
    let Some(tool) = planned.tool else { return Outcome::Failed(invalid(planned)) };
    if planned.invalid.is_some() {
        return Outcome::Failed(invalid(planned));
    }
    let cx = turn.context(id, identity.clone(), approval);
    let settings = &turn.agent().settings;
    let limit = tool.timeout().unwrap_or(settings.tool_timeout);
    let left = turn.active_left().max(Duration::from_millis(1));
    let ran = select! {
        _ = turn.cancel.cancelled() => return Outcome::Cancelled,
        _ = sleep(left) => {
            return Outcome::Failed(Budget::ActiveSeconds(settings.max_active.as_secs()).stopped_call(true));
        }
        ran = timeout(limit, tool.execute(&cx, planned.input.clone())) => ran,
    };
    match ran {
        Err(_) => {
            let words = format!(
                "{} took longer than {} wall-clock seconds and was stopped; its effect is unknown.",
                planned.name,
                limit.as_secs()
            );
            Outcome::Failed(Problem::new(ProblemType::ToolTimeout, words))
        }
        Ok(Err(problem)) => Outcome::Failed(problem),
        Ok(Ok(result)) => {
            let size = result.to_string().len();
            if size > RESULT_LIMIT {
                let words =
                    format!("The answer is {size} bytes, past the 64 KiB a result may hold: ask a narrower question.");
                return Outcome::Failed(Problem::new(ProblemType::ResultTooLarge, words));
            }
            Outcome::Ok(result)
        }
    }
}

fn invalid<S: Wps>(planned: &Planned<'_, S>) -> Problem {
    let mut problem = Problem::new(ProblemType::ValidationFailed, format!("The call of {} was not run.", planned.name));
    problem.detail = format!("{} {}", problem.detail, planned.invalid.clone().unwrap_or_default().join("; "));
    problem
}

/// The call's row and `tool.finished` as `outcome` says.
async fn settle<S: Wps>(
    turn: &Turn<S>,
    id: &CallId,
    planned: &Planned<'_, S>,
    outcome: Outcome,
) -> Result<(), SqlxError> {
    let described = match (planned.tool, &planned.invalid) {
        (Some(tool), None) => tool.describe(&planned.input),
        _ => format!("Calling {}", planned.name),
    };
    let (state, result, problem, summary) = match outcome {
        Outcome::Ok(result) => {
            let summary =
                planned.tool.map_or_else(|| described.clone(), |tool| tool.summarize(&planned.input, &result));
            (CallState::Ok, Some(result), None, summary)
        }
        Outcome::Failed(problem) => {
            let summary = format!("{described}: failed: {}", problem.detail);
            (CallState::Failed, None, Some(json!(problem)), summary)
        }
        Outcome::Cancelled => return Ok(()),
    };
    finished(turn, id, state, result, problem, summary).await
}

async fn finished<S: Wps>(
    turn: &Turn<S>,
    id: &CallId,
    state: CallState,
    result: Option<Value>,
    problem: Option<Value>,
    summary: String,
) -> Result<(), SqlxError> {
    let mut tx = turn.gate.app.pool.begin().await?;
    sqlx::query(
        "UPDATE tool_calls SET state = $2, result = $3, problem = $4, summary = $5, ended_at = now() WHERE id = $1",
    )
    .bind(id.as_str())
    .bind(word(state))
    .bind(&result)
    .bind(&problem)
    .bind(&summary)
    .execute(&mut *tx)
    .await?;
    let body = ToolFinished { call: id.clone(), outcome: state, summary, refs: Vec::new(), problem };
    let written =
        journal(&mut tx, &turn.conversation, Some(&turn.id), JournalKind::ToolFinished, body, Some(id.to_string()))
            .await?;
    tx.commit().await?;
    turn.gate.app.hub.announce([written]);
    Ok(())
}

/// An asking call (spec 01 section 3.10): denied at once when nobody is attached; its card step's refusal fails it;
/// else the approval waits `pending` with its stored input and the turn waits on it. Stop ends the card's drawing.
async fn request_approval<S: Wps>(turn: &Turn<S>, id: &CallId, planned: &Planned<'_, S>) -> Result<Asked, SqlxError> {
    let tool = planned.tool.expect("an asking call names its tool");
    if !turn.attended {
        let words = "This needs a person's approval, and nobody is attached to this turn to give it.";
        let problem = json!(Problem::new(ProblemType::ApprovalRequired, words));
        let summary = format!("{}: denied: nobody was attached to approve", tool.describe(&planned.input));
        finished(turn, id, CallState::Denied, None, Some(problem), summary).await?;
        return Ok(Asked::Settled);
    }
    let cx = turn.context(id, turn.identity.clone(), None);
    let settings = &turn.agent().settings;
    let drawn = select! {
        biased;
        _ = turn.cancel.cancelled() => return Ok(Asked::Stopped),
        _ = sleep(turn.active_left()) => None,
        drawn = timeout(settings.tool_timeout, tool.card(&cx, &planned.input)) => Some(drawn),
    };
    let card: Card = match drawn {
        Some(Ok(Ok(card))) => card,
        refused => {
            let problem = match refused {
                Some(Ok(Err(problem))) => problem,
                Some(Err(_)) => Problem::new(ProblemType::ToolTimeout, "The approval card could not be drawn in time."),
                _ => Budget::ActiveSeconds(settings.max_active.as_secs()).stopped_call(false),
            };
            settle(turn, id, planned, Outcome::Failed(problem)).await?;
            return Ok(Asked::Settled);
        }
    };
    let approval = mint_ulid();
    let mut canonical = Vec::new();
    write_canonical(&card.stored_input, &mut canonical);
    let digest = format!("{:x}", Sha256::digest(&canonical));
    let mut tx = turn.gate.app.pool.begin().await?;
    if !may_wait(turn, &mut tx).await? {
        return Ok(Asked::Stopped);
    }
    sqlx::query(
        "INSERT INTO approvals (id, conversation, turn, call, tool, class, input, input_digest, summary, facts, via, \
             state) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'card', 'pending')",
    )
    .bind(approval.as_str())
    .bind(turn.conversation.as_str())
    .bind(turn.id.as_str())
    .bind(id.as_str())
    .bind(tool.name())
    .bind(tool.class().as_str())
    .bind(&card.stored_input)
    .bind(digest)
    .bind(&card.summary)
    .bind(json!(card.facts))
    .execute(&mut *tx)
    .await?;
    let requested = ApprovalRequested {
        approval: approval.clone(),
        call: Some(id.clone()),
        tool: tool.name().to_owned(),
        class: tool.class().as_str().to_owned(),
        summary: card.summary,
        input: card.stored_input,
        facts: card.facts,
    };
    let mut written = vec![
        journal(
            &mut tx,
            &turn.conversation,
            Some(&turn.id),
            JournalKind::ApprovalRequested,
            requested,
            Some(id.to_string()),
        )
        .await?,
    ];
    written.push(wait(turn, &mut tx, id, WaitingOn::Approval, &approval).await?);
    tx.commit().await?;
    turn.gate.app.hub.announce(written);
    Ok(Asked::Paused)
}

/// `ask_user` (spec 01 section 3.9): the question `open`, the turn waiting on it.
async fn ask<S: Wps>(turn: &Turn<S>, id: &CallId, planned: &Planned<'_, S>) -> Result<Asked, SqlxError> {
    let items: Vec<QuestionItem> = (planned.input["items"].as_array().into_iter().flatten())
        .map(|item| QuestionItem {
            header: item["header"].as_str().unwrap_or_default().to_owned(),
            text: item["text"].as_str().unwrap_or_default().to_owned(),
            multi: item["multi"].as_bool().unwrap_or(false),
            other: item["other"].as_bool().unwrap_or(false),
            options: serde_json::from_value(item["options"].clone()).unwrap_or_default(),
        })
        .collect();
    let question = mint_ulid();
    let mut tx = turn.gate.app.pool.begin().await?;
    if !may_wait(turn, &mut tx).await? {
        return Ok(Asked::Stopped);
    }
    sqlx::query("INSERT INTO questions (id, turn, call, items) VALUES ($1, $2, $3, $4)")
        .bind(question.as_str())
        .bind(turn.id.as_str())
        .bind(id.as_str())
        .bind(json!(items))
        .execute(&mut *tx)
        .await?;
    let asked = QuestionAsked { question: question.clone(), call: Some(id.clone()), items };
    let mut written = vec![
        journal(&mut tx, &turn.conversation, Some(&turn.id), JournalKind::QuestionAsked, asked, Some(id.to_string()))
            .await?,
    ];
    written.push(wait(turn, &mut tx, id, WaitingOn::Question, &question).await?);
    tx.commit().await?;
    turn.gate.app.hub.announce(written);
    Ok(Asked::Paused)
}

/// Inside `tx`, before a card is written: the turn's locks in the order Stop takes them, then whether the turn still
/// runs and nobody stopped it, so a Stop that cancelled the task before this commit is never followed by a wait.
async fn may_wait<S: Wps>(turn: &Turn<S>, tx: &mut Transaction<'_, Postgres>) -> Result<bool, SqlxError> {
    let held = lock_turn(tx, turn.id.as_str()).await?;
    Ok(held.is_some_and(|held| held.state == TurnState::Running) && !turn.cancel.is_cancelled())
}

/// Inside `tx`, after `may_wait`: the call and the turn waiting on `card`, the lease let go and the stretch's time
/// counted, and `turn.waiting` naming the reply before the wait.
async fn wait<S: Wps>(
    turn: &Turn<S>,
    tx: &mut Transaction<'_, Postgres>,
    call: &CallId,
    on: WaitingOn,
    card: &Ulid,
) -> Result<Written, SqlxError> {
    sqlx::query("UPDATE tool_calls SET state = 'waiting' WHERE id = $1").bind(call.as_str()).execute(&mut **tx).await?;
    sqlx::query(
        "UPDATE turns SET state = 'waiting', waiting_on = $2, waiting_id = $3, waiting_since = now(), \
             lease_owner = NULL, lease_expires_at = NULL, active_ms = active_ms + $4 WHERE id = $1",
    )
    .bind(turn.id.as_str())
    .bind(word(on))
    .bind(card.as_str())
    .bind(turn.stretch.elapsed().as_millis() as i64)
    .execute(&mut **tx)
    .await?;
    let reply_block = reply_block(tx, &turn.conversation, &turn.id).await?;
    let body = TurnWaiting { on, id: card.clone(), reply_block };
    journal(tx, &turn.conversation, Some(&turn.id), JournalKind::TurnWaiting, body, Some(card.to_string())).await
}

/// Settle the call the turn waited on, then run the rest of its reply's calls (spec 01 section 3.10, rules 2 and 3).
pub(super) async fn resume<S: Wps>(turn: &Turn<S>) -> Result<Ran, SqlxError> {
    let pool = &turn.gate.app.pool;
    let waiting = sqlx::query(
        "SELECT id, model_call, position, tool FROM tool_calls WHERE turn = $1 AND state = 'waiting' \
         ORDER BY model_call DESC, position DESC LIMIT 1",
    )
    .bind(turn.id.as_str())
    .fetch_optional(pool)
    .await?;
    let Some(waiting) = waiting else {
        return Ok(Ran::Ended(Ending::failed("internal", "The agent resumed with no call waiting.".to_owned())));
    };
    let id = CallId::new(waiting.try_get::<String, _>("id")?);
    let (model_call, position): (i32, i32) = (waiting.try_get("model_call")?, waiting.try_get("position")?);
    let mut conn = pool.acquire().await?;
    let rows = transcript::rows(&mut conn, &turn.conversation).await?;
    drop(conn);
    let calls: Vec<ToolCall> = transcript::unanswered(&rows);
    let Some(call) = calls.get(position as usize) else {
        return Ok(Ran::Ended(Ending::failed("internal", "The waiting call is not in the reply.".to_owned())));
    };
    let planned = plan(turn, call);
    let settled = match planned.name.as_str() {
        ASK_USER => answered(turn, &id).await?,
        _ => decided(turn, &id, &planned).await?,
    };
    if let Some(ending) = settled {
        return Ok(Ran::Ended(ending));
    }
    run_from(turn, model_call, &calls, position as usize + 1).await
}

/// The question's answer as the `ask_user` result.
async fn answered<S: Wps>(turn: &Turn<S>, id: &CallId) -> Result<Option<Ending>, SqlxError> {
    let (items, answers, in_words): (Value, Option<Value>, bool) =
        sqlx::query_as("SELECT items, answers, in_words FROM questions WHERE call = $1")
            .bind(id.as_str())
            .fetch_one(&turn.gate.app.pool)
            .await?;
    let result = match in_words {
        true => json!({ "answered_in_words": true }),
        false => {
            let answers: Vec<ItemAnswer> = serde_json::from_value(answers.unwrap_or_default()).map_err(decode)?;
            let answered: Vec<Value> = (answers.iter())
                .map(|answer| {
                    let header = items[answer.item as usize]["header"].clone();
                    json!({ "header": header, "selected": answer.selected, "other": answer.other })
                })
                .collect();
            json!({ "answers": answered })
        }
    };
    let summary = match in_words {
        true => "Asked the person; they answered in words".to_owned(),
        false => "Asked the person; they answered".to_owned(),
    };
    finished(turn, id, CallState::Ok, Some(result), None, summary).await?;
    Ok(None)
}

/// The approval's decision as the asking call's result: an approved input executes once, as the approver, under the
/// approval's id, the act's own worker claiming and settling the approval's execution as it does for a person's press;
/// a denial is an error result naming who denied and why.
async fn decided<S: Wps>(turn: &Turn<S>, id: &CallId, planned: &Planned<'_, S>) -> Result<Option<Ending>, SqlxError> {
    let pool = &turn.gate.app.pool;
    let row = sqlx::query(
        "SELECT a.id, a.state, a.input, a.reason, a.decided_by, p.email, p.name FROM approvals a \
         LEFT JOIN people p ON p.subject = a.decided_by WHERE a.call = $1",
    )
    .bind(id.as_str())
    .fetch_one(pool)
    .await?;
    let approval: Ulid = row.try_get::<String, _>("id")?.parse().map_err(decode)?;
    let subject: Option<String> = row.try_get("decided_by")?;
    let name: String = row.try_get::<Option<String>, _>("name")?.or(subject.clone()).unwrap_or_default();
    let described = planned.tool.map_or_else(|| planned.name.clone(), |tool| tool.describe(&planned.input));
    if row.try_get::<String, _>("state")? != "approved" {
        let reason: Option<String> = row.try_get("reason")?;
        let denial = json!({ "denied": true, "by": name, "reason": reason });
        let because = reason.map(|reason| format!(": {reason}")).unwrap_or_default();
        let summary = format!("{described}: denied by {name}{because}");
        finished(turn, id, CallState::Denied, Some(denial), None, summary).await?;
        return Ok(None);
    }
    let approver = Identity {
        subject: subject.unwrap_or_default().parse().map_err(decode)?,
        email: row.try_get::<Option<String>, _>("email")?.unwrap_or_default(),
        name,
        developer: false,
    };
    let stored: Value = row.try_get("input")?;
    let approved =
        Planned { call: planned.call, tool: planned.tool, name: planned.name.clone(), input: stored, invalid: None };
    match execute(turn, &approved, id, &approver, Some(approval.clone())).await {
        Outcome::Cancelled => {
            let words = "The turn was stopped before this approval's act started; nothing was written.";
            unstarted(turn, &approval, Execution::Interrupted, words, None).await?;
            Ok(Some(Ending::stopped()))
        }
        outcome => {
            if let Outcome::Failed(problem) = &outcome {
                unstarted(turn, &approval, Execution::Failed, &problem.detail, Some(&json!(problem))).await?;
            }
            settle(turn, id, &approved, outcome).await?;
            Ok(None)
        }
    }
}

/// An approved act that ended before it took its approval's execution (spec 01 section 3.10's execution states, rule
/// 2): the execution runs and settles `failed` or `interrupted` in one transaction under the turn's locks, with its
/// `approval.execution` entries; one the act took is its worker's to settle, and stays as it is.
async fn unstarted<S: Wps>(
    turn: &Turn<S>,
    approval: &Ulid,
    execution: Execution,
    sentence: &str,
    outcome: Option<&Value>,
) -> Result<(), SqlxError> {
    let mut tx = turn.gate.app.pool.begin().await?;
    lock_turn(&mut tx, turn.id.as_str()).await?;
    let Some(Moved(running)) = executed(&mut tx, approval, Execution::Running, None, None).await? else {
        return Ok(());
    };
    let settled = executed(&mut tx, approval, execution, Some(sentence), outcome).await?;
    tx.commit().await?;
    turn.gate.app.hub.announce(running.into_iter().chain(settled.and_then(|Moved(entry)| entry)));
    Ok(())
}

/// Every call of the reply as its result, in the calls' order: what an `ok` row holds, else its problem or denial as
/// an error result.
async fn results<S: Wps>(turn: &Turn<S>, model_call: i32) -> Result<Vec<ToolResult>, SqlxError> {
    let rows: Vec<(Value, String, Option<Value>, Option<Value>)> = sqlx::query_as(
        "SELECT wire_call, state, result, problem FROM tool_calls WHERE turn = $1 AND model_call = $2 ORDER BY position",
    )
    .bind(turn.id.as_str())
    .bind(model_call)
    .fetch_all(&turn.gate.app.pool)
    .await?;
    rows.into_iter()
        .map(|(wire, state, result, problem)| {
            let call: ToolCall = serde_json::from_value(wire).map_err(decode)?;
            let ok = state == word(CallState::Ok);
            let value = if ok { result.unwrap_or(Value::Null) } else { problem.or(result).unwrap_or(Value::Null) };
            Ok(ToolResult { is_error: !ok, ..call.result(vec![ToolResultContent::Json { value }]) })
        })
        .collect()
}
