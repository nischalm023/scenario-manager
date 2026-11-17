//! Within `results`, `inputs` is a run's inputs as values (spec 08 section 3.12; brief I1.10, task 33), which a
//! comparison is read through, so that what differs between two runs is stated as values a customer's inputs document
//! can quote: every scenario field of spec 07's field catalogue as the run read it, the sweep's value, the repeat's
//! seed and an experiment's override each marked by where it came from, then the provenance, every scope read with its
//! changeset, the read's exactness, the generated workload, the compiled flow plan against the check's, the compiler,
//! the runner's image and commit, and the generated workload's request, seed and digest as the manifest names them.
//! Each row carries its value unrounded as canonical JSON beside its words, which a comparison classifies on. Flow
//! parameters, class figures, the map census and the settings the runner resolved wait for BACKLOG tasks 17 and 20,
//! so every answer says `complete: false` (lead decided).

use std::collections::BTreeMap;

use axum::Json;
use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, Uri};
use axum::response::{IntoResponse, Response};
use desunits::SimDuration;
use run_invocation::Fault;
use runstore::RunStorage;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{PgConnection, Row};
use studio_protocol::{
    AppliedTo, Expectation, Experiment, FieldEntry, Finding, FindingKind, InputRow, InputSection, Manifest, Patch,
    PlanRunner, ReportWindow, RunInputs, RunSummary, RunnerChosen, ScenarioContent, ScopePath, Seed, Seq, Ulid,
    VersionNumber, WorkloadRead,
};

use crate::AppState;
use crate::problem::{ProblemAnswer, internal, validation_failed};
use crate::runs::{json, no_run, run_in, summary_of};
use crate::scenarios::{catalogue, read_experiment, read_version};

use super::stored::run_row;
use super::words::{field_number, field_value, number};

/// What a run read and how its latest attempt went, as its inputs and a comparison read them.
pub struct Gathered {
    pub summary: RunSummary,
    pub inputs: RunInputs,
    pub stub: bool,
    pub experiment: Experiment,
    pub project: Ulid,
}

#[derive(Deserialize)]
pub struct InputsQuery {
    format: Option<String>,
}

/// `GET /api/runs/{r}/inputs?format=json|text`.
pub async fn inputs(
    State(app): State<AppState>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
    query: Result<Query<InputsQuery>, QueryRejection>,
) -> Result<Response, ProblemAnswer> {
    let instance = uri.path();
    let run = run_in(path, instance)?;
    let Query(InputsQuery { format }) = query.map_err(|fault| validation_failed(fault.body_text(), instance))?;
    let text = match format.as_deref() {
        None | Some("json") => false,
        Some("text") => true,
        Some(other) => return Err(validation_failed(format!("`format` is json or text, not {other}"), instance)),
    };
    let fault = |fault: Fault| internal("read the run's inputs", fault, instance);
    let mut conn = app.pool.acquire().await.map_err(|failure| fault(failure.into()))?;
    let gathered = gather(&app, &mut conn, &run).await.map_err(fault)?.ok_or_else(|| no_run(instance))?;
    if !text {
        return Ok(Json(gathered.inputs).into_response());
    }
    let mut answer = gathered.inputs.text.into_response();
    answer.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    Ok(answer)
}

