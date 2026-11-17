//! Within `studio-api`, `scenarios` are spec 07's scenarios and experiments (sections 3.1 to 3.10): the routes that
//! create, read, edit, duplicate and delete a scenario, each version's readiness at its site's heads of `main`, the
//! version diff, the field catalogue, and the experiments a person prepares from a version; and, for spec 08's plan
//! and execution (brief I1.09), `check`, `validate_variants`, `seal` and `sealed_version`. A scenario's data is its
//! spec and seeds in Postgres; its workload is generated on demand and never stored (ruling D96).

mod check;
mod experiments;
mod fields;
mod merge;
mod readiness;
mod reads;
mod rows;
mod seal;
mod writes;

use std::sync::Arc;

use axum::extract::Path;
use axum::extract::rejection::PathRejection;
use axum::routing::{get, post};
use axum::{Extension, Router};
use scenario_spec::Violation;
use studio_protocol::{FieldConflict, Problem, ProblemItem, ProblemType, ScenarioRow, Severity, Ulid};
use wps_client::Compute;
use wps_client::wps::Wps;

use crate::idempotency::idempotent;
use crate::problem::ProblemAnswer;
use crate::{AppState, GateState};

pub use check::{CheckOutcome, check};
pub use experiments::{VariantCheck, validate_variants};
pub use fields::{catalogue, label};
pub use readiness::{ReadinessCache, SiteReader, answered, read as readiness};
pub use rows::{VersionRecord, experiment as read_experiment, scenario as read_scenario, version as read_version};
pub use seal::{SealError, Sealed, seal, sealed_version};

/// The routes of spec 07 section 3.8 that the gate-facing listener serves; the two runner routes are I1.09's.
pub fn routes<S: Wps>(app: &AppState) -> Router<GateState<S>> {
    Router::new()
        .route("/api/scenarios", idempotent(app, post(writes::create::<S>)).get(reads::list))
        .route("/api/scenarios/{scenario}", get(reads::read).patch(writes::edit).delete(writes::delete))
        .route("/api/scenarios/{scenario}/duplicate", idempotent(app, post(writes::duplicate)))
        .route("/api/scenarios/{scenario}/readiness", get(readiness::route::<S>))
        .route("/api/scenarios/{scenario}/diff", get(reads::diff))
        .route("/api/scenario-fields", get(reads::fields))
        .route("/api/experiments", idempotent(app, post(experiments::create)))
        .route("/api/experiments/{experiment}", get(experiments::read))
        .layer(Extension(Arc::new(ReadinessCache::of_image(app.runs.image.as_ref().map_or("", |image| &image.digest)))))
}

/// The compute pool readiness and the check run on (spec 01 section 3.15).
pub fn compute<S: Wps>(state: &GateState<S>) -> Arc<dyn Compute> {
    state.app.compute.clone()
}

/// An object's id from a route's path; one that is no ULID names nothing.
fn id_in(path: Result<Path<String>, PathRejection>, what: &str, instance: &str) -> Result<Ulid, ProblemAnswer> {
    let nothing = || not_found(what, instance);
    let Ok(Path(id)) = path else { return Err(nothing()) };
    id.parse().map_err(|_| nothing())
}

fn not_found(what: &str, instance: &str) -> ProblemAnswer {
    Problem::new(ProblemType::NotFound, format!("No {what} has that id.")).at(instance).into()
}

/// A refusal of the checks of spec 07 section 3.2, each finding named by its field's label.
fn refused(found: Vec<Violation>, instance: &str) -> ProblemAnswer {
    let items: Vec<ProblemItem> = found
        .iter()
        .map(|violation| ProblemItem {
            code: "validation-failed".to_owned(),
            severity: Severity::Error,
            ref_: None,
            scope: None,
            field: Some(violation.pointer.clone()),
            knob: None,
            text: format!("{} {}.", label(&violation.pointer), violation.rule),
        })
        .collect();
    let detail = items.iter().map(|item| item.text.as_str()).collect::<Vec<_>>().join(" ");
    let mut problem = Problem::new(ProblemType::ValidationFailed, detail).at(instance);
    problem.problems = items;
    problem.into()
}

fn deleted(row: &ScenarioRow, instance: &str) -> ProblemAnswer {
    let by = row.deleted_by.as_ref().map_or("someone", |person| person.name.as_str());
    let detail = format!("{} was deleted by {by}: it reads as it was, and nothing edits or runs it.", row.name);
    Problem::new(ProblemType::ScenarioDeleted, detail).at(instance).into()
}

fn changed(detail: String, conflicts: Vec<FieldConflict>, instance: &str) -> ProblemAnswer {
    let mut problem = Problem::new(ProblemType::ScenarioChanged, detail).at(instance);
    problem.conflicts = conflicts;
    problem.into()
}
