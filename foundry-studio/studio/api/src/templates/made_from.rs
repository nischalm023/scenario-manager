//! Within `templates`, `made_from` is `GET /api/made-from/{*site}` (spec 05 sections 3.7 and 3.8): the acts that made
//! or updated a site from a template, newest first, each with the template at the changesets it was read at, the
//! knob values with the labels and units the draft read with them, and who acted when; and the one act a cut stands
//! on, which the Map panel's summary carries.

use std::collections::BTreeMap;

use axum::Json;
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, State};
use axum::http::Uri;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::{Error as SqlxError, PgPool, Row};
use studio_protocol::{
    KnobDeclaration, KnobValue, MadeFrom, MadeFromKnob, MadeFromList, Person, ScopePath, Seq, SitePath, WallClock,
};

use crate::AppState;
use crate::db::{decode, evaluation_of, read_word};
use crate::problem::{ProblemAnswer, internal, validation_failed};

/// `GET /api/made-from/{*site}`.
pub async fn list(
    State(state): State<AppState>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<MadeFromList>, ProblemAnswer> {
    let instance = uri.path();
    let Ok(Path(site)) = path else { return Err(validation_failed("the path names no site", instance)) };
    let site: SitePath = site.parse().map_err(|fault| validation_failed(fault, instance))?;
    let fault = |fault| internal("read the site's made-from records", fault, instance);
    let rows = sqlx::query(ROWS).bind(site.as_str()).fetch_all(&state.pool).await.map_err(fault)?;
    let items = rows.iter().map(|row| made_from(&site, row)).collect::<Result<_, _>>().map_err(fault)?;
    Ok(Json(MadeFromList { items }))
}

/// A site's made and updated acts, newest first.
const ROWS: &str = "SELECT m.id, m.act, m.template, m.tag, m.template_seqs, m.knobs, m.site_seqs, m.at, m.author, \
         p.name AS author_name, m.draft, m.evaluation, d.declarations \
     FROM site_made_from m JOIN template_drafts d ON d.id = m.draft LEFT JOIN people p ON p.subject = m.author \
     WHERE m.site = $1 AND m.state IN ('made', 'updated') ORDER BY m.at DESC, m.id DESC";

/// The act a cut of `site` stands on: the newest whose every scope's changeset is at or below the cut's (spec 05
/// section 3.7, as a run's cut finds it), which the Map panel's "Made from" shows (spec 03 section 3.6).
pub async fn newest_made_from(
    pool: &PgPool,
    site: &SitePath,
    cut: &BTreeMap<ScopePath, Seq>,
) -> Result<Option<MadeFrom>, SqlxError> {
    for row in sqlx::query(ROWS).bind(site.as_str()).fetch_all(pool).await? {
        let made = made_from(site, &row)?;
        if made.site_seqs.iter().all(|(scope, seq)| cut.get(scope).is_some_and(|at| at >= seq)) {
            return Ok(Some(made));
        }
    }
    Ok(None)
}

fn made_from(site: &SitePath, row: &PgRow) -> Result<MadeFrom, SqlxError> {
    let json = |column: &str| row.try_get::<Value, _>(column);
    let declarations: Vec<KnobDeclaration> = serde_json::from_value(json("declarations")?).map_err(decode)?;
    let values: BTreeMap<String, KnobValue> = serde_json::from_value(json("knobs")?).map_err(decode)?;
    let knobs = declarations
        .iter()
        .filter_map(|knob| {
            let value = *values.get(&knob.key)?;
            Some(MadeFromKnob { key: knob.key.clone(), label: knob.label.clone(), unit: knob.unit, value })
        })
        .collect();
    let subject: String = row.try_get("author")?;
    let name: Option<String> = row.try_get("author_name")?;
    Ok(MadeFrom {
        id: row.try_get::<String, _>("id")?.parse().map_err(decode)?,
        site: site.clone(),
        act: read_word(&row.try_get::<String, _>("act")?)?,
        template: row.try_get::<String, _>("template")?.parse().map_err(decode)?,
        tag: row.try_get::<Option<String>, _>("tag")?.map(|tag| tag.parse()).transpose().map_err(decode)?,
        template_seqs: serde_json::from_value(json("template_seqs")?).map_err(decode)?,
        knobs,
        site_seqs: serde_json::from_value(json("site_seqs")?).map_err(decode)?,
        at: WallClock::from(row.try_get::<DateTime<Utc>, _>("at")?),
        author: Person { name: name.unwrap_or_else(|| subject.clone()), subject: subject.parse().map_err(decode)? },
        draft: row.try_get::<String, _>("draft")?.parse().map_err(decode)?,
        evaluation: evaluation_of(row.try_get("evaluation")?)?,
    })
}
