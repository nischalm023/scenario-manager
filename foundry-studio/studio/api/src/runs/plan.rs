//! Within `runs`, `plan` is spec 08 section 3.3's submission plan as the API prepares it, so that an approval approves
//! pinned inputs: one consistent cut of the site at `main` (the one a press showed, or refused as stale), the export
//! whose `scopes` are the plan's reads, and every distinct variant checked on the compute pool (the site WPS generated,
//! patched and loaded through dessim's own reader, its workload generated, and compiled through the seam), each run's
//! expectation recorded, and the plan hashed without its queue. A plan is prepared in wall clock after `202`, read once
//! a second until it is ready or failed; preparing writes nothing but its own row.

use std::collections::BTreeMap;

use axum::Extension;
use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, State};
use axum::http::{StatusCode, Uri};
use desunits::SimDuration;
use scenario_spec::{RUN_FILES, SLICE_MS, apply_variant, effective};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{Error as SqlxError, PgConnection, Row};
use studio_protocol::{
    Expectation, Experiment, Plan, PlanAccepted, PlanFormat, PlanProgress, PlanRead, PlanRequest, PlanRunner,
    PlanScenario, PlanState, PlannedRun, Problem, ProblemItem, ProblemType, QueueSnapshot, ReportWindow, RunCount,
    RunPosition, RunnerChosen, ScopePath, Seq, Severity, Ulid, Variant, VariantNumber, WallClock, WorldCount,
};
use wps_client::WpsError;
use wps_client::wps::Wps;

use crate::db::{decode, mint_ulid, read_word, word};
use crate::identity::Identity;
use crate::problem::{ProblemAnswer, internal, json_body, wps_problem};
use crate::scenarios::{
    SiteReader, VariantCheck, VersionRecord, answered, read_experiment, read_scenario, read_version, validate_variants,
};
use crate::{AppState, GateState};

/// The route a failed plan's problem names.
const PLANS: &str = "/api/runs/plan";

/// `POST /api/runs/plan`: `202 { plan }` at once, the plan prepared on the compute pool after; a preparation of the
/// same experiment, image and shown cut already under way answers that plan.
pub async fn ask<S: Wps>(
    State(gate): State<GateState<S>>,
    Extension(identity): Extension<Identity>,
    uri: Uri,
    body: Bytes,
) -> Result<(StatusCode, Json<PlanAccepted>), ProblemAnswer> {
    let instance = uri.path();
    let request: PlanRequest = json_body(&body, instance)?;
    let (plan, fresh) = start(&gate.app, &identity, &request, instance).await?;
    if fresh {
        let gate = gate.clone();
        let id = plan.clone();
        tokio::spawn(async move { prepare(&gate.app, &*gate.world, &id).await });
    }
    Ok((StatusCode::ACCEPTED, Json(PlanAccepted { plan })))
}

