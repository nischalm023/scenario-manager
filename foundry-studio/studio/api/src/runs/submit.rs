//! Within `runs`, `submit` is spec 08 section 3.3's execution of a ready plan (rule 8), which a person's press of Run
//! approves (spec 01 section 3.10, rule 6), in two steps a `202` divides. The press: under the plan's row lock it checks
//! the hash the person saw, that the plan is ready, that its experiment has no other press or submission and that the
//! scenario version still stands at the plan's revision, then records the press as their approval and keeps it on the
//! plan, so a press repeated or racing answers that approval. The execution, the approval's one worker (LEAD-DECISIONS
//! entry 54), in a task of its own: it claims the approval, then in one transaction seals the version (spec 07 section
//! 3.5, which re-checks the revision under its lock), inserts the submission, its runs and one queued attempt each, and
//! records the execution `ok`, or `failed` with the refusal's words, every move forward only. It reads nothing from
//! world-proto-server and writes no object, the runner reading at the pins once its Job starts. A start resumes every
//! execution a stopped process left unsettled.

use std::fmt::Display;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{StatusCode, Uri};
use axum::{Extension, Json};
use serde_json::{Value, json};
use sqlx::{Error as SqlxError, Postgres, Row, Transaction};
use studio_protocol::{
    ApprovalAnswer, ApprovalFacts, ApprovalRecord, Execution, FactRow, Plan, Problem, ProblemType, SitePath,
    SubmitRequest, Submitted, Ulid,
};
use wps_client::wps::Wps;

use crate::approvals::{Moved, Press, approval_record, claim, executed, record_button};
use crate::conversations::{ConversationError, open_site};
use crate::db::{decode, mint_ulid};
use crate::identity::Identity;
use crate::problem::{ProblemAnswer, internal, json_body};
use crate::scenarios::{SealError, read_version, seal};
use crate::{AppState, GateState};

/// The route an execution's refusal names as its instance.
const RUNS: &str = "/api/runs";

/// `POST /api/runs`: `202 { approval }`, the approval's record (spec 01 section 3.10, rule 6), once the press is
/// recorded and before its execution; a press repeated while or after it executes answers the same approval.
pub async fn submit<S: Wps>(
    State(gate): State<GateState<S>>,
    Extension(identity): Extension<Identity>,
    uri: Uri,
    body: Bytes,
) -> Result<(StatusCode, Json<ApprovalAnswer>), ProblemAnswer> {
    let instance = uri.path();
    let request: SubmitRequest = json_body(&body, instance)?;
    let approval = press(&gate.app, &identity, &request, instance).await?;
    tokio::spawn(execute(gate.app.clone(), approval.id.clone()));
    Ok((StatusCode::ACCEPTED, Json(ApprovalAnswer { approval })))
}

