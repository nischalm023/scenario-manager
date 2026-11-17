//! Within `results`, `pins` is `PUT /api/projects/{p}/metric-pins` (spec 08 section 3.7): a project's headline metrics
//! set as data, at most eight names in order, replacing the project's list whole, so no metric name is ever written
//! into code (section 3.9). No panel control sets them yet (brief I1.10, lead decided); the agent's tools and a
//! person's request do.

use axum::Extension;
use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, State};
use axum::http::Uri;
use studio_protocol::{MetricPins, Problem, ProblemType, Ulid};

use crate::AppState;
use crate::identity::Identity;
use crate::problem::{ProblemAnswer, internal, json_body, validation_failed};

/// The most metrics a project headlines.
const MOST: usize = 8;

/// `PUT /api/projects/{p}/metric-pins`: `{ metrics }`, answered with the list as stored.
pub async fn put(
    State(app): State<AppState>,
    Extension(identity): Extension<Identity>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
    body: Bytes,
) -> Result<Json<MetricPins>, ProblemAnswer> {
    let instance = uri.path();
    let nothing = || ProblemAnswer::from(Problem::new(ProblemType::NotFound, "No project has that id.").at(instance));
    let Ok(Path(id)) = path else { return Err(nothing()) };
    let project: Ulid = id.parse().map_err(|_| nothing())?;
    let MetricPins { metrics } = json_body(&body, instance)?;
    if metrics.len() > MOST {
        return Err(validation_failed(
            format!("a project pins at most {MOST} metrics; this names {}", metrics.len()),
            instance,
        ));
    }
    if let Some(blank) = metrics.iter().position(|metric| metric.trim().is_empty()) {
        return Err(validation_failed(format!("metric {} of the list is blank", blank + 1), instance));
    }
    if let Some(twice) = (1..metrics.len()).find(|&at| metrics[..at].contains(&metrics[at])) {
        return Err(validation_failed(format!("{} is named twice", metrics[twice]), instance));
    }
    let fault = |fault| internal("pin the project's metrics", fault, instance);
    let mut tx = app.pool.begin().await.map_err(fault)?;
    let known: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM projects WHERE id = $1)")
        .bind(project.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(fault)?;
    if !known {
        return Err(nothing());
    }
    sqlx::query("DELETE FROM metric_pins WHERE project = $1")
        .bind(project.as_str())
        .execute(&mut *tx)
        .await
        .map_err(fault)?;
    for (at, metric) in metrics.iter().enumerate() {
        sqlx::query("INSERT INTO metric_pins (project, position, metric, pinned_by) VALUES ($1, $2, $3, $4)")
            .bind(project.as_str())
            .bind(i32::try_from(at + 1).unwrap_or(i32::MAX))
            .bind(metric)
            .bind(identity.subject.to_string())
            .execute(&mut *tx)
            .await
            .map_err(fault)?;
    }
    tx.commit().await.map_err(fault)?;
    Ok(Json(MetricPins { metrics }))
}
