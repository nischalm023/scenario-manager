//! Within `templates`, `create` is Create site (spec 05 section 4.4; spec 01 section 3.10's rule 6 and section 3.17's
//! binding): a person's press, recorded as their approval, makes an evaluation a site in ONE changeset, which
//! world-proto-server applies whole or refuses whole. Under the draft's row lock the press is checked (the draft's
//! latest ready evaluation without an error, the generators it ran still the registry's, the site's path free), the
//! site's plan is built from the evaluation's kept plan and kept with the approval, its one changeset with the draft,
//! and the draft moves to `writing`; the write follows the `202`, run by the approval's one worker (LEAD-DECISIONS
//! entry 54). Once the site lands, its made-from record, the draft's `created` and the thread's binding commit in one
//! transaction. Only world-proto-server's own refusal of the kept changeset returns the draft to `open` with its
//! problem; any other failure leaves the outcome unknown, and the kept changeset goes again as it was, which WPS
//! answers by replaying the site if it landed, within a window of wall clock and again at the next start.

use std::fmt::Display;

use axum::Extension;
use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, State};
use axum::http::{StatusCode, Uri};
use runstore::RunStorage;
use serde_json::{Value, json};
use sqlx::{Error as SqlxError, Postgres, Row, Transaction};
use studio_protocol::{
    ApprovalFacts, CreateSite, DraftMode, EvaluationEngine, EvaluationNumber, EvaluationProblem, EvaluationState,
    Execution, FactRow, Person, Problem, ProblemType, Severity, SiteMade, SitePath, TemplateDraftState, Ulid,
    WriteAccepted,
};
use wps_client::site::{SitePlan, create_site, site_changeset};
use wps_client::wire::WireChangeset;
use wps_client::wps::Wps;
use wps_client::{Applied, Author, NotLanded, WorldStore, WpsError};

use super::drafts::{DraftRow, draft_in, lock, no_draft, not_open, number_next};
use super::evaluation::{generators_used, run};
use super::plan::PlanFile;
use crate::GateState;
use crate::agent::press_decides;
use crate::approvals::{Asks, Moved, Press, approval_record, claim, executed, record_button, record_sent};
use crate::conversations::{ConversationError, bind_thread};
use crate::db::{decode, evaluation_column, mint_ulid, word};
use crate::identity::Identity;
use crate::journal::Written;
use crate::problem::{ProblemAnswer, internal, json_body, validation_failed, wps_problem};

/// How many times a body refused for a scope another writer made is built again before that refusal stands.
const REBUILDS: u32 = 4;

/// What a press writes: the evaluation's kept plan, its engine stamp and every knob's value.
struct Writable {
    plan_key: String,
    engine: EvaluationEngine,
    knobs: Value,
}

/// `POST /api/template-drafts/{d}/site`: Create site from evaluation n, answered `202` once the press is recorded. A
/// press while the agent's approval of the same act waits decides that approval (spec 01 section 3.10, rule 6).
pub async fn create<S: Wps>(
    State(gate): State<GateState<S>>,
    Extension(identity): Extension<Identity>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
    body: Bytes,
) -> Result<(StatusCode, Json<WriteAccepted>), ProblemAnswer> {
    let instance = uri.path();
    let id = draft_in(path, instance)?;
    let create: CreateSite = json_body(&body, instance)?;
    let fault = |fault: &dyn Display| internal("create the site", fault, instance);
    let thread: Option<Option<String>> = sqlx::query_scalar("SELECT conversation FROM template_drafts WHERE id = $1")
        .bind(id.as_str())
        .fetch_optional(&gate.app.pool)
        .await
        .map_err(|f| fault(&f))?;
    let thread: Option<Ulid> = thread.flatten().and_then(|thread| thread.parse().ok());
    let act = json!({ "draft": id, "evaluation": create.evaluation, "customer": create.customer, "slug": create.slug,
                      "name": create.name.trim() });
    let decided =
        press_decides(&gate, thread.as_ref(), "site.create_from_template", &act, &identity, "Create site", instance)
            .await?;
    if let Some(approval) = decided {
        let mut tx = gate.app.pool.begin().await.map_err(|f| fault(&f))?;
        let answer = accepted(&mut tx, &approval, id, instance).await?;
        tx.commit().await.map_err(|f| fault(&f))?;
        return Ok(answer);
    }
    create_under(&gate, &identity, id, create, None, instance).await
}

