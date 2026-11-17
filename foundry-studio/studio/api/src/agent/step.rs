//! Within `agent`, a `step` is one request to the model and its reply (spec 01 section 3.7): a `model_calls` row per
//! request sent, retries past an unavailable model on the agent's schedule, a text block's fragments relayed to the
//! hub as `text.delta` and each finished text block journaled as `text.done` with the references its links resolve
//! to, so a block written before a call is journaled before its row; the reply's usage and the provider's document
//! kept on its row; and the assistant message appended to the transcript only once whole, the model's reasoning in it
//! and in no journal entry (D117). Before each attempt the turn's budgets are checked, and the wall-clock time it has
//! left to run bounds the stream and the waits between retries (section 3.4); a summary request, the budgets' last, is
//! bounded by neither. A compaction's request (section 3.4) is a step whose words reach no tab: its summary, when the
//! reply stopped usably with text, becomes the transcript's next user row, and the log keeps every entry with one note.

use agent_loop::prompt::SYSTEM_PROMPT;
use agent_loop::rig::{Message, ToolCall};
use agent_loop::{Block, ErrorClass, ModelError, ModelRequest, StepEnd, StepEvent, StepStream, Stop, Tokens};
use futures::StreamExt;
use rustc_hash::FxHashMap;
use serde_json::{Value, json};
use sqlx::Error as SqlxError;
use studio_protocol::{
    BlockId, JournalKind, Key, ObjectKind, Ref, ResolvedRef, SystemNote, TextDelta, TextDone, TextKind, TextOffset,
};
use tokio::select;
use tokio::time::sleep;
use wps_client::wps::Wps;

use super::end::{Budget, Ending, Next, over_budget};
use super::journal;
use super::transcript::{self, closing, user_message};
use super::turn::Turn;
use crate::map::site_at;

const SUMMARISE: &str = "Agent Studio: this conversation has grown long, so earlier turns will be replaced by your \
                         summary. Do not call any tool. Summarise what was decided, which sites, templates, drafts, \
                         runs and references matter, and what remains to do, for yourself to continue from.";
const SUMMARISED: &str = "Earlier turns were summarised for the agent; the log keeps them in full.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Purpose {
    Step,
    Summary,
    Compaction,
}

impl Purpose {
    fn word(self) -> &'static str {
        match self {
            Self::Step => "step",
            Self::Summary => "summary",
            Self::Compaction => "compaction",
        }
    }
}

pub(super) enum Failure {
    Stopped,
    Model(ModelError),
    /// A budget was reached before the request or while it streamed.
    Budget(Budget),
}

/// A reply, whole, and the request it answered.
pub(super) struct Reply {
    pub stop: Stop,
    pub calls: Vec<ToolCall>,
    pub message: Option<Message>,
    pub model_call: i32,
}

/// Send `request` for `turn`, asked again while the model is unavailable, until a reply ends, the turn is stopped or
/// the retries run out.
pub(super) async fn request<S: Wps>(
    turn: &Turn<S>,
    request: ModelRequest,
    purpose: Purpose,
) -> Result<Result<Reply, Failure>, SqlxError> {
    let Some(backend) = turn.backend() else {
        let words = "the agent's model is not configured".to_owned();
        let error = ModelError { class: ErrorClass::Failed, status: None, words, retry_after: None };
        return Ok(Err(Failure::Model(error)));
    };
    let bounded = purpose != Purpose::Summary;
    let mut attempt = 0;
    loop {
        if bounded && let Some(budget) = over_budget(turn, Next::Request).await? {
            return Ok(Err(Failure::Budget(budget)));
        }
        let n = started(turn, purpose, backend.provider().as_str(), backend.model()).await?;
        match relay(turn, n, backend.step(request.clone()), purpose).await? {
            Ok(end) => {
                ended(turn, n, &end).await?;
                return Ok(Ok(Reply { stop: end.stop, calls: end.calls, message: end.message, model_call: n }));
            }
            Err(Failure::Stopped) => {
                failed(turn, n, &json!({ "stopped": true })).await?;
                return Ok(Err(Failure::Stopped));
            }
            Err(Failure::Budget(budget)) => {
                failed(turn, n, &json!({ "budget": budget.word() })).await?;
                return Ok(Err(Failure::Budget(budget)));
            }
            Err(Failure::Model(error)) => {
                let recorded =
                    json!({ "class": format!("{:?}", error.class), "status": error.status, "words": error.words });
                failed(turn, n, &recorded).await?;
                let Some(wait) = turn.agent().retry.wait_after(attempt, &error) else {
                    return Ok(Err(Failure::Model(error)));
                };
                let wait = if bounded { wait.min(turn.active_left()) } else { wait };
                select! {
                    _ = turn.cancel.cancelled() => return Ok(Err(Failure::Stopped)),
                    _ = sleep(wait) => {}
                }
                attempt += 1;
            }
        }
    }
}

