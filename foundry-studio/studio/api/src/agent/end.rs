//! Within `agent`, `end` is how a turn stops and ends (spec 01 sections 3.4 and 3.14): every ending (done, with the
//! budget it reached; failed, with its reason and a note in words; stopped; interrupted) committed with its journal
//! entries in one transaction, calls still executing marked as the ending says; Stop, which under the turn's locks
//! cancels the task running the turn (so the task, which takes the same locks to wait, never publishes a wait after it)
//! or ends a waiting turn at once, its open question or approval expired; the start's sweep, which marks
//! interrupted every turn a stopped process left running (an approved act that never started ending `interrupted`
//! with it); the wait sweep, which stops a turn whose question or
//! approval outlived `STUDIO_WAIT_TTL_HOURS`; the per-turn budgets; and the turn's calls as its details read them.

use std::sync::Arc;

use agent_loop::{ErrorClass, ModelBackend, ModelError, Tokens};
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, State};
use axum::http::{StatusCode, Uri};
use axum::{Extension, Json};
use serde_json::Value;
use sqlx::{Error as SqlxError, PgPool, Postgres, Row, Transaction};
use studio_protocol::{
    ApprovalExpired, BlockId, CallId, CallRead, CallState, Execution, JournalKind, Problem, ProblemType,
    QuestionExpired, StopAccepted, SystemNote, ToolFinished, TurnCalls, TurnFinished, TurnState, Ulid, Usage,
};
use wps_client::wps::Wps;

use super::turn::{Turn, state_word};
use super::{journal, lock_turn};
use crate::approvals::{Moved, executed};
use crate::db::{decode, read_word, word};
use crate::identity::Identity;
use crate::journal::Written;
use crate::problem::{ProblemAnswer, internal};
use crate::{AppState, GateState};

/// The most of a call's result the details disclosure reads, as JSON text (spec 01 section 3.6).
const RESULT_SHOWN: usize = 4 * 1024;