/// Create site from `create` for `identity`: under `approval` when one already approved it (the agent's card, approved
/// before its stored input executes), else under the press's own approval, recorded here.
pub async fn create_under<S: Wps>(
    gate: &GateState<S>,
    identity: &Identity,
    id: Ulid,
    CreateSite { customer, slug, name, evaluation: n, .. }: CreateSite,
    approval: Option<Ulid>,
    instance: &str,
) -> Result<(StatusCode, Json<WriteAccepted>), ProblemAnswer> {
    let site: SitePath =
        format!("customer/{customer}/site/{slug}").parse().map_err(|fault| validation_failed(fault, instance))?;
    let name = name.trim();
    if name.is_empty() {
        return Err(validation_failed("`name` is the site's name, which it needs", instance));
    }
    let fault = |fault: &dyn Display| internal("create the site", fault, instance);
    let mut tx = gate.app.pool.begin().await.map_err(|f| fault(&f))?;
    let Some(draft) = lock(&mut tx, &id).await.map_err(|f| fault(&f))? else { return Err(no_draft(instance)) };
    match (draft.state, &draft.write_approval) {
        (TemplateDraftState::Writing, Some(writing)) if approval.as_ref().is_none_or(|ours| ours == writing) => {
            return accepted(&mut tx, writing, id, instance).await;
        }
        (TemplateDraftState::Writing, Some(_)) => {
            let detail = "Another approval is already writing a site from this template draft.";
            return Err(Problem::new(ProblemType::DraftNotOpen, detail).at(instance).into());
        }
        (TemplateDraftState::Open, _) if draft.mode == DraftMode::Create => {}
        (TemplateDraftState::Open, _) => {
            return Err(validation_failed("an update draft writes with …/update", instance));
        }
        (state, _) => return Err(not_open(state, instance)),
    }
    let Some(writable) = writable(&mut tx, &draft, n).await.map_err(|f| fault(&f))? else {
        let detail = format!("Evaluation {n} is not this draft's latest ready evaluation without an error.");
        return Err(Problem::new(ProblemType::EvaluationStale, detail).at(instance).into());
    };
    let registry = gate.world.registry().snapshot();
    if generators_used(&registry.generators) != writable.engine.generators {
        let started = number_next(&mut tx, &draft, draft.revision, false).await.map_err(|f| fault(&f))?;
        tx.commit().await.map_err(|f| fault(&f))?;
        started.tell(gate, &draft);
        tokio::spawn(run(gate.clone(), id.clone(), started.n));
        let detail = format!(
            "The generators that derive the site changed after evaluation {n}; evaluation {} runs with them.",
            started.n
        );
        return Err(Problem::new(ProblemType::GeneratorsMoved, detail).at(instance).into());
    }
    let kept = gate.app.runstore.get(&writable.plan_key).await.map_err(|f| fault(&f))?;
    let kept: PlanFile = serde_json::from_slice(&kept).map_err(|f| fault(&f))?;
    let press = Press {
        conversation: draft.conversation.as_ref(),
        tool: "site.create_from_template",
        class: "apply",
        input: json!({ "draft": id, "evaluation": n, "customer": customer, "slug": slug, "name": name }),
        digest: None,
        summary: format!("Create the site {name} at {site} from {}, evaluation {n}", draft.template),
        facts: facts(&draft, &site, name, n, &writable.knobs),
        by: &identity.subject,
    };
    let (approval, taken) = match approval {
        Some(approval) => {
            let taken = take(&mut tx, &approval, instance).await?;
            (approval, taken)
        }
        None => (record_button(&mut tx, press).await.map_err(|f| fault(&f))?, None),
    };
    let author = Author::new(identity.email.clone()).map_err(|f| fault(&f))?;
    let seqs: Vec<String> = draft.template_seqs.iter().map(|(scope, seq)| format!("{scope} at {seq}")).collect();
    let message = format!(
        "Agent Studio: Create site from {} ({}), evaluation {n} of template draft {id}",
        draft.template,
        seqs.join(", ")
    );
    let generators = kept.generators(&registry);
    let plan = kept
        .site_plan(&site, name, author, message, approval.as_str(), generators)
        .map_err(|error| wps_problem(&error, instance))?;
    let body = site_changeset(&gate.world, &plan).await.map_err(|error| wps_problem(&error, instance))?;
    record_sent(&mut tx, &approval, &json!(plan)).await.map_err(|f| fault(&f))?;
    sqlx::query(
        "UPDATE template_drafts SET state = 'writing', write_approval = $2, write_evaluation = $3, write_body = $4, \
             write_result = NULL, write_problem = NULL, used_at = now(), expires_at = now() + interval '7 days' \
         WHERE id = $1",
    )
    .bind(id.as_str())
    .bind(approval.as_str())
    .bind(evaluation_column(n).map_err(|f| fault(&f))?)
    .bind(json!(body))
    .execute(&mut *tx)
    .await
    .map_err(|f| fault(&f))?;
    let answer = accepted(&mut tx, &approval, id.clone(), instance).await?;
    tx.commit().await.map_err(|f| fault(&f))?;
    gate.app.hub.announce(taken);
    tokio::spawn(write(gate.clone(), id, approval));
    Ok(answer)
}

