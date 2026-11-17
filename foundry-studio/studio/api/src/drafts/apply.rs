//! Within `drafts`, `apply` carries out a press its draft is claimed for (spec 01 sections 3.10 and 3.11; spec 04
//! section 3.9): the approval's one worker (`approvals::Executing`, its claim going forward only) sends the changeset
//! body kept at the claim, as it was, under the approval's operation identifier, and while the outcome stays unknown
//! sends it again under Create site's one policy (`approvals::Asks`: five minutes of wall clock, then the next start),
//! since only WPS's replay of that identifier settles an answer that was lost.
//! Applied, the draft is `applied` with each written scope's changeset; refused by WPS itself, its refusal object in
//! hand (`NotLanded::Refused`), it is open again with that problem and its claim released, or `stale` when the refusal
//! is that a scope moved. Anything else, a lost answer, a timeout, an answer that does not read, leaves the outcome
//! unknown and the claim held. Each settlement moves the draft and the approval and journals both in one transaction,
//! and only while that approval still holds the draft; a start resumes every draft left claimed, by a stopped process
//! or by a worker whose asks ran out (`recover`), before the listeners take a request.

use serde_json::{Value, json};
use sqlx::{Error as SqlxError, Postgres, Transaction};
use studio_protocol::{
    DraftApplied, DraftStale, DraftState, Execution, JournalKind, MovedScope, Problem, ProblemItem, ProblemType,
    Severity, StaleReason, Ulid,
};
use wps_client::wire::WireChangeset;
use wps_client::wps::Wps;
use wps_client::{Applied, ChangesetDraft, NotLanded, WorldStore, WpsError};

use super::site::client_item;
use super::{Draft, locked, to_site};
use crate::GateState;
use crate::approvals::{Asks, Moved, applying, claim_applying};
use crate::db::decode;
use crate::journal::{NewEntry, Written};

/// Carry out `approval` on `draft`; a fault of our own database is printed, since no request waits on it.
pub(super) async fn execute<S: Wps>(gate: GateState<S>, draft: Ulid, approval: Ulid) {
    if let Err(fault) = attempt(&gate, &draft, &approval).await {
        eprintln!("studio-api: the apply of draft {draft} under approval {approval} stopped: {fault}");
    }
}

/// Resume every draft left `applying`, by a stopped process or by a worker whose asks ran out, each under the approval
/// that claimed it, from the body kept at the claim; the start awaits this scan before its listeners take a request
/// (`main`'s `run`).
pub async fn recover<S: Wps>(gate: &GateState<S>) -> Result<usize, SqlxError> {
    let claimed: Vec<(String, String)> =
        sqlx::query_as("SELECT id, approval FROM changeset_drafts WHERE state = 'applying' ORDER BY id")
            .fetch_all(&gate.app.pool)
            .await?;
    for (draft, approval) in &claimed {
        let (draft, approval) = (draft.parse().map_err(decode)?, approval.parse().map_err(decode)?);
        tokio::spawn(execute(gate.clone(), draft, approval));
    }
    Ok(claimed.len())
}

async fn attempt<S: Wps>(gate: &GateState<S>, draft: &Ulid, approval: &Ulid) -> Result<(), SqlxError> {
    let Some(_holding) = gate.app.executing.hold(approval) else { return Ok(()) };
    if !claim_applying(&gate.app, approval, Some(draft)).await? {
        return Ok(());
    }
    let Some((changeset, body)) = kept(gate, draft, approval).await? else { return Ok(()) };
    let mut asks = Asks::new();
    loop {
        match gate.world.resend(&body, &changeset).await {
            Ok(written) => return applied(gate, draft, approval, &written).await,
            Err(NotLanded::Refused(refusal)) => return refused(gate, draft, approval, &refusal).await,
            Err(NotLanded::Unknown(error)) => unknown(gate, draft, approval, &error).await?,
        }
        if !asks.wait().await {
            eprintln!(
                "studio-api: the apply of draft {draft} under approval {approval} is not settled; the next start asks \
                 again"
            );
            return Ok(());
        }
    }
}

/// What the claim kept: the changeset with the approval and its body with the draft, while that approval still holds
/// the draft; nothing once it is settled.
async fn kept<S: Wps>(
    gate: &GateState<S>,
    draft: &Ulid,
    approval: &Ulid,
) -> Result<Option<(ChangesetDraft, WireChangeset)>, SqlxError> {
    let kept: Option<(Option<Value>, Option<Value>)> = sqlx::query_as(
        "SELECT a.sent, d.apply_body FROM approvals a JOIN changeset_drafts d ON d.approval = a.id \
         WHERE a.id = $1 AND d.id = $2 AND d.state = 'applying' AND a.execution IN ('running', 'unknown')",
    )
    .bind(approval.as_str())
    .bind(draft.as_str())
    .fetch_optional(&gate.app.pool)
    .await?;
    let Some((Some(sent), Some(body))) = kept else { return Ok(None) };
    Ok(Some((serde_json::from_value(sent).map_err(decode)?, serde_json::from_value(body).map_err(decode)?)))
}

/// The draft under its row lock while `approval` still holds it `applying`.
async fn held(tx: &mut Transaction<'_, Postgres>, draft: &Ulid, approval: &Ulid) -> Result<Option<Draft>, SqlxError> {
    let row = locked(tx, draft).await?;
    Ok(row.filter(|row| row.state == DraftState::Applying && row.approval.as_ref() == Some(approval)))
}

