//! Within `studio-api`, `templates` is spec 05's template workflow on the API's side (sections 3.6 to 3.12 and 4.1
//! to 4.4): the Template list with each template's thumbnail, one template, template drafts whose knobs a person
//! sets and whose evaluations run on the compute pool with their plans and floors kept in the run-object store,
//! Create site, which writes an evaluation as a new site, and the made-from records a site keeps. The engine is
//! `site-template`; world-proto-server is reached through `wps-client` alone.

mod check;
mod create;
mod declarations;
mod drafts;
mod evaluation;
mod list;
mod made_from;
mod phases;
mod plan;
mod read;
mod words;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::routing::{get, patch, post};
use rustc_hash::FxHashMap;
use serde_json::Value;
use sqlx::Error as SqlxError;
use studio_protocol::{ScopePath, Seq, Ulid};
use wps_client::wps::Wps;

use crate::db::{decode, evaluation_of};
use crate::idempotency::idempotent;
use crate::{AppState, GateState};

/// The routes and acts the agent's template tools call as the Template panels do (brief I1.03 section 3).
pub(crate) use create::create_under;
pub(crate) use drafts::{KnobSetter, again, evaluation as read_evaluation, read_draft, set_knobs_as, start};
pub use evaluation::{Computed, Inputs, compute};
pub(crate) use list::{list, one};
pub use made_from::newest_made_from;
pub use plan::PlanFile;
pub use read::{Read, ReadFault, read};

/// A template and the changeset each of its scopes was read at.
type ReadKey = (ScopePath, BTreeMap<ScopePath, Seq>);

/// What the API keeps between template requests: each template read at its changesets, the schemas the registry
/// published (a published version never changes), and nothing else; thumbnails are kept in Postgres.
#[derive(Default)]
pub struct Kept {
    reads: Mutex<FxHashMap<ReadKey, Arc<Read>>>,
    schemas: Mutex<FxHashMap<(String, String), Value>>,
}

pub fn routes<S: Wps>(app: &AppState) -> Router<GateState<S>> {
    Router::new()
        .route("/api/templates", get(list::list))
        .route("/api/templates/{*location}", get(list::one))
        .route("/api/template-drafts", idempotent(app, post(drafts::start)))
        .route("/api/template-drafts/{draft}", get(drafts::read_draft).delete(drafts::abandon))
        .route("/api/template-drafts/{draft}", idempotent(app, patch(drafts::set_knobs)))
        .route("/api/template-drafts/{draft}/evaluate", idempotent(app, post(drafts::again)))
        .route("/api/template-drafts/{draft}/site", idempotent(app, post(create::create)))
        .route("/api/template-drafts/{draft}/evaluations/{n}", get(drafts::evaluation))
        .route("/api/made-from/{*site}", get(made_from::list))
}

/// What a stopped process left, settled once at start: each draft's newest evaluation left running is run again
/// and any older one superseded, a thumbnail left computing is dropped so the next list computes it, and a Create
/// site left writing gets its worker, which sends the kept changeset again as it was and so finds the site if it
/// landed. The process's start awaits this scan before its listeners accept a request (`main`'s `run`); the work it
/// starts runs on after it.
pub async fn recover<S: Wps>(gate: &GateState<S>) -> Result<(), SqlxError> {
    let pool = &gate.app.pool;
    sqlx::query("DELETE FROM template_thumbnails WHERE state = 'evaluating'").execute(pool).await?;
    sqlx::query(
        "UPDATE template_evaluations e SET state = 'superseded' FROM template_drafts d \
         WHERE e.draft = d.id AND e.state = 'in-progress' AND e.n < d.latest_n",
    )
    .execute(pool)
    .await?;
    let running: Vec<(String, i32)> =
        sqlx::query_as("SELECT draft, n FROM template_evaluations WHERE state = 'in-progress'").fetch_all(pool).await?;
    for (draft, n) in running {
        let id = draft.parse::<Ulid>().map_err(|_| decode(format!("template draft id {draft} is not a ULID")))?;
        tokio::spawn(evaluation::run(gate.clone(), id, evaluation_of(n)?));
    }
    let writing: Vec<(String, String)> =
        sqlx::query_as("SELECT id, write_approval FROM template_drafts WHERE state = 'writing'")
            .fetch_all(pool)
            .await?;
    for (draft, approval) in writing {
        let (Ok(id), Ok(approval)) = (draft.parse::<Ulid>(), approval.parse::<Ulid>()) else {
            return Err(decode(format!("template draft {draft} writes under no approval's id")));
        };
        tokio::spawn(create::write(gate.clone(), id, approval));
    }
    Ok(())
}