/// The agent's approved card taking its act in the transaction that reserves the draft (spec 01 section 3.10's
/// execution states): its execution `running` there, after its conversation's lock (the draft's, then the
/// conversation's, then the approval's), so an act refused before this commit leaves it `not_started` for the agent
/// loop to settle, and one past it is the worker's alone; refused when the loop settled it first.
async fn take(
    tx: &mut Transaction<'_, Postgres>,
    approval: &Ulid,
    instance: &str,
) -> Result<Option<Written>, ProblemAnswer> {
    let fault = |fault: SqlxError| internal("take the approval's act", fault, instance);
    sqlx::query("SELECT 1 FROM conversations WHERE id = (SELECT conversation FROM approvals WHERE id = $1) FOR UPDATE")
        .bind(approval.as_str())
        .execute(&mut **tx)
        .await
        .map_err(fault)?;
    match executed(tx, approval, Execution::Running, None, None).await.map_err(fault)? {
        Some(Moved(entry)) => Ok(entry),
        None => Err(Problem::new(ProblemType::ApprovalClosed, "This approval's act was already settled.")
            .at(instance)
            .into()),
    }
}

async fn accepted(
    tx: &mut Transaction<'_, Postgres>,
    approval: &Ulid,
    draft: Ulid,
    instance: &str,
) -> Result<(StatusCode, Json<WriteAccepted>), ProblemAnswer> {
    let fault = |fault: &dyn Display| internal("answer the press", fault, instance);
    let record = approval_record(tx, approval).await.map_err(|f| fault(&f))?;
    let approval = record.ok_or_else(|| fault(&format!("approval {approval} is not recorded")))?;
    Ok((StatusCode::ACCEPTED, Json(WriteAccepted { approval, draft })))
}

/// Evaluation `n` when it is the draft's latest, ready and without an error: the only one Create site writes (spec
/// 05 section 3.12).
async fn writable(
    tx: &mut Transaction<'_, Postgres>,
    draft: &DraftRow,
    n: EvaluationNumber,
) -> Result<Option<Writable>, SqlxError> {
    if n != draft.latest_n {
        return Ok(None);
    }
    let row = sqlx::query(
        "SELECT state, problems, plan_key, engine, knob_values FROM template_evaluations WHERE draft = $1 AND n = $2",
    )
    .bind(draft.id.as_str())
    .bind(evaluation_column(n)?)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let kept = (row.try_get("plan_key")?, row.try_get("engine")?, row.try_get("knob_values")?);
    let (Some(plan_key), Some(engine), Some(knobs)): (Option<String>, Option<Value>, Option<Value>) = kept else {
        return Ok(None);
    };
    let state: String = row.try_get("state")?;
    let problems: Vec<EvaluationProblem> = serde_json::from_value(row.try_get("problems")?).map_err(decode)?;
    if state != word(EvaluationState::Ready) || problems.iter().any(|problem| problem.severity == Severity::Error) {
        return Ok(None);
    }
    Ok(Some(Writable { plan_key, engine: serde_json::from_value(engine).map_err(decode)?, knobs }))
}

/// The facts a card would show for this act, kept with the approval though a button's draws no card: the template at
/// its changesets, the evaluation and every knob's value.
fn facts(draft: &DraftRow, site: &SitePath, name: &str, n: EvaluationNumber, knobs: &Value) -> Value {
    let row = |label: &str, value: String| FactRow { label: label.to_owned(), value };
    let mut display: Vec<FactRow> =
        draft.template_seqs.iter().map(|(scope, seq)| row("Template", format!("{scope} at changeset {seq}"))).collect();
    display.push(row("Evaluation", n.to_string()));
    for (key, value) in knobs.as_object().into_iter().flatten() {
        display.push(row("Knob", format!("{key} = {value}")));
    }
    let lead = format!("Create the site {name} at {site}");
    json!(ApprovalFacts { lead, display, notes: Vec::new(), action_label: "Create site".to_owned(), footnote: None })
}