/// Record `identity`'s press of `request`'s plan as their approval, its execution not started, and read its record
/// under the plan's lock; the record of the approval a press of this plan already made, when one did.
pub async fn press(
    app: &AppState,
    identity: &Identity,
    request: &SubmitRequest,
    instance: &str,
) -> Result<ApprovalRecord, ProblemAnswer> {
    let fault = |fault: &dyn Display| internal("submit the runs", fault, instance);
    let refuse = |type_, detail: String| -> ProblemAnswer { Problem::new(type_, detail).at(instance).into() };
    let mut tx = app.pool.begin().await.map_err(|f| fault(&f))?;
    let row = sqlx::query("SELECT state, plan, plan_sha256, approval FROM plans WHERE id = $1 FOR UPDATE")
        .bind(request.plan.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(|f| fault(&f))?
        .ok_or_else(|| refuse(ProblemType::NotFound, "No plan has that id.".to_owned()))?;
    if let Some(approval) = row.try_get::<Option<String>, _>("approval").map_err(|f| fault(&f))? {
        let approval: Ulid = approval.parse().map_err(|f| fault(&decode(f)))?;
        return record_of(&mut tx, &approval, instance).await;
    }
    let (state, stored, hash): (String, Option<Value>, Option<String>) = (
        row.try_get("state").map_err(|f| fault(&f))?,
        row.try_get("plan").map_err(|f| fault(&f))?,
        row.try_get("plan_sha256").map_err(|f| fault(&f))?,
    );
    let (Some(stored), Some(hash)) = (stored.filter(|_| state == "ready"), hash) else {
        let detail = format!("These runs are {state}, not ready: wait for the check to finish, then submit.");
        return Err(refuse(ProblemType::PlanNotReady, detail));
    };
    if hash != request.plan_sha256 {
        let detail = "These runs are not the ones you were shown: prepare them again, then submit.".to_owned();
        return Err(refuse(ProblemType::PlanStale, detail));
    }
    let plan: Plan = serde_json::from_value(stored).map_err(|f| fault(&f))?;
    let taken: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM submissions WHERE experiment = $1) \
             OR EXISTS (SELECT 1 FROM plans WHERE experiment = $1 AND approval IS NOT NULL)",
    )
    .bind(plan.experiment.as_str())
    .fetch_one(&mut *tx)
    .await
    .map_err(|f| fault(&f))?;
    if taken {
        let detail = "These runs were submitted from another plan; prepare a new experiment to run them again.";
        return Err(refuse(ProblemType::ExperimentSubmitted, detail.to_owned()));
    }
    // The execution's seal decides under the version's lock; this read answers a stale plan before anything is
    // approved.
    let version =
        read_version(&mut tx, &plan.scenario.id, plan.scenario.version, false).await.map_err(|f| fault(&f))?;
    if let Some(version) = version.filter(|version| version.revision != plan.scenario.revision) {
        let (planned, now) = (plan.scenario.revision.0, version.revision.0);
        return Err(SealError::PlanStale { version: plan.scenario.version.0, planned, now }.answer(instance));
    }
    let site: String = sqlx::query_scalar("SELECT site_scope FROM scenarios WHERE id = $1")
        .bind(plan.scenario.id.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(|f| fault(&f))?;
    let site: SitePath = site.parse().map_err(|f| fault(&f))?;
    let conversation = match open_site(&mut tx, &site, &identity.subject, &app.model_label).await {
        Ok((conversation, _)) => conversation.id,
        Err(ConversationError::Database(f)) => return Err(fault(&f)),
        Err(other) => return Err(other.answer(instance)),
    };
    let pressed = facts(&plan, &hash, &conversation, identity);
    let approval = record_button(&mut tx, pressed).await.map_err(|f| fault(&f))?;
    sqlx::query("UPDATE plans SET approval = $2 WHERE id = $1")
        .bind(request.plan.as_str())
        .bind(approval.as_str())
        .execute(&mut *tx)
        .await
        .map_err(|f| fault(&f))?;
    let record = record_of(&mut tx, &approval, instance).await?;
    tx.commit().await.map_err(|f| fault(&f))?;
    Ok(record)
}

/// `approval` as a press answers it, read in the press's transaction, as Create site's `202` reads its own.
async fn record_of(
    tx: &mut Transaction<'_, Postgres>,
    approval: &Ulid,
    instance: &str,
) -> Result<ApprovalRecord, ProblemAnswer> {
    let fault = |fault: &dyn Display| internal("answer the press", fault, instance);
    let record = approval_record(tx, approval).await.map_err(|f| fault(&f))?;
    record.ok_or_else(|| fault(&format!("approval {approval} is not recorded")))
}

/// Execute the plan `approval` approved, as the approval's one worker in this process: claimed, then submitted, or
/// refused with its words. A fault leaves it `running`, and the next start resumes it.
pub async fn execute(app: AppState, approval: Ulid) {
    let Some(_holding) = app.executing.hold(&approval) else { return };
    if let Err(fault) = submission(&app, &approval).await {
        eprintln!(
            "studio-api: the submission under approval {approval} stopped, for the next start to resume: {fault}"
        );
    }
}

