//! Within `scenarios`, `experiments` are spec 07 section 3.6's experiments: what one person's yes would launch, a
//! single run, its repeats with the next simulation seeds, or a sweep of one field over 2 to 12 values. Creating one
//! checks its request only (the form, the field a sweep may vary, each value against the content's checks) and
//! generates, compiles and loads nothing; spec 08's plan preparation alone checks each variant against the plan's
//! changesets (`validate_variants`). Until spec 06's parameter list (BACKLOG task 20), a sweep varies the scenario's
//! own fields only (lead decided).

use std::ops::RangeInclusive;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, State};
use axum::http::{StatusCode, Uri};
use axum::{Extension, Json};
use serde_json::Value;
use sqlx::{Error as SqlxError, PgConnection};
use studio_protocol::{
    AppliedTo, Experiment, ExperimentRead, ExperimentRequest, ObjectKey, ObjectKind, Patch, Person, ProblemItem,
    ScenarioContent, ScenarioRow, Seed, Severity, Sweep, Ulid, Variant, VariantNumber, Varied, VariesRequest,
};
use wps_client::compute::run;
use wps_client::{Compute, UnitCut as WpsCut, WpsError};

use scenario_spec::{SEED_LIMIT, Violation, apply_variant, this_build_accepts, validate, validate_overrides};

use crate::AppState;
use crate::db::mint_ulid;
use crate::identity::Identity;
use crate::problem::{ProblemAnswer, internal, json_body};
use crate::runs::submission_runs;

use super::check::{CheckOutcome, check};
use super::fields::entry;
use super::readiness::SiteReader;
use super::reads::no_version;
use super::rows::{self, NewExperiment, derive};
use super::{changed, deleted, id_in, not_found, refused};

const REPEATS_MOST: u32 = 10;
const VALUES: RangeInclusive<usize> = 2..=12;

/// `POST /api/experiments`: the request checked and the experiment stored, unchangeable from here on.
pub async fn create(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    uri: Uri,
    body: Bytes,
) -> Result<(StatusCode, Json<Experiment>), ProblemAnswer> {
    let instance = uri.path();
    let request: ExperimentRequest = json_body(&body, instance)?;
    let fault = |fault| internal("prepare the experiment", fault, instance);
    let mut tx = state.pool.begin().await.map_err(fault)?;
    let row = rows::scenario(&mut tx, &request.scenario, false).await.map_err(fault)?;
    let row = row.ok_or_else(|| not_found("scenario", instance))?;
    if row.deleted_at.is_some() {
        return Err(deleted(&row, instance));
    }
    let record = rows::version(&mut tx, &row.id, request.version, false).await.map_err(fault)?;
    let record = record.ok_or_else(|| no_version(request.version, instance))?;
    if record.revision != request.revision {
        let detail = format!(
            "Version {} is at revision {}, not {}: prepare the runs again from what it holds now.",
            record.n.0, record.revision.0, request.revision.0
        );
        return Err(changed(detail, Vec::new(), instance));
    }
    let experiment = prepare(&row, &record.content, request).map_err(|found| refused(found, instance))?;
    let conversation = live_conversation(&mut tx, &row).await.map_err(fault)?;
    let by = Person { subject: identity.subject.clone(), name: identity.name.clone() };
    let new = NewExperiment { experiment: &experiment, conversation: conversation.as_ref(), by: &by };
    rows::insert_experiment(&mut tx, new).await.map_err(fault)?;
    tx.commit().await.map_err(fault)?;
    Ok((StatusCode::CREATED, Json(experiment)))
}