/// The run `run` with its inputs, its latest attempt's stub flag, its experiment and its scenario's project.
pub async fn gather(app: &AppState, conn: &mut PgConnection, run: &Ulid) -> Result<Option<Gathered>, Fault> {
    let Some(summary) = summary_of(conn, run).await? else { return Ok(None) };
    let row = sqlx::query(
        "SELECT r.experiment, r.variant, r.reads, r.cut ->> 'branch' AS branch, r.expected, p.plan -> 'runner' AS runner, \
             l.manifest_key, l.findings, l.runner_commit, l.runner_dirty \
         FROM runs r JOIN submissions s ON s.id = r.submission JOIN plans p ON p.id = s.plan \
         JOIN LATERAL (SELECT * FROM attempts a WHERE a.run_id = r.id ORDER BY a.n DESC LIMIT 1) l ON true \
         WHERE r.id = $1",
    )
    .bind(run.as_str())
    .fetch_one(&mut *conn)
    .await?;
    let experiment: Ulid = row.try_get::<String, _>("experiment")?.parse()?;
    let experiment = read_experiment(conn, &experiment).await?.ok_or("the run's experiment is gone")?;
    let project = run_row(conn, run).await?.ok_or("the run's scenario is gone")?.project;
    let version = read_version(conn, &summary.scenario.id, summary.scenario.version, false)
        .await?
        .ok_or("its version is gone")?;
    let manifest = match row.try_get::<Option<String>, _>("manifest_key")? {
        Some(key) => Some(serde_json::from_slice::<Manifest>(&app.runstore.get(&key).await?)?),
        None => None,
    };
    let variant: Vec<Patch> = json(row.try_get("variant")?)?;
    // ordered: the provenance lists the reads in scope order.
    let reads: BTreeMap<ScopePath, Seq> = json(row.try_get("reads")?)?;
    let runner: PlanRunner = json(row.try_get("runner")?)?;
    let expected: Expectation = json(row.try_get("expected")?)?;
    let findings: Vec<Finding> = json(row.try_get("findings")?)?;
    let commit: Option<String> = row.try_get("runner_commit")?;
    let facts = Facts {
        scenario: &summary.scenario.name,
        version: summary.scenario.version,
        content: &version.content,
        experiment: &experiment,
        variant: &variant,
        seed: summary.seed,
        duration: summary.duration_sim_ms,
        report_window: &summary.report_window,
        reads: &reads,
        branch: row.try_get::<Option<String>, _>("branch")?.unwrap_or_default(),
        runner: &runner,
        expected: &expected,
        manifest: manifest.as_ref(),
        findings: &findings,
        commit: commit.map(|commit| (commit, row.try_get::<Option<bool>, _>("runner_dirty").ok().flatten())),
    };
    let rows = rows(&facts);
    let title = format!("Inputs of run {}, {}, attempt {}", summary.id, summary.label, summary.attempt.0);
    let source = format!("{}, version {}", summary.scenario.name, summary.scenario.version.0);
    let workload = manifest.as_ref().map(|manifest| manifest.workload.clone());
    let text = text(&title, &source, &rows, workload.as_ref());
    let inputs = RunInputs { run: run.clone(), complete: false, rows, workload, text };
    let stub = manifest.as_ref().is_some_and(|manifest| manifest.stub);
    Ok(Some(Gathered { summary, inputs, stub, experiment, project }))
}

/// What a run's input rows are built from: its pins, its experiment, and its latest attempt's manifest and findings.
pub struct Facts<'a> {
    pub scenario: &'a str,
    pub version: VersionNumber,
    pub content: &'a ScenarioContent,
    pub experiment: &'a Experiment,
    pub variant: &'a [Patch],
    pub seed: Seed,
    pub duration: SimDuration,
    pub report_window: &'a ReportWindow,
    // ordered: the provenance lists the reads in scope order.
    pub reads: &'a BTreeMap<ScopePath, Seq>,
    pub branch: String,
    pub runner: &'a PlanRunner,
    pub expected: &'a Expectation,
    pub manifest: Option<&'a Manifest>,
    pub findings: &'a [Finding],
    pub commit: Option<(String, Option<bool>)>,
}

/// Every input row of a run: the scenario's fields in the catalogue's order, a world target the sweep patched, then
/// the provenance.
pub fn rows(facts: &Facts) -> Vec<InputRow> {
    let held = serde_json::to_value(facts.content).unwrap_or_default();
    let mut read = held.clone();
    set(&mut read, "/run_controls/duration_sim_ms", Value::from(facts.duration.as_ms()));
    set(&mut read, "/run_controls/report_window", serde_json::to_value(facts.report_window).unwrap_or_default());
    set(&mut read, "/run_controls/seed", Value::from(facts.seed.0));
    for patch in facts.variant.iter().filter(|patch| patch.applied_to == AppliedTo::Spec) {
        set(&mut read, &patch.pointer, patch.value.clone());
    }
    let source = format!("{}, version {}", facts.scenario, facts.version.0);
    let mut rows: Vec<InputRow> =
        catalogue().items.iter().map(|field| scenario_row(facts, field, &held, &read, &source)).collect();
    for patch in facts.variant.iter().filter(|patch| patch.applied_to == AppliedTo::World) {
        // A sweepable world field is a flow parameter or a fleet figure (spec 07 section 3.6): its pointer tells which.
        let section = if patch.pointer.starts_with("/flow.") { InputSection::Flow } else { InputSection::Fleet };
        let value = patch.value.as_str().map_or_else(|| patch.value.to_string(), str::to_owned);
        let field = format!("{}{}", patch.target, patch.pointer);
        let mut row = input(section, &field, &patch.label, (value, &patch.value), varied(facts, patch));
        row.varied = true;
        row.number = patch.value.as_f64();
        rows.push(row);
    }
    rows.extend(provenance(facts));
    rows
}