/// How a turn ends, or that it paused on a person.
pub(super) enum Ending {
    Done { cut: bool, budget: Option<&'static str> },
    Failed { reason: &'static str, note: String },
    Stopped { reason: Option<&'static str> },
    Paused,
}

impl Ending {
    pub fn done(cut: bool) -> Self {
        Self::Done { cut, budget: None }
    }

    pub fn failed(reason: &'static str, note: String) -> Self {
        Self::Failed { reason, note }
    }

    pub fn stopped() -> Self {
        Self::Stopped { reason: None }
    }

    /// A model request that failed past its retries (spec 01 sections 3.7 and 3.14): the reason section 3.7 names, or
    /// `agent-provider-failed`, the note naming the status and the provider's words.
    pub fn of_model(error: &ModelError, backend: Option<Arc<dyn ModelBackend>>) -> Self {
        let provider = backend.map_or_else(|| "the model".to_owned(), |backend| backend.provider().to_string());
        let status = error.status.map(|status| format!(" ({status})")).unwrap_or_default();
        let (reason, lead) = match error.class {
            ErrorClass::Unavailable => ("model-unavailable", "The model stayed unavailable after 3 retries"),
            ErrorClass::Rejected => ("model-request-rejected", "The model refused the request as written"),
            ErrorClass::Refused => ("model-refused", "The model refused to continue"),
            ErrorClass::Failed => ("agent-provider-failed", "The model's provider failed the request"),
        };
        if error.class == ErrorClass::Rejected {
            eprintln!("studio-api: {provider} rejected a request (a bug of ours){status}: {}", error.words);
        }
        Self::failed(reason, format!("{lead}{status}. {provider} said: {}", error.words))
    }
}

/// A per-turn budget reached (spec 01 section 3.1's three `STUDIO_TURN_MAX_*`).
pub(super) enum Budget {
    ModelCalls(u32),
    OutputTokens(Tokens),
    ActiveSeconds(u64),
}

impl Budget {
    pub fn word(&self) -> &'static str {
        match self {
            Self::ModelCalls(_) => "model-calls",
            Self::OutputTokens(_) => "output-tokens",
            Self::ActiveSeconds(_) => "active-seconds",
        }
    }

    /// What a call the budget stopped, or never let run, answers the model.
    pub fn stopped_call(&self, ran: bool) -> Problem {
        let what = match ran {
            true => "while this call ran; its effect is unknown",
            false => "before this call ran, so it was not run",
        };
        Problem::new(ProblemType::TurnBudget, format!("The turn reached its budget of {} {what}.", self.amount()))
    }

    fn amount(&self) -> String {
        match self {
            Self::ModelCalls(calls) => format!("{calls} model requests"),
            Self::OutputTokens(tokens) => format!("{} output tokens", tokens.0),
            Self::ActiveSeconds(seconds) => format!("{seconds} wall-clock seconds spent running"),
        }
    }

    /// The text the summary request carries after the turn's last results.
    pub fn notice(&self) -> String {
        format!(
            "Agent Studio: this turn reached its budget of {}. Do not call any tool. Tell the person in a few \
             sentences what you did and what remains to do.",
            self.amount()
        )
    }
}

/// What a budget check comes before: a model request, which every budget bounds, or a tool call, which the model-call
/// budget leaves alone, since it counts requests.
#[derive(PartialEq, Eq)]
pub(super) enum Next {
    Request,
    Call,
}

/// The budget the running turn has reached before `next`, if any: its requests, its output tokens, or its time running,
/// counted across its waits.
pub(super) async fn over_budget<S: Wps>(turn: &Turn<S>, next: Next) -> Result<Option<Budget>, SqlxError> {
    let settings = &turn.agent().settings;
    let (calls, output): (i32, i64) = sqlx::query_as("SELECT model_calls, output_tokens FROM turns WHERE id = $1")
        .bind(turn.id.as_str())
        .fetch_one(&turn.gate.app.pool)
        .await?;
    Ok(match () {
        _ if next == Next::Request && calls as u32 >= settings.max_model_calls.0 => {
            Some(Budget::ModelCalls(settings.max_model_calls.0))
        }
        _ if Tokens(output as u64) >= settings.max_output_tokens => {
            Some(Budget::OutputTokens(settings.max_output_tokens))
        }
        _ if turn.active_spent() >= settings.max_active => Some(Budget::ActiveSeconds(settings.max_active.as_secs())),
        _ => None,
    })
}

/// End the running turn as `ending` says; a turn already ended or waiting is left as it is.
pub(super) async fn finish<S: Wps>(turn: &mut Turn<S>, ending: Ending) -> Result<(), SqlxError> {
    if matches!(ending, Ending::Paused) {
        return Ok(());
    }
    let active = turn.stretch.elapsed().as_millis() as i64;
    let mut tx = turn.gate.app.pool.begin().await?;
    let written = close(&mut tx, &turn.id, &[TurnState::Running], ending, active).await?;
    tx.commit().await?;
    turn.gate.app.hub.announce(written);
    Ok(())
}

/// `finish` for a turn no task holds: its id alone.
pub(super) async fn ending<S: Wps>(gate: &GateState<S>, id: &Ulid, ending: Ending) -> Result<(), SqlxError> {
    let mut tx = gate.app.pool.begin().await?;
    let written = close(&mut tx, id, &[TurnState::Running], ending, 0).await?;
    tx.commit().await?;
    gate.app.hub.announce(written);
    Ok(())
}

/// Inside `tx`: end turn `id` as `ending` says when it stands in one of `from`. Its calls still running or waiting
/// end stopped (interrupted for an interruption), its open question or pending approval expires, then its note and
/// `turn.finished`; the entries, for the hub once `tx` commits.
async fn close(
    tx: &mut Transaction<'_, Postgres>,
    id: &Ulid,
    from: &[TurnState],
    ending: Ending,
    active_ms: i64,
) -> Result<Vec<Written>, SqlxError> {
    let Some(held) = lock_turn(tx, id.as_str()).await? else { return Ok(Vec::new()) };
    if !from.contains(&held.state) {
        return Ok(Vec::new());
    }
    let conversation = held.conversation;
    let (outcome, reason, note, budget, calls_end) = match ending {
        Ending::Done { cut, budget } => {
            let note = cut.then(|| "The agent's reply was cut short at its output ceiling.".to_owned());
            (TurnState::Done, None, note, budget, CallState::Stopped)
        }
        Ending::Failed { reason, note } => (TurnState::Failed, Some(reason), Some(note), None, CallState::Stopped),
        Ending::Stopped { reason } if reason == Some("interrupted") => {
            let note = "The agent was interrupted by a restart of the service.".to_owned();
            (TurnState::Interrupted, reason, Some(note), None, CallState::Interrupted)
        }
        Ending::Stopped { reason } => (TurnState::Stopped, reason, None, None, CallState::Stopped),
        Ending::Paused => return Ok(Vec::new()),
    };
    let mut written = Vec::new();
    let caused = |text: &str| Some(text.to_owned());
    let waiting: Option<(String, String)> = held.waiting_on.zip(held.waiting_id);
    match waiting.as_ref().map(|(on, id)| (on.as_str(), id)) {
        Some(("question", question)) => {
            sqlx::query("UPDATE questions SET state = 'expired' WHERE id = $1 AND state = 'open'")
                .bind(question)
                .execute(&mut **tx)
                .await?;
            let body = QuestionExpired { question: question.parse().map_err(decode)? };
            written.push(
                journal(tx, &conversation, Some(id), JournalKind::QuestionExpired, body, caused(question)).await?,
            );
        }
        Some((_, approval)) => {
            sqlx::query("UPDATE approvals SET state = 'expired' WHERE id = $1 AND state = 'pending'")
                .bind(approval)
                .execute(&mut **tx)
                .await?;
            let body = ApprovalExpired { approval: approval.parse().map_err(decode)? };
            written.push(
                journal(tx, &conversation, Some(id), JournalKind::ApprovalExpired, body, caused(approval)).await?,
            );
        }
        None => {}
    }
    let unstarted: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM approvals WHERE turn = $1 AND state = 'approved' AND execution = 'not_started' ORDER BY id",
    )
    .bind(id.as_str())
    .fetch_all(&mut **tx)
    .await?;
    for approval in unstarted {
        let approval: Ulid = approval.parse().map_err(decode)?;
        let words = "The turn ended before this approval's act started; nothing was written.";
        for (execution, sentence) in [(Execution::Running, None), (Execution::Interrupted, Some(words))] {
            if let Some(Moved(entry)) = executed(tx, &approval, execution, sentence, None).await? {
                written.extend(entry);
            }
        }
    }
    let open: Vec<(String, String)> = sqlx::query_as(
        "UPDATE tool_calls SET state = $2, ended_at = now() WHERE turn = $1 AND state IN ('running', 'waiting') \
         RETURNING id, summary",
    )
    .bind(id.as_str())
    .bind(word(calls_end))
    .fetch_all(&mut **tx)
    .await?;
    for (call, summary) in open {
        let words = match calls_end {
            CallState::Interrupted => format!("{summary}: interrupted; its effect is unknown"),
            _ => format!("{summary}: stopped; its effect is unknown"),
        };
        let body = ToolFinished {
            call: CallId::new(call.clone()),
            outcome: calls_end,
            summary: words,
            refs: Vec::new(),
            problem: None,
        };
        written.push(journal(tx, &conversation, Some(id), JournalKind::ToolFinished, body, Some(call)).await?);
    }
    if let Some(text) = note {
        written.push(journal(tx, &conversation, Some(id), JournalKind::SystemNote, SystemNote { text }, None).await?);
    }
    let spent = sqlx::query(
        "UPDATE turns SET state = $2, end_reason = $3, budget = $4, ended_at = now(), waiting_on = NULL, \
             waiting_id = NULL, waiting_since = NULL, lease_owner = NULL, lease_expires_at = NULL, \
             active_ms = active_ms + $5 \
         WHERE id = $1 RETURNING model_calls, input_tokens, output_tokens, cache_read_input_tokens",
    )
    .bind(id.as_str())
    .bind(state_word(outcome))
    .bind(reason)
    .bind(budget)
    .bind(active_ms)
    .fetch_one(&mut **tx)
    .await?;
    let usage = Usage {
        model_calls: spent.try_get::<i32, _>("model_calls")? as u64,
        input_tokens: spent.try_get::<i64, _>("input_tokens")? as u64,
        output_tokens: spent.try_get::<i64, _>("output_tokens")? as u64,
        cache_read_input_tokens: spent.try_get::<i64, _>("cache_read_input_tokens")? as u64,
    };
    let reply_block = reply_block(tx, &conversation, id).await?;
    let finished = TurnFinished {
        outcome,
        end_reason: reason.map(str::to_owned),
        reply_block,
        budget: budget.map(str::to_owned),
        usage,
    };
    written.push(journal(tx, &conversation, Some(id), JournalKind::TurnFinished, finished, None).await?);
    Ok(written)
}

/// The turn's last `text` block, the reply spec 02 speaks (ruling D70).
pub(super) async fn reply_block(
    tx: &mut Transaction<'_, Postgres>,
    conversation: &Ulid,
    turn: &Ulid,
) -> Result<Option<BlockId>, SqlxError> {
    let block: Option<String> = sqlx::query_scalar(
        "SELECT body->>'block' FROM journal WHERE conversation = $1 AND turn = $2 AND kind = 'text.done' \
         AND body->>'kind' = 'text' ORDER BY seq DESC LIMIT 1",
    )
    .bind(conversation.as_str())
    .bind(turn.as_str())
    .fetch_optional(&mut **tx)
    .await?;
    Ok(block.map(BlockId::new))
}

/// Before the service serves (spec 01 section 3.4, "Restart"): every turn left running by a stopped process ends
/// interrupted, its executing calls with it. Waiting turns keep waiting: their state is in Postgres.
pub async fn sweep(app: &AppState) -> Result<(), SqlxError> {
    let running: Vec<String> =
        sqlx::query_scalar("SELECT id FROM turns WHERE state = 'running'").fetch_all(&app.pool).await?;
    for id in running {
        let id: Ulid = id.parse().map_err(decode)?;
        let mut tx = app.pool.begin().await?;
        let written =
            close(&mut tx, &id, &[TurnState::Running], Ending::Stopped { reason: Some("interrupted") }, 0).await?;
        tx.commit().await?;
        app.hub.announce(written);
    }
    Ok(())
}

/// Stop every waiting turn whose wait began longer ago than `STUDIO_WAIT_TTL_HOURS` (wall clock): its question or
/// approval expires, and it ends `stopped` with `wait-expired`.
pub async fn expire_waits(app: &AppState) -> Result<(), SqlxError> {
    let ttl = app.agent.settings.wait_ttl.as_secs() as i64;
    let expired: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM turns WHERE state = 'waiting' AND waiting_since < now() - make_interval(secs => $1)",
    )
    .bind(ttl as f64)
    .fetch_all(&app.pool)
    .await?;
    for id in expired {
        let id: Ulid = id.parse().map_err(decode)?;
        let mut tx = app.pool.begin().await?;
        let ending = Ending::Stopped { reason: Some("wait-expired") };
        let written = close(&mut tx, &id, &[TurnState::Waiting], ending, 0).await?;
        tx.commit().await?;
        app.hub.announce(written);
    }
    Ok(())
}

