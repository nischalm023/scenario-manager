//! Within `scenarios`, `writes` are the routes that change a scenario (spec 07 sections 3.5 and 3.8): create version
//! 1 with every field stated, edit the latest version under its row's lock with section 3.5's field merge (an open
//! version takes the next revision, a sealed one opens the next version as a copy with the edit applied), duplicate
//! a version into a new scenario on the same site, and delete, which marks the row and keeps every version.

use axum::Extension;
use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, State};
use axum::http::{StatusCode, Uri};
use sqlx::{Connection, Error as SqlxError, PgConnection, Postgres, Transaction};
use studio_protocol::{
    CreateScenario, CreatedScenario, DuplicateScenario, FieldConflict, FieldSet, Person, Problem, ProblemItem,
    ProblemType, Revision, ScenarioContent, ScenarioEdit, ScenarioEdited, ScenarioRow, SetBy, Setter, Severity,
    SitePath, Ulid, VersionNumber, VersionState, WallClock,
};
use wps_client::wps::Wps;

use scenario_spec::{Violation, stated_defaults, validate};

use crate::db::mint_ulid;
use crate::identity::Identity;
use crate::problem::{ProblemAnswer, internal, json_body};
use crate::projects::default_project;
use crate::sites::is_listed;
use crate::{AppState, GateState};

use super::fields::ROW_FIELDS;
use super::merge::{SetByMap, apply, conflicts, every_field};
use super::reads::no_version;
use super::rows::{self, NewScenario, VersionRecord};
use super::{changed, deleted, id_in, not_found, refused};

const NAME_MOST: usize = 120;
const DESCRIPTION_MOST: usize = 2_000;

/// `POST /api/scenarios`: version 1, open at revision 1, on a site world-proto-server lists, every field stated
/// (section 3.2's defaults for a group the body leaves out) and set by the person.
pub async fn create<S: Wps>(
    State(state): State<GateState<S>>,
    Extension(identity): Extension<Identity>,
    uri: Uri,
    body: Bytes,
) -> Result<(StatusCode, Json<CreatedScenario>), ProblemAnswer> {
    let instance = uri.path();
    let CreateScenario { name, description, site_scope, workload, run_controls } = json_body(&body, instance)?;
    let description = description.unwrap_or_default();
    texts(&name, &description).map_err(|found| refused(found, instance))?;
    if !is_listed(&state, &site_scope, instance).await? {
        let detail = format!("world-proto-server lists no site {site_scope}, so it has no scenario to make.");
        return Err(Problem::new(ProblemType::NotFound, detail).at(instance).into());
    }
    let defaults = stated_defaults();
    let content = ScenarioContent {
        workload: workload.unwrap_or(defaults.workload),
        run_controls: run_controls.unwrap_or(defaults.run_controls),
    };
    let found = validate(&content);
    if !found.is_empty() {
        return Err(refused(found, instance));
    }
    let person = person(&identity);
    let fault = |fault| internal("make the scenario", fault, instance);
    let mut tx = state.app.pool.begin().await.map_err(fault)?;
    let id = make(&mut tx, &site_scope, &name, &description, &person, &content, instance).await?;
    tx.commit().await.map_err(fault)?;
    Ok((StatusCode::CREATED, Json(CreatedScenario { scenario: id, version: VersionNumber(1), revision: Revision(1) })))
}