/// Write the site `approval` keeps the plan of to its end, as the approval's one worker in this process: made once it
/// lands, the draft `open` again on world-proto-server's own refusal, and `writing` while the outcome is unknown.
pub async fn write<S: Wps>(gate: GateState<S>, draft: Ulid, approval: Ulid) {
    let Some(_holding) = gate.app.executing.hold(&approval) else { return };
    if let Err(fault) = attempt(&gate, &draft, &approval).await {
        eprintln!("studio-api: Create site of draft {draft} under approval {approval} stopped: {fault}");
    }
}

async fn attempt<S: Wps>(gate: &GateState<S>, draft: &Ulid, approval: &Ulid) -> Result<(), SqlxError> {
    if !claim(&gate.app, approval).await? {
        return Ok(());
    }
    let (plan, body): (Option<Value>, Option<Value>) =
        sqlx::query_as("SELECT a.sent, d.write_body FROM approvals a, template_drafts d WHERE a.id = $1 AND d.id = $2")
            .bind(approval.as_str())
            .bind(draft.as_str())
            .fetch_one(&gate.app.pool)
            .await?;
    let plan: SitePlan = serde_json::from_value(plan.unwrap_or_default()).map_err(decode)?;
    let mut body: WireChangeset = serde_json::from_value(body.unwrap_or_default()).map_err(decode)?;
    let (mut asks, mut rebuilds) = (Asks::new(), 0);
    loop {
        let unsettled = match create_site(&gate.world, &plan, &body).await {
            Ok(applied) => return made(gate, draft, approval, &plan, &applied).await,
            // WPS refused the kept body for a scope it creates that exists, storing nothing under its identifier: the
            // body is built again over the scopes as they stand, past a prefix another writer made, and kept in place
            // of the refused one before it is sent; a root that stands refuses the building, `site-exists`.
            Err(NotLanded::Refused(WpsError::ScopeExists { .. })) if rebuilds < REBUILDS => {
                rebuilds += 1;
                match site_changeset(&gate.world, &plan).await {
                    Ok(again) => {
                        keep(gate, draft, approval, &again).await?;
                        body = again;
                        continue;
                    }
                    Err(taken @ WpsError::ScopeExists { .. }) => return refused(gate, draft, approval, &taken).await,
                    Err(unread) => unread,
                }
            }
            Err(NotLanded::Refused(refusal)) => return refused(gate, draft, approval, &refusal).await,
            Err(NotLanded::Unknown(unknown)) => {
                not_yet_known(gate, approval, &unknown).await?;
                unknown
            }
        };
        if !asks.wait().await {
            eprintln!(
                "studio-api: Create site of draft {draft} under approval {approval} is not settled ({unsettled}); \
                 the next start asks again"
            );
            return Ok(());
        }
    }
}

async fn keep<S: Wps>(
    gate: &GateState<S>,
    draft: &Ulid,
    approval: &Ulid,
    body: &WireChangeset,
) -> Result<(), SqlxError> {
    sqlx::query("UPDATE template_drafts SET write_body = $2 WHERE id = $1 AND write_approval = $3")
        .bind(draft.as_str())
        .bind(json!(body))
        .bind(approval.as_str())
        .execute(&gate.app.pool)
        .await?;
    Ok(())
}

/// The write's outcome is not known: `running` becomes `unknown` with the sentence a person reads, once.
async fn not_yet_known<S: Wps>(gate: &GateState<S>, approval: &Ulid, unknown: &WpsError) -> Result<(), SqlxError> {
    let mut tx = gate.app.pool.begin().await?;
    let moved = executed(&mut tx, approval, Execution::Unknown, Some(&unknown.to_string()), None).await?;
    tx.commit().await?;
    if let Some(Moved(entry)) = moved {
        gate.app.hub.announce(entry);
    }
    Ok(())
}

/// The draft under its row lock while it writes under `approval`; `None` once another ask settled it.
async fn writing(
    tx: &mut Transaction<'_, Postgres>,
    draft: &Ulid,
    approval: &Ulid,
) -> Result<Option<DraftRow>, SqlxError> {
    let row = lock(tx, draft).await?;
    Ok(row.filter(|row| row.state == TemplateDraftState::Writing && row.write_approval.as_ref() == Some(approval)))
}

