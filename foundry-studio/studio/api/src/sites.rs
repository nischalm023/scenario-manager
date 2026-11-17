//! Within `studio-api`, `sites` are the sites world-proto-server holds as Home's list and the context bar read them
//! (spec 01 section 3.6's `GET /api/sites`): every `customer/*/site/*` root that has landed a changeset, which Create
//! site's one changeset does with the site's other three scopes, each named by its root's prologue and shown with its
//! live conversation and whether its turn waits or works, except a root Create site is still writing, which is no site
//! until its made-from record and its thread's binding land (LEAD-DECISIONS entry 54); a site the list leaves out has
//! no conversation to open.

use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::http::Uri;
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use rustc_hash::{FxHashMap, FxHashSet};
use serde::Deserialize;
use sqlx::{Error as SqlxError, PgPool};
use studio_protocol::{
    BranchName, ScopePath, SiteBranch, SiteConversation, SiteItem, SiteList, SitePath, WaitingOn, WallClock,
};
use wps_client::site::{SiteRow, list_sites};
use wps_client::wps::Wps;

use crate::GateState;
use crate::db::{decode, read_word};
use crate::problem::{ProblemAnswer, internal, validation_failed, wps_problem};

pub fn routes<S: Wps>() -> Router<GateState<S>> {
    Router::new().route("/api/sites", get(sites))
}

#[derive(Deserialize)]
pub(crate) struct SitesQuery {
    pub customer: Option<String>,
}

/// `GET /api/sites?customer=`: the sites, of one customer when `customer` names its slug, newest change first.
pub(crate) async fn sites<S: Wps>(
    State(state): State<GateState<S>>,
    uri: Uri,
    query: Result<Query<SitesQuery>, QueryRejection>,
) -> Result<Json<SiteList>, ProblemAnswer> {
    let instance = uri.path();
    let Query(SitesQuery { customer }) = query.map_err(|fault| validation_failed(fault.body_text(), instance))?;
    if let Some(customer) = &customer
        && !format!("customer/{customer}").parse::<ScopePath>().is_ok_and(|path| path.slugs().count() == 2)
    {
        return Err(validation_failed(format!("`{customer}` is no customer's slug"), instance));
    }
    let rows = listed(&state, customer.as_deref(), instance).await?;
    let live = live_conversations(&state.app.pool, &rows)
        .await
        .map_err(|fault| internal("read the sites' conversations", fault, instance))?;
    let items = rows.into_iter().map(|row| item(row, &live, instance)).collect::<Result<_, _>>()?;
    Ok(Json(SiteList { items }))
}

/// Whether world-proto-server lists `site`, which opening its conversation asks first: the customer's whole list is
/// read, a read of every scope's newest changeset included, so the answer is the list's own.
pub async fn is_listed<S: Wps>(state: &GateState<S>, site: &SitePath, instance: &str) -> Result<bool, ProblemAnswer> {
    let customer = site.as_scope().slugs().nth(1).unwrap_or_default();
    Ok(listed(state, Some(customer), instance).await?.iter().any(|row| &row.root == site.as_scope()))
}

async fn listed<S: Wps>(
    state: &GateState<S>,
    customer: Option<&str>,
    instance: &str,
) -> Result<Vec<SiteRow>, ProblemAnswer> {
    let rows = list_sites(&state.world, customer).await.map_err(|error| wps_problem(&error, instance))?;
    let making = being_made(&state.app.pool)
        .await
        .map_err(|fault| internal("read the sites Create site is still writing", fault, instance))?;
    Ok(rows.into_iter().filter(|row| !making.contains(row.root.as_str())).collect())
}

/// The roots a Create site is still writing, each as the plan its approval keeps names it; read after world-proto's
/// listing, so a root listed here has landed its made-from record and its binding, which commit with the draft's end.
async fn being_made(pool: &PgPool) -> Result<FxHashSet<String>, SqlxError> {
    let roots: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT a.sent ->> 'root' FROM template_drafts d JOIN approvals a ON a.id = d.write_approval \
         WHERE d.state = 'writing' AND d.mode = 'create'",
    )
    .fetch_all(pool)
    .await?;
    Ok(roots.into_iter().flatten().collect())
}

/// The live conversation of each listed site, by its root, with what its open turn waits on and whether one works.
async fn live_conversations(pool: &PgPool, rows: &[SiteRow]) -> Result<FxHashMap<String, SiteConversation>, SqlxError> {
    let roots: Vec<&str> = rows.iter().map(|row| row.root.as_str()).collect();
    let live: Vec<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT c.site, c.id, t.state, t.waiting_on FROM conversations c \
         LEFT JOIN turns t ON t.conversation = c.id AND t.state IN ('running', 'waiting') \
         WHERE c.state = 'live' AND c.site = ANY($1)",
    )
    .bind(&roots[..])
    .fetch_all(pool)
    .await?;
    (live.into_iter())
        .map(|(site, id, state, waiting)| {
            let waiting = waiting.map(|on| read_word::<WaitingOn>(&on)).transpose()?;
            let working = Some(state.as_deref() == Some("running"));
            Ok((site, SiteConversation { id: id.parse().map_err(decode)?, waiting, working }))
        })
        .collect()
}

/// One listed site as the page reads it: `main` its one branch and no tags, which world-proto-server does not hold
/// yet, and its live conversation with its turn's state.
fn item(row: SiteRow, live: &FxHashMap<String, SiteConversation>, instance: &str) -> Result<SiteItem, ProblemAnswer> {
    let site: SitePath =
        row.root.as_str().parse().map_err(|fault| internal("read a listed site's path", fault, instance))?;
    let changed_at = DateTime::parse_from_rfc3339(&row.changed_at)
        .map_err(|fault| internal("read when a listed site last changed", fault, instance))?;
    let conversation = live.get(row.root.as_str()).cloned();
    Ok(SiteItem {
        site,
        name: row.name,
        branches: vec![SiteBranch { name: BranchName::main(), heads: row.heads }],
        tags: Vec::new(),
        changed_at: WallClock::from(changed_at.with_timezone(&Utc)),
        conversation,
    })
}
