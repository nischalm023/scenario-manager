//! Within `templates`, `drafts` are template drafts as their routes serve them (spec 05 sections 3.8, 3.12, 4.2 and
//! 4.3): starting one from a template at its head, reading it, a knob change that numbers the next evaluation under
//! the draft's row lock (refusing a knob someone set after the revision the change read, `knob-changed`, merging the
//! rest and superseding an unfinished evaluation), evaluating again at the same knobs, reading one evaluation, and
//! abandoning a draft. A draft keeps only the values a person or the agent set; every other knob follows its default,
//! an expression's worked out by each evaluation (D120).

use std::collections::BTreeMap;

use axum::Extension;
use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, State};
use axum::http::{StatusCode, Uri};
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sqlx::postgres::PgRow;
use sqlx::{Error as SqlxError, Postgres, Row, Transaction};
use studio_protocol::{
    BranchName, DeclaredId, DraftMode, DraftStarted, DraftWrite, EvaluationEngine, EvaluationEvent, EvaluationNumber,
    EvaluationPrologue, EvaluationRef, EvaluationState, FloorRef, KnobChange, KnobDeclaration, KnobSetBy, KnobSource,
    KnobValue, KnobsSet, LiveItem, LiveKind, Location, Person, Pin, Problem, ProblemItem, ProblemType, ScopePath, Seq,
    SetByKind, SetKnobs, Severity, StartDraft, TemplateAt, TemplateDraft, TemplateDraftState, TemplateEvaluation, Ulid,
    WallClock,
};
use wps_client::WorldStore;
use wps_client::wps::Wps;

use super::declarations::admits;
use super::evaluation::run;
use super::read::{ReadFault, read};
use super::words::knob_value;
use crate::GateState;
use crate::db::{decode, evaluation_column, evaluation_of, mint_ulid, read_word, word};
use crate::identity::Identity;
use crate::problem::{ProblemAnswer, internal, json_body, validation_failed, wps_problem};

/// `POST /api/template-drafts`: a draft of the template a location names, at each of its scopes' heads on main, no
/// knob set, evaluation 1 started.
pub async fn start<S: Wps>(
    State(gate): State<GateState<S>>,
    Extension(identity): Extension<Identity>,
    uri: Uri,
    body: Bytes,
) -> Result<(StatusCode, Json<DraftStarted>), ProblemAnswer> {
    let instance = uri.path();
    let StartDraft { template, site, conversation, .. } = json_body(&body, instance)?;
    if site.is_some() {
        return Err(validation_failed(
            "a draft from a made site (update) is not built yet; it waits for task 37",
            instance,
        ));
    }
    let Some(template) = template else {
        return Err(validation_failed("`template` names the template to start from", instance));
    };
    let location: Location = template.parse().map_err(|fault| validation_failed(fault, instance))?;
    let root = location.scope().clone();
    if root.slugs().count() != 3 || root.slugs().next() != Some("template") || root.slugs().nth(1) != Some("site") {
        return Err(validation_failed(format!("`{root}` is no site template's root, template/site/{{t}}"), instance));
    }
    if location.pin() != &Pin::Branch(BranchName::main()) {
        return Err(validation_failed("a template is started at @main until world-proto-server has tags", instance));
    }
    if let Some(conversation) = &conversation {
        thread_of(&gate, conversation, &identity, instance).await?;
    }
    let cut = gate.world.resolve_cut(&root, location.pin()).await.map_err(|error| wps_problem(&error, instance))?;
    if cut.seqs.values().any(|seq| *seq == Seq::new(0)) {
        let detail = format!("The template {root} holds nothing yet: a scope of it has no changeset.");
        return Err(Problem::new(ProblemType::NotFound, detail).at(instance).into());
    }
    let read = read(&gate.app.templates, &gate.world, &root, &cut.seqs)
        .await
        .map_err(|fault| read_problem(&fault, instance))?;
    let knobs: BTreeMap<String, KnobValue> = BTreeMap::new();
    let set_by: BTreeMap<String, KnobSetBy> = read
        .knobs
        .iter()
        .map(|knob| (knob.key.clone(), KnobSetBy { by: SetByKind::Template, turn: None, author: None, revision: 1 }))
        .collect();
    let draft = mint_ulid();
    let fault = |fault| internal("start the template draft", fault, instance);
    let mut tx = gate.app.pool.begin().await.map_err(fault)?;
    sqlx::query(
        "INSERT INTO template_drafts (id, conversation, template, template_seqs, doc, title, declarations, knobs, \
             set_by, latest_n, created_by) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 1, $10)",
    )
    .bind(draft.as_str())
    .bind(conversation.as_ref().map(Ulid::as_str))
    .bind(root.as_str())
    .bind(json!(cut.seqs))
    .bind(DeclaredId::mint().to_string())
    .bind(&read.description.title)
    .bind(json!(read.knobs))
    .bind(json!(knobs))
    .bind(json!(set_by))
    .bind(identity.subject.as_str())
    .execute(&mut *tx)
    .await
    .map_err(fault)?;
    sqlx::query("INSERT INTO template_evaluations (draft, n, revision, knobs) VALUES ($1, 1, 1, $2)")
        .bind(draft.as_str())
        .bind(json!(knobs))
        .execute(&mut *tx)
        .await
        .map_err(fault)?;
    tx.commit().await.map_err(fault)?;
    let first = EvaluationNumber(1);
    tokio::spawn(run(gate.clone(), draft.clone(), first));
    let evaluation = EvaluationRef { n: first, state: EvaluationState::InProgress };
    Ok((StatusCode::CREATED, Json(DraftStarted { draft, revision: 1, evaluation })))
}

