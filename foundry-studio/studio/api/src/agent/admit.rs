//! Within `agent`, `admit` is a person's message to a conversation (spec 01 sections 3.4, 3.6 and 3.10): with no turn
//! open it starts one, the turn row, `message.user` and `turn.started` committed together and the turn's task spawned;
//! while a turn waits on a question it answers it in words, and while it waits on an approval it denies it with the
//! message as the reason (rule 3), the turn resuming either way, the message's words kept on the turn's row until they
//! follow the results of their reply's calls; while one runs it is refused `turn-running`. Before
//! that, the conversation must be live and, for a new-site thread, the sender's; the model must be one this process
//! can call, and the one the conversation began with; the conversation must still fit the agent after its last
//! attempted compaction; and the process must have a permit for the turn's driver,
//! fresh or resumed, taken before anything commits.

use agent_loop::Tokens;
use axum::body::Bytes;
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, State};
use axum::http::{StatusCode, Uri};
use axum::{Extension, Json};
use serde_json::json;
use sqlx::{Error as SqlxError, Postgres, Row, Transaction};
use studio_protocol::{
    ApprovalDecided, Decision, JournalKind, MessageAccepted, Person, PostMessage, Problem, ProblemType,
    QuestionAnswered, ResumedBy, TurnNumber, TurnResumed, TurnStarted, Ulid, UserMessage,
};
use wps_client::wps::Wps;

use super::transcript::page_line;
use super::turn::{Start, drive};
use super::{Agent, Backend, busy, journal};
use crate::GateState;
use crate::conversations::conversation_in;
use crate::db::{decode, mint_ulid, seq_of};
use crate::identity::Identity;
use crate::journal::Written;
use crate::problem::{ProblemAnswer, internal, json_body, validation_failed};

const ONE_OPEN_TURN: &str = "turns_one_open_per_conversation";

fn refuse(type_: ProblemType, detail: impl Into<String>, instance: &str) -> ProblemAnswer {
    Problem::new(type_, detail).at(instance).into()
}