/// Resume, once at start, every press a stopped process recorded and left unsettled.
pub async fn resume(app: &AppState) -> Result<(), SqlxError> {
    let unsettled: Vec<String> = sqlx::query_scalar(
        "SELECT p.approval FROM plans p JOIN approvals a ON a.id = p.approval \
         WHERE a.execution IN ('not_started', 'running')",
    )
    .fetch_all(&app.pool)
    .await?;
    for approval in unsettled {
        tokio::spawn(execute(app.clone(), approval.parse().map_err(decode)?));
    }
    Ok(())
}

/// Why a submission did not happen: a refusal, which settles the execution `failed`, or a fault, which leaves it.
enum NotSubmitted {
    Refused(Box<Problem>),
    Fault(SqlxError),
}

impl From<SqlxError> for NotSubmitted {
    fn from(fault: SqlxError) -> Self {
        Self::Fault(fault)
    }
}

async fn submission(app: &AppState, approval: &Ulid) -> Result<(), SqlxError> {
    if !claim(app, approval).await? {
        return Ok(());
    }
    let mut tx = app.pool.begin().await?;
    let outcome = submit_plan(&mut tx, approval).await;
    let (moved, submitted) = match outcome {
        Ok(submitted) => {
            let outcome = json!(submitted);
            (executed(&mut tx, approval, Execution::Ok, None, Some(&outcome)).await?, true)
        }
        Err(NotSubmitted::Refused(problem)) => {
            tx.rollback().await?;
            tx = app.pool.begin().await?;
            let outcome = json!(problem);
            (executed(&mut tx, approval, Execution::Failed, Some(&problem.detail), Some(&outcome)).await?, false)
        }
        Err(NotSubmitted::Fault(fault)) => return Err(fault),
    };
    tx.commit().await?;
    if let Some(Moved(entry)) = moved {
        app.hub.announce(entry);
    }
    if submitted {
        app.runs.wakes.dispatch();
    }
    Ok(())
}

/// Inside `tx`: the plan `approval` approved, sealed and inserted as its submission.
async fn submit_plan(tx: &mut Transaction<'_, Postgres>, approval: &Ulid) -> Result<Submitted, NotSubmitted> {
    let row = sqlx::query(
        "SELECT p.plan, p.plan_sha256, a.conversation, a.decided_by FROM plans p JOIN approvals a ON a.id = p.approval \
         WHERE p.approval = $1 FOR UPDATE OF p",
    )
    .bind(approval.as_str())
    .fetch_one(&mut **tx)
    .await?;
    let plan: Plan = serde_json::from_value(row.try_get("plan")?).map_err(decode)?;
    let hash: String = row.try_get("plan_sha256")?;
    let conversation: Option<String> = row.try_get("conversation")?;
    let by: String = row.try_get("decided_by")?;
    let refused = |answer: ProblemAnswer| NotSubmitted::Refused(answer.0);
    match seal(tx, &plan.scenario.id, plan.scenario.version, plan.scenario.revision, &plan.experiment).await {
        Ok(_) => {}
        Err(SealError::Database(fault)) => return Err(fault.into()),
        Err(stale) => return Err(refused(stale.answer(RUNS))),
    }
    let inserted = insert(tx, &plan, &hash, conversation.as_deref(), approval, &by).await;
    match inserted {
        // Another plan of the experiment was submitted first: one submission per experiment.
        Err(SqlxError::Database(conflict)) if conflict.is_unique_violation() => {
            let detail = "These runs were submitted from another plan; prepare a new experiment to run them again.";
            Err(refused(Problem::new(ProblemType::ExperimentSubmitted, detail).at(RUNS).into()))
        }
        inserted => Ok(inserted?),
    }
}