/// Record a plan to prepare for `request`, keyed by the experiment, the preparer's image and the cut a press showed
/// (spec 08 section 3.3); true when the caller must prepare it, false when one already is.
pub async fn start(
    app: &AppState,
    identity: &Identity,
    request: &PlanRequest,
    instance: &str,
) -> Result<(Ulid, bool), ProblemAnswer> {
    let fault = |fault| internal("start preparing the runs", fault, instance);
    let mut tx = app.pool.begin().await.map_err(fault)?;
    let experiment = read_experiment(&mut tx, &request.experiment).await.map_err(fault)?;
    let experiment =
        experiment.ok_or_else(|| problem(ProblemType::NotFound, "No experiment has that id.", instance))?;
    let submitted: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM submissions WHERE experiment = $1)")
        .bind(experiment.id.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(fault)?;
    if submitted {
        let detail = "These runs were submitted already; prepare a new experiment to run them again.";
        return Err(problem(ProblemType::ExperimentSubmitted, detail, instance));
    }
    let image = app.runs.image.as_ref().map(|image| image.digest.clone());
    let chosen = word(RunnerChosen::Default);
    let shown = request.cut.as_ref().map(|cut| json!(cut));
    let preparing: Option<String> = sqlx::query_scalar(
        "SELECT id FROM plans WHERE experiment = $1 AND runner_image IS NOT DISTINCT FROM $2 AND runner_chosen = $3 \
         AND shown_cut IS NOT DISTINCT FROM $4 AND state = 'preparing' LIMIT 1",
    )
    .bind(experiment.id.as_str())
    .bind(image.as_deref())
    .bind(&chosen)
    .bind(&shown)
    .fetch_optional(&mut *tx)
    .await
    .map_err(fault)?;
    if let Some(id) = preparing {
        return Ok((id.parse().map_err(decode).map_err(fault)?, false));
    }
    let id = mint_ulid();
    let refused = image.is_none().then(|| {
        let detail =
            "No runner image is registered to run these: ask whoever runs CI to build main, then prepare again.";
        json!(Problem::new(ProblemType::NoRunnerImage, detail).at(PLANS))
    });
    sqlx::query(
        "INSERT INTO plans (id, experiment, conversation, prepared_by, runner_image, runner_chosen, shown_cut, state, \
             problem) \
         SELECT $1, $2, (SELECT c.id FROM conversations c JOIN scenarios s ON s.site_scope = c.site \
                         WHERE s.id = $3 AND c.state = 'live'), $4, $5, $6, $7, $8, $9",
    )
    .bind(id.as_str())
    .bind(experiment.id.as_str())
    .bind(experiment.scenario.as_str())
    .bind(identity.subject.as_str())
    .bind(image.as_deref())
    .bind(&chosen)
    .bind(&shown)
    .bind(if refused.is_some() { "failed" } else { "preparing" })
    .bind(refused.as_ref())
    .execute(&mut *tx)
    .await
    .map_err(fault)?;
    tx.commit().await.map_err(fault)?;
    Ok((id.clone(), refused.is_none()))
}

/// Prepare plan `id` through `reader` and record what came of it: ready with its hash, or failed with one problem.
pub async fn prepare<R: SiteReader>(app: &AppState, reader: &R, id: &Ulid) {
    let outcome = build(app, reader, id).await;
    let written = match outcome {
        Ok((plan, hash)) => {
            sqlx::query(
                "UPDATE plans SET state = 'ready', plan = $2, plan_sha256 = $3, ready_at = now() \
                 WHERE id = $1 AND state = 'preparing'",
            )
            .bind(id.as_str())
            .bind(json!(plan))
            .bind(hash)
            .execute(&app.pool)
            .await
        }
        Err(problem) => {
            sqlx::query("UPDATE plans SET state = 'failed', problem = $2 WHERE id = $1 AND state = 'preparing'")
                .bind(id.as_str())
                .bind(json!(problem.0))
                .execute(&app.pool)
                .await
        }
    };
    if let Err(fault) = written {
        eprintln!("studio-api: plan {id}'s outcome was not recorded: {fault}");
    }
}

/// What a plan is prepared from, read once.
struct Inputs {
    experiment: Experiment,
    name: String,
    site: ScopePath,
    record: VersionRecord,
    shown: Option<BTreeMap<ScopePath, Seq>>,
    image: String,
    tag: String,
}

