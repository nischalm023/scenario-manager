//! Within `scenarios`, `reads` are the routes that read a scenario (spec 07 section 3.8): a site's list, one version
//! with every version's summary, the fields two versions differ in, and the field catalogue every surface names a
//! field by. None writes, and none reads the world.

use axum::Json;
use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::Uri;
use serde::Deserialize;
use serde_json::Value;
use studio_protocol::{
    FieldCatalogue, FieldChange, Problem, ProblemType, ScenarioDiff, ScenarioList, ScenarioRead, SitePath,
    VersionNumber, VersionRead,
};

use crate::AppState;
use crate::problem::{ProblemAnswer, internal, validation_failed};

use super::rows::{self, VersionRecord};
use super::{catalogue, id_in, not_found};

#[derive(Deserialize)]
pub struct ListQuery {
    site: SitePath,
    deleted: Option<bool>,
}

/// `GET /api/scenarios?site=&deleted=`: the site's scenarios, newest change first; the deleted ones too with
/// `deleted=true` (lead decided: the parameter adds them).
pub async fn list(
    State(state): State<AppState>,
    uri: Uri,
    query: Result<Query<ListQuery>, QueryRejection>,
) -> Result<Json<ScenarioList>, ProblemAnswer> {
    let instance = uri.path();
    let Query(ListQuery { site, deleted }) = query.map_err(|fault| validation_failed(fault.body_text(), instance))?;
    let mut conn = state.pool.acquire().await.map_err(|fault| internal("reach the database", fault, instance))?;
    let items = rows::list(&mut conn, &site, deleted.unwrap_or(false))
        .await
        .map_err(|fault| internal("list the site's scenarios", fault, instance))?;
    Ok(Json(ScenarioList { items }))
}

#[derive(Deserialize)]
pub struct VersionQuery {
    pub version: Option<u32>,
}

/// `GET /api/scenarios/{s}?version=`: a version, the latest by default, with every version's summary.
pub async fn read(
    State(state): State<AppState>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
    query: Result<Query<VersionQuery>, QueryRejection>,
) -> Result<Json<ScenarioRead>, ProblemAnswer> {
    let instance = uri.path();
    let id = id_in(path, "scenario", instance)?;
    let Query(VersionQuery { version }) = query.map_err(|fault| validation_failed(fault.body_text(), instance))?;
    let fault = |fault| internal("read the scenario", fault, instance);
    let mut conn = state.pool.acquire().await.map_err(fault)?;
    let scenario =
        rows::scenario(&mut conn, &id, false).await.map_err(fault)?.ok_or_else(|| not_found("scenario", instance))?;
    let n = version.map_or(scenario.latest_version, VersionNumber);
    let record =
        rows::version(&mut conn, &id, n, false).await.map_err(fault)?.ok_or_else(|| no_version(n, instance))?;
    let versions = rows::versions(&mut conn, &id).await.map_err(fault)?;
    Ok(Json(ScenarioRead { scenario, versions, version: as_read(record) }))
}

#[derive(Deserialize)]
pub struct DiffQuery {
    from: u32,
    to: u32,
}

/// `GET /api/scenarios/{s}/diff?from=&to=`: each field the two versions hold differently, in the catalogue's order.
pub async fn diff(
    State(state): State<AppState>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
    query: Result<Query<DiffQuery>, QueryRejection>,
) -> Result<Json<ScenarioDiff>, ProblemAnswer> {
    let instance = uri.path();
    let id = id_in(path, "scenario", instance)?;
    let Query(DiffQuery { from, to }) = query.map_err(|fault| validation_failed(fault.body_text(), instance))?;
    let fault = |fault| internal("read the scenario's versions", fault, instance);
    let mut conn = state.pool.acquire().await.map_err(fault)?;
    let mut read = async |n: u32| {
        let record = rows::version(&mut conn, &id, VersionNumber(n), false).await.map_err(fault)?;
        let record = record.ok_or_else(|| no_version(VersionNumber(n), instance))?;
        Ok::<Value, ProblemAnswer>(serde_json::to_value(record.content).expect("a scenario's content is JSON"))
    };
    let (from, to) = (read(from).await?, read(to).await?);
    let changes = catalogue()
        .items
        .iter()
        .filter_map(|entry| {
            let (was, is) = (field(&from, &entry.pointer), field(&to, &entry.pointer));
            (was != is).then(|| FieldChange {
                pointer: entry.pointer.clone(),
                label: entry.label.clone(),
                from: was,
                to: is,
            })
        })
        .collect();
    Ok(Json(ScenarioDiff { changes }))
}

/// `GET /api/scenario-fields`: the field catalogue (spec 07 section 3.2), which the panel names every field by.
pub async fn fields() -> Json<FieldCatalogue> {
    Json(catalogue().clone())
}

pub fn as_read(record: VersionRecord) -> VersionRead {
    VersionRead {
        n: record.n,
        state: record.state,
        revision: record.revision,
        content: record.content,
        set_by: record.set_by,
    }
}

fn field(content: &Value, pointer: &str) -> Value {
    content.pointer(pointer).cloned().unwrap_or(Value::Null)
}

pub fn no_version(n: VersionNumber, instance: &str) -> ProblemAnswer {
    Problem::new(ProblemType::NotFound, format!("This scenario has no version {}.", n.0)).at(instance).into()
}
