//! Within `scenarios`, `rows` reads and writes spec 07 section 3.1's three tables: a scenario's row with the people
//! who made and deleted it, its versions, each version's content split into the `workload` and `run_controls`
//! columns, and the experiments prepared from them. Callers hold the transaction and the locks; a value a column
//! promises but does not hold is a decode fault.

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::{Error as SqlxError, PgConnection, Row};
use studio_protocol::{
    Experiment, ExperimentKind, Person, Revision, RunCount, RunCounts, RunOverrides, RunSlot, ScenarioContent,
    ScenarioRow, ScenarioSummary, Seed, SitePath, Sweep, Ulid, Variant, VersionNumber, VersionState, VersionSummary,
    WallClock,
};

use crate::db::decode;

use super::merge::SetByMap;

/// One version as stored: its content and each field's last change.
#[derive(Debug, Clone, PartialEq)]
pub struct VersionRecord {
    pub n: VersionNumber,
    pub state: VersionState,
    pub revision: Revision,
    pub content: ScenarioContent,
    pub set_by: SetByMap,
    pub sealed_by: Option<Ulid>,
}

const SCENARIO: &str = "SELECT s.id, s.name, s.description, s.site_scope, s.latest_version, s.created_by, \
     c.name AS created_name, s.created_at, s.changed_at, s.deleted_by, d.name AS deleted_name, s.deleted_at \
     FROM scenarios s JOIN people c ON c.subject = s.created_by LEFT JOIN people d ON d.subject = s.deleted_by \
     WHERE s.id = $1";

/// The scenario `id`, locked for the caller's transaction when `lock`.
pub async fn scenario(conn: &mut PgConnection, id: &Ulid, lock: bool) -> Result<Option<ScenarioRow>, SqlxError> {
    let sql = if lock { format!("{SCENARIO} FOR UPDATE OF s") } else { SCENARIO.to_owned() };
    let row = sqlx::query(&sql).bind(id.as_str()).fetch_optional(conn).await?;
    row.as_ref().map(scenario_row).transpose()
}

fn scenario_row(row: &PgRow) -> Result<ScenarioRow, SqlxError> {
    let person = |subject: Option<String>, name: Option<String>| -> Result<Option<Person>, SqlxError> {
        match (subject, name) {
            (Some(subject), Some(name)) => Ok(Some(Person { subject: subject.parse().map_err(decode)?, name })),
            _ => Ok(None),
        }
    };
    let at = |column: &str| row.try_get::<DateTime<Utc>, _>(column).map(WallClock::from);
    Ok(ScenarioRow {
        id: row.try_get::<String, _>("id")?.parse().map_err(decode)?,
        name: row.try_get("name")?,
        description: row.try_get("description")?,
        site_scope: row.try_get::<String, _>("site_scope")?.parse().map_err(decode)?,
        latest_version: VersionNumber(number(row.try_get("latest_version")?)?),
        created_by: person(row.try_get("created_by")?, row.try_get("created_name")?)?
            .ok_or_else(|| decode("a scenario names who made it"))?,
        created_at: at("created_at")?,
        changed_at: at("changed_at")?,
        deleted_by: person(row.try_get("deleted_by")?, row.try_get("deleted_name")?)?,
        deleted_at: row.try_get::<Option<DateTime<Utc>>, _>("deleted_at")?.map(WallClock::from),
    })
}

/// Version `n` of `scenario`, locked for the caller's transaction when `lock`.
pub async fn version(
    conn: &mut PgConnection,
    scenario: &Ulid,
    n: VersionNumber,
    lock: bool,
) -> Result<Option<VersionRecord>, SqlxError> {
    let sql = format!(
        "SELECT version, state, revision, workload, run_controls, set_by, sealed_by FROM scenario_versions \
         WHERE scenario = $1 AND version = $2{}",
        if lock { " FOR UPDATE" } else { "" }
    );
    let row = sqlx::query(&sql).bind(scenario.as_str()).bind(column(n.0)).fetch_optional(conn).await?;
    let Some(row) = row else { return Ok(None) };
    let content = ScenarioContent {
        workload: json(row.try_get("workload")?)?,
        run_controls: json(row.try_get("run_controls")?)?,
    };
    Ok(Some(VersionRecord {
        n: VersionNumber(number(row.try_get("version")?)?),
        state: state(&row.try_get::<String, _>("state")?)?,
        revision: Revision(number(row.try_get("revision")?)?),
        content,
        set_by: json(row.try_get("set_by")?)?,
        sealed_by: row.try_get::<Option<String>, _>("sealed_by")?.map(|id| id.parse().map_err(decode)).transpose()?,
    }))
}