async fn build<R: SiteReader>(app: &AppState, reader: &R, id: &Ulid) -> Result<(Plan, String), ProblemAnswer> {
    let fault = |fault: SqlxError| internal("prepare the runs", fault, PLANS);
    let wps = |fault: WpsError| wps_problem(&fault, PLANS);
    let mut conn = app.pool.acquire().await.map_err(fault)?;
    let inputs = inputs(&mut conn, app, id).await.map_err(fault)?;
    drop(conn);
    let site =
        inputs.site.as_str().parse().map_err(|_| refusal(ProblemType::NotFound, "The scenario names no site."))?;
    // Section 3.3's rule 1, the cut: one consistent read of every scope at main, the one a press showed or refused.
    let cut = reader.resolve_cut(&site).await.map_err(wps)?;
    if let Some(shown) = &inputs.shown
        && let Some(scope) = moved(shown, &cut.seqs)
    {
        let detail = format!("{scope} moved after the panel read it: read the scenario again, then run it.");
        return Err(refusal(ProblemType::PlanStale, &detail));
    }
    // Rule 2, what the world holds: the export's `scopes` are the plan's reads.
    // Rule 3, the image, was named when the plan was asked for.
    let export = reader.export(&cut).await.map_err(wps)?;
    let experiment = &inputs.experiment;
    if inputs.record.revision != experiment.revision {
        let detail = format!(
            "{} changed after these runs were prepared: version {} is at revision {}, not {}.",
            inputs.name, inputs.record.n.0, inputs.record.revision.0, experiment.revision.0
        );
        return Err(refusal(ProblemType::PlanStale, &detail));
    }
    // Rule 4, each distinct variant checked against the plan's changesets and compiled through the seam, its
    // progress counted in worlds.
    let fetched: Value =
        serde_json::from_slice(&export.bytes).map_err(|fault| internal("read the site's export", fault, PLANS))?;
    let worlds = experiment.variants.len() as u32;
    progress(app, id, 0, worlds).await;
    let mut checks: Vec<VariantCheck> = Vec::with_capacity(experiment.variants.len());
    for (done, variant) in experiment.variants.iter().enumerate() {
        let one = Experiment { variants: vec![variant.clone()], ..experiment.clone() };
        let found = validate_variants(reader, app.compute.clone(), &one, &inputs.record.content, &cut).await;
        checks.extend(found.map_err(wps)?);
        progress(app, id, done as u32 + 1, worlds).await;
    }
    let mut items: Vec<ProblemItem> = Vec::new();
    for check in &checks {
        let label = experiment.variants.iter().find(|variant| variant.n == check.variant).map(|variant| &variant.label);
        for item in check.outcome.problems.iter().filter(|item| item.severity == Severity::Error) {
            let mut item = item.clone();
            if let Some(label) = label.filter(|_| worlds > 1) {
                item.text = format!("{label}: {}", item.text);
            }
            items.push(item);
        }
    }
    let mut runs = Vec::with_capacity(experiment.run_order.len());
    for (n, slot) in experiment.run_order.iter().enumerate() {
        let (variant, check) = variant_of(experiment, &checks, slot.variant)
            .ok_or_else(|| internal("prepare the runs", format!("variant {} has no check", slot.variant.0), PLANS))?;
        let Some(compiled) = &check.outcome.compiled else { continue };
        let (mut world, mut content) = (fetched.clone(), inputs.record.content.clone());
        apply_variant(variant, &mut world, &mut content)
            .map_err(|fault| internal("prepare the checked variant", fault, PLANS))?;
        let (duration, window) = effective(&content, &experiment.run_overrides);
        content.run_controls.seed = slot.seed;
        let spec = compiled.spec(&content, &experiment.run_overrides, RUN_FILES);
        items.extend(run_refusals(&spec, &window, duration));
        let Some(workload_sha256) = check.outcome.workload_sha256.clone() else { continue };
        runs.push(PlannedRun {
            run: mint_ulid(),
            position: RunPosition(n as u32 + 1),
            seed: slot.seed,
            duration_sim_ms: duration,
            report_window: window,
            variant: variant.patches.clone(),
            expected: Expectation {
                compile_sha256: Some(compiled.digest.clone()),
                world_sha256: check.world_sha256.clone(),
                compiler: compiled.compiler.clone(),
                workload_sha256,
            },
        });
    }
    // Rule 5: every refusal names its field or entity; a check that generated nothing has already said why.
    if !items.is_empty() || runs.len() != experiment.run_order.len() {
        let detail = items.iter().map(|item| item.text.as_str()).collect::<Vec<_>>().join(" ");
        let mut problem = refusal(ProblemType::ValidationFailed, &detail);
        problem.0.problems = items;
        return Err(problem);
    }
    let queue = queue(app).await.map_err(fault)?;
    let plan = Plan {
        v: PlanFormat(1),
        id: id.clone(),
        experiment: experiment.id.clone(),
        kind: experiment.kind,
        scenario: PlanScenario {
            id: experiment.scenario.clone(),
            version: experiment.version,
            revision: experiment.revision,
            name: inputs.name.clone(),
        },
        cut: answered(&cut),
        reads: export.scopes.clone(),
        prepared_at: WallClock::now(),
        runner: PlanRunner { image: inputs.image, tag: inputs.tag, chosen: RunnerChosen::Default },
        runs,
        queue,
    };
    let hash = plan_sha256(&plan);
    Ok((plan, hash))
}

