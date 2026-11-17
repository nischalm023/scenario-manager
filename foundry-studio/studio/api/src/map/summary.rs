//! Within `map`, `summary` is `GET /api/map/{*location}?kin=` (spec 03 section 3.6): the Map panel's `MapSummary` of
//! a site at a cut, which names the floor's key the panel then asks `GET /api/floors/{key}` for. A document foundry's
//! reader refuses answers `floor-compile-failed`; one that loads but has no floor answers its counts beside the
//! problem, so the side lists still show what the site holds.

use axum::Json;
use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::Uri;
use serde::Deserialize;
use studio_protocol::{FloorRef, MapFrame, MapSite, MapSummary, Problem, ProblemType, ScopeSeq};
use worldview_render::FLOOR_VERSION;
use wps_client::Exactness;
use wps_client::wps::Wps;

use super::read::site_at;
use crate::GateState;
use crate::floors::key_text;
use crate::problem::{ProblemAnswer, internal, validation_failed};
use crate::templates::newest_made_from;

#[derive(Deserialize)]
pub struct KinQuery {
    kin: Option<String>,
}

/// `GET /api/map/{*location}?kin=`.
pub async fn summary<S: Wps>(
    State(state): State<GateState<S>>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
    query: Result<Query<KinQuery>, QueryRejection>,
) -> Result<Json<MapSummary>, ProblemAnswer> {
    let instance = uri.path();
    let Ok(Path(location)) = path else { return Err(validation_failed("the path names no location", instance)) };
    let Query(KinQuery { kin }) = query.map_err(|fault| validation_failed(fault.body_text(), instance))?;
    let at = site_at(&state, &location, kin.as_deref(), instance).await?;
    let loaded = match at.loaded.as_ref() {
        Ok(loaded) => loaded,
        Err(why) => {
            let detail = format!("This map cannot be drawn: {why}.");
            return Err(Problem::new(ProblemType::FloorCompileFailed, detail).at(instance).into());
        }
    };
    let (floor, floor_problem) = match &loaded.floor {
        Ok(size) => (Some(FloorRef { key: key_text(&at.key), floor_version: FLOOR_VERSION, bytes: size.get() }), None),
        Err(why) => {
            let detail = format!("This map cannot be drawn: {why}.");
            (None, Some(Problem::new(ProblemType::FloorCompileFailed, detail).at(instance)))
        }
    };
    let made_from = newest_made_from(&state.app.pool, &at.site, &at.cut.seqs)
        .await
        .map_err(|fault| internal("read the site's made-from records", fault, instance))?;
    let label = loaded.name.clone().unwrap_or_else(|| at.site.to_string());
    let (exact, exact_reason) = match &at.cut.exact {
        Exactness::Exact => (true, None),
        Exactness::ComponentsOnly { reason } => (false, Some(reason.clone())),
    };
    Ok(Json(MapSummary {
        location: at.location,
        kin: at.kin,
        resolved: at.cut.seqs.iter().map(|(scope, seq)| ScopeSeq { scope: scope.clone(), seq: *seq }).collect(),
        site: MapSite { scope: at.site.clone(), label },
        floor,
        floor_problem,
        empty: loaded.map_empty,
        frame: MapFrame { id: loaded.primary.clone(), unit_mm: loaded.unit_mm },
        axes: loaded.axes.clone(),
        counts: loaded.counts.clone(),
        made_from,
        exact,
        exact_reason,
    }))
}
