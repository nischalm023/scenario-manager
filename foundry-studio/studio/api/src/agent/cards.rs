//! Within `agent`, `cards` are a waiting turn's question and approval as people act on them (spec 01 sections 3.4,
//! 3.6 and 3.10): an answer to every item with labels the item offers, or a decision, recorded once with who pressed;
//! the same answer or decision again answers as the first did, a different one `question-closed` or
//! `approval-closed`, one for a turn that no longer waits on it `turn-not-waiting`, and one whose conversation began
//! with another provider than the agent's now `provider-changed`. The answer or decision, its
//! `turn.resumed` and the turn running again commit together, under the conversation's lock, then the turn's, then the
//! card's, and a new task resumes the turn under a permit taken before the commit. A press of Create site or Apply
//! while the agent's approval of the same act waits decides that approval (rule 6); one after that approval was
//! approved, before its act took the draft, answers with it, since it owns the act.

use std::mem;

use axum::body::Bytes;
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, State};
use axum::http::Uri;
use axum::{Extension, Json};
use serde_json::{Value, json};
use sqlx::{Error as SqlxError, Postgres, Transaction};
use studio_protocol::{
    AnswersAccepted, ApprovalAnswer, ApprovalDecided, ApprovalState, Decision, ItemAnswer, JournalKind, Person,
    PostAnswers, PostDecision, Problem, ProblemType, QuestionAnswered, QuestionItem, QuestionRef, QuestionState,
    ResumedBy, TurnResumed, TurnState, Ulid,
};
use wps_client::wps::Wps;

use super::admit::same_provider;
use super::turn::{Start, drive};
use super::{busy, journal, lock_turn};
use crate::GateState;
use crate::approvals::approval_record;
use crate::db::{decode, read_word};
use crate::idempotency::write_canonical;
use crate::identity::Identity;
use crate::journal::Written;
use crate::problem::{ProblemAnswer, internal, json_body, validation_failed};

/// A card's conversation and turn, as acting on it reads them under their locks.
struct Waiting {
    conversation: Ulid,
    turn: Ulid,
    turn_waits_on_it: bool,
    /// The provider the conversation began with, which its turn's resumption must still be.
    began: Option<String>,
}

fn refuse(type_: ProblemType, detail: impl Into<String>, instance: &str) -> ProblemAnswer {
    Problem::new(type_, detail).at(instance).into()
}

/// Inside `tx`, before the card's own row is locked: the card's conversation and its turn locked in the one order;
/// the conversation must be live and, for a new-site thread, the asker's.
async fn locked(
    tx: &mut Transaction<'_, Postgres>,
    turn: &str,
    card: &str,
    identity: &Identity,
    instance: &str,
) -> Result<Waiting, ProblemAnswer> {
    let fault = |fault| internal("read the card's turn", fault, instance);
    let held = lock_turn(tx, turn).await.map_err(fault)?.ok_or_else(|| fault(decode("a card names no turn")))?;
    if held.lived.as_deref() != Some("live") {
        let detail = "This conversation reads as history; nothing in it can be answered or decided again.";
        return Err(refuse(ProblemType::ConversationClosed, detail, instance));
    }
    let owner = held.created_by.unwrap_or_default();
    if held.site.is_none() && owner != identity.subject.as_str() {
        return Err(refuse(
            ProblemType::NotConversationOwner,
            format!("This new site's conversation is {owner}'s; you can read it."),
            instance,
        ));
    }
    Ok(Waiting {
        conversation: held.conversation,
        turn: turn.parse().map_err(|why| fault(decode(why)))?,
        turn_waits_on_it: held.state == TurnState::Waiting && held.waiting_id.as_deref() == Some(card),
        began: held.provider,
    })
}

/// Inside `tx`: the turn running again under this process's lease, with `turn.resumed`.
async fn resumed(
    tx: &mut Transaction<'_, Postgres>,
    waiting: &Waiting,
    owner: &Ulid,
    by: ResumedBy,
    card: &str,
) -> Result<Written, SqlxError> {
    sqlx::query(
        "UPDATE turns SET state = 'running', waiting_on = NULL, waiting_id = NULL, waiting_since = NULL, \
             lease_owner = $2, lease_expires_at = now() + interval '30 seconds' WHERE id = $1",
    )
    .bind(waiting.turn.as_str())
    .bind(owner.as_str())
    .execute(&mut **tx)
    .await?;
    journal(
        tx,
        &waiting.conversation,
        Some(&waiting.turn),
        JournalKind::TurnResumed,
        TurnResumed { by },
        Some(card.to_owned()),
    )
    .await
}