/// The conversation a draft runs in: the asking person's live new-site thread, never a site's.
async fn thread_of<S: Wps>(
    gate: &GateState<S>,
    conversation: &Ulid,
    identity: &Identity,
    instance: &str,
) -> Result<(), ProblemAnswer> {
    let row: Option<(Option<String>, String, String)> =
        sqlx::query_as("SELECT site, state, created_by FROM conversations WHERE id = $1")
            .bind(conversation.as_str())
            .fetch_optional(&gate.app.pool)
            .await
            .map_err(|fault| internal("read the draft's conversation", fault, instance))?;
    match row {
        Some((None, state, by)) if state == "live" && by == identity.subject.as_str() => Ok(()),
        Some(_) => Err(validation_failed(
            format!("{conversation} is not your live New site conversation, which a template draft runs in"),
            instance,
        )),
        None => Err(Problem::new(ProblemType::NotFound, format!("There is no conversation {conversation}."))
            .at(instance)
            .into()),
    }
}

/// How a template read that failed is answered.
pub fn read_problem(fault: &ReadFault, instance: &str) -> ProblemAnswer {
    match fault {
        ReadFault::Wps(error) => wps_problem(error, instance),
        other => validation_failed(format!("This template cannot be read: {other}"), instance),
    }
}

/// A draft's id from a route's path; one that is no ULID names nothing.
pub fn draft_in(path: Result<Path<String>, PathRejection>, instance: &str) -> Result<Ulid, ProblemAnswer> {
    let Ok(Path(id)) = path else { return Err(no_draft(instance)) };
    id.parse().map_err(|_| no_draft(instance))
}

pub fn no_draft(instance: &str) -> ProblemAnswer {
    Problem::new(ProblemType::NotFound, "No template draft has that id.").at(instance).into()
}