/// Number the request and count it against the turn's budget.
async fn started<S: Wps>(turn: &Turn<S>, purpose: Purpose, provider: &str, model: &str) -> Result<i32, SqlxError> {
    let mut tx = turn.gate.app.pool.begin().await?;
    let n: i32 =
        sqlx::query_scalar("UPDATE turns SET model_calls = model_calls + 1 WHERE id = $1 RETURNING model_calls")
            .bind(turn.id.as_str())
            .fetch_one(&mut *tx)
            .await?;
    sqlx::query("INSERT INTO model_calls (turn, n, provider, model, purpose) VALUES ($1, $2, $3, $4, $5)")
        .bind(turn.id.as_str())
        .bind(n)
        .bind(provider)
        .bind(model)
        .bind(purpose.word())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(n)
}

async fn ended<S: Wps>(turn: &Turn<S>, n: i32, end: &StepEnd) -> Result<(), SqlxError> {
    let mut tx = turn.gate.app.pool.begin().await?;
    let usage = end.usage;
    sqlx::query(
        "UPDATE model_calls SET ended_at = now(), stop_reason = $3, input_tokens = $4, output_tokens = $5, \
             cache_read_input_tokens = $6, provider_request_id = $7, raw = $8 WHERE turn = $1 AND n = $2",
    )
    .bind(turn.id.as_str())
    .bind(n)
    .bind(format!("{:?}", end.stop))
    .bind(usage.input_tokens.0 as i64)
    .bind(usage.output_tokens.0 as i64)
    .bind(usage.cache_read_input_tokens.0 as i64)
    .bind(&end.request_id)
    .bind(&end.raw)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE turns SET input_tokens = input_tokens + $2, output_tokens = output_tokens + $3, \
             cache_read_input_tokens = cache_read_input_tokens + $4 WHERE id = $1",
    )
    .bind(turn.id.as_str())
    .bind(usage.input_tokens.0 as i64)
    .bind(usage.output_tokens.0 as i64)
    .bind(usage.cache_read_input_tokens.0 as i64)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

async fn failed<S: Wps>(turn: &Turn<S>, n: i32, error: &Value) -> Result<(), SqlxError> {
    sqlx::query("UPDATE model_calls SET ended_at = now(), error = $3 WHERE turn = $1 AND n = $2")
        .bind(turn.id.as_str())
        .bind(n)
        .bind(error)
        .execute(&turn.gate.app.pool)
        .await?;
    Ok(())
}

/// One reply's events: fragments to the hub and finished blocks to the journal, a compaction's to neither.
async fn relay<S: Wps>(
    turn: &Turn<S>,
    n: i32,
    mut events: StepStream,
    purpose: Purpose,
) -> Result<Result<StepEnd, Failure>, SqlxError> {
    let (app, heard) = (&turn.gate.app, purpose != Purpose::Compaction);
    let mut lengths: FxHashMap<u32, u64> = FxHashMap::default();
    let bounded = purpose != Purpose::Summary;
    let deadline = sleep(turn.active_left());
    tokio::pin!(deadline);
    loop {
        let event = select! {
            biased;
            _ = turn.cancel.cancelled() => return Ok(Err(Failure::Stopped)),
            _ = &mut deadline, if bounded => {
                return Ok(Err(Failure::Budget(Budget::ActiveSeconds(turn.agent().settings.max_active.as_secs()))));
            }
            event = events.next() => event,
        };
        let block = |part: u32| BlockId::new(format!("{}.{n}.{part}", turn.n.get()));
        match event {
            None => {
                let words = "the reply ended before the provider ended it".to_owned();
                let error = ModelError { class: ErrorClass::Unavailable, status: None, words, retry_after: None };
                return Ok(Err(Failure::Model(error)));
            }
            Some(Err(error)) => return Ok(Err(Failure::Model(error))),
            Some(Ok(StepEvent::Ended(end))) => return Ok(Ok(*end)),
            Some(Ok(_)) if !heard => {}
            Some(Ok(StepEvent::Delta { part, text })) => {
                let offset = lengths.entry(part).or_default();
                let delta = TextDelta { block: block(part), offset: TextOffset::new(*offset), delta: text.clone() };
                *offset += text.encode_utf16().count() as u64;
                if let Err(refused) = app.hub.text_delta(&turn.conversation, delta) {
                    eprintln!("studio-api: turn {}'s text delta was refused by the hub: {refused}", turn.id);
                }
            }
            Some(Ok(StepEvent::Done { part, block: done })) => {
                // The chat never shows the model's reasoning (D117); the next request reads it, signatures and all,
                // from the message `appended` keeps whole.
                let Block::Text(text) = done else { continue };
                let refs = Some(resolved(turn, &text).await);
                let body = TextDone { block: block(part), kind: TextKind::Text, text, refs };
                let mut tx = app.pool.begin().await?;
                let written =
                    journal(&mut tx, &turn.conversation, Some(&turn.id), JournalKind::TextDone, body, None).await?;
                tx.commit().await?;
                app.hub.announce([written]);
            }
        }
    }
}