fn scenario_row(facts: &Facts, field: &FieldEntry, held: &Value, read: &Value, source: &str) -> InputRow {
    let pointer = field.pointer.as_str();
    let now = read.pointer(pointer).cloned().unwrap_or(Value::Null);
    let was = held.pointer(pointer).cloned().unwrap_or(Value::Null);
    let patch = facts.variant.iter().find(|patch| patch.applied_to == AppliedTo::Spec && patch.pointer == pointer);
    let repeats = facts.experiment.seeds.len();
    let (from, varied) = if let Some(patch) = patch {
        (varied(facts, patch), true)
    } else if pointer == "/run_controls/seed" && repeats > 1 {
        let at = facts.experiment.seeds.iter().position(|seed| *seed == facts.seed).map_or(0, |at| at + 1);
        (format!("the experiment's repeat {at} of {repeats}"), true)
    } else if now != was {
        ("the experiment's override".to_owned(), false)
    } else {
        (source.to_owned(), false)
    };
    let mut row = input(InputSection::Scenario, pointer, &field.label, (field_value(field, &now), &now), from);
    row.held = (now != was).then(|| field_value(field, &was));
    row.varied = varied;
    row.clock = field.clock;
    row.unit = (!field.unit.is_empty()).then(|| field.unit.clone());
    row.number = field_number(field, &now);
    row
}

/// Where a sweep's value came from: "varied by the sweep, value 4 of 5".
fn varied(facts: &Facts, patch: &Patch) -> String {
    let values = facts.experiment.sweep.as_ref().map(|sweep| &sweep.varies.values);
    let at = values.and_then(|values| values.iter().position(|value| *value == patch.value));
    match (at, values) {
        (Some(at), Some(values)) => format!("varied by the sweep, value {} of {}", at + 1, values.len()),
        _ => "varied by the sweep".to_owned(),
    }
}

fn provenance(facts: &Facts) -> Vec<InputRow> {
    let pins = "the plan's pins".to_owned();
    let attempt = |what: &str| {
        if facts.manifest.is_some() { format!("the runner's manifest, {what}") } else { format!("the check, {what}") }
    };
    let scenario = format!("{}, version {}", facts.scenario, facts.version.0);
    let held = json!({ "scenario": facts.scenario, "version": facts.version.0 });
    let mut rows = vec![input(InputSection::Provenance, "scenario", "Scenario", (scenario, &held), pins.clone())];
    for (scope, seq) in facts.reads {
        let from = format!("the plan's cut, branch {} as it stood when the runs were prepared", facts.branch);
        let read = (format!("changeset {seq}"), &json!(seq));
        let mut row = input(InputSection::Provenance, &format!("read:{scope}"), scope.as_str(), read, from);
        row.scope = Some(scope.clone());
        row.changeset = Some(*seq);
        rows.push(row);
    }
    let (exact, figures) = match facts.manifest {
        None => ("not known yet: the runner has not handed in what it read".to_owned(), Value::Null),
        Some(manifest) => {
            let figures = json!({ "exact": manifest.world.exact, "reason": manifest.world.exact_reason });
            if manifest.world.exact {
                ("yes: every scope read at its head, its labels and prologue with it".to_owned(), figures)
            } else {
                let reason = manifest.world.exact_reason.as_deref().unwrap_or("it read below a moved head");
                (format!("no: {reason}"), figures)
            }
        }
    };
    rows.push(input(InputSection::Provenance, "exact", "Read exact", (exact, &figures), attempt("its world")));
    let (workload, figures) = match facts.manifest {
        Some(manifest) => {
            let read = &manifest.workload;
            let said = format!(
                "sha256 {}, {} bytes from generator {}, seed {}",
                short(&read.sha256),
                number(read.bytes.0 as f64),
                read.generator,
                read.seed.0
            );
            (
                said,
                json!({ "sha256": read.sha256, "bytes": read.bytes, "generator": read.generator, "seed": read.seed }),
            )
        }
        None => {
            let sha256 = &facts.expected.workload_sha256;
            (format!("sha256 {}, as the check generated it", short(sha256)), json!({ "checked_sha256": sha256 }))
        }
    };
    let generated = (workload, &figures);
    rows.push(input(InputSection::Provenance, "workload", "Generated workload", generated, attempt("its workload")));
    let (digest, compiler) = match facts.manifest {
        Some(manifest) => (manifest.compile.digest.clone(), manifest.compile.compiler.clone()),
        None => (facts.expected.compile_sha256.clone(), facts.expected.compiler.clone()),
    };
    let plan = digest
        .as_ref()
        .map_or_else(|| "none: the simulator's default flow".to_owned(), |digest| format!("digest {}", short(digest)));
    let compiled = (plan, &json!(digest));
    rows.push(input(InputSection::Provenance, "compile", "Compiled flow plan", compiled, attempt("its compile")));
    let compiled_by = (compiler.clone(), &json!(compiler));
    rows.push(input(InputSection::Provenance, "compiler", "Compiler", compiled_by, attempt("its compile")));
    let differs = facts.findings.iter().find(|finding| finding.kind == FindingKind::PlanDiffers);
    let sentence = differs.map(|finding| finding.sentence.clone());
    let said = sentence.as_ref().map_or_else(|| "no".to_owned(), |sentence| format!("yes: {sentence}"));
    let from = "the attempt's findings".to_owned();
    rows.push(input(
        InputSection::Provenance,
        "plan-differs",
        "Differs from the check",
        (said, &json!(sentence)),
        from,
    ));
    let chosen = match facts.runner.chosen {
        RunnerChosen::Default => "the default",
        RunnerChosen::Developer => "chosen by a developer",
    };
    let image = format!("{} ({}), {chosen}", facts.runner.tag, facts.runner.image);
    let figures = json!({ "tag": facts.runner.tag, "image": facts.runner.image, "chosen": facts.runner.chosen });
    rows.push(input(InputSection::Provenance, "runner.image", "Runner image", (image, &figures), pins));
    let (commit, figures) = match &facts.commit {
        Some((commit, dirty)) => {
            let state = match dirty {
                Some(true) => "with uncommitted changes",
                Some(false) => "clean",
                None => "its state not reported",
            };
            let said = format!("{}, {state}", commit.chars().take(9).collect::<String>());
            (said, json!({ "commit": commit, "dirty": dirty }))
        }
        None => ("not reported yet".to_owned(), Value::Null),
    };
    let from = "the runner".to_owned();
    rows.push(input(InputSection::Provenance, "runner.commit", "Runner commit", (commit, &figures), from));
    rows
}