/// A draft's row as its routes read it.
pub struct DraftRow {
    pub id: Ulid,
    pub conversation: Option<Ulid>,
    pub mode: DraftMode,
    pub template: ScopePath,
    pub template_seqs: BTreeMap<ScopePath, Seq>,
    pub title: String,
    pub declarations: Vec<KnobDeclaration>,
    pub knobs: BTreeMap<String, KnobValue>,
    pub set_by: BTreeMap<String, KnobSetBy>,
    pub revision: u32,
    pub latest_n: EvaluationNumber,
    pub state: TemplateDraftState,
    pub write_approval: Option<Ulid>,
    pub write_evaluation: Option<EvaluationNumber>,
    pub write_result: Option<Value>,
    pub write_problem: Option<Value>,
}

const DRAFT_COLUMNS: &str = "id, conversation, mode, template, template_seqs, title, declarations, knobs, \
    set_by, revision, latest_n, CASE WHEN state = 'open' AND expires_at < now() THEN 'expired' ELSE state END AS state, \
    write_approval, write_evaluation, write_result, write_problem";

fn draft_row(row: &PgRow) -> Result<DraftRow, SqlxError> {
    let json = |column: &str| row.try_get::<Value, _>(column);
    let ulid = |text: Option<String>| text.map(|text| text.parse::<Ulid>()).transpose().map_err(decode);
    Ok(DraftRow {
        id: row.try_get::<String, _>("id")?.parse().map_err(decode)?,
        conversation: ulid(row.try_get("conversation")?)?,
        mode: read_word(&row.try_get::<String, _>("mode")?)?,
        template: row.try_get::<String, _>("template")?.parse().map_err(decode)?,
        template_seqs: serde_json::from_value(json("template_seqs")?).map_err(decode)?,
        title: row.try_get("title")?,
        declarations: serde_json::from_value(json("declarations")?).map_err(decode)?,
        knobs: serde_json::from_value(json("knobs")?).map_err(decode)?,
        set_by: serde_json::from_value(json("set_by")?).map_err(decode)?,
        revision: row.try_get::<i32, _>("revision")? as u32,
        latest_n: evaluation_of(row.try_get("latest_n")?)?,
        state: read_word(&row.try_get::<String, _>("state")?)?,
        write_approval: ulid(row.try_get("write_approval")?)?,
        write_evaluation: row.try_get::<Option<i32>, _>("write_evaluation")?.map(evaluation_of).transpose()?,
        write_result: row.try_get("write_result")?,
        write_problem: row.try_get("write_problem")?,
    })
}

/// The draft `id` read under its row lock inside `tx`, or `None`.
pub async fn lock(tx: &mut Transaction<'_, Postgres>, id: &Ulid) -> Result<Option<DraftRow>, SqlxError> {
    let sql = format!("SELECT {DRAFT_COLUMNS} FROM template_drafts WHERE id = $1 FOR UPDATE");
    sqlx::query(&sql).bind(id.as_str()).fetch_optional(&mut **tx).await?.as_ref().map(draft_row).transpose()
}

/// `draft-not-open`, saying what the person can do next.
pub fn not_open(state: TemplateDraftState, instance: &str) -> ProblemAnswer {
    let next = match state {
        TemplateDraftState::Writing => "wait until its site is written",
        TemplateDraftState::Created | TemplateDraftState::Updated => "open the site it made",
        _ => "start a new draft from the template",
    };
    let detail = format!("This draft is {}: {next}.", word(state));
    Problem::new(ProblemType::DraftNotOpen, detail).at(instance).into()
}

