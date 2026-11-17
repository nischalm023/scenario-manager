//! Within `studio-api`, `drafts` are spec 01 section 3.11's draft changesets as their routes serve them, in the first
//! cut a person's own (BACKLOG task 20): made on a site's first edit at the cut the person's panel read, amended at the
//! revision they last read, each edit folded, checked on the complete would-be site and told to the site's live
//! conversation as a `flow.draft_changed` panel event, applied by a press (`press`, then `apply`), or discarded. The
//! draft holds patches only, to flow entities and map sectors (lean 8); declare and delete arrive with the agent's
//! `flow.draft` (lead decided, say if not). While a press has claimed it, no edit or discard changes it.

mod apply;
pub mod fold;
mod press;
pub mod site;

use std::collections::BTreeMap;

use axum::body::Bytes;
use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, Uri};
use axum::routing::{get, patch, post};
use axum::{Extension, Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::postgres::PgRow;
use sqlx::{Error as SqlxError, PgPool, Postgres, Row, Transaction};
use studio_protocol::{
    AmendDraft, ChangesetOp, ChangesetPart, DraftChecked, DraftDiscarded, DraftList, DraftRead, DraftState,
    JournalKind, Location, MovedScope, NewDraft, PanelEvent, PanelEventKind, Problem, ProblemType, ScopePath, Seq,
    Severity, SitePath, Subject, Ulid,
};
use wps_client::wps::Wps;
use wps_client::{UnitCut, WorldStore};

use crate::conversations::open_site;
use crate::db::{decode, mint_ulid, read_word, word};
use crate::idempotency::{idempotent, write_canonical};
use crate::identity::Identity;
use crate::journal::{Appended, JournalError, NewEntry, Written, append_to_site};
use crate::problem::{ProblemAnswer, internal, json_body, validation_failed, wps_problem};
use crate::{AppState, GateState};
pub use apply::recover;
use fold::fold;
use site::{base, check, value_at};

pub fn routes<S: Wps>(app: &AppState) -> Router<GateState<S>> {
    Router::new()
        .route("/api/drafts", get(list).merge(idempotent(app, post(create))))
        .route("/api/drafts/{draft}", get(read).merge(idempotent(app, patch(amend))))
        .route("/api/drafts/{draft}/apply", idempotent(app, post(press::press)))
        .route("/api/drafts/{draft}/discard", idempotent(app, post(discard)))
}

/// A draft as its row holds it.
pub(crate) struct Draft {
    pub id: Ulid,
    pub site: SitePath,
    pub created_by: Subject,
    pub cut: UnitCut,
    pub state: DraftState,
    pub stale_reason: Option<Value>,
    pub summary: String,
    pub ops: Vec<ChangesetOp>,
    pub ops_digest: String,
    pub revision: u32,
    pub problems: Value,
    pub approval: Option<Ulid>,
    pub applied_seqs: Option<BTreeMap<ScopePath, Seq>>,
}

const COLUMNS: &str = "id, site, created_by, cut, state, stale_reason, summary, ops, ops_digest, revision, problems, \
                       approval, applied_seqs";

fn from_row(row: &PgRow) -> Result<Draft, SqlxError> {
    let json = |column: &str| -> Result<Value, SqlxError> { row.try_get(column) };
    Ok(Draft {
        id: row.try_get::<String, _>("id")?.parse().map_err(decode)?,
        site: row.try_get::<String, _>("site")?.parse().map_err(decode)?,
        created_by: row.try_get::<String, _>("created_by")?.parse().map_err(decode)?,
        cut: serde_json::from_value(json("cut")?).map_err(decode)?,
        state: read_word(&row.try_get::<String, _>("state")?)?,
        stale_reason: row.try_get("stale_reason")?,
        summary: row.try_get("summary")?,
        ops: serde_json::from_value(json("ops")?).map_err(decode)?,
        ops_digest: row.try_get("ops_digest")?,
        revision: row.try_get::<i32, _>("revision")? as u32,
        problems: json("problems")?,
        approval: (row.try_get::<Option<String>, _>("approval")?)
            .map(|text| text.parse().map_err(decode))
            .transpose()?,
        applied_seqs: row
            .try_get::<Option<Value>, _>("applied_seqs")?
            .map(serde_json::from_value)
            .transpose()
            .map_err(decode)?,
    })
}

pub(crate) async fn load(pool: &PgPool, id: &Ulid) -> Result<Option<Draft>, SqlxError> {
    let row = sqlx::query(&format!("SELECT {COLUMNS} FROM changeset_drafts WHERE id = $1"))
        .bind(id.as_str())
        .fetch_optional(pool)
        .await?;
    row.as_ref().map(from_row).transpose()
}

/// The draft under its row lock, inside the caller's transaction.
async fn locked(tx: &mut Transaction<'_, Postgres>, id: &Ulid) -> Result<Option<Draft>, SqlxError> {
    let row = sqlx::query(&format!("SELECT {COLUMNS} FROM changeset_drafts WHERE id = $1 FOR UPDATE"))
        .bind(id.as_str())
        .fetch_optional(&mut **tx)
        .await?;
    row.as_ref().map(from_row).transpose()
}

impl Draft {
    fn read(&self) -> DraftRead {
        let checked: Vec<_> = serde_json::from_value(self.problems.clone()).unwrap_or_default();
        DraftRead {
            draft: self.id.clone(),
            site: self.site.as_scope().clone(),
            revision: self.revision,
            state: self.state,
            stale_reason: self.stale_reason.clone(),
            summary: self.summary.clone(),
            cut: self.cut.seqs.clone(),
            ops: self.ops.clone(),
            parts: parts(self.site.as_scope(), &self.ops),
            problems: checked,
            approval: self.approval.clone(),
            applied_seqs: self.applied_seqs.clone(),
        }
    }
}

/// The scopes the operations write, each with its count: a flow patch writes the flow, a sector patch the map.
fn parts(site: &ScopePath, ops: &[ChangesetOp]) -> Vec<ChangesetPart> {
    let mut counted: BTreeMap<ScopePath, u32> = BTreeMap::new(); // ordered: user-visible JSON
    for op in ops {
        let ChangesetOp::Patch { pointer, .. } = op;
        let sub = if pointer.starts_with("/sector") { "map" } else { "flow" };
        if let Ok(scope) = site.join(&sub.parse().expect("a slug")) {
            *counted.entry(scope).or_default() += 1;
        }
    }
    counted.into_iter().map(|(scope, ops)| ChangesetPart { scope, ops }).collect()
}

/// Journal `entry` in the live conversation of `site` inside `tx`, opening one for `by` when nobody has: a person's
/// draft has no conversation of its own, and its events and endings go to its site's (spec 01 sections 3.11, 3.17).
pub(crate) async fn to_site<S: Wps>(
    gate: &GateState<S>,
    tx: &mut Transaction<'_, Postgres>,
    site: &SitePath,
    by: &Subject,
    entry: NewEntry,
) -> Result<Option<Written>, SqlxError> {
    open_site(tx, site, by, &gate.app.model_label).await.map_err(|fault| decode(fault.to_string()))?;
    match append_to_site(tx, site, entry).await {
        Ok(Appended::Wrote(written)) => Ok(Some(written)),
        Ok(Appended::Held(_)) => Ok(None),
        Err(JournalError::Database(fault)) => Err(fault),
        Err(other) => Err(decode(other.to_string())),
    }
}

/// The `flow.draft_changed` panel event an edit tells the site's conversation (spec 06 section 3.4): what the draft
/// holds in one plain sentence, which starts no turn.
fn changed(by: &str, checked: &DraftChecked) -> NewEntry {
    let changes = if checked.ops_total == 1 { "1 change".to_owned() } else { format!("{} changes", checked.ops_total) };
    let text = match checked.problems.iter().filter(|problem| problem.severity == Severity::Error).count() {
        0 => format!("{by}'s draft of this site holds {changes}, with no error."),
        1 => format!("{by}'s draft of this site holds {changes}, with 1 error."),
        errors => format!("{by}'s draft of this site holds {changes}, with {errors} errors."),
    };
    let body = json!({ "draft": checked.draft, "ops_total": checked.ops_total, "text": text });
    let event = PanelEvent { kind: PanelEventKind::FlowDraftChanged, text, body };
    NewEntry::new(JournalKind::PanelEvent, json!(event))
}

fn digest(ops: &[ChangesetOp]) -> String {
    let mut canonical = Vec::new();
    write_canonical(&json!(ops), &mut canonical);
    format!("{:x}", Sha256::digest(&canonical))
}

fn not_found(id: &str, instance: &str) -> ProblemAnswer {
    Problem::new(ProblemType::NotFound, format!("There is no draft {id}.")).at(instance).into()
}

fn refuse(type_: ProblemType, detail: String, instance: &str) -> ProblemAnswer {
    Problem::new(type_, detail).at(instance).into()
}

fn draft_in(path: Result<Path<String>, PathRejection>, instance: &str) -> Result<Ulid, ProblemAnswer> {
    let Ok(Path(id)) = path else { return Err(not_found("", instance)) };
    id.parse().map_err(|_| not_found(&id, instance))
}

/// The person's draft at `path`: a draft they may change.
async fn owned<S: Wps>(
    gate: &GateState<S>,
    path: Result<Path<String>, PathRejection>,
    identity: &Identity,
    instance: &str,
) -> Result<Draft, ProblemAnswer> {
    let id = draft_in(path, instance)?;
    let draft = (load(&gate.app.pool, &id).await)
        .map_err(|fault| internal("read the draft", fault, instance))?
        .ok_or_else(|| not_found(id.as_str(), instance))?;
    mine(&draft, identity, instance)?;
    Ok(draft)
}

fn mine(draft: &Draft, identity: &Identity, instance: &str) -> Result<(), ProblemAnswer> {
    if draft.created_by == identity.subject {
        return Ok(());
    }
    Err(refuse(ProblemType::NotDraftOwner, "Only the person who made this draft changes it.".into(), instance))
}

/// An open draft passes; one a press has claimed answers `draft-applying`, any other `stale-draft`.
fn open(draft: &Draft, instance: &str) -> Result<(), ProblemAnswer> {
    match draft.state {
        state if state.is_open() => Ok(()),
        DraftState::Applying => {
            let detail = "This draft is being applied; it takes no change until the apply ends.".to_owned();
            Err(refuse(ProblemType::DraftApplying, detail, instance))
        }
        state => {
            let detail = format!("This draft is {}; build a new one to change the site.", word(state));
            Err(refuse(ProblemType::StaleDraft, detail, instance))
        }
    }
}

/// `POST /api/drafts`: the person's draft on a site, made at the cut their panel read, with its first operations
/// folded and checked; a scope of the site that moved past the changeset the panel read it at refuses it `head-moved`,
/// since a draft applies against the head that was read (ruling D43) and never adopts a newer one.
async fn create<S: Wps>(
    State(gate): State<GateState<S>>,
    Extension(identity): Extension<Identity>,
    uri: Uri,
    body: Bytes,
) -> Result<(StatusCode, Json<DraftChecked>), ProblemAnswer> {
    let instance = uri.path();
    let NewDraft { site, seqs, summary, ops } = json_body(&body, instance)?;
    let location: Location = site.parse().map_err(|fault| validation_failed(fault, instance))?;
    let root: SitePath = location.scope().as_str().parse().map_err(|fault| validation_failed(fault, instance))?;
    if ops.is_empty() {
        return Err(validation_failed("a draft is made by its first edit, and `ops` holds none", instance));
    }
    let held: Option<(String, i32)> = sqlx::query_as(
        "SELECT id, revision FROM changeset_drafts \
         WHERE site = $1 AND created_by = $2 AND state IN ('drafting', 'ready', 'applying')",
    )
    .bind(root.as_str())
    .bind(identity.subject.as_str())
    .fetch_optional(&gate.app.pool)
    .await
    .map_err(|fault| internal("read the person's drafts", fault, instance))?;
    if let Some((held, revision)) = held {
        let detail = format!("You hold the open draft {held} on this site, at revision {revision}; amend it instead.");
        return Err(refuse(ProblemType::DraftChanged, detail, instance));
    }
    let world = &gate.world;
    let cut =
        world.resolve_cut(root.as_scope(), location.pin()).await.map_err(|error| wps_problem(&error, instance))?;
    let moved: Vec<MovedScope> = (cut.seqs.iter())
        .filter_map(|(scope, head)| {
            let base = seqs.get(scope).copied().unwrap_or(Seq::new(0));
            (base != *head).then(|| MovedScope { scope: scope.clone(), base, head: *head })
        })
        .collect();
    if !moved.is_empty() {
        let detail = "The site moved on after your panel read it; nothing was made. Read it again and make the change.";
        let mut problem = Problem::new(ProblemType::HeadMoved, detail);
        problem.moved = moved;
        return Err(problem.at(instance).into());
    }
    let document = base(&gate, &cut, instance).await?;
    let folded = fold(Vec::new(), ops, |id, pointer| value_at(&document, id, pointer));
    let id = mint_ulid();
    let checked =
        check(&gate, root.as_scope(), &cut, &document, &summary, &folded, id.as_str(), &identity.name, instance)
            .await?;
    let state = if checked.has_errors() { DraftState::Drafting } else { DraftState::Ready };
    let answer = DraftChecked {
        draft: id.clone(),
        revision: 1,
        state,
        ops_total: folded.len() as u32,
        parts: checked.parts,
        problems: checked.problems,
        minted: BTreeMap::new(),
    };
    let fault = |fault| internal("make the draft", fault, instance);
    let mut tx = gate.app.pool.begin().await.map_err(fault)?;
    sqlx::query(
        "INSERT INTO changeset_drafts (id, site, created_by, cut, state, summary, ops, ops_digest, revision, problems) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 1, $9)",
    )
    .bind(id.as_str())
    .bind(root.as_str())
    .bind(identity.subject.as_str())
    .bind(json!(cut))
    .bind(word(state))
    .bind(&summary)
    .bind(json!(folded))
    .bind(digest(&folded))
    .bind(json!(answer.problems))
    .execute(&mut *tx)
    .await
    .map_err(fault)?;
    let told =
        to_site(&gate, &mut tx, &root, &identity.subject, changed(&identity.name, &answer)).await.map_err(fault)?;
    tx.commit().await.map_err(fault)?;
    gate.app.hub.announce(told);
    Ok((StatusCode::CREATED, Json(answer)))
}

/// `PATCH /api/drafts/{d}`: more operations at the revision the person last read, folded and checked again on the
/// draft's own cut; a stale revision answers `draft-changed` and changes nothing.
async fn amend<S: Wps>(
    State(gate): State<GateState<S>>,
    Extension(identity): Extension<Identity>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
    body: Bytes,
) -> Result<Json<DraftChecked>, ProblemAnswer> {
    let instance = uri.path();
    let AmendDraft { revision, summary, ops } = json_body(&body, instance)?;
    let draft = owned(&gate, path, &identity, instance).await?;
    open(&draft, instance)?;
    if revision != draft.revision {
        let detail = format!("The draft is at revision {}; read it again and resend your edit.", draft.revision);
        return Err(refuse(ProblemType::DraftChanged, detail, instance));
    }
    let document = base(&gate, &draft.cut, instance).await?;
    let folded = fold(draft.ops.clone(), ops, |id, pointer| value_at(&document, id, pointer));
    let summary = summary.unwrap_or(draft.summary.clone());
    let operation = draft.id.as_str();
    let site = draft.site.as_scope();
    let checked =
        check(&gate, site, &draft.cut, &document, &summary, &folded, operation, &identity.name, instance).await?;
    let state = if checked.has_errors() { DraftState::Drafting } else { DraftState::Ready };
    let next = draft.revision + 1;
    let answer = DraftChecked {
        draft: draft.id.clone(),
        revision: next,
        state,
        ops_total: folded.len() as u32,
        parts: checked.parts,
        problems: checked.problems,
        minted: BTreeMap::new(),
    };
    let fault = |fault| internal("amend the draft", fault, instance);
    let mut tx = gate.app.pool.begin().await.map_err(fault)?;
    let updated = sqlx::query(
        "UPDATE changeset_drafts SET ops = $3, ops_digest = $4, revision = $5, summary = $6, problems = $7, \
             state = $8, updated_at = now() WHERE id = $1 AND revision = $2 AND state IN ('drafting', 'ready')",
    )
    .bind(draft.id.as_str())
    .bind(draft.revision as i32)
    .bind(json!(folded))
    .bind(digest(&folded))
    .bind(next as i32)
    .bind(&summary)
    .bind(json!(answer.problems))
    .bind(word(state))
    .execute(&mut *tx)
    .await
    .map_err(fault)?;
    if updated.rows_affected() == 0 {
        let detail = "The draft changed while your edit was checked; read it again and resend your edit.".to_owned();
        return Err(refuse(ProblemType::DraftChanged, detail, instance));
    }
    let told = to_site(&gate, &mut tx, &draft.site, &identity.subject, changed(&identity.name, &answer))
        .await
        .map_err(fault)?;
    tx.commit().await.map_err(fault)?;
    gate.app.hub.announce(told);
    Ok(Json(answer))
}

/// `GET /api/drafts/{d}`.
async fn read<S: Wps>(
    State(gate): State<GateState<S>>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<DraftRead>, ProblemAnswer> {
    let instance = uri.path();
    let id = draft_in(path, instance)?;
    let draft = (load(&gate.app.pool, &id).await)
        .map_err(|fault| internal("read the draft", fault, instance))?
        .ok_or_else(|| not_found(id.as_str(), instance))?;
    Ok(Json(draft.read()))
}

#[derive(Debug, Deserialize)]
struct ListQuery {
    site: String,
    mine: Option<u8>,
}

/// `GET /api/drafts?site=&mine=1`: the drafts on a site still open or being applied; with `mine=1` only the person's.
async fn list<S: Wps>(
    State(gate): State<GateState<S>>,
    Extension(identity): Extension<Identity>,
    uri: Uri,
    query: Result<Query<ListQuery>, QueryRejection>,
) -> Result<Json<DraftList>, ProblemAnswer> {
    let instance = uri.path();
    let Query(ListQuery { site, mine }) = query.map_err(|fault| validation_failed(fault, instance))?;
    let site: ScopePath = site.parse().map_err(|fault| validation_failed(fault, instance))?;
    let rows = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM changeset_drafts WHERE site = $1 AND state IN ('drafting', 'ready', 'applying') \
             AND ($2::text IS NULL OR created_by = $2) ORDER BY id"
    ))
    .bind(site.as_str())
    .bind((mine == Some(1)).then(|| identity.subject.as_str().to_owned()))
    .fetch_all(&gate.app.pool)
    .await
    .map_err(|fault| internal("list the drafts", fault, instance))?;
    let items = (rows.iter().map(from_row).collect::<Result<Vec<_>, _>>())
        .map_err(|fault| internal("read the drafts", fault, instance))?
        .iter()
        .map(Draft::read)
        .collect();
    Ok(Json(DraftList { items }))
}