/// The press as its approval keeps it: the plan without its queue, which `plan_sha256` hashes, under that digest.
fn facts<'a>(plan: &Plan, hash: &str, conversation: &'a Ulid, identity: &'a Identity) -> Press<'a> {
    let mut input = json!(plan);
    if let Some(fields) = input.as_object_mut() {
        fields.remove("queue");
    }
    let runs = plan.runs.len();
    let what = if runs == 1 { "1 run".to_owned() } else { format!("{runs} runs") };
    let reads: Vec<String> = plan.cut.seqs.iter().map(|(scope, seq)| format!("{scope} at changeset {seq}")).collect();
    let row = |label: &str, value: String| FactRow { label: label.to_owned(), value };
    let facts = ApprovalFacts {
        lead: format!("Submit {what} of {}, version {}", plan.scenario.name, plan.scenario.version.0),
        display: vec![
            row("Action", "run.submit".to_owned()),
            row("Site", reads.join(", ")),
            row("Runs", format!("{what}, one job each")),
            row("Checked", format!("{what}, each loaded by the simulator's reader; plan {}", &hash[..12])),
            row("Runner", format!("{}, the default", plan.runner.tag)),
        ],
        notes: Vec::new(),
        action_label: format!("Approve and submit {what}"),
        footnote: None,
    };
    Press {
        conversation: Some(conversation),
        tool: "run.submit",
        class: "submit",
        input,
        digest: Some(hash.to_owned()),
        summary: format!("Submit {what} of {}", plan.scenario.name),
        facts: json!(facts),
        by: &identity.subject,
    }
}

/// The submission, its runs from the plan and their first attempts, submitted and queued at the approval's instant,
/// which orders the queue (spec 08 section 3.4: first approved, first started).
async fn insert(
    tx: &mut Transaction<'_, Postgres>,
    plan: &Plan,
    hash: &str,
    conversation: Option<&str>,
    approval: &Ulid,
    by: &str,
) -> Result<Submitted, SqlxError> {
    let submission = mint_ulid();
    sqlx::query(
        "INSERT INTO submissions (id, plan, plan_sha256, experiment, conversation, approval, submitted_by, \
             submitted_at) \
         SELECT $1, $2, $3, $4, $5, $6, $7, decided_at FROM approvals WHERE id = $6",
    )
    .bind(submission.as_str())
    .bind(plan.id.as_str())
    .bind(hash)
    .bind(plan.experiment.as_str())
    .bind(conversation)
    .bind(approval.as_str())
    .bind(by)
    .execute(&mut **tx)
    .await?;
    let cut = json!(plan.cut);
    let reads = json!(plan.reads);
    for run in &plan.runs {
        sqlx::query(
            "INSERT INTO runs (id, submission, experiment, position, scenario, scenario_version, variant, seed, \
                 duration_sim_ms, report_window, cut, reads, runner_image, expected) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
        )
        .bind(run.run.as_str())
        .bind(submission.as_str())
        .bind(plan.experiment.as_str())
        .bind(i32::try_from(run.position.0).unwrap_or(i32::MAX))
        .bind(plan.scenario.id.as_str())
        .bind(i32::try_from(plan.scenario.version.0).unwrap_or(i32::MAX))
        .bind(json!(run.variant))
        .bind(i64::try_from(run.seed.0).unwrap_or(i64::MAX))
        .bind(i64::try_from(run.duration_sim_ms.as_ms()).unwrap_or(i64::MAX))
        .bind(json!(run.report_window))
        .bind(&cut)
        .bind(&reads)
        .bind(&plan.runner.image)
        .bind(json!(run.expected))
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "INSERT INTO attempts (run_id, n, requested_at) SELECT $1, 1, decided_at FROM approvals WHERE id = $2",
        )
        .bind(run.run.as_str())
        .bind(approval.as_str())
        .execute(&mut **tx)
        .await?;
    }
    Ok(Submitted { submission, runs: plan.runs.iter().map(|run| run.run.clone()).collect() })
}
