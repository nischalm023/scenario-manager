//! Within `drafts`, the `press` is Apply on a person's draft (spec 01 section 3.10's rule 6 and section 3.11): their
//! approval of exactly the revision they checked. The would-be site is checked once more on the draft's stored cut (an
//! error answers `validation-failed` with its problems) and a scope of the cut that moved stales the draft
//! (`head-moved`); then, under the draft's row lock and in one transaction, the press is recorded as the approval,
//! the one changeset that revision writes is built under the approval's identifier and kept, and the draft is claimed
//! `applying`. The answer is `202` with the approval, and `apply` carries it out. A press while the draft is applying,
//! or once it applied, answers the approval that claimed it; a press while the agent's approval of the same Apply
//! waits decides that approval first (section 3.10's rule 6, `agent::press_decides`).

use axum::extract::rejection::PathRejection;
use axum::extract::{Path, State};
use axum::http::{StatusCode, Uri};
use axum::{Extension, Json};
use serde_json::{Value, json};
use sqlx::{Error as SqlxError, PgConnection, Postgres, Transaction};
use studio_protocol::{
    ApprovalAnswer, ApprovalFacts, DraftStale, DraftState, FactRow, JournalKind, MovedScope, Pin, Problem, ProblemType,
    StaleReason, Ulid,
};
use wps_client::apply::changeset_of;
use wps_client::wps::Wps;
use wps_client::{UnitCut, WorldStore, WpsError};

use super::apply::execute;
use super::site::{Checked, base, check};
use super::{Draft, draft_in, locked, mine, not_found, open, refuse, to_site};
use crate::GateState;
use crate::agent::press_decides;
use crate::approvals::{self, Press, record_button, record_sent};
use crate::conversations::{live_of_site, open_site};
use crate::db::decode;
use crate::identity::Identity;
use crate::journal::NewEntry;
use crate::problem::{ProblemAnswer, internal, wps_problem};

/// `POST /api/drafts/{d}/apply`.
pub async fn press<S: Wps>(
    State(gate): State<GateState<S>>,
    Extension(identity): Extension<Identity>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
) -> Result<(StatusCode, Json<ApprovalAnswer>), ProblemAnswer> {
    let instance = uri.path();
    let id = draft_in(path, instance)?;
    let fault = |fault| internal("record the press", fault, instance);
    let draft =
        (super::load(&gate.app.pool, &id).await).map_err(fault)?.ok_or_else(|| not_found(id.as_str(), instance))?;
    mine(&draft, &identity, instance)?;
    let mut conn = gate.app.pool.acquire().await.map_err(fault)?;
    if let Some(claimed) = claimed_by(&mut conn, &draft).await.map_err(fault)? {
        return Ok(claimed);
    }
    let live = live_of_site(&mut conn, &draft.site, &gate.app.model_label).await.map_err(fault)?;
    let thread = live.map(|conversation| conversation.id);
    let act = approved_input(&draft);
    if let Some(approval) =
        press_decides(&gate, thread.as_ref(), "changeset.apply", &act, &identity, "Apply", instance).await?
    {
        let record = approvals::approval_record(&mut conn, &approval).await.map_err(fault)?;
        let record = record.ok_or_else(|| fault(decode("the agent's approval is not recorded")))?;
        return Ok((StatusCode::ACCEPTED, Json(ApprovalAnswer { approval: record })));
    }
    drop(conn);
    open(&draft, instance)?;
    let document = base(&gate, &draft.cut, instance).await?;
    let site = draft.site.as_scope();
    let operation = draft.id.as_str();
    let checked =
        check(&gate, site, &draft.cut, &document, &draft.summary, &draft.ops, operation, &identity.name, instance)
            .await?;
    if checked.has_errors() {
        let mut problem = Problem::new(ProblemType::ValidationFailed, "Not applied: the flow would not compile.");
        problem.problems = checked.problems;
        return Err(problem.at(instance).into());
    }
    let moved = moved(&gate, &draft.cut).await.map_err(|error| wps_problem(&error, instance))?;
    if !moved.is_empty() {
        let mut problem =
            Problem::new(ProblemType::HeadMoved, "The site moved on after this draft was built; nothing was applied.");
        problem.moved = moved;
        go_stale(&gate, &draft, &problem).await.map_err(fault)?;
        return Err(problem.at(instance).into());
    }
    let mut tx = gate.app.pool.begin().await.map_err(fault)?;
    let now = locked(&mut tx, &id).await.map_err(fault)?.ok_or_else(|| not_found(id.as_str(), instance))?;
    if let Some(claimed) = claimed_by(&mut tx, &now).await.map_err(fault)? {
        return Ok(claimed);
    }
    if now.revision != draft.revision || !now.state.is_open() {
        let detail = "The draft changed while it was checked; read it again and press Apply once more.".to_owned();
        return Err(refuse(ProblemType::DraftChanged, detail, instance));
    }
    let approval = claim(&gate, &mut tx, &draft, &identity, checked).await.map_err(fault)?;
    let record = approvals::approval_record(&mut tx, &approval)
        .await
        .map_err(fault)?
        .ok_or_else(|| fault(decode("no approval")))?;
    tx.commit().await.map_err(fault)?;
    tokio::spawn(execute(gate.clone(), draft.id.clone(), approval));
    Ok((StatusCode::ACCEPTED, Json(ApprovalAnswer { approval: record })))
}