/// `GET /api/template-drafts/{d}`.
pub async fn read_draft<S: Wps>(
    State(gate): State<GateState<S>>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<TemplateDraft>, ProblemAnswer> {
    let instance = uri.path();
    let id = draft_in(path, instance)?;
    let fault = |fault| internal("read the template draft", fault, instance);
    let sql = format!("SELECT {DRAFT_COLUMNS} FROM template_drafts WHERE id = $1");
    let row = sqlx::query(&sql).bind(id.as_str()).fetch_optional(&gate.app.pool).await.map_err(fault)?;
    let Some(row) = row else { return Err(no_draft(instance)) };
    let draft = draft_row(&row).map_err(fault)?;
    let latest: Option<(i32, String)> =
        sqlx::query_as("SELECT n, state FROM template_evaluations WHERE draft = $1 ORDER BY n DESC LIMIT 1")
            .bind(id.as_str())
            .fetch_optional(&gate.app.pool)
            .await
            .map_err(fault)?;
    let latest = latest
        .map(|(n, state)| Ok::<_, SqlxError>(EvaluationRef { n: evaluation_of(n)?, state: read_word(&state)? }))
        .transpose()
        .map_err(fault)?;
    let write = match (&draft.write_approval, draft.write_evaluation) {
        (Some(approval), Some(evaluation)) => Some(DraftWrite {
            approval: approval.clone(),
            evaluation,
            result: draft.write_result.clone().map(serde_json::from_value).transpose().map_err(|f| fault(decode(f)))?,
            problem: draft
                .write_problem
                .clone()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|f| fault(decode(f)))?,
        }),
        _ => None,
    };
    Ok(Json(TemplateDraft {
        draft: draft.id,
        state: draft.state,
        mode: draft.mode,
        revision: draft.revision,
        template: TemplateAt { ref_: draft.template, tag: None, seqs: draft.template_seqs },
        title: draft.title,
        conversation: draft.conversation,
        knobs: draft.knobs,
        set_by: draft.set_by,
        declarations: draft.declarations,
        latest,
        write,
    }))
}

/// Who sets a draft's knobs: a person in the Template panel, or the agent in turn `n` (spec 05 section 3.9's
/// `template.set_knobs`), whose changes the panel's `knob.changed` never tells back to it.
pub enum KnobSetter {
    Person,
    Agent { turn: u32 },
}

/// `PATCH /api/template-drafts/{d}`: set knob values read at `revision`, `null` returning a knob to its default; the
/// next evaluation starts at once.
pub async fn set_knobs<S: Wps>(
    State(gate): State<GateState<S>>,
    Extension(identity): Extension<Identity>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
    body: Bytes,
) -> Result<Json<KnobsSet>, ProblemAnswer> {
    let instance = uri.path();
    let id = draft_in(path, instance)?;
    let set: SetKnobs = json_body(&body, instance)?;
    set_knobs_as(&gate, &identity, id, set, KnobSetter::Person, instance).await
}

