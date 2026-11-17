//! Within `floors`, `route` is `GET /api/floors/{*key}` (spec 03 section 3.6): every floor by the key its summary
//! named, `application/vnd.worldview-floor+msgpack` and immutable, since one key never holds two floors. A site's
//! floor comes from the client's floor cache, compiled again from the cut its key was minted for when the cache has
//! let it go; a template evaluation's from the run-object store, where spec 05's evaluation keeps it under
//! `template-drafts/{draft}/{n}/floor.msgpack`, and a template's default evaluation behind its card under
//! `template-thumbnails/{row}/floor.msgpack`, keys holding slashes, which is why the route ends in a wildcard.

use axum::Router;
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use bytes::Bytes;
use runstore::RunStorage;
use studio_protocol::{Problem, ProblemType, Ulid};
use wps_client::WorldStore;
use wps_client::cache::FloorKey;
use wps_client::wps::Wps;

use super::{floor_problem, read_key, site_floor};
use crate::GateState;
use crate::problem::{ProblemAnswer, wps_problem};

pub const FLOOR_TYPE: &str = "application/vnd.worldview-floor+msgpack";
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

pub fn routes<S: Wps>() -> Router<GateState<S>> {
    Router::new().route("/api/floors/{*key}", get(floor))
}

/// `GET /api/floors/{*key}`.
async fn floor<S: Wps>(
    State(state): State<GateState<S>>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
) -> Result<Response, ProblemAnswer> {
    let instance = uri.path();
    let Ok(Path(key)) = path else { return Err(gone(instance)) };
    let bytes = match read_key(&key) {
        Some(key) => site(&state, key, instance).await?,
        None if is_kept_floor(&key) => state.app.runstore.get(&key).await.map_err(|_| gone(instance))?,
        None => return Err(gone(instance)),
    };
    let headers =
        [(CONTENT_TYPE, HeaderValue::from_static(FLOOR_TYPE)), (CACHE_CONTROL, HeaderValue::from_static(IMMUTABLE))];
    Ok((headers, bytes).into_response())
}

/// A site's floor: held, or compiling now and answered when that compile ends, or compiled again from its origin.
async fn site<S: Wps>(state: &GateState<S>, key: FloorKey, instance: &str) -> Result<Bytes, ProblemAnswer> {
    let floors = state.world.floors();
    if let Some(held) = floors.held(&key) {
        return Ok(held);
    }
    let Some(cut) = state.app.floor_origins.of(&key) else { return Err(gone(instance)) };
    let export = state.world.export(&cut).await.map_err(|error| wps_problem(&error, instance))?;
    // Until world-proto-server keeps exact history, a cut below the head reads the head's labels, so its export can
    // change; bytes other than the key's are never served under it.
    if export.sha256 != key.document {
        return Err(gone(instance));
    }
    site_floor(floors, state.app.compute.clone(), key, export.bytes)
        .await
        .map_err(|failure| floor_problem(&failure, instance))
}

/// `template-drafts/{draft}/{n}/floor.msgpack` or `template-thumbnails/{row}/floor.msgpack`, and nothing else of the
/// store's.
fn is_kept_floor(key: &str) -> bool {
    match key.split('/').collect::<Vec<_>>()[..] {
        ["template-drafts", draft, n, "floor.msgpack"] => {
            draft.parse::<Ulid>().is_ok() && n.parse::<u32>().is_ok_and(|number| number > 0 && number.to_string() == n)
        }
        ["template-thumbnails", row, "floor.msgpack"] => row.parse::<Ulid>().is_ok(),
        _ => false,
    }
}

fn gone(instance: &str) -> ProblemAnswer {
    let detail = "No floor is held under this key; read the map again for its floor's key now.";
    Problem::new(ProblemType::NotFound, detail).at(instance).into()
}