/// `POST /api/questions/{q}/answers`.
pub async fn answers<S: Wps>(
    State(gate): State<GateState<S>>,
    Extension(identity): Extension<Identity>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
    body: Bytes,
) -> Result<Json<AnswersAccepted>, ProblemAnswer> {
    let instance = uri.path();
    let nothing = || refuse(ProblemType::NotFound, "No question has that id.", instance);
    let Ok(Path(id)) = path else { return Err(nothing()) };
    let id: Ulid = id.parse().map_err(|_| nothing())?;
    let PostAnswers { answers } = json_body(&body, instance)?;
    let fault = |fault| internal("answer the question", fault, instance);
    let mut tx = gate.app.pool.begin().await.map_err(fault)?;
    let turn: Option<String> = sqlx::query_scalar("SELECT turn FROM questions WHERE id = $1")
        .bind(id.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(fault)?;
    let Some(turn) = turn else { return Err(nothing()) };
    let waiting = locked(&mut tx, &turn, id.as_str(), &identity, instance).await?;
    let (state, items, held, in_words): (String, Value, Option<Value>, bool) =
        sqlx::query_as("SELECT state, items, answers, in_words FROM questions WHERE id = $1 FOR UPDATE")
            .bind(id.as_str())
            .fetch_one(&mut *tx)
            .await
            .map_err(fault)?;
    let accepted = |state| Json(AnswersAccepted { question: QuestionRef { id: id.clone(), state } });
    match read_word::<QuestionState>(&state).map_err(fault)? {
        QuestionState::Answered if !in_words && held == Some(json!(answers)) => {
            return Ok(accepted(QuestionState::Answered));
        }
        QuestionState::Open => {}
        _ => {
            return Err(refuse(
                ProblemType::QuestionClosed,
                "This question was already answered or has expired.",
                instance,
            ));
        }
    }
    if !waiting.turn_waits_on_it {
        return Err(refuse(
            ProblemType::TurnNotWaiting,
            "The agent's turn no longer waits on this question.",
            instance,
        ));
    }
    let items: Vec<QuestionItem> = serde_json::from_value(items).map_err(|why| fault(decode(why)))?;
    check_answers(&items, &answers).map_err(|why| validation_failed(why, instance))?;
    same_provider(waiting.began.as_deref(), &gate.app.agent, instance)?;
    sqlx::query(
        "UPDATE questions SET state = 'answered', answers = $2, answered_by = $3, answered_at = now() WHERE id = $1",
    )
    .bind(id.as_str())
    .bind(json!(answers))
    .bind(identity.subject.as_str())
    .execute(&mut *tx)
    .await
    .map_err(fault)?;
    let author = Person { subject: identity.subject.clone(), name: identity.name.clone() };
    let body = QuestionAnswered { question: id.clone(), answers, in_words: false, author };
    let caused = Some(id.to_string());
    let mut written = vec![
        journal(&mut tx, &waiting.conversation, Some(&waiting.turn), JournalKind::QuestionAnswered, body, caused)
            .await
            .map_err(fault)?,
    ];
    let owner = gate.app.agent.instance.clone();
    written.push(resumed(&mut tx, &waiting, &owner, ResumedBy::Answer, id.as_str()).await.map_err(fault)?);
    let permit = gate.app.agent.permit(&waiting.turn).ok_or_else(|| busy(instance))?;
    tx.commit().await.map_err(fault)?;
    gate.app.hub.announce(written);
    tokio::spawn(drive(gate.clone(), waiting.turn, Start::Resumed, permit));
    Ok(accepted(QuestionState::Answered))
}

/// An answer for every item, once each, with labels the item offers: one for a single choice, one or more for many.
fn check_answers(items: &[QuestionItem], answers: &[ItemAnswer]) -> Result<(), String> {
    let mut seen = vec![false; items.len()];
    for answer in answers {
        let Some(item) = items.get(answer.item as usize) else {
            return Err(format!("there is no item {}", answer.item));
        };
        if mem::replace(&mut seen[answer.item as usize], true) {
            return Err(format!("item {} is answered twice", answer.item));
        }
        if answer.selected.is_empty() || (!item.multi && answer.selected.len() > 1) {
            return Err(format!(
                "item {} takes {}",
                answer.item,
                if item.multi { "one label or more" } else { "one label" }
            ));
        }
        if let Some(label) =
            answer.selected.iter().find(|label| !item.options.iter().any(|option| &&option.label == label))
        {
            return Err(format!("item {} offers no label `{label}`", answer.item));
        }
    }
    match seen.iter().position(|seen| !seen) {
        Some(missing) => Err(format!("item {missing} has no answer")),
        None => Ok(()),
    }
}

/// `POST /api/approvals/{a}/decision`.
pub async fn decision<S: Wps>(
    State(gate): State<GateState<S>>,
    Extension(identity): Extension<Identity>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
    body: Bytes,
) -> Result<Json<ApprovalAnswer>, ProblemAnswer> {
    let instance = uri.path();
    let nothing = || refuse(ProblemType::NotFound, "No approval has that id.", instance);
    let Ok(Path(id)) = path else { return Err(nothing()) };
    let id: Ulid = id.parse().map_err(|_| nothing())?;
    let PostDecision { decision, reason } = json_body(&body, instance)?;
    let fault = |fault| internal("decide the approval", fault, instance);
    let mut tx = gate.app.pool.begin().await.map_err(fault)?;
    let decided = decide(&gate, &mut tx, &id, decision, reason, &identity, instance).await?;
    let record = approval_record(&mut tx, &id).await.map_err(fault)?.ok_or_else(nothing)?;
    let resumes = match decided {
        Some((turn, written)) => Some((gate.app.agent.permit(&turn).ok_or_else(|| busy(instance))?, turn, written)),
        None => None,
    };
    tx.commit().await.map_err(fault)?;
    if let Some((permit, turn, written)) = resumes {
        gate.app.hub.announce(written);
        tokio::spawn(drive(gate.clone(), turn, Start::Resumed, permit));
    }
    Ok(Json(ApprovalAnswer { approval: record }))
}

/// Inside `tx`: record `decision` on approval `id`, or find it already made; the turn to resume and the entries, when
/// this call decided it.
async fn decide<S: Wps>(
    gate: &GateState<S>,
    tx: &mut Transaction<'_, Postgres>,
    id: &Ulid,
    decision: Decision,
    reason: Option<String>,
    identity: &Identity,
    instance: &str,
) -> Result<Option<(Ulid, Vec<Written>)>, ProblemAnswer> {
    let fault = |fault| internal("decide the approval", fault, instance);
    let row: Option<(Option<String>, String)> = sqlx::query_as("SELECT turn, via FROM approvals WHERE id = $1")
        .bind(id.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(fault)?;
    let Some((turn, via)) = row else {
        return Err(refuse(ProblemType::NotFound, "No approval has that id.", instance));
    };
    let wanted = match decision {
        Decision::Approve => ApprovalState::Approved,
        Decision::Deny => ApprovalState::Denied,
    };
    let closed = || refuse(ProblemType::ApprovalClosed, "This approval was already decided or has expired.", instance);
    let Some(turn) = turn.filter(|_| via == "card") else {
        return if wanted == ApprovalState::Approved { Ok(None) } else { Err(closed()) };
    };
    let waiting = locked(tx, &turn, id.as_str(), identity, instance).await?;
    let state: String = sqlx::query_scalar("SELECT state FROM approvals WHERE id = $1 FOR UPDATE")
        .bind(id.as_str())
        .fetch_one(&mut **tx)
        .await
        .map_err(fault)?;
    match read_word::<ApprovalState>(&state).map_err(fault)? {
        ApprovalState::Pending => {}
        same if same == wanted => return Ok(None),
        _ => return Err(closed()),
    }
    if !waiting.turn_waits_on_it {
        return Err(refuse(
            ProblemType::TurnNotWaiting,
            "The agent's turn no longer waits on this approval.",
            instance,
        ));
    }
    same_provider(waiting.began.as_deref(), &gate.app.agent, instance)?;
    let reason = reason.filter(|_| decision == Decision::Deny).filter(|reason| !reason.trim().is_empty());
    let written = record_decision(gate, tx, &waiting, id, decision, reason, identity, ResumedBy::Decision)
        .await
        .map_err(fault)?;
    Ok(Some((waiting.turn, written)))
}

/// Inside `tx`: the approval decided by `identity`, its `approval.decided`, and the turn running again.
#[allow(clippy::too_many_arguments)]
async fn record_decision<S: Wps>(
    gate: &GateState<S>,
    tx: &mut Transaction<'_, Postgres>,
    waiting: &Waiting,
    id: &Ulid,
    decision: Decision,
    reason: Option<String>,
    identity: &Identity,
    by: ResumedBy,
) -> Result<Vec<Written>, SqlxError> {
    let (state, execution) = match decision {
        Decision::Approve => ("approved", Some("not_started")),
        Decision::Deny => ("denied", None),
    };
    sqlx::query(
        "UPDATE approvals SET state = $2, execution = $3, decided_by = $4, decided_at = now(), reason = $5 WHERE id = $1",
    )
    .bind(id.as_str())
    .bind(state)
    .bind(execution)
    .bind(identity.subject.as_str())
    .bind(&reason)
    .execute(&mut **tx)
    .await?;
    let author = Person { subject: identity.subject.clone(), name: identity.name.clone() };
    let body = ApprovalDecided { approval: id.clone(), decision, author, reason };
    let caused = Some(id.to_string());
    let mut written = vec![
        journal(tx, &waiting.conversation, Some(&waiting.turn), JournalKind::ApprovalDecided, body, caused).await?,
    ];
    written.push(resumed(tx, waiting, &gate.app.agent.instance, by, id.as_str()).await?);
    Ok(written)
}

/// Spec 01 section 3.10, rule 6, a press's half: before a person's press of `tool` records their own approval, the
/// agent's approval of the same act in `conversation` is decided. A pending one its turn waits on is approved by the
/// same input, and the press answers with it (`Some`); another input for the same draft denies it, the press then
/// recording its own (`None`). One already approved whose act has not started owns the act, so the press answers with
/// it whatever its values, as a press while the act executes does.
pub async fn press_decides<S: Wps>(
    gate: &GateState<S>,
    conversation: Option<&Ulid>,
    tool: &str,
    input: &Value,
    identity: &Identity,
    action: &str,
    instance: &str,
) -> Result<Option<Ulid>, ProblemAnswer> {
    let Some(conversation) = conversation else { return Ok(None) };
    let fault = |fault| internal("decide the agent's approval of the same act", fault, instance);
    let mut tx = gate.app.pool.begin().await.map_err(fault)?;
    let owning: Option<(String, String)> = sqlx::query_as(
        "SELECT id, turn FROM approvals WHERE conversation = $1 AND tool = $2 AND via = 'card' \
         AND input->>'draft' = $3 AND (state = 'pending' OR (state = 'approved' AND execution = 'not_started')) \
         ORDER BY requested_at DESC LIMIT 1",
    )
    .bind(conversation.as_str())
    .bind(tool)
    .bind(input["draft"].as_str().unwrap_or_default())
    .fetch_optional(&mut *tx)
    .await
    .map_err(fault)?;
    let Some((approval, turn)) = owning else { return Ok(None) };
    let held =
        lock_turn(&mut tx, &turn).await.map_err(fault)?.ok_or_else(|| fault(decode("an approval names no turn")))?;
    let (state, execution, stored): (String, Option<String>, Value) =
        sqlx::query_as("SELECT state, execution, input FROM approvals WHERE id = $1 FOR UPDATE")
            .bind(&approval)
            .fetch_one(&mut *tx)
            .await
            .map_err(fault)?;
    let approval: Ulid = approval.parse().map_err(|why| fault(decode(why)))?;
    match (state.as_str(), execution.as_deref()) {
        ("approved", Some("not_started")) => return Ok(Some(approval)),
        ("pending", _) if held.state == TurnState::Waiting && held.waiting_id.as_deref() == Some(approval.as_str()) => {
        }
        _ => return Ok(None),
    }
    same_provider(held.provider.as_deref(), &gate.app.agent, instance)?;
    let canonical = |value: &Value| {
        let mut bytes = Vec::new();
        write_canonical(value, &mut bytes);
        bytes
    };
    let same = canonical(&stored) == canonical(input);
    let (decision, reason) = match same {
        true => (Decision::Approve, None),
        false => (Decision::Deny, Some(format!("{} pressed {action} in the panel with other values.", identity.name))),
    };
    let waiting = Waiting {
        conversation: held.conversation,
        turn: turn.parse().map_err(|why| fault(decode(why)))?,
        turn_waits_on_it: true,
        began: held.provider,
    };
    let written = record_decision(gate, &mut tx, &waiting, &approval, decision, reason, identity, ResumedBy::Decision)
        .await
        .map_err(fault)?;
    let permit = gate.app.agent.permit(&waiting.turn).ok_or_else(|| busy(instance))?;
    tx.commit().await.map_err(fault)?;
    gate.app.hub.announce(written);
    tokio::spawn(drive(gate.clone(), waiting.turn, Start::Resumed, permit));
    Ok(same.then_some(approval))
}