/// Every version of `scenario`, oldest first, each with the experiments prepared from it.
pub async fn versions(conn: &mut PgConnection, scenario: &Ulid) -> Result<Vec<VersionSummary>, SqlxError> {
    let rows = sqlx::query(
        "SELECT v.version, v.state, v.sealed_at, \
             COALESCE(array_agg(e.id ORDER BY e.prepared_at, e.id) FILTER (WHERE e.id IS NOT NULL), '{}') AS experiments \
         FROM scenario_versions v LEFT JOIN experiments e ON e.scenario = v.scenario AND e.version = v.version \
         WHERE v.scenario = $1 GROUP BY v.version, v.state, v.sealed_at ORDER BY v.version",
    )
    .bind(scenario.as_str())
    .fetch_all(conn)
    .await?;
    rows.iter()
        .map(|row| {
            let experiments: Vec<String> = row.try_get("experiments")?;
            Ok(VersionSummary {
                version: VersionNumber(number(row.try_get("version")?)?),
                state: state(&row.try_get::<String, _>("state")?)?,
                sealed_at: row.try_get::<Option<DateTime<Utc>>, _>("sealed_at")?.map(WallClock::from),
                experiments: experiments.iter().map(|id| id.parse().map_err(decode)).collect::<Result<_, _>>()?,
            })
        })
        .collect()
}

/// A site's scenarios, newest change first; the deleted ones too when `deleted`.
pub async fn list(conn: &mut PgConnection, site: &SitePath, deleted: bool) -> Result<Vec<ScenarioSummary>, SqlxError> {
    let rows = sqlx::query(
        "SELECT s.id, s.name, s.latest_version, v.state, s.changed_at, s.deleted_at FROM scenarios s \
         JOIN scenario_versions v ON v.scenario = s.id AND v.version = s.latest_version \
         WHERE s.site_scope = $1 AND ($2 OR s.deleted_at IS NULL) ORDER BY s.changed_at DESC, s.id",
    )
    .bind(site.as_str())
    .bind(deleted)
    .fetch_all(conn)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(ScenarioSummary {
                id: row.try_get::<String, _>("id")?.parse().map_err(decode)?,
                name: row.try_get("name")?,
                latest_version: VersionNumber(number(row.try_get("latest_version")?)?),
                state: state(&row.try_get::<String, _>("state")?)?,
                runs: RunCounts { total: RunCount(0), running: RunCount(0) },
                changed_at: WallClock::from(row.try_get::<DateTime<Utc>, _>("changed_at")?),
                deleted_at: row.try_get::<Option<DateTime<Utc>>, _>("deleted_at")?.map(WallClock::from),
            })
        })
        .collect()
}

/// The undeleted scenario of the site that holds `name`, compared case-insensitively.
pub async fn named(conn: &mut PgConnection, site: &SitePath, name: &str) -> Result<Option<Ulid>, SqlxError> {
    let id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM scenarios WHERE site_scope = $1 AND lower(name) = lower($2) AND deleted_at IS NULL",
    )
    .bind(site.as_str())
    .bind(name)
    .fetch_optional(conn)
    .await?;
    id.map(|id| id.parse().map_err(decode)).transpose()
}

pub struct NewScenario<'a> {
    pub id: &'a Ulid,
    pub project: &'a Ulid,
    pub name: &'a str,
    pub description: &'a str,
    pub site: &'a SitePath,
    pub by: &'a Person,
}