/// `POST /api/drafts/{d}/discard`: the person drops their open draft, journaling `draft.discarded` in the same
/// transaction; one already discarded answers as it stands, and one a press has claimed answers `draft-applying`.
async fn discard<S: Wps>(
    State(gate): State<GateState<S>>,
    Extension(identity): Extension<Identity>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<DraftRead>, ProblemAnswer> {
    let instance = uri.path();
    let id = draft_in(path, instance)?;
    let fault = |fault| internal("discard the draft", fault, instance);
    let mut tx = gate.app.pool.begin().await.map_err(fault)?;
    let draft = locked(&mut tx, &id).await.map_err(fault)?.ok_or_else(|| not_found(id.as_str(), instance))?;
    mine(&draft, &identity, instance)?;
    if draft.state == DraftState::Discarded {
        return Ok(Json(draft.read()));
    }
    open(&draft, instance)?;
    sqlx::query("UPDATE changeset_drafts SET state = 'discarded', updated_at = now() WHERE id = $1")
        .bind(draft.id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(fault)?;
    let entry = NewEntry::new(JournalKind::DraftDiscarded, json!(DraftDiscarded { draft: draft.id.clone() }));
    let told = to_site(&gate, &mut tx, &draft.site, &identity.subject, entry).await.map_err(fault)?;
    tx.commit().await.map_err(fault)?;
    gate.app.hub.announce(told);
    Ok(Json(Draft { state: DraftState::Discarded, ..draft }.read()))
}