/// `PATCH /api/scenarios/{s}`: section 3.5's edit, in one transaction under the scenario's and the version's locks.
pub async fn edit(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
    body: Bytes,
) -> Result<Json<ScenarioEdited>, ProblemAnswer> {
    let instance = uri.path();
    let id = id_in(path, "scenario", instance)?;
    let edit: ScenarioEdit = json_body(&body, instance)?;
    if edit.set.is_empty() {
        let found = vec![Violation { pointer: "/set".to_owned(), rule: "holds at least one field".to_owned() }];
        return Err(refused(found, instance));
    }
    let (row_fields, content_fields): (Vec<FieldSet>, Vec<FieldSet>) =
        edit.set.iter().cloned().partition(|field| ROW_FIELDS.contains(&field.pointer.as_str()));
    let fault = |fault| internal("edit the scenario", fault, instance);
    let mut tx = state.pool.begin().await.map_err(fault)?;
    let row =
        rows::scenario(&mut tx, &id, true).await.map_err(fault)?.ok_or_else(|| not_found("scenario", instance))?;
    if row.deleted_at.is_some() {
        return Err(deleted(&row, instance));
    }
    let latest = rows::version(&mut tx, &id, row.latest_version, true).await.map_err(fault)?;
    let latest = latest.ok_or_else(|| no_version(row.latest_version, instance))?;
    if edit.version != latest.n {
        let detail = format!(
            "Version {} is not the latest: version {} is, and only the latest version is edited.",
            edit.version.0, latest.n.0
        );
        return Err(changed(detail, older(&latest, &edit.set), instance));
    }
    if !row_fields.is_empty() {
        rename(&mut tx, &row, &row_fields, instance).await?;
    }
    let edited = if content_fields.is_empty() {
        ScenarioEdited { version: latest.n, revision: latest.revision, opened: false }
    } else {
        merge(&mut tx, &row, latest, edit.revision, &content_fields, &person(&identity), instance).await?
    };
    tx.commit().await.map_err(fault)?;
    Ok(Json(edited))
}

/// The edit's fields merged into the latest version: the next revision of an open version, or a sealed version's
/// copy opened as the next version, its fields' records carried at revision 1.
async fn merge(
    conn: &mut PgConnection,
    row: &ScenarioRow,
    latest: VersionRecord,
    base: Revision,
    fields: &[FieldSet],
    person: &Person,
    instance: &str,
) -> Result<ScenarioEdited, ProblemAnswer> {
    if base > latest.revision {
        let rule = format!("is at most {}, this version's revision", latest.revision.0);
        return Err(refused(vec![Violation { pointer: "/revision".to_owned(), rule }], instance));
    }
    let found = conflicts(&latest.content, &latest.set_by, base, fields);
    if !found.is_empty() {
        let detail = format!(
            "{} changed after revision {} of version {}, which this edit was based on; it was not saved.",
            changed_fields(&found),
            base.0,
            latest.n.0
        );
        return Err(changed(detail, found, instance));
    }
    let fault = |fault| internal("save the edit", fault, instance);
    let opened = latest.state == VersionState::Sealed;
    let (n, revision, prior) = if opened {
        let carried: SetByMap = latest
            .set_by
            .iter()
            .map(|(pointer, last)| (pointer.clone(), SetBy { revision: Revision(1), ..last.clone() }))
            .collect();
        (VersionNumber(latest.n.0 + 1), Revision(1), carried)
    } else {
        (latest.n, Revision(latest.revision.0 + 1), latest.set_by.clone())
    };
    let change = SetBy { by: Setter::User, author: person.clone(), turn: None, at: WallClock::now(), revision };
    let (content, set_by) =
        apply(&latest.content, &prior, fields, &change).map_err(|found| refused(found, instance))?;
    if opened {
        rows::insert_version(conn, &row.id, n, &content, &set_by, person).await.map_err(fault)?;
    } else {
        let record = VersionRecord { revision, content, set_by, ..latest };
        rows::update_version(conn, &row.id, &record).await.map_err(fault)?;
    }
    Ok(ScenarioEdited { version: n, revision, opened })
}

/// An edit of an older version meets the latest version's value of each field it sets.
fn older(latest: &VersionRecord, set: &[FieldSet]) -> Vec<FieldConflict> {
    let now = serde_json::to_value(&latest.content).expect("a scenario's content is JSON");
    set.iter()
        .map(|field| FieldConflict {
            pointer: field.pointer.clone(),
            value: now.pointer(&field.pointer).cloned().unwrap_or_default(),
            set_by: latest.set_by.get(&field.pointer).cloned(),
        })
        .collect()
}

fn changed_fields(found: &[FieldConflict]) -> String {
    let labels: Vec<String> = found.iter().map(|conflict| super::label(&conflict.pointer)).collect();
    match labels.as_slice() {
        [one] => one.clone(),
        _ => format!("{} fields ({})", labels.len(), labels.join(", ")),
    }
}