/// The text of a summary's reply when it stopped usably, at its end or its output ceiling, with words in it; only such
/// a summary replaces earlier turns or is kept after a budget (spec 01 section 3.4).
pub(super) fn usable(reply: &Reply) -> Option<String> {
    let text = match &reply.message {
        Some(Message::Assistant(said)) => said.text(),
        _ => String::new(),
    };
    (matches!(reply.stop, Stop::EndTurn | Stop::MaxTokens) && !text.trim().is_empty()).then_some(text)
}

/// Append the reply's message to the transcript, whole, and answer the request's number.
pub(super) async fn appended<S: Wps>(turn: &Turn<S>, reply: &Reply) -> Result<i32, SqlxError> {
    if let Some(message) = &reply.message {
        let mut tx = turn.gate.app.pool.begin().await?;
        transcript::append(&mut tx, &turn.conversation, &turn.id, message, None).await?;
        tx.commit().await?;
    }
    Ok(reply.model_call)
}

/// Before a turn's first request (spec 01 section 3.4, "Compaction"): when the last request's input and the new
/// message would pass `STUDIO_COMPACT_AT_INPUT_TOKENS`, one request with tools off summarises the transcript at this
/// turn boundary, and the summary replaces every row before it; `Some` ending when the request failed.
pub(super) async fn compact_if_needed<S: Wps>(turn: &Turn<S>) -> Result<Option<Ending>, SqlxError> {
    let pool = &turn.gate.app.pool;
    let last: Option<i64> = sqlx::query_scalar(
        "SELECT m.input_tokens FROM model_calls m JOIN turns t ON t.id = m.turn \
         WHERE t.conversation = $1 AND m.input_tokens IS NOT NULL AND m.purpose <> 'compaction' \
         ORDER BY m.started_at DESC, m.n DESC LIMIT 1",
    )
    .bind(turn.conversation.as_str())
    .fetch_optional(pool)
    .await?;
    let words: i64 = sqlx::query_scalar(
        "SELECT COALESCE(LENGTH(j.body->>'text'), 0)::BIGINT FROM journal j JOIN turns t \
         ON t.conversation = j.conversation AND t.message_seq = j.seq WHERE t.id = $1",
    )
    .bind(turn.id.as_str())
    .fetch_one(pool)
    .await?;
    let Some(last) = last else { return Ok(None) };
    if Tokens((last + words / 4) as u64) < turn.agent().settings.compact_at {
        return Ok(None);
    }
    let mut conn = pool.acquire().await?;
    let rows = transcript::rows(&mut conn, &turn.conversation).await?;
    drop(conn);
    let Some(through) = rows.last().map(|row| row.n) else { return Ok(None) };
    let mut history = transcript::history(&rows);
    let closings = transcript::unanswered(&rows).iter().map(closing).collect();
    history.push(user_message(closings, vec![SUMMARISE.to_owned()]));
    let mut summary = ModelRequest {
        system: SYSTEM_PROMPT.to_owned(),
        history,
        tools: turn.registry.definitions(),
        tools_off: true,
        max_tokens: turn.ceiling,
    };
    summary.max_tokens = summary.max_tokens.max(Tokens(1));
    let reply = match request(turn, summary, Purpose::Compaction).await? {
        Ok(reply) => reply,
        Err(Failure::Stopped) => return Ok(Some(Ending::stopped())),
        Err(Failure::Model(error)) => return Ok(Some(Ending::of_model(&error, turn.backend()))),
        Err(Failure::Budget(budget)) => return Ok(Some(Ending::Done { cut: false, budget: Some(budget.word()) })),
    };
    let Some(text) = usable(&reply) else {
        let (reason, why) = match &reply.stop {
            Stop::Refused(_) => ("model-refused", "the model refused to write it".to_owned()),
            Stop::Other(reason) => ("agent-provider-failed", format!("the provider ended the reply ({reason})")),
            _ => ("agent-provider-failed", "the model's reply held no text".to_owned()),
        };
        let words = format!("The summary of earlier turns could not be used: {why}. Nothing was replaced.");
        return Ok(Some(Ending::failed(reason, words)));
    };
    let summarised = Message::user(format!("Summary of the conversation before this turn, written by you:\n{text}"));
    let mut tx = pool.begin().await?;
    let n = transcript::append(&mut tx, &turn.conversation, &turn.id, &summarised, Some(through)).await?;
    sqlx::query("UPDATE conversations SET thinking_floor = $2 WHERE id = $1")
        .bind(turn.conversation.as_str())
        .bind(n)
        .execute(&mut *tx)
        .await?;
    let note = SystemNote { text: SUMMARISED.to_owned() };
    let written = journal(&mut tx, &turn.conversation, None, JournalKind::SystemNote, note, None).await?;
    tx.commit().await?;
    turn.gate.app.hub.announce([written]);
    Ok(None)
}