async fn inputs(conn: &mut PgConnection, app: &AppState, id: &Ulid) -> Result<Inputs, SqlxError> {
    let row = sqlx::query("SELECT experiment, runner_image, shown_cut FROM plans WHERE id = $1")
        .bind(id.as_str())
        .fetch_one(&mut *conn)
        .await?;
    let experiment: Ulid = row.try_get::<String, _>("experiment")?.parse().map_err(decode)?;
    let experiment =
        read_experiment(conn, &experiment).await?.ok_or_else(|| decode("the plan's experiment is gone"))?;
    let scenario = read_scenario(conn, &experiment.scenario, false).await?.ok_or_else(|| decode("no scenario"))?;
    let record =
        read_version(conn, &scenario.id, experiment.version, false).await?.ok_or_else(|| decode("no version"))?;
    let shown: Option<Value> = row.try_get("shown_cut")?;
    let image: Option<String> = row.try_get("runner_image")?;
    let image = image.ok_or_else(|| decode("a plan being prepared names its image"))?;
    let tag = app
        .runs
        .image
        .as_ref()
        .filter(|known| known.digest == image)
        .map_or_else(String::new, |known| known.tag.clone());
    Ok(Inputs {
        experiment,
        name: scenario.name,
        site: scenario.site_scope.as_scope().clone(),
        record,
        shown: shown.map(serde_json::from_value).transpose().map_err(decode)?,
        image,
        tag,
    })
}

/// The first scope whose changeset the press showed differs from the one resolved now.
fn moved(shown: &BTreeMap<ScopePath, Seq>, now: &BTreeMap<ScopePath, Seq>) -> Option<ScopePath> {
    let scopes = shown.keys().chain(now.keys());
    scopes.into_iter().find(|scope| shown.get(*scope) != now.get(*scope)).cloned()
}

fn variant_of<'a>(
    experiment: &'a Experiment,
    checks: &'a [VariantCheck],
    n: VariantNumber,
) -> Option<(&'a Variant, &'a VariantCheck)> {
    let variant = experiment.variants.iter().find(|variant| variant.n == n)?;
    let check = checks.iter().find(|check| check.variant == n)?;
    Some((variant, check))
}

/// Rule 5's run settings and window (lean): a run that stops by wall clock or without its watchdog would count as
/// finished at a machine-dependent instant, and a window ending between slices names a value the recording cannot
/// give exactly (section 3.9); each refused naming its field.
pub fn run_refusals(spec: &Value, window: &ReportWindow, duration: SimDuration) -> Vec<ProblemItem> {
    let item = |code: &str, field: &str, text: String| ProblemItem {
        code: code.to_owned(),
        severity: Severity::Error,
        ref_: None,
        scope: None,
        field: Some(field.to_owned()),
        knob: None,
        text,
    };
    let mut items = Vec::new();
    let settings = &spec["settings"];
    if settings.get("wall_timeout_secs").is_some_and(|value| !value.is_null()) {
        let text = "A run must not stop by wall clock: such a stop counts as finished at a time that depends on the \
                    machine.";
        items.push(item("validation-failed", "/settings/wall_timeout_secs", text.to_owned()));
    }
    if settings.get("no_watchdog") == Some(&Value::Bool(true)) {
        let text = "A run must keep its watchdog: without it a deadlocked run would run to its deadline.";
        items.push(item("validation-failed", "/settings/no_watchdog", text.to_owned()));
    }
    let ends =
        [("from_sim_ms", Some(window.from_sim_ms.as_ms())), ("to_sim_ms", window.to_sim_ms.map(|to| to.as_ms()))];
    for (end, at) in ends {
        let Some(at) = at.filter(|at| !at.is_multiple_of(SLICE_MS) && *at < duration.as_ms()) else { continue };
        let (before, after) = (at - at % SLICE_MS, at - at % SLICE_MS + SLICE_MS);
        let text = format!(
            "The report window's {} at {at} ms of simtime falls between the recorded instants {before} ms and {after} \
             ms; choose one of them.",
            if end == "from_sim_ms" { "start" } else { "end" }
        );
        items.push(item("window-not-recorded", &format!("/report_window/{end}"), text));
    }
    items
}