/// The `202` a press answers when the draft is already claimed or applied: the approval that claimed it.
async fn claimed_by(
    conn: &mut PgConnection,
    draft: &Draft,
) -> Result<Option<(StatusCode, Json<ApprovalAnswer>)>, SqlxError> {
    let (DraftState::Applying | DraftState::Applied, Some(approval)) = (draft.state, &draft.approval) else {
        return Ok(None);
    };
    let record =
        approvals::approval_record(conn, approval).await?.ok_or_else(|| decode("a draft names no approval"))?;
    Ok(Some((StatusCode::ACCEPTED, Json(ApprovalAnswer { approval: record }))))
}

/// What an Apply approves: the draft at the revision, operations and cut its press checked.
fn approved_input(draft: &Draft) -> Value {
    json!({ "draft": draft.id, "revision": draft.revision, "ops_digest": draft.ops_digest, "cut": draft.cut.seqs })
}

/// Inside the press's transaction: the press recorded as the person's approval on the site's live conversation, the
/// checked revision's changeset built under the approval's identifier and kept, the plan with the approval and the
/// body with the draft, and the draft claimed `applying` by that approval.
async fn claim<S: Wps>(
    gate: &GateState<S>,
    tx: &mut Transaction<'_, Postgres>,
    draft: &Draft,
    identity: &Identity,
    checked: Checked,
) -> Result<Ulid, SqlxError> {
    let (conversation, _) =
        open_site(tx, &draft.site, &identity.subject, &gate.app.model_label).await.map_err(decode)?;
    let input = approved_input(draft);
    let display = (checked.parts.iter())
        .map(|part| FactRow { label: part.scope.to_string(), value: format!("{} changes", part.ops) })
        .collect();
    let facts = ApprovalFacts {
        lead: draft.summary.clone(),
        display,
        notes: Vec::new(),
        action_label: "Apply".to_owned(),
        footnote: None,
    };
    let press = Press {
        conversation: Some(&conversation.id),
        tool: "changeset.apply",
        class: "apply",
        input,
        digest: None,
        summary: draft.summary.clone(),
        facts: json!(facts),
        by: &identity.subject,
    };
    let approval = record_button(tx, press).await?;
    let mut changeset = checked.changeset;
    changeset.operation = approval.to_string();
    let body = changeset_of(&changeset).map_err(decode)?;
    record_sent(tx, &approval, &json!(changeset)).await?;
    sqlx::query(
        "UPDATE changeset_drafts SET state = 'applying', approval = $2, apply_body = $3, updated_at = now() \
         WHERE id = $1",
    )
    .bind(draft.id.as_str())
    .bind(approval.as_str())
    .bind(json!(body))
    .execute(&mut **tx)
    .await?;
    Ok(approval)
}

/// The scopes of `cut` that moved past the changeset it holds them at.
async fn moved<S: Wps>(gate: &GateState<S>, cut: &UnitCut) -> Result<Vec<MovedScope>, WpsError> {
    let heads = gate.world.resolve_cut(&cut.unit, &Pin::Branch(cut.branch.clone())).await?;
    Ok((cut.seqs.iter())
        .filter_map(|(scope, base)| {
            let head = *heads.seqs.get(scope)?;
            (head != *base).then(|| MovedScope { scope: scope.clone(), base: *base, head })
        })
        .collect())
}

/// A draft whose cut moved, stale (`head-moved`) with the scopes that moved, and `draft.stale` journaled with the
/// problem in the same transaction, while it is still the open draft the press read.
async fn go_stale<S: Wps>(gate: &GateState<S>, draft: &Draft, problem: &Problem) -> Result<(), SqlxError> {
    let mut tx = gate.app.pool.begin().await?;
    let reason = json!({ "reason": "head-moved", "moved": problem.moved });
    let staled = sqlx::query(
        "UPDATE changeset_drafts SET state = 'stale', stale_reason = $3, updated_at = now() \
         WHERE id = $1 AND revision = $2 AND state IN ('drafting', 'ready')",
    )
    .bind(draft.id.as_str())
    .bind(draft.revision as i32)
    .bind(&reason)
    .execute(&mut *tx)
    .await?;
    if staled.rows_affected() == 0 {
        return Ok(());
    }
    let body = DraftStale { draft: draft.id.clone(), reason: StaleReason::HeadMoved, problem: Some(json!(problem)) };
    let entry = NewEntry::new(JournalKind::DraftStale, json!(body));
    let told = to_site(gate, &mut tx, &draft.site, &draft.created_by, entry).await?;
    tx.commit().await?;
    gate.app.hub.announce(told);
    Ok(())
}