/// `set_knobs` for `setter`.
pub async fn set_knobs_as<S: Wps>(
    gate: &GateState<S>,
    identity: &Identity,
    id: Ulid,
    SetKnobs { revision, knobs }: SetKnobs,
    setter: KnobSetter,
    instance: &str,
) -> Result<Json<KnobsSet>, ProblemAnswer> {
    let fault = |fault| internal("change the template draft's knobs", fault, instance);
    let mut tx = gate.app.pool.begin().await.map_err(fault)?;
    let Some(mut draft) = lock(&mut tx, &id).await.map_err(fault)? else { return Err(no_draft(instance)) };
    if draft.state != TemplateDraftState::Open {
        return Err(not_open(draft.state, instance));
    }
    if revision > draft.revision {
        return Err(validation_failed(
            format!("revision {revision} is newer than the draft's, {}", draft.revision),
            instance,
        ));
    }
    let mut invalid = Vec::new();
    let mut taken = Vec::new();
    for (key, value) in &knobs {
        let Some(knob) = draft.declarations.iter().find(|knob| &knob.key == key) else {
            invalid.push(item("validation-failed", key, format!("The template has no knob `{key}`.")));
            continue;
        };
        if let Some(Err(why)) = value.map(|value| admits(knob, value)) {
            invalid.push(item("validation-failed", key, format!("{why}.")));
        }
        let set = &draft.set_by[key];
        if set.revision > revision {
            let by = match (&set.by, &set.author) {
                (SetByKind::Agent, _) => "the agent".to_owned(),
                (_, Some(author)) => author.name.clone(),
                _ => "the template".to_owned(),
            };
            let now = match draft.knobs.get(key) {
                Some(value) => format!("set to {}", knob_value(knob, *value)),
                None => "returned to its default".to_owned(),
            };
            let text = format!("{} was {now} by {by}; your change was not applied.", knob.label);
            taken.push(item("knob-changed", key, text));
        }
    }
    if !invalid.is_empty() {
        let mut problem = Problem::new(ProblemType::ValidationFailed, "A knob's value is outside its declaration.");
        problem.problems = invalid;
        return Err(problem.at(instance).into());
    }
    if !taken.is_empty() {
        let mut problem = Problem::new(ProblemType::KnobChanged, taken[0].text.clone());
        problem.problems = taken;
        return Err(problem.at(instance).into());
    }
    let next = draft.revision + 1;
    let author = Person { subject: identity.subject.clone(), name: identity.name.clone() };
    let mut changes = Vec::new();
    for (key, value) in &knobs {
        changes.push(KnobChange { knob: key.clone(), from: draft.knobs.get(key).copied(), to: *value });
        // A knob returned to its default is no longer set; who returned it, and when, stay for `knob-changed`.
        let (by, turn) = match (value, &setter) {
            (Some(value), KnobSetter::Person) => {
                draft.knobs.insert(key.clone(), *value);
                (SetByKind::User, None)
            }
            (Some(value), KnobSetter::Agent { turn }) => {
                draft.knobs.insert(key.clone(), *value);
                (SetByKind::Agent, Some(*turn))
            }
            (None, _) => {
                draft.knobs.remove(key);
                (SetByKind::Template, None)
            }
        };
        draft.set_by.insert(key.clone(), KnobSetBy { by, turn, author: Some(author.clone()), revision: next });
    }
    let by_person = matches!(setter, KnobSetter::Person);
    let told = if by_person { json!(changes) } else { json!([]) };
    let started = number_next(&mut tx, &draft, next, by_person).await.map_err(fault)?;
    sqlx::query(
        "UPDATE template_drafts SET knobs = $2, set_by = $3, revision = $4, \
             unjournaled = unjournaled || $5::jsonb, used_at = now(), expires_at = now() + interval '7 days' \
         WHERE id = $1",
    )
    .bind(id.as_str())
    .bind(json!(draft.knobs))
    .bind(json!(draft.set_by))
    .bind(next as i32)
    .bind(told)
    .execute(&mut *tx)
    .await
    .map_err(fault)?;
    tx.commit().await.map_err(fault)?;
    started.tell(gate, &draft);
    tokio::spawn(run(gate.clone(), id.clone(), started.n));
    let evaluation = EvaluationRef { n: started.n, state: EvaluationState::InProgress };
    Ok(Json(KnobsSet { draft: id, revision: next, evaluation }))
}

fn item(code: &str, knob: &str, text: String) -> ProblemItem {
    ProblemItem {
        code: code.into(),
        severity: Severity::Error,
        ref_: None,
        scope: None,
        field: None,
        knob: Some(knob.to_owned()),
        text,
    }
}

/// The evaluation a change numbered, and those it superseded, each with the revision it read.
pub struct Numbered {
    pub n: EvaluationNumber,
    superseded: Vec<(EvaluationNumber, u32)>,
}

impl Numbered {
    /// Tell the draft's conversation which evaluations a later change superseded.
    pub fn tell<S: Wps>(&self, gate: &GateState<S>, draft: &DraftRow) {
        let Some(conversation) = &draft.conversation else { return };
        for (n, revision) in &self.superseded {
            let event = EvaluationEvent {
                draft: draft.id.clone(),
                n: *n,
                revision: *revision,
                state: EvaluationState::Superseded,
                problems: Vec::new(),
            };
            let _ =
                gate.app.hub.live(conversation, LiveItem { kind: LiveKind::TemplateEvaluation, body: json!(event) });
        }
    }
}