/// Each `ref:` link of a text block that names an entity or an object, resolved at the turn's site (spec 01 section
/// 3.5's `refs`): a template draft, a scenario or a run by its address; an entity by the site's map at its head, its
/// label the map's; a link that resolves to nothing keeps `route: null`, which the chip says. A site's or a template's
/// location names no entity (D60), so it is no chip: the chat links it to its panel route, and it is not listed.
async fn resolved<S: Wps>(turn: &Turn<S>, text: &str) -> Vec<ResolvedRef> {
    let mut refs: Vec<ResolvedRef> = Vec::new();
    let mut map = None;
    for (words, target) in links(text) {
        let Ok(reference) = target.parse::<Ref>() else { continue };
        let key = reference.key();
        if refs.iter().any(|known| known.key == key.to_string()) {
            continue;
        }
        let (kind, route, label) = match &key {
            Key::Scope(_) => continue,
            Key::Object(object) => {
                let (kind, route) = match object.kind() {
                    ObjectKind::Run => (Some("run"), Some(format!("/runs/{}", object.id()))),
                    ObjectKind::Scenario => (Some("scenario"), Some(format!("/scenarios/{}", object.id()))),
                    ObjectKind::TemplateDraft => {
                        (Some("template draft"), Some(format!("/template-drafts/{}", object.id())))
                    }
                    _ => (None, None),
                };
                (kind, route, words.clone())
            }
            Key::Entity(id) => {
                let Some(site) = &turn.site else {
                    refs.push(ResolvedRef { key: key.to_string(), kind: None, label: words, route: None });
                    continue;
                };
                if map.is_none() {
                    map = site_at(&turn.gate, &format!("{site}/map@main"), None, "/agent").await.ok();
                }
                let entry = map
                    .as_ref()
                    .and_then(|at| at.loaded.as_ref().as_ref().ok())
                    .and_then(|loaded| loaded.entities.get(id));
                match entry {
                    Some(entry) => {
                        let route = format!("/w/{site}/map@main?select={id}");
                        (Some("entity"), Some(route), entry.label.clone())
                    }
                    None => (None, None, words.clone()),
                }
            }
            Key::Field(_) => (None, None, words.clone()),
        };
        refs.push(ResolvedRef { key: key.to_string(), kind: kind.map(str::to_owned), label, route });
    }
    refs
}

/// Each `[words](ref:target)` of a text, in order.
fn links(text: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("](ref:") {
        let words = rest[..at].rfind('[').map(|open| rest[open + 1..at].to_owned()).unwrap_or_default();
        let after = &rest[at + "](ref:".len()..];
        let Some(close) = after.find(')') else { break };
        found.push((words, after[..close].to_owned()));
        rest = &after[close..];
    }
    found
}