/// `GET /api/experiments/{e}`: the experiment with its submission and that submission's runs, spec 08's.
pub async fn read(
    State(state): State<AppState>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<ExperimentRead>, ProblemAnswer> {
    let instance = uri.path();
    let id = id_in(path, "experiment", instance)?;
    let fault = |fault| internal("read the experiment", fault, instance);
    let mut conn = state.pool.acquire().await.map_err(fault)?;
    let experiment = rows::experiment(&mut conn, &id).await.map_err(fault)?;
    let experiment = experiment.ok_or_else(|| not_found("experiment", instance))?;
    let runs = match &experiment.submission {
        Some(submission) => submission_runs(&mut conn, submission).await.map_err(fault)?,
        None => Vec::new(),
    };
    Ok(Json(ExperimentRead { experiment, runs }))
}

/// The experiment a request names: one of the three forms within its bounds, its variants derived, every value
/// checked against the version's content.
pub fn prepare(
    row: &ScenarioRow,
    content: &ScenarioContent,
    request: ExperimentRequest,
) -> Result<Experiment, Vec<Violation>> {
    let at = |pointer: &str, rule: String| vec![Violation { pointer: pointer.to_owned(), rule }];
    let repeats = request.repeats.map_or(1, |repeats| repeats.0);
    if !(1..=REPEATS_MOST).contains(&repeats) {
        return Err(at("/repeats", format!("takes 1 to {REPEATS_MOST}; this is {repeats}")));
    }
    let overrides = request.run_overrides.unwrap_or_default();
    let found = validate_overrides(content, &overrides);
    if !found.is_empty() {
        return Err(found);
    }
    let first = content.run_controls.seed.0;
    if first + u64::from(repeats) - 1 > SEED_LIMIT {
        return Err(at("/repeats", format!("would take the simulation seed past {SEED_LIMIT}")));
    }
    let seeds = (0..u64::from(repeats)).map(|n| Seed(first + n)).collect();
    let (sweep, variants) = match request.varies {
        None => (
            None,
            vec![Variant { n: VariantNumber(1), label: "as the scenario states".to_owned(), patches: Vec::new() }],
        ),
        Some(varies) => {
            let (varied, variants) = sweep(row, content, varies)?;
            (Some(Sweep { varies: varied }), variants)
        }
    };
    Ok(derive(mint_ulid(), row.id.clone(), (request.version, request.revision), sweep, variants, seeds, overrides))
}

/// A sweep's variants, one per value: the field must be one of the scenario's a sweep may vary, and each value must
/// leave the content passing its checks.
fn sweep(
    row: &ScenarioRow,
    content: &ScenarioContent,
    varies: VariesRequest,
) -> Result<(Varied, Vec<Variant>), Vec<Violation>> {
    let at = |pointer: &str, rule: String| vec![Violation { pointer: pointer.to_owned(), rule }];
    let own = ObjectKey::new(ObjectKind::Scenario, row.id.clone()).to_string();
    if varies.target != own {
        let rule = format!("names {}; a sweep varies only this scenario's own fields ({own})", varies.target);
        return Err(at("/varies/target", rule));
    }
    let Some(field) = entry(&varies.pointer).filter(|field| field.sweepable) else {
        return Err(at("/varies/pointer", format!("names {}, which is no field a sweep may vary", varies.pointer)));
    };
    let count = varies.values.len();
    if !VALUES.contains(&count) || (1..count).any(|n| varies.values[..n].contains(&varies.values[n])) {
        return Err(at("/varies/values", format!("takes 2 to 12 different values; this has {count}")));
    }
    let mut variants = Vec::with_capacity(count);
    for (n, value) in varies.values.iter().enumerate() {
        let label = format!("{} {}", field.short_label, shown(value, field.per_unit));
        let patch = Patch {
            target: varies.target.clone(),
            pointer: field.pointer.clone(),
            value: value.clone(),
            label: field.label.clone(),
            applied_to: AppliedTo::Spec,
        };
        let variant = Variant { n: VariantNumber(n as u32 + 1), label, patches: vec![patch] };
        let (mut world, mut patched) = (Value::Null, content.clone());
        let checked = match apply_variant(&variant, &mut world, &mut patched) {
            Ok(_) => validate(&patched),
            Err(fault) => at(&field.pointer, format!("cannot take {value}: {fault}")),
        };
        if let Some(first) = checked.into_iter().next() {
            return Err(at("/varies/values", format!("value {} ({}): {}", n + 1, variant.label, first.rule)));
        }
        variants.push(variant);
    }
    let varied = Varied {
        target: varies.target,
        pointer: field.pointer.clone(),
        label: field.label.clone(),
        short_label: field.short_label.clone(),
        unit: field.unit.clone(),
        values: varies.values,
    };
    Ok((varied, variants))
}

/// A value as a variant's label shows it: a number in the field's shown unit, a closed value as written.
fn shown(value: &Value, per_unit: f64) -> String {
    match value {
        Value::Number(number) => {
            let shown = number.as_f64().unwrap_or_default() / per_unit;
            if shown.fract() == 0.0 { format!("{shown:.0}") } else { format!("{shown}") }
        }
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// The site's live conversation, which the experiment names (ruling D94); none when nobody has opened one.
async fn live_conversation(conn: &mut PgConnection, row: &ScenarioRow) -> Result<Option<Ulid>, SqlxError> {
    let id: Option<String> = sqlx::query_scalar("SELECT id FROM conversations WHERE site = $1 AND state = 'live'")
        .bind(row.site_scope.as_str())
        .fetch_optional(conn)
        .await?;
    Ok(id.and_then(|id| id.parse().ok()))
}

/// One variant checked against the plan's changesets: what the check found and the digests spec 08's expectation
/// records, the world's before the variant and the workload's.
#[derive(Debug, Clone, PartialEq)]
pub struct VariantCheck {
    pub variant: VariantNumber,
    pub outcome: CheckOutcome,
    pub world_sha256: String,
}

/// Spec 08's per-variant check (section 3.6; brief I1.09 calls it while preparing a plan): the cut's export once,
/// then for each variant the patched world and content loaded through `check` on the compute pool. A variant whose
/// target the world lacks is `variant-target-missing`, naming the variant, the target and the field.
pub async fn validate_variants<R: SiteReader>(
    reader: &R,
    pool: Arc<dyn Compute>,
    experiment: &Experiment,
    content: &ScenarioContent,
    cut: &WpsCut,
) -> Result<Vec<VariantCheck>, WpsError> {
    let export = reader.export(cut).await?;
    let fetched: Value =
        serde_json::from_slice(&export.bytes).map_err(|fault| WpsError::Invalid { text: fault.to_string() })?;
    let mut checks = Vec::with_capacity(experiment.variants.len());
    for variant in &experiment.variants {
        let (mut world, mut patched) = (fetched.clone(), content.clone());
        if let Err(fault) = apply_variant(variant, &mut world, &mut patched) {
            let text = format!("Variant {}: {fault}: prepare the sweep again.", variant.label);
            let item = ProblemItem {
                code: "variant-target-missing".to_owned(),
                severity: Severity::Error,
                ref_: None,
                scope: None,
                field: variant.patches.first().map(|patch| patch.pointer.clone()),
                knob: None,
                text,
            };
            let outcome = CheckOutcome { problems: vec![item], workload: None, workload_sha256: None, compiled: None };
            checks.push(VariantCheck { variant: variant.n, outcome, world_sha256: export.sha256.clone() });
            continue;
        }
        let bytes = if world == fetched { export.bytes.to_vec() } else { serde_json::to_vec(&world).expect("JSON") };
        let overrides = experiment.run_overrides.clone();
        let accepts = this_build_accepts();
        let outcome = run(&*pool, move || check(&bytes, &patched, &overrides, &accepts)).await?;
        checks.push(VariantCheck { variant: variant.n, outcome, world_sha256: export.sha256.clone() });
    }
    Ok(checks)
}