/// The scenario row's name or description, which open no version; a name the site already uses is refused.
async fn rename(
    conn: &mut PgConnection,
    row: &ScenarioRow,
    fields: &[FieldSet],
    instance: &str,
) -> Result<(), ProblemAnswer> {
    let (mut name, mut description) = (row.name.clone(), row.description.clone());
    for field in fields {
        let Some(text) = field.value.as_str() else {
            let found = vec![Violation { pointer: field.pointer.clone(), rule: "is text".to_owned() }];
            return Err(refused(found, instance));
        };
        if field.pointer == "/name" { name = text.trim().to_owned() } else { description = text.to_owned() }
    }
    texts(&name, &description).map_err(|found| refused(found, instance))?;
    let fault = |fault| internal("rename the scenario", fault, instance);
    if let Some(other) = rows::named(conn, &row.site_scope, &name).await.map_err(fault)?
        && other != row.id
    {
        return Err(name_taken(&name, &other, instance));
    }
    let mut attempt = conn.begin().await.map_err(fault)?;
    let written = rows::rename(&mut attempt, &row.id, &name, &description).await;
    match settle(attempt, written, instance).await? {
        Named::Written => Ok(()),
        Named::Taken => Err(winner(conn, &row.site_scope, &name, instance).await),
    }
}

/// What a write of a name the site's unique index guards came to.
enum Named {
    Written,
    Taken,
}

/// Settle `written`, made under the savepoint `attempt`: released when it went through; rolled back, so the
/// transaction reads on, when the site's unique index refused a name another request took between the check and the
/// write. Every other fault is the service's.
async fn settle(
    attempt: Transaction<'_, Postgres>,
    written: Result<(), SqlxError>,
    instance: &str,
) -> Result<Named, ProblemAnswer> {
    let fault = |fault| internal("write the scenario", fault, instance);
    match written {
        Ok(()) => attempt.commit().await.map(|()| Named::Written).map_err(fault),
        Err(refused) if refused.as_database_error().and_then(|error| error.constraint()) == Some(NAME_INDEX) => {
            attempt.rollback().await.map(|()| Named::Taken).map_err(fault)
        }
        Err(refused) => Err(fault(refused)),
    }
}

const NAME_INDEX: &str = "scenarios_one_name_per_site";

/// The answer to a name the index refused: `name-taken` with the winning row's reference, read by a statement after
/// the rollback, which sees the winner's commit, as the check's own collision answers it.
async fn winner(conn: &mut PgConnection, site: &SitePath, name: &str, instance: &str) -> ProblemAnswer {
    match rows::named(conn, site, name).await {
        Ok(Some(winner)) => name_taken(name, &winner, instance),
        // The winner was deleted or renamed since its write refused this one: the refusal stands, linking nothing.
        Ok(None) => {
            Problem::new(ProblemType::NameTaken, format!("{name} already exists on this site.")).at(instance).into()
        }
        Err(fault) => internal("read the scenario that holds the name", fault, instance),
    }
}

/// `POST /api/scenarios/{s}/duplicate`: a version, the latest by default, copied into a new scenario on the same
/// site at version 1, named "{name} (copy)" unless the body names it.
pub async fn duplicate(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
    body: Bytes,
) -> Result<(StatusCode, Json<CreatedScenario>), ProblemAnswer> {
    let instance = uri.path();
    let id = id_in(path, "scenario", instance)?;
    let DuplicateScenario { name, version } = json_body(&body, instance)?;
    let fault = |fault| internal("duplicate the scenario", fault, instance);
    let mut tx = state.pool.begin().await.map_err(fault)?;
    let row =
        rows::scenario(&mut tx, &id, false).await.map_err(fault)?.ok_or_else(|| not_found("scenario", instance))?;
    let n = version.unwrap_or(row.latest_version);
    let source = rows::version(&mut tx, &id, n, false).await.map_err(fault)?.ok_or_else(|| no_version(n, instance))?;
    let name = match name {
        Some(name) => name.trim().to_owned(),
        None => free_copy_name(&mut tx, &row, instance).await?,
    };
    let made =
        make(&mut tx, &row.site_scope, &name, &row.description, &person(&identity), &source.content, instance).await?;
    tx.commit().await.map_err(fault)?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedScenario { scenario: made, version: VersionNumber(1), revision: Revision(1) }),
    ))
}