/// The site landed: in one transaction its made-from record `made` with every knob's value the evaluation took, the
/// draft `created` with what it made, the draft's thread bound to the site and `approval.execution` `ok`. Until it
/// commits the root is no site (`sites`), so the binding cannot meet a conversation someone opened on it.
async fn made<S: Wps>(
    gate: &GateState<S>,
    draft: &Ulid,
    approval: &Ulid,
    plan: &SitePlan,
    applied: &Applied,
) -> Result<(), SqlxError> {
    let mut tx = gate.app.pool.begin().await?;
    let Some(row) = writing(&mut tx, draft, approval).await? else { return Ok(()) };
    let site: SitePath = plan.root.as_str().parse().map_err(decode)?;
    let made_from = mint_ulid();
    let (subject, name): (String, Option<String>) = sqlx::query_as(
        "INSERT INTO site_made_from (id, site, state, act, template, template_seqs, knobs, site_seqs, draft, \
             evaluation, approval, author) \
         SELECT $1, $2, 'made', 'create', $3, $4, e.knob_values, $5, $6, e.n, a.id, a.decided_by \
         FROM template_evaluations e, approvals a WHERE e.draft = $6 AND e.n = $7 AND a.id = $8 \
         RETURNING author, (SELECT p.name FROM people p WHERE p.subject = author)",
    )
    .bind(made_from.as_str())
    .bind(site.as_str())
    .bind(row.template.as_str())
    .bind(json!(row.template_seqs))
    .bind(json!(applied.seqs))
    .bind(draft.as_str())
    .bind(row.write_evaluation.map(evaluation_column).transpose()?)
    .bind(approval.as_str())
    .fetch_one(&mut *tx)
    .await?;
    let result = SiteMade { site: site.clone(), seqs: applied.seqs.clone(), made_from };
    sqlx::query("UPDATE template_drafts SET state = 'created', write_result = $2 WHERE id = $1")
        .bind(draft.as_str())
        .bind(json!(result))
        .execute(&mut *tx)
        .await?;
    let mut written = Vec::new();
    if let Some(thread) = &row.conversation {
        let by = Person { name: name.unwrap_or_else(|| subject.clone()), subject: subject.parse().map_err(decode)? };
        let title = plan.prologues.get(&plan.root).and_then(|prologue| prologue.name.clone()).unwrap_or_default();
        match bind_thread(&mut tx, thread, &site, &title, &by).await {
            Ok(entry) => written.push(entry),
            // A thread its person discarded, or one bound already to a site another of its drafts made, takes no
            // binding; the site is made all the same.
            Err(ConversationError::NotFound(_) | ConversationError::NotAThread(_)) => {}
            Err(ConversationError::Database(fault)) => return Err(fault),
            // Nobody can open the conversation of a root still being made, which is no site yet (spec 01 section
            // 3.17), so a site holding one already is a fault: the landing fails whole, nothing committed.
            Err(refused) => return Err(decode(format!("{site} landed, and its landing failed: {refused}"))),
        }
    }
    let Some(Moved(entry)) = executed(&mut tx, approval, Execution::Ok, None, Some(&json!(result))).await? else {
        return Err(decode(format!("approval {approval} was settled while draft {draft} still wrote under it")));
    };
    written.extend(entry);
    tx.commit().await?;
    gate.app.hub.announce(written);
    Ok(())
}

/// World-proto-server refused the site and wrote nothing: the draft `open` again with the problem,
/// `approval.execution` `failed` with its sentence, and, when the generators moved, the next evaluation started
/// (spec 05 section 4.4, step 2).
async fn refused<S: Wps>(
    gate: &GateState<S>,
    draft: &Ulid,
    approval: &Ulid,
    error: &WpsError,
) -> Result<(), SqlxError> {
    let problem = wps_problem(error, &format!("/api/template-drafts/{draft}/site")).0;
    let mut tx = gate.app.pool.begin().await?;
    let Some(row) = writing(&mut tx, draft, approval).await? else { return Ok(()) };
    sqlx::query("UPDATE template_drafts SET state = 'open', write_problem = $2 WHERE id = $1")
        .bind(draft.as_str())
        .bind(json!(problem))
        .execute(&mut *tx)
        .await?;
    let started = match problem.type_ {
        ProblemType::GeneratorsMoved => Some(number_next(&mut tx, &row, row.revision, false).await?),
        _ => None,
    };
    let failed = executed(&mut tx, approval, Execution::Failed, Some(&problem.detail), Some(&json!(problem))).await?;
    let Some(Moved(entry)) = failed else {
        return Err(decode(format!("approval {approval} was settled while draft {draft} still wrote under it")));
    };
    tx.commit().await?;
    gate.app.hub.announce(entry);
    if let Some(started) = started {
        started.tell(gate, &row);
        tokio::spawn(run(gate.clone(), draft.clone(), started.n));
    }
    Ok(())
}
