//! Within `results`, `headline` is spec 08 section 3.9's headline rule: a run's headline metrics are its project's
//! pinned metrics in pin order (migration 0011, data and never code), else the metrics of the agent's `show` command
//! that opened the panel, named by the route's `highlights` (spec 03 section 3.3), else the first four the run
//! declared, in descriptor order; each source counts only the metrics the run declared, and one that names none falls
//! to the next. It also fills the run list's headline column, `RunSummary.headline`, by the same rule and with the
//! list route's `highlights`, so a run's list cell and its page's first tile name one metric: the first headline
//! metric over the run's report window as far as its attempt got, for every run whose latest attempt reported metrics.

use desunits::SimTime;
use rustc_hash::FxHashMap;
use serde_json::Value;
use sqlx::{Error as SqlxError, PgConnection, Row};
use studio_protocol::{JournalSeq, Panel, PanelCommandEntry, RunSummary, SimWindow, Ulid};

use crate::db::{decode, seq_column};
use crate::runs::json;

use super::stored::{declared, series};
use super::values::{Declared, Instants, row};

/// How many metrics a run headlines when neither the project nor the agent chose any.
const FIRST: usize = 4;

/// The headline metrics of a run that declared `declared`, by section 3.9's rule.
pub fn choose(declared: &[Declared], pins: &[String], shown: Option<&[String]>) -> Vec<String> {
    let known = |names: &[String]| -> Vec<String> {
        names.iter().filter(|name| declared.iter().any(|metric| &metric.metric == *name)).cloned().collect()
    };
    let pinned = known(pins);
    if !pinned.is_empty() {
        return pinned;
    }
    if let Some(shown) = shown.map(known).filter(|shown| !shown.is_empty()) {
        return shown;
    }
    declared.iter().take(FIRST).map(|metric| metric.metric.clone()).collect()
}

/// The project's pinned metrics, in pin order.
pub async fn pins(conn: &mut PgConnection, project: &Ulid) -> Result<Vec<String>, SqlxError> {
    sqlx::query_scalar("SELECT metric FROM metric_pins WHERE project = $1 ORDER BY position")
        .bind(project.as_str())
        .fetch_all(conn)
        .await
}

/// The metrics of the Runs panel's `show` entry numbered `seq` in `site`'s journal; none when that entry is no such
/// command or names none.
pub async fn shown(conn: &mut PgConnection, site: &str, seq: JournalSeq) -> Result<Option<Vec<String>>, SqlxError> {
    let body: Option<Value> = sqlx::query_scalar(
        "SELECT j.body FROM journal j JOIN conversations c ON c.id = j.conversation \
         WHERE c.site = $1 AND j.seq = $2 AND j.kind = 'panel.command'",
    )
    .bind(site)
    .bind(seq_column(seq))
    .fetch_optional(conn)
    .await?;
    let command: Option<PanelCommandEntry> = body.map(json).transpose()?;
    Ok(command.filter(|command| command.panel == Panel::Runs).and_then(|command| command.metrics))
}

/// Each listed run's first headline metric over its report window as far as its attempt got, the agent's `show`
/// entry `highlights` names in the run's site's journal choosing where its project pins none, once its latest attempt
/// reported metrics and the window falls on instants the attempt recorded; the others keep none.
pub async fn fill(
    conn: &mut PgConnection,
    items: &mut [RunSummary],
    highlights: Option<JournalSeq>,
) -> Result<(), SqlxError> {
    let ended: Vec<usize> = (0..items.len()).filter(|&at| items[at].state.ended()).collect();
    if ended.is_empty() {
        return Ok(());
    }
    let runs: Vec<String> = ended.iter().map(|&at| items[at].id.to_string()).collect();
    let rows = sqlx::query(
        "SELECT r.id, sc.project, sc.site_scope FROM runs r JOIN scenarios sc ON sc.id = r.scenario \
         WHERE r.id = ANY($1)",
    )
    .bind(&runs)
    .fetch_all(&mut *conn)
    .await?;
    let mut projects: FxHashMap<String, String> = FxHashMap::default();
    let mut sites: FxHashMap<String, String> = FxHashMap::default();
    for row in &rows {
        let run: String = row.try_get("id")?;
        sites.insert(run.clone(), row.try_get("site_scope")?);
        projects.insert(run, row.try_get("project")?);
    }
    // A `show` entry is numbered in its site's journal, so each site's entry is read once.
    let mut commanded: FxHashMap<String, Option<Vec<String>>> = FxHashMap::default();
    if let Some(seq) = highlights {
        for site in sites.values() {
            if !commanded.contains_key(site) {
                let metrics = shown(conn, site, seq).await?;
                commanded.insert(site.clone(), metrics);
            }
        }
    }
    let mut pinned: FxHashMap<String, Vec<String>> = FxHashMap::default();
    for project in projects.values() {
        if !pinned.contains_key(project) {
            let project_id: Ulid = project.parse().map_err(decode)?;
            pinned.insert(project.clone(), pins(conn, &project_id).await?);
        }
    }
    for at in ended {
        let run = &items[at];
        let n = i32::try_from(run.attempt.0).map_err(decode)?;
        let declared = declared(conn, &run.id, n).await?;
        let pins = projects.get(run.id.as_str()).and_then(|project| pinned.get(project)).cloned().unwrap_or_default();
        let shown = sites.get(run.id.as_str()).and_then(|site| commanded.get(site)).cloned().flatten();
        let Some(first) = choose(&declared, &pins, shown.as_deref()).into_iter().next() else { continue };
        let Some(metric) = declared.iter().find(|metric| metric.metric == first) else { continue };
        let stored = series(conn, &run.id, n, Some(&first)).await?.remove(&first);
        let instants = Instants::new(stored.as_ref(), run.progress.sim_ms);
        let reached = instants.reached(&run.report_window);
        let Some(window) = recorded(conn, &run.id, n, &instants, reached.from_sim_ms, reached.to_sim_ms).await? else {
            continue;
        };
        let headline = row(metric, stored.as_ref(), window, instants.whole());
        items[at].headline = Some(headline);
    }
    Ok(())
}

/// The window from `from` to `to` when both ends are instants the attempt recorded and it holds a span: the one
/// metric's series answers first, and the attempt's every series for an end it does not hold.
async fn recorded(
    conn: &mut PgConnection,
    run: &Ulid,
    n: i32,
    instants: &Instants,
    from: SimTime,
    to: SimTime,
) -> Result<Option<SimWindow>, SqlxError> {
    for end in [from, to] {
        if instants.exact(end).is_err() && !held(conn, run, n, end).await? {
            return Ok(None);
        }
    }
    Ok((from < to).then_some(SimWindow { from_sim_ms: from, to_sim_ms: to }))
}

async fn held(conn: &mut PgConnection, run: &Ulid, n: i32, at: SimTime) -> Result<bool, SqlxError> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM attempt_series WHERE run_id = $1 AND n = $2 AND $3 = ANY(at_sim_ms))",
    )
    .bind(run.as_str())
    .bind(n)
    .bind(i64::try_from(at.as_ms()).map_err(decode)?)
    .fetch_one(conn)
    .await
}