fn no_turn(instance: &str) -> ProblemAnswer {
    Problem::new(ProblemType::NotFound, "No turn has that id.").at(instance).into()
}

fn turn_in(path: Result<Path<String>, PathRejection>, instance: &str) -> Result<Ulid, ProblemAnswer> {
    let Ok(Path(id)) = path else { return Err(no_turn(instance)) };
    id.parse().map_err(|_| no_turn(instance))
}

/// `POST /api/turns/{t}/stop`: a running turn's task is cancelled and ends it within a second; a waiting turn ends
/// here, its question or approval expired; an ended turn answers `200`. Anyone signed in may stop a site's turn, and
/// only its maker a new-site thread's (spec 01 section 3.2).
pub async fn stop<S: Wps>(
    State(gate): State<GateState<S>>,
    Extension(identity): Extension<Identity>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
) -> Result<(StatusCode, Json<StopAccepted>), ProblemAnswer> {
    let instance = uri.path();
    let id = turn_in(path, instance)?;
    let fault = |fault| internal("stop the turn", fault, instance);
    let mut tx = gate.app.pool.begin().await.map_err(fault)?;
    let Some(held) = lock_turn(&mut tx, id.as_str()).await.map_err(fault)? else { return Err(no_turn(instance)) };
    let owner = held.created_by.unwrap_or_default();
    if held.site.is_none() && owner != identity.subject.as_str() {
        let detail = format!("This new site's conversation is {owner}'s; you can read it.");
        return Err(Problem::new(ProblemType::NotConversationOwner, detail).at(instance).into());
    }
    let accepted = (StatusCode::ACCEPTED, Json(StopAccepted { turn: id.clone() }));
    match held.state {
        TurnState::Running if gate.app.agent.cancel(&id) => {
            tx.commit().await.map_err(fault)?;
            Ok(accepted)
        }
        TurnState::Running | TurnState::Waiting => {
            let from = [TurnState::Running, TurnState::Waiting];
            let written = close(&mut tx, &id, &from, Ending::stopped(), 0).await.map_err(fault)?;
            tx.commit().await.map_err(fault)?;
            gate.app.hub.announce(written);
            Ok(accepted)
        }
        _ => Ok((StatusCode::OK, Json(StopAccepted { turn: id }))),
    }
}

