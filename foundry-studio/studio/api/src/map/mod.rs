//! Within `studio-api`, `map` is the Map panel's server half (spec 03 sections 3.5 and 3.6; BACKLOG task 11): a site
//! read at a cut as the panel, its highlights and the agent's references need it. The site's flagged export, the
//! bytes a run of that cut loads, is loaded on the compute pool (`loaded`) and kept per floor key (`read`); from it
//! `GET /api/map/{*location}` answers the summary (`summary`), `GET /api/find/{*location}` searches (`find`), and
//! `POST /api/resolve` expands references into marks (`resolve`) by one rule table (`marks`).

mod find;
mod loaded;
mod marks;
mod read;
mod resolve;
mod summary;

use axum::Router;
use axum::routing::{get, post};
use wps_client::wps::Wps;

use crate::GateState;

pub(crate) use loaded::frames;
pub use loaded::{Entry, Loaded, Place};
pub use marks::{in_mark_order, marks_for_map};
pub use read::{Kept, SiteAt, site_at};
/// The routes the agent's `site.read`, `entity.find` and panel commands call as the panels do (brief I1.03 section 3).
pub(crate) use {find::find, resolve::map_marks, resolve::resolve, summary::summary};

pub fn routes<S: Wps>() -> Router<GateState<S>> {
    Router::new()
        .route("/api/map/{*location}", get(summary::summary))
        .route("/api/find/{*location}", get(find::find))
        .route("/api/resolve", post(resolve::resolve))
}