/// A row saying `value`'s words, with its unrounded figures as canonical JSON.
fn input(section: InputSection, field: &str, label: &str, value: (String, &Value), from: String) -> InputRow {
    let (value, figures) = value;
    InputRow {
        section,
        field: field.to_owned(),
        label: label.to_owned(),
        value,
        canonical: serde_jcs::to_string(figures).expect("a JSON value has canonical bytes"),
        all_in: None,
        from,
        held: None,
        varied: false,
        scope: None,
        clock: None,
        unit: None,
        number: None,
        changeset: None,
    }
}

/// The rows as plain lines under `title`, a section's name before its rows, for a copy; `source` names the scenario
/// version a held value is that version's; the generated workload's request closes its provenance.
pub fn text(title: &str, source: &str, rows: &[InputRow], workload: Option<&WorkloadRead>) -> String {
    let mut lines = vec![title.to_owned()];
    let mut section = None;
    for row in rows {
        if section != Some(row.section) {
            section = Some(row.section);
            lines.push(String::new());
            lines.push(
                match row.section {
                    InputSection::Scenario => "Scenario",
                    InputSection::Flow => "Flow",
                    InputSection::Fleet => "Fleet",
                    InputSection::Map => "Map",
                    InputSection::Settings => "Settings",
                    InputSection::Provenance => "Where they came from",
                }
                .to_owned(),
            );
        }
        let held = row.held.as_ref().map(|held| format!("; {source} holds {held}")).unwrap_or_default();
        lines.push(format!("{}: {} ({}{held})", row.label, row.value, row.from));
    }
    if let Some(workload) = workload {
        let request = serde_jcs::to_string(&workload.spec).expect("a generator request is JSON");
        lines.push(format!("Generator request: {request} (the runner's manifest, its workload)"));
    }
    lines.push(String::new());
    lines.push("Not read yet: the flow's parameters, the class figures, the map's census and the settings the runner resolved.".to_owned());
    lines.join("\n")
}

/// `value` written at `pointer` into `content`, the objects on the way made where missing.
fn set(content: &mut Value, pointer: &str, value: Value) {
    let mut at = content;
    let keys: Vec<&str> = pointer.split('/').skip(1).collect();
    for (n, key) in keys.iter().enumerate() {
        if !at.is_object() {
            *at = Value::Object(Default::default());
        }
        let object = at.as_object_mut().expect("an object, made so above");
        if n + 1 == keys.len() {
            object.insert((*key).to_owned(), value);
            return;
        }
        at = object.entry((*key).to_owned()).or_insert(Value::Null);
    }
}

fn short(digest: &str) -> String {
    digest.chars().take(12).collect()
}