/// `plan_sha256`: the SHA-256 of the plan's RFC 8785 canonical JSON without `queue`, which is information, never input.
pub fn plan_sha256(plan: &Plan) -> String {
    let mut value = json!(plan);
    if let Some(fields) = value.as_object_mut() {
        fields.remove("queue");
    }
    let canonical = serde_jcs::to_vec(&value).expect("a plan is JSON");
    format!("{:x}", Sha256::digest(canonical))
}

/// The queue as it stands: the seats held and the attempts waiting for one.
pub async fn queue(app: &AppState) -> Result<QueueSnapshot, SqlxError> {
    let (going, waiting): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE job IS NOT NULL AND job_stopped_at IS NULL), \
                count(*) FILTER (WHERE state = 'queued' AND job IS NULL) FROM attempts",
    )
    .fetch_one(&app.pool)
    .await?;
    let counted = |count: i64| RunCount(u32::try_from(count).unwrap_or(u32::MAX));
    Ok(QueueSnapshot {
        limit: RunCount(app.runs.config.at_once.0),
        going: counted(going),
        waiting: counted(waiting),
        as_of: WallClock::now(),
    })
}

async fn progress(app: &AppState, id: &Ulid, done: u32, worlds: u32) {
    let progress = json!(PlanProgress { worlds_done: WorldCount(done), worlds: WorldCount(worlds) });
    let written = sqlx::query("UPDATE plans SET progress = $2 WHERE id = $1 AND state = 'preparing'")
        .bind(id.as_str())
        .bind(progress)
        .execute(&app.pool)
        .await;
    if let Err(fault) = written {
        eprintln!("studio-api: plan {id}'s progress was not recorded: {fault}");
    }
}

/// `GET /api/runs/plan/{p}`.
pub async fn read(
    State(app): State<AppState>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<PlanRead>, ProblemAnswer> {
    let instance = uri.path();
    let nothing = || problem(ProblemType::NotFound, "No plan has that id.", instance);
    let Ok(Path(id)) = path else { return Err(nothing()) };
    let id: Ulid = id.parse().map_err(|_| nothing())?;
    let fault = |fault| internal("read the plan", fault, instance);
    let row = sqlx::query("SELECT state, progress, plan, plan_sha256, problem FROM plans WHERE id = $1")
        .bind(id.as_str())
        .fetch_optional(&app.pool)
        .await
        .map_err(fault)?
        .ok_or_else(nothing)?;
    let read = || -> Result<PlanRead, SqlxError> {
        let json = |column: &str| row.try_get::<Option<Value>, _>(column);
        Ok(PlanRead {
            id: id.clone(),
            state: read_word::<PlanState>(&row.try_get::<String, _>("state")?)?,
            progress: serde_json::from_value(row.try_get("progress")?).map_err(decode)?,
            plan: json("plan")?.map(serde_json::from_value).transpose().map_err(decode)?,
            plan_sha256: row.try_get("plan_sha256")?,
            problem: json("problem")?.map(serde_json::from_value).transpose().map_err(decode)?,
        })
    };
    read().map(Json).map_err(fault)
}

/// When the service starts, a plan a stopped process left preparing fails, since nothing prepares it any more.
pub async fn settle_unfinished(app: &AppState) -> Result<u64, SqlxError> {
    let detail = "Agent Studio restarted while these runs were prepared: prepare them again.";
    let failed = sqlx::query("UPDATE plans SET state = 'failed', problem = $1 WHERE state = 'preparing'")
        .bind(json!(Problem::new(ProblemType::Internal, detail).at(PLANS)))
        .execute(&app.pool)
        .await?;
    Ok(failed.rows_affected())
}

fn refusal(type_: ProblemType, detail: &str) -> ProblemAnswer {
    Problem::new(type_, detail).at(PLANS).into()
}

fn problem(type_: ProblemType, detail: &str, instance: &str) -> ProblemAnswer {
    Problem::new(type_, detail).at(instance).into()
}