pub async fn insert_scenario(conn: &mut PgConnection, new: NewScenario<'_>) -> Result<(), SqlxError> {
    sqlx::query(
        "INSERT INTO scenarios (id, project, name, description, site_scope, created_by) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(new.id.as_str())
    .bind(new.project.as_str())
    .bind(new.name)
    .bind(new.description)
    .bind(new.site.as_str())
    .bind(new.by.subject.as_str())
    .execute(conn)
    .await
    .map(drop)
}

/// A new open version at revision 1, which becomes the scenario's latest.
pub async fn insert_version(
    conn: &mut PgConnection,
    scenario: &Ulid,
    n: VersionNumber,
    content: &ScenarioContent,
    set_by: &SetByMap,
    by: &Person,
) -> Result<(), SqlxError> {
    sqlx::query(
        "INSERT INTO scenario_versions (scenario, version, workload, run_controls, set_by, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(scenario.as_str())
    .bind(column(n.0))
    .bind(to_json(&content.workload))
    .bind(to_json(&content.run_controls))
    .bind(to_json(set_by))
    .bind(by.subject.as_str())
    .execute(&mut *conn)
    .await?;
    sqlx::query("UPDATE scenarios SET latest_version = $2, changed_at = now() WHERE id = $1")
        .bind(scenario.as_str())
        .bind(column(n.0))
        .execute(conn)
        .await
        .map(drop)
}

/// An open version's accepted edit.
pub async fn update_version(conn: &mut PgConnection, scenario: &Ulid, record: &VersionRecord) -> Result<(), SqlxError> {
    sqlx::query(
        "UPDATE scenario_versions SET revision = $3, workload = $4, run_controls = $5, set_by = $6 \
         WHERE scenario = $1 AND version = $2 AND state = 'open'",
    )
    .bind(scenario.as_str())
    .bind(column(record.n.0))
    .bind(column(record.revision.0))
    .bind(to_json(&record.content.workload))
    .bind(to_json(&record.content.run_controls))
    .bind(to_json(&record.set_by))
    .execute(&mut *conn)
    .await?;
    touch(conn, scenario).await
}

/// The scenario row's name and description, which open no version.
pub async fn rename(conn: &mut PgConnection, scenario: &Ulid, name: &str, description: &str) -> Result<(), SqlxError> {
    sqlx::query("UPDATE scenarios SET name = $2, description = $3, changed_at = now() WHERE id = $1")
        .bind(scenario.as_str())
        .bind(name)
        .bind(description)
        .execute(conn)
        .await
        .map(drop)
}

pub async fn touch(conn: &mut PgConnection, scenario: &Ulid) -> Result<(), SqlxError> {
    sqlx::query("UPDATE scenarios SET changed_at = now() WHERE id = $1")
        .bind(scenario.as_str())
        .execute(conn)
        .await
        .map(drop)
}

/// Mark the scenario deleted, unless it already is.
pub async fn delete(conn: &mut PgConnection, scenario: &Ulid, by: &Person) -> Result<(), SqlxError> {
    sqlx::query(
        "UPDATE scenarios SET deleted_by = $2, deleted_at = now(), changed_at = now() WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(scenario.as_str())
    .bind(by.subject.as_str())
    .execute(conn)
    .await
    .map(drop)
}

/// Seal version `n` as the experiment's submission does.
pub async fn seal(conn: &mut PgConnection, scenario: &Ulid, n: VersionNumber, by: &Ulid) -> Result<(), SqlxError> {
    sqlx::query(
        "UPDATE scenario_versions SET state = 'sealed', sealed_at = now(), sealed_by = $3 \
         WHERE scenario = $1 AND version = $2 AND state = 'open'",
    )
    .bind(scenario.as_str())
    .bind(column(n.0))
    .bind(by.as_str())
    .execute(&mut *conn)
    .await?;
    touch(conn, scenario).await
}

pub struct NewExperiment<'a> {
    pub experiment: &'a Experiment,
    pub conversation: Option<&'a Ulid>,
    pub by: &'a Person,
}

pub async fn insert_experiment(conn: &mut PgConnection, new: NewExperiment<'_>) -> Result<(), SqlxError> {
    let experiment = new.experiment;
    let seeds: Vec<i64> = experiment.seeds.iter().map(|seed| i64::try_from(seed.0).unwrap_or(i64::MAX)).collect();
    sqlx::query(
        "INSERT INTO experiments (id, scenario, version, revision, conversation, prepared_by, sweep, variants, seeds, \
         run_overrides) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(experiment.id.as_str())
    .bind(experiment.scenario.as_str())
    .bind(column(experiment.version.0))
    .bind(column(experiment.revision.0))
    .bind(new.conversation.map(Ulid::as_str))
    .bind(new.by.subject.as_str())
    .bind(experiment.sweep.as_ref().map(to_json))
    .bind(to_json(&experiment.variants))
    .bind(seeds)
    .bind(to_json(&experiment.run_overrides))
    .execute(conn)
    .await
    .map(drop)
}

/// An experiment as the API answers it, its run order and kind derived from its variants and seeds.
pub async fn experiment(conn: &mut PgConnection, id: &Ulid) -> Result<Option<Experiment>, SqlxError> {
    let row = sqlx::query(
        "SELECT e.id, e.scenario, e.version, e.revision, e.sweep, e.variants, e.seeds, e.run_overrides, \
             s.id AS submission \
         FROM experiments e LEFT JOIN submissions s ON s.experiment = e.id WHERE e.id = $1",
    )
    .bind(id.as_str())
    .fetch_optional(conn)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let sweep: Option<Sweep> = row.try_get::<Option<Value>, _>("sweep")?.map(json).transpose()?;
    let variants: Vec<Variant> = json(row.try_get("variants")?)?;
    let seeds: Vec<i64> = row.try_get("seeds")?;
    let seeds =
        seeds.into_iter().map(|seed| u64::try_from(seed).map(Seed).map_err(decode)).collect::<Result<_, _>>()?;
    let overrides: RunOverrides = json(row.try_get("run_overrides")?)?;
    let submission =
        row.try_get::<Option<String>, _>("submission")?.map(|id| id.parse().map_err(decode)).transpose()?;
    let experiment = derive(
        row.try_get::<String, _>("id")?.parse().map_err(decode)?,
        row.try_get::<String, _>("scenario")?.parse().map_err(decode)?,
        (VersionNumber(number(row.try_get("version")?)?), Revision(number(row.try_get("revision")?)?)),
        sweep,
        variants,
        seeds,
        overrides,
    );
    Ok(Some(Experiment { submission, ..experiment }))
}

/// An experiment with what is derived from it: every variant at the first seed, then every variant at the next, and
/// its kind (spec 07 section 3.6).
pub fn derive(
    id: Ulid,
    scenario: Ulid,
    (version, revision): (VersionNumber, Revision),
    sweep: Option<Sweep>,
    variants: Vec<Variant>,
    seeds: Vec<Seed>,
    run_overrides: RunOverrides,
) -> Experiment {
    let run_order = seeds
        .iter()
        .flat_map(|seed| variants.iter().map(|variant| RunSlot { variant: variant.n, seed: *seed }))
        .collect();
    let kind = match (variants.len(), seeds.len()) {
        (1, 1) => ExperimentKind::Single,
        (1, _) => ExperimentKind::Repeats,
        _ => ExperimentKind::Sweep,
    };
    Experiment {
        id,
        scenario,
        version,
        revision,
        sweep,
        variants,
        seeds,
        run_overrides,
        run_order,
        kind,
        submission: None,
    }
}

fn state(text: &str) -> Result<VersionState, SqlxError> {
    match text {
        "open" => Ok(VersionState::Open),
        "sealed" => Ok(VersionState::Sealed),
        _ => Err(decode(format!("`{text}` is no version state"))),
    }
}

fn number(value: i32) -> Result<u32, SqlxError> {
    u32::try_from(value).map_err(decode)
}

/// A version or revision number as Postgres's `INTEGER` holds it.
pub fn column(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

fn json<T: DeserializeOwned>(value: Value) -> Result<T, SqlxError> {
    serde_json::from_value(value).map_err(decode)
}

fn to_json<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("a stored value is JSON")
}