/// WPS applied the body (or replayed its first applying): the draft `applied` with each written scope's changeset,
/// `approval.execution` `ok`, and `draft.applied`, in one transaction.
async fn applied<S: Wps>(
    gate: &GateState<S>,
    draft: &Ulid,
    approval: &Ulid,
    written: &Applied,
) -> Result<(), SqlxError> {
    let mut tx = gate.app.pool.begin().await?;
    let Some(row) = held(&mut tx, draft, approval).await? else { return Ok(()) };
    let seqs = json!(written.seqs);
    sqlx::query("UPDATE changeset_drafts SET state = 'applied', applied_seqs = $2, updated_at = now() WHERE id = $1")
        .bind(draft.as_str())
        .bind(&seqs)
        .execute(&mut *tx)
        .await?;
    let outcome = json!({ "seqs": seqs, "replayed": written.replayed });
    let ok = applying(&mut tx, approval, draft, Execution::Ok, None, Some(&outcome)).await?;
    let mut told: Vec<Written> = ok.and_then(|Moved(entry)| entry).into_iter().collect();
    let body = DraftApplied { draft: draft.clone(), seqs: written.seqs.clone() };
    told.extend(
        to_site(gate, &mut tx, &row.site, &row.created_by, NewEntry::new(JournalKind::DraftApplied, json!(body)))
            .await?,
    );
    tx.commit().await?;
    gate.app.hub.announce(told);
    Ok(())
}

/// WPS refused the body, writing nothing: a moved scope leaves the draft `stale` (`head-moved`); any other refusal
/// opens it again, `drafting` with the refusal among its problems and its claim released. Either way the approval's
/// execution is `failed` with the refusal, journaled in the same transaction.
async fn refused<S: Wps>(
    gate: &GateState<S>,
    draft: &Ulid,
    approval: &Ulid,
    error: &WpsError,
) -> Result<(), SqlxError> {
    let problem = refusal_problem(error, draft);
    let mut tx = gate.app.pool.begin().await?;
    let Some(row) = held(&mut tx, draft, approval).await? else { return Ok(()) };
    let mut told = Vec::new();
    if problem.type_ == ProblemType::HeadMoved {
        let reason = json!({ "reason": "head-moved", "moved": problem.moved });
        sqlx::query("UPDATE changeset_drafts SET state = 'stale', stale_reason = $2, updated_at = now() WHERE id = $1")
            .bind(draft.as_str())
            .bind(&reason)
            .execute(&mut *tx)
            .await?;
        let body = DraftStale { draft: draft.clone(), reason: StaleReason::HeadMoved, problem: Some(json!(problem)) };
        told.extend(
            to_site(gate, &mut tx, &row.site, &row.created_by, NewEntry::new(JournalKind::DraftStale, json!(body)))
                .await?,
        );
    } else {
        let mut problems: Vec<ProblemItem> = serde_json::from_value(row.problems.clone()).unwrap_or_default();
        problems.extend(refusal_items(error));
        sqlx::query(
            "UPDATE changeset_drafts SET state = 'drafting', approval = NULL, apply_body = NULL, problems = $2, \
                 updated_at = now() WHERE id = $1",
        )
        .bind(draft.as_str())
        .bind(json!(problems))
        .execute(&mut *tx)
        .await?;
    }
    let outcome = json!(problem);
    let failed = applying(&mut tx, approval, draft, Execution::Failed, Some(&problem.detail), Some(&outcome)).await?;
    told.extend(failed.and_then(|Moved(entry)| entry));
    tx.commit().await?;
    gate.app.hub.announce(told);
    Ok(())
}

/// The outcome is unknown: `approval.execution` `unknown` with spec 04's sentence, journaled once, the claim held.
async fn unknown<S: Wps>(
    gate: &GateState<S>,
    draft: &Ulid,
    approval: &Ulid,
    error: &WpsError,
) -> Result<(), SqlxError> {
    eprintln!(
        "studio-api: whether the apply of draft {draft} under approval {approval} landed is not yet known: {error}"
    );
    let mut tx = gate.app.pool.begin().await?;
    let sentence = "Not yet known whether this change landed; checking again.";
    let unknown = applying(&mut tx, approval, draft, Execution::Unknown, Some(sentence), None).await?;
    tx.commit().await?;
    gate.app.hub.announce(unknown.and_then(|Moved(entry)| entry));
    Ok(())
}

/// The refusal as spec 01's problem, as the approval's outcome and a stale draft's journal carry it.
fn refusal_problem(error: &WpsError, draft: &Ulid) -> Problem {
    let mut problem = match error {
        WpsError::HeadMoved { moved } => {
            let mut problem = Problem::new(
                ProblemType::HeadMoved,
                "The site moved on after this draft was built; nothing was applied.",
            );
            problem.moved = moved
                .iter()
                .map(|(scope, base, head)| MovedScope { scope: scope.clone(), base: *base, head: *head })
                .collect();
            problem
        }
        other => Problem::new(ProblemType::ValidationFailed, format!("Not applied: {other}.")),
    };
    problem.problems = refusal_items(error);
    problem.at(format!("/api/drafts/{draft}/apply"))
}

/// The refusal as the draft's problem items: WPS's own problem, or the refusal's sentence.
fn refusal_items(error: &WpsError) -> Vec<ProblemItem> {
    match error {
        WpsError::Refused { problem } => vec![client_item(problem)],
        WpsError::HeadMoved { .. } => Vec::new(),
        other => vec![ProblemItem {
            code: match other {
                WpsError::GeneratorsMoved { .. } => "generators-moved".into(),
                _ => "too-large".into(),
            },
            severity: Severity::Error,
            ref_: None,
            scope: None,
            field: None,
            knob: None,
            text: other.to_string(),
        }],
    }
}