/// Inside the transaction holding the draft's lock: supersede its unfinished evaluation, and number the next at
/// `revision`'s knob values, the draft's newest.
pub async fn number_next(
    tx: &mut Transaction<'_, Postgres>,
    draft: &DraftRow,
    revision: u32,
    by_person: bool,
) -> Result<Numbered, SqlxError> {
    let superseded: Vec<(i32, i32)> = sqlx::query_as(
        "UPDATE template_evaluations SET state = 'superseded' WHERE draft = $1 AND state = 'in-progress' \
         RETURNING n, revision",
    )
    .bind(draft.id.as_str())
    .fetch_all(&mut **tx)
    .await?;
    let n = EvaluationNumber(draft.latest_n.0 + 1);
    sqlx::query("INSERT INTO template_evaluations (draft, n, revision, knobs, by_person) VALUES ($1, $2, $3, $4, $5)")
        .bind(draft.id.as_str())
        .bind(evaluation_column(n)?)
        .bind(revision as i32)
        .bind(json!(draft.knobs))
        .bind(by_person)
        .execute(&mut **tx)
        .await?;
    sqlx::query("UPDATE template_drafts SET latest_n = $2 WHERE id = $1")
        .bind(draft.id.as_str())
        .bind(evaluation_column(n)?)
        .execute(&mut **tx)
        .await?;
    let superseded = superseded
        .into_iter()
        .map(|(n, revision)| Ok((evaluation_of(n)?, revision as u32)))
        .collect::<Result<_, SqlxError>>()?;
    Ok(Numbered { n, superseded })
}

/// `POST /api/template-drafts/{d}/evaluate`: evaluate again at the same knobs (the agent's `template.evaluate`).
pub async fn again<S: Wps>(
    State(gate): State<GateState<S>>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
) -> Result<(StatusCode, Json<KnobsSet>), ProblemAnswer> {
    let instance = uri.path();
    let id = draft_in(path, instance)?;
    let fault = |fault| internal("evaluate the template draft again", fault, instance);
    let mut tx = gate.app.pool.begin().await.map_err(fault)?;
    let Some(draft) = lock(&mut tx, &id).await.map_err(fault)? else { return Err(no_draft(instance)) };
    if draft.state != TemplateDraftState::Open {
        return Err(not_open(draft.state, instance));
    }
    let started = number_next(&mut tx, &draft, draft.revision, false).await.map_err(fault)?;
    tx.commit().await.map_err(fault)?;
    started.tell(&gate, &draft);
    tokio::spawn(run(gate.clone(), id.clone(), started.n));
    let evaluation = EvaluationRef { n: started.n, state: EvaluationState::InProgress };
    Ok((StatusCode::ACCEPTED, Json(KnobsSet { draft: id, revision: draft.revision, evaluation })))
}