/// `GET /api/turns/{t}/calls`: the turn's calls in the order they started, each result trimmed to 4 KiB of JSON text.
pub async fn calls(
    State(app): State<AppState>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<TurnCalls>, ProblemAnswer> {
    let instance = uri.path();
    let id = turn_in(path, instance)?;
    let fault = |fault| internal("read the turn's calls", fault, instance);
    if !turn_exists(&app.pool, &id).await.map_err(fault)? {
        return Err(no_turn(instance));
    }
    let rows = sqlx::query(
        "SELECT id, tool, input, state, result, problem, summary FROM tool_calls WHERE turn = $1 \
         ORDER BY model_call, position",
    )
    .bind(id.as_str())
    .fetch_all(&app.pool)
    .await
    .map_err(fault)?;
    let mut calls = Vec::new();
    for row in rows {
        let read = || -> Result<CallRead, SqlxError> {
            let result: Option<Value> = row.try_get::<Option<Value>, _>("result")?.or(row.try_get("problem")?);
            Ok(CallRead {
                call: CallId::new(row.try_get::<String, _>("id")?),
                tool: row.try_get("tool")?,
                input: row.try_get("input")?,
                result: result.map(trimmed),
                outcome: read_word(&row.try_get::<String, _>("state")?)?,
                summary: row.try_get("summary")?,
            })
        };
        calls.push(read().map_err(fault)?);
    }
    Ok(Json(TurnCalls { calls }))
}

async fn turn_exists(pool: &PgPool, id: &Ulid) -> Result<bool, SqlxError> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM turns WHERE id = $1)").bind(id.as_str()).fetch_one(pool).await
}

/// A result as the details show it: whole up to 4 KiB of JSON text, else its first 4 KiB as text, marked cut.
fn trimmed(result: Value) -> Value {
    let text = result.to_string();
    if text.len() <= RESULT_SHOWN {
        return result;
    }
    let cut: String = text.char_indices().take_while(|(at, _)| *at < RESULT_SHOWN).map(|(_, c)| c).collect();
    Value::String(format!("{cut}… (cut at 4 KiB)"))
}