/// `POST /api/conversations/{c}/messages`.
pub async fn message<S: Wps>(
    State(gate): State<GateState<S>>,
    Extension(identity): Extension<Identity>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
    body: Bytes,
) -> Result<(StatusCode, Json<MessageAccepted>), ProblemAnswer> {
    let instance = uri.path();
    let conversation = conversation_in(path, instance)?;
    let post: PostMessage = json_body(&body, instance)?;
    if post.text.trim().is_empty() {
        return Err(validation_failed("`text` holds the message, which is empty", instance));
    }
    let agent = &gate.app.agent;
    match &agent.backend {
        Backend::Ready(_) => {}
        Backend::NotBuilt => {
            let detail =
                "Claude on Vertex AI is task 52's and not built yet; locally, set STUDIO_MODEL_TRANSPORT=gemini.";
            return Err(refuse(ProblemType::NotBuilt, detail, instance));
        }
        Backend::Unavailable(why) => {
            return Err(refuse(ProblemType::ModelUnavailable, format!("The agent cannot answer: {why}."), instance));
        }
    }
    let fault = |fault| internal("take the message", fault, instance);
    let mut tx = gate.app.pool.begin().await.map_err(fault)?;
    let row =
        sqlx::query("SELECT state, site, created_by, provider, last_seq FROM conversations WHERE id = $1 FOR UPDATE")
            .bind(conversation.as_str())
            .fetch_optional(&mut *tx)
            .await
            .map_err(fault)?;
    let Some(row) = row else {
        return Err(refuse(ProblemType::NotFound, format!("There is no conversation {conversation}."), instance));
    };
    let text = |column: &str| row.try_get::<Option<String>, _>(column).map_err(fault);
    if text("state")?.as_deref() != Some("live") {
        let detail = "This conversation reads as history; send your message in the site's live conversation.";
        return Err(refuse(ProblemType::ConversationClosed, detail, instance));
    }
    let owner = text("created_by")?.unwrap_or_default();
    if text("site")?.is_none() && owner != identity.subject.as_str() {
        return Err(refuse(
            ProblemType::NotConversationOwner,
            format!("This new site's conversation is {owner}'s; you can read it."),
            instance,
        ));
    }
    same_provider(text("provider")?.as_deref(), agent, instance)?;
    let ours = agent.provider_word();
    let next = seq_of(row.try_get::<i64, _>("last_seq").map_err(fault)? + 1).map_err(fault)?;
    let open: Option<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT id, state, waiting_on, waiting_id FROM turns WHERE conversation = $1 AND state IN ('running', 'waiting') \
         FOR UPDATE",
    )
    .bind(conversation.as_str())
    .fetch_optional(&mut *tx)
    .await
    .map_err(fault)?;
    let author = Person { subject: identity.subject.clone(), name: identity.name.clone() };
    let attended = post.unattended != Some(true) && !agent.settings.service_subjects.contains(&identity.subject);
    let browser_tab = attended.then(|| post.browser_tab.clone());
    let user = UserMessage {
        author: author.clone(),
        text: post.text.clone(),
        context: post.context.clone(),
        source: post.source,
        browser_tab: browser_tab.clone(),
        client_message_id: Some(post.client_message_id.clone()),
        act: post.act.clone(),
    };
    let (mut answered, mut denied) = (None, None);
    let (turn, written, start, permit) = match open {
        Some((_, state, _, _)) if state == "running" => {
            return Err(refuse(ProblemType::TurnRunning, "The agent is working. Stop it or wait.", instance));
        }
        Some((id, _, on, card)) => {
            let turn: Ulid = id.parse().map_err(|why| fault(decode(why)))?;
            let permit = agent.permit(&turn).ok_or_else(|| busy(instance))?;
            let card = card.unwrap_or_default();
            let caused = Some(card.clone());
            let mut written = vec![
                journal(&mut tx, &conversation, Some(&turn), JournalKind::MessageUser, &user, None)
                    .await
                    .map_err(fault)?,
            ];
            if on.as_deref() == Some("question") {
                written.push(answer_in_words(&mut tx, &conversation, &turn, &card, &author).await.map_err(fault)?);
                answered = Some(card.parse().map_err(|why| fault(decode(why)))?);
            } else {
                written.push(
                    deny_by_message(&mut tx, &conversation, &turn, &card, &author, &post.text).await.map_err(fault)?,
                );
                denied = Some(card.parse().map_err(|why| fault(decode(why)))?);
            }
            let mut words: Vec<String> = post.context.iter().map(page_line).collect();
            words.push(post.text.clone());
            sqlx::query(
                "UPDATE turns SET state = 'running', waiting_on = NULL, waiting_id = NULL, waiting_since = NULL, \
                     lease_owner = $2, lease_expires_at = now() + interval '30 seconds', \
                     pending_words = pending_words || $3 WHERE id = $1",
            )
            .bind(turn.as_str())
            .bind(agent.instance.as_str())
            .bind(json!(words))
            .execute(&mut *tx)
            .await
            .map_err(fault)?;
            let body = TurnResumed { by: ResumedBy::Message };
            written.push(
                journal(&mut tx, &conversation, Some(&turn), JournalKind::TurnResumed, body, caused)
                    .await
                    .map_err(fault)?,
            );
            (turn, written, Start::Resumed, permit)
        }
        None => {
            let turn = mint_ulid();
            let permit = agent.permit(&turn).ok_or_else(|| busy(instance))?;
            if too_long(&mut tx, &conversation, agent.settings.conversation_max).await.map_err(fault)? {
                let detail =
                    "This conversation is too long for the agent. Dismiss it to start a new one for this site.";
                return Err(refuse(ProblemType::ConversationTooLong, detail, instance));
            }
            let n: i32 = sqlx::query_scalar(
                "INSERT INTO turns (id, conversation, n, started_by, attended, browser_tab, state, message_seq, \
                     lease_owner, lease_expires_at) \
                 SELECT $1, $2, COALESCE(MAX(n), 0) + 1, $3, $4, $5, 'running', $6, $7, now() + interval '30 seconds' \
                 FROM turns WHERE conversation = $2 RETURNING n",
            )
            .bind(turn.as_str())
            .bind(conversation.as_str())
            .bind(identity.subject.as_str())
            .bind(attended)
            .bind(&browser_tab)
            .bind(next.get() as i64)
            .bind(agent.instance.as_str())
            .fetch_one(&mut *tx)
            .await
            .map_err(|fault| match fault.as_database_error().and_then(|fault| fault.constraint()) {
                Some(ONE_OPEN_TURN) => {
                    refuse(ProblemType::TurnRunning, "The agent is working. Stop it or wait.", instance)
                }
                _ => internal("take the message", fault, instance),
            })?;
            sqlx::query("UPDATE conversations SET provider = COALESCE(provider, $2) WHERE id = $1")
                .bind(conversation.as_str())
                .bind(ours)
                .execute(&mut *tx)
                .await
                .map_err(fault)?;
            let message = journal(&mut tx, &conversation, Some(&turn), JournalKind::MessageUser, &user, None)
                .await
                .map_err(fault)?;
            let started =
                TurnStarted { n: TurnNumber::new(n as u32), author, message_seq: next, attended, browser_tab };
            let begun = journal(&mut tx, &conversation, Some(&turn), JournalKind::TurnStarted, started, None)
                .await
                .map_err(fault)?;
            (turn, vec![message, begun], Start::Fresh, permit)
        }
    };
    tx.commit().await.map_err(fault)?;
    gate.app.hub.announce(written);
    tokio::spawn(drive(gate.clone(), turn.clone(), start, permit));
    let accepted = MessageAccepted { turn, seq: next, client_message_id: post.client_message_id, answered, denied };
    Ok((StatusCode::ACCEPTED, Json(accepted)))
}