/// `GET /api/template-drafts/{d}/evaluations/{n}`.
pub async fn evaluation<S: Wps>(
    State(gate): State<GateState<S>>,
    uri: Uri,
    path: Result<Path<(String, String)>, PathRejection>,
) -> Result<Json<TemplateEvaluation>, ProblemAnswer> {
    let instance = uri.path();
    let nothing = || ProblemAnswer::from(Problem::new(ProblemType::NotFound, "No such evaluation.").at(instance));
    let Ok(Path((id, n))) = path else { return Err(nothing()) };
    let (Ok(id), Ok(n)) = (id.parse::<Ulid>(), n.parse::<u32>().map(EvaluationNumber)) else { return Err(nothing()) };
    let fault = |fault| internal("read the evaluation", fault, instance);
    let row = sqlx::query(
        "SELECT e.*, d.mode, d.template, d.template_seqs FROM template_evaluations e \
         JOIN template_drafts d ON d.id = e.draft WHERE e.draft = $1 AND e.n = $2",
    )
    .bind(id.as_str())
    .bind(evaluation_column(n).map_err(fault)?)
    .fetch_optional(&gate.app.pool)
    .await
    .map_err(fault)?
    .ok_or_else(nothing)?;
    let json = |column: &str| row.try_get::<Option<Value>, _>(column);
    let floor = match (row.try_get::<Option<String>, _>("floor_key"), row.try_get::<Option<i64>, _>("floor_bytes")) {
        (Ok(Some(key)), Ok(Some(bytes))) => Some(FloorRef {
            key,
            floor_version: row.try_get::<Option<i32>, _>("floor_version").ok().flatten().unwrap_or_default() as u32,
            bytes: bytes as u64,
        }),
        _ => None,
    };
    let build = || -> Result<TemplateEvaluation, SqlxError> {
        let set: BTreeMap<String, KnobValue> = serde_json::from_value(row.try_get("knobs")?).map_err(decode)?;
        let sources = match json("knob_sources")? {
            Some(sources) => serde_json::from_value(sources).map_err(decode)?,
            None => set.keys().map(|key| (key.clone(), KnobSource::Set)).collect(),
        };
        let knobs = match json("knob_values")? {
            Some(values) => serde_json::from_value(values).map_err(decode)?,
            None => set,
        };
        Ok(TemplateEvaluation {
            draft: id.clone(),
            n,
            state: read_word(&row.try_get::<String, _>("state")?)?,
            mode: read_word(&row.try_get::<String, _>("mode")?)?,
            revision: row.try_get::<i32, _>("revision")? as u32,
            knobs,
            sources,
            template: TemplateAt {
                ref_: row.try_get::<String, _>("template")?.parse().map_err(decode)?,
                tag: None,
                seqs: serde_json::from_value(row.try_get("template_seqs")?).map_err(decode)?,
            },
            prologue: decoded::<EvaluationPrologue>(json("prologue")?)?.filter(|prologue| !prologue.doc.is_empty()),
            engine: decoded::<EvaluationEngine>(json("engine")?)?.filter(|engine| !engine.version.is_empty()),
            census: decoded(json("census")?)?,
            changes_from_template: decoded(json("changes_from_template")?)?,
            problems: serde_json::from_value(row.try_get("problems")?).map_err(decode)?,
            notes: serde_json::from_value(row.try_get("notes")?).map_err(decode)?,
            diff: json("diff")?,
            floor,
            axes: decoded(json("axes")?)?,
            started_at: WallClock::from(row.try_get::<DateTime<Utc>, _>("started_at")?),
            finished_at: row.try_get::<Option<DateTime<Utc>>, _>("finished_at")?.map(WallClock::from),
        })
    };
    build().map(Json).map_err(fault)
}

/// `DELETE /api/template-drafts/{d}`: abandon a draft, which binds nothing; refused while it writes.
pub async fn abandon<S: Wps>(
    State(gate): State<GateState<S>>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<TemplateDraftState>, ProblemAnswer> {
    let instance = uri.path();
    let id = draft_in(path, instance)?;
    let fault = |fault| internal("abandon the template draft", fault, instance);
    let mut tx = gate.app.pool.begin().await.map_err(fault)?;
    let Some(draft) = lock(&mut tx, &id).await.map_err(fault)? else { return Err(no_draft(instance)) };
    match draft.state {
        TemplateDraftState::Abandoned => return Ok(Json(TemplateDraftState::Abandoned)),
        TemplateDraftState::Open | TemplateDraftState::Expired => {}
        other => return Err(not_open(other, instance)),
    }
    sqlx::query("UPDATE template_evaluations SET state = 'superseded' WHERE draft = $1 AND state = 'in-progress'")
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(fault)?;
    sqlx::query("UPDATE template_drafts SET state = 'abandoned' WHERE id = $1")
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(fault)?;
    tx.commit().await.map_err(fault)?;
    Ok(Json(TemplateDraftState::Abandoned))
}

/// A JSON column read as `T`, `None` when the column is null.
fn decoded<T: DeserializeOwned>(value: Option<Value>) -> Result<Option<T>, SqlxError> {
    value.map(serde_json::from_value).transpose().map_err(decode)
}