/// `DELETE /api/scenarios/{s}`: the row marked deleted, its versions, experiments and runs kept; a second delete
/// answers the same.
pub async fn delete(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<ScenarioRow>, ProblemAnswer> {
    let instance = uri.path();
    let id = id_in(path, "scenario", instance)?;
    let fault = |fault| internal("delete the scenario", fault, instance);
    let mut tx = state.pool.begin().await.map_err(fault)?;
    rows::scenario(&mut tx, &id, true).await.map_err(fault)?.ok_or_else(|| not_found("scenario", instance))?;
    rows::delete(&mut tx, &id, &person(&identity)).await.map_err(fault)?;
    let row =
        rows::scenario(&mut tx, &id, false).await.map_err(fault)?.ok_or_else(|| not_found("scenario", instance))?;
    tx.commit().await.map_err(fault)?;
    Ok(Json(row))
}

/// A new scenario and its version 1 in the caller's transaction, every field set by `by` at revision 1.
async fn make(
    conn: &mut PgConnection,
    site: &SitePath,
    name: &str,
    description: &str,
    by: &Person,
    content: &ScenarioContent,
    instance: &str,
) -> Result<Ulid, ProblemAnswer> {
    texts(name, description).map_err(|found| refused(found, instance))?;
    let fault = |fault| internal("make the scenario", fault, instance);
    let project = default_project(conn).await.map_err(fault)?.ok_or_else(|| {
        let detail = "A scenario needs a project, and there is no default project.";
        ProblemAnswer::from(Problem::new(ProblemType::ProjectRequired, detail).at(instance))
    })?;
    if let Some(existing) = rows::named(conn, site, name).await.map_err(fault)? {
        return Err(name_taken(name, &existing, instance));
    }
    let id = mint_ulid();
    let new = NewScenario { id: &id, project: &project, name, description, site, by };
    let mut attempt = conn.begin().await.map_err(fault)?;
    let written = rows::insert_scenario(&mut attempt, new).await;
    if let Named::Taken = settle(attempt, written, instance).await? {
        return Err(winner(conn, site, name, instance).await);
    }
    let change =
        SetBy { by: Setter::User, author: by.clone(), turn: None, at: WallClock::now(), revision: Revision(1) };
    rows::insert_version(conn, &id, VersionNumber(1), content, &every_field(&change), by).await.map_err(fault)?;
    Ok(id)
}

/// "{name} (copy)", or "(copy 2)" and on when that is taken, the name cut short to fit.
async fn free_copy_name(conn: &mut PgConnection, row: &ScenarioRow, instance: &str) -> Result<String, ProblemAnswer> {
    let fault = |fault| internal("name the copy", fault, instance);
    for n in 1..=99 {
        let tail = if n == 1 { " (copy)".to_owned() } else { format!(" (copy {n})") };
        let stem: String = row.name.chars().take(NAME_MOST - tail.chars().count()).collect();
        let name = format!("{stem}{tail}");
        if rows::named(conn, &row.site_scope, &name).await.map_err(fault)?.is_none() {
            return Ok(name);
        }
    }
    Err(name_taken(&row.name, &row.id, instance))
}

fn name_taken(name: &str, existing: &Ulid, instance: &str) -> ProblemAnswer {
    let detail = format!("{name} already exists on this site.");
    let mut problem = Problem::new(ProblemType::NameTaken, detail.clone()).at(instance);
    problem.problems = vec![ProblemItem {
        code: "name-taken".to_owned(),
        severity: Severity::Error,
        ref_: format!("scenario:{existing}").parse().ok(),
        scope: None,
        field: Some("/name".to_owned()),
        knob: None,
        text: detail,
    }];
    problem.into()
}

/// A name of 1 to 120 characters and a description of at most 2,000.
fn texts(name: &str, description: &str) -> Result<(), Vec<Violation>> {
    let mut found = Vec::new();
    if name.trim().is_empty() || name.chars().count() > NAME_MOST {
        found.push(Violation { pointer: "/name".to_owned(), rule: "takes 1 to 120 characters".to_owned() });
    }
    if description.chars().count() > DESCRIPTION_MOST {
        found.push(Violation { pointer: "/description".to_owned(), rule: "takes at most 2,000 characters".to_owned() });
    }
    if found.is_empty() { Ok(()) } else { Err(found) }
}

fn person(identity: &Identity) -> Person {
    Person { subject: identity.subject.clone(), name: identity.name.clone() }
}