/// A conversation keeps the provider it began with (brief I1.03 section 2): a message, an answer or a decision that
/// would start or resume one of its turns on another provider is refused `provider-changed`, and nothing changes.
pub(super) fn same_provider(began: Option<&str>, agent: &Agent, instance: &str) -> Result<(), ProblemAnswer> {
    let ours = agent.provider_word();
    match began {
        Some(began) if began != ours => Err(refuse(
            ProblemType::ProviderChanged,
            format!(
                "This conversation began with {began}, and the agent now runs on {ours}, so none of its turns can start \
                 or go on; Dismiss it to start a new one."
            ),
            instance,
        )),
        _ => Ok(()),
    }
}

/// Whether the conversation is still past `STUDIO_CONVERSATION_MAX_INPUT_TOKENS` after an attempted compaction (spec 01
/// section 3.4): its last request with a known input followed a compaction directly and was past the most. One past
/// it with no compaction since takes the turn, whose start compacts first.
async fn too_long(tx: &mut Transaction<'_, Postgres>, conversation: &Ulid, most: Tokens) -> Result<bool, SqlxError> {
    let last: Vec<(String, Option<i64>)> = sqlx::query_as(
        "SELECT m.purpose, m.input_tokens FROM model_calls m JOIN turns t ON t.id = m.turn \
         WHERE t.conversation = $1 AND (m.purpose = 'compaction' OR (m.purpose = 'step' AND m.input_tokens IS NOT NULL)) \
         ORDER BY m.started_at DESC, m.n DESC LIMIT 2",
    )
    .bind(conversation.as_str())
    .fetch_all(&mut **tx)
    .await?;
    Ok(match last.as_slice() {
        [(step, Some(tokens)), (compaction, _)] => {
            step == "step" && compaction == "compaction" && Tokens(*tokens as u64) > most
        }
        _ => false,
    })
}

/// The open question answered by the message's words (spec 01 section 3.4).
async fn answer_in_words(
    tx: &mut Transaction<'_, Postgres>,
    conversation: &Ulid,
    turn: &Ulid,
    question: &str,
    author: &Person,
) -> Result<Written, SqlxError> {
    sqlx::query(
        "UPDATE questions SET state = 'answered', answers = '[]', in_words = true, answered_by = $2, \
             answered_at = now() WHERE id = $1 AND state = 'open'",
    )
    .bind(question)
    .bind(author.subject.as_str())
    .execute(&mut **tx)
    .await?;
    let id: Ulid = question.parse().map_err(decode)?;
    let body = QuestionAnswered { question: id, answers: Vec::new(), in_words: true, author: author.clone() };
    journal(tx, conversation, Some(turn), JournalKind::QuestionAnswered, body, Some(question.to_owned())).await
}

/// The pending approval denied, the message its reason (spec 01 section 3.10, rule 3).
async fn deny_by_message(
    tx: &mut Transaction<'_, Postgres>,
    conversation: &Ulid,
    turn: &Ulid,
    approval: &str,
    author: &Person,
    reason: &str,
) -> Result<Written, SqlxError> {
    sqlx::query(
        "UPDATE approvals SET state = 'denied', decided_by = $2, decided_at = now(), reason = $3 \
         WHERE id = $1 AND state = 'pending'",
    )
    .bind(approval)
    .bind(author.subject.as_str())
    .bind(reason)
    .execute(&mut **tx)
    .await?;
    let id: Ulid = approval.parse().map_err(decode)?;
    let body = ApprovalDecided {
        approval: id,
        decision: Decision::Deny,
        author: author.clone(),
        reason: Some(reason.to_owned()),
    };
    journal(tx, conversation, Some(turn), JournalKind::ApprovalDecided, body, Some(approval.to_owned())).await
}
