//! Within `map`, `find` is `GET /api/find/{*location}?kin=&q=&limit=50` (spec 03 section 3.6), the Map panel's Find
//! and the `entity.find` tool's function: the entities of a site at a cut whose label, a component's name or an
//! `external_ids` value holds `q`, ignoring case, each as its own mark, in mark order. It searches the loaded export,
//! declared and derived entities alike, which also places each match on the floor.

use axum::Json;
use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::Uri;
use serde::Deserialize;
use studio_protocol::{FindResults, Problem, ProblemType};
use wps_client::wps::Wps;

use super::marks::{in_mark_order, mark};
use super::read::site_at;
use crate::GateState;
use crate::problem::{ProblemAnswer, validation_failed};

/// How many matches one answer holds: 50 unless `limit` asks, never fewer than one nor more than 500.
#[derive(Clone, Copy, Deserialize)]
#[serde(transparent)]
struct ResultLimit(usize);

impl ResultLimit {
    const UNASKED: Self = Self(50);

    fn held(self) -> usize {
        self.0.clamp(1, 500)
    }
}

#[derive(Deserialize)]
pub struct FindQuery {
    kin: Option<String>,
    q: Option<String>,
    limit: Option<ResultLimit>,
}

/// `GET /api/find/{*location}?kin=&q=&limit=`.
pub async fn find<S: Wps>(
    State(state): State<GateState<S>>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
    query: Result<Query<FindQuery>, QueryRejection>,
) -> Result<Json<FindResults>, ProblemAnswer> {
    let instance = uri.path();
    let Ok(Path(location)) = path else { return Err(validation_failed("the path names no location", instance)) };
    let Query(FindQuery { kin, q, limit }) = query.map_err(|fault| validation_failed(fault.body_text(), instance))?;
    let wanted = q.unwrap_or_default().trim().to_lowercase();
    if wanted.is_empty() {
        return Err(validation_failed("`q` names what to find", instance));
    }
    let limit = limit.unwrap_or(ResultLimit::UNASKED).held();
    let at = site_at(&state, &location, kin.as_deref(), instance).await?;
    let Ok(loaded) = at.loaded.as_ref() else {
        let detail = "This map cannot be searched: the site's document will not load.";
        return Err(Problem::new(ProblemType::FloorCompileFailed, detail).at(instance).into());
    };
    let holds = |text: &str| text.to_lowercase().contains(&wanted);
    let mut items: Vec<_> = loaded
        .entities
        .iter()
        .filter(|(_, entry)| {
            holds(&entry.label)
                || entry.components.iter().any(|name| holds(name))
                || entry.external.iter().any(|value| holds(value))
        })
        .map(|(id, entry)| mark(id, entry))
        .collect();
    in_mark_order(&mut items);
    let more = items.len() > limit;
    items.truncate(limit);
    Ok(Json(FindResults { items, more }))
}
