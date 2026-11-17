//! Within `studio-api`, `flows` is spec 06 section 3.4's flow routes (BACKLOG task 20's reads): `GET
//! /api/flow/{*location}`, a site's flow graph read at one cut and compiled in validation mode; `GET
//! /api/flow-parameters/{*location}`, its parameter list; and `POST /api/flow-validate/{*location}`, a compile with
//! patches applied in memory, in run mode when given a setup. Each resolves the site's scopes at one cut and reads
//! their flagged export, the bytes a run loads (rulings D40 and D185), loaded and compiled with dessim's reader and
//! flow compiler on the compute pool; a site with no flow sub-scope at that cut reads as the empty flow. A read with
//! `draft` reads at an open draft's stored cut: the graph drawn with the draft over it, each entity it patches
//! carrying was and becomes, beside the plan that cut compiles to without it; the parameter list, that cut's values.
//! A draft no longer open is named with its state over the location's own read.
//!
//! A read answers the export's provenance: every scope it holds entities from, a pinned catalog item's at its pin,
//! and its exactness, which folds in the cut's and each held scope read below its head (spec 06 section 3.3).
//! Its compile is kept under the export's sha256, the compiler version and the runner image, single-flight as floors
//! are, and only while that exactness holds. `AppState` is I1.07b's, so the keep is this module's own map, which
//! bytes key so no two worlds share an entry (lead decided, say if not).

pub mod answer;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock};

use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::Uri;
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use dessim::composition::flowgraph::{
    Accepts, COMPILER, OrderKind, Patch, Request, RunSetup, compile, parameters as list_parameters,
};
use dessim::worldmap::flows::{DOMAIN, read as read_flows};
use dessim::worldmap::load_from_bytes;
use rustc_hash::FxHashMap;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Error as SqlxError, PgPool};
use studio_protocol::{
    ChangesetOp, DraftState, EntityId, FlowBase, FlowDiff, FlowDraft, FlowFieldDiff, FlowMadeFrom, FlowParameters,
    FlowRead, FlowValidate, FlowValidated, Location, Problem, ProblemItem, ProblemType, ScopePath, Seq, Severity,
    SitePath, Ulid,
};
use wps_client::cache::Flight;
use wps_client::compute::run as on_pool;
use wps_client::wps::Wps;
use wps_client::{CutExport, Exactness, UnitCut, WorldStore, WpsError};

use crate::GateState;
use crate::db::decode;
use crate::drafts::site::{patched, value_at};
use crate::drafts::{Draft, load as load_draft};
use crate::problem::{ProblemAnswer, internal, json_body, validation_failed, wps_problem};
use answer::Body;

/// The runner image every compile answers for in the first cut: the one local image, whose `accepts` are this
/// build's own (ruling D109).
const IMAGE: &str = "local";

/// The keep's budget: bodies are a few kilobytes per entity, so this holds hundreds of sites.
const BUDGET: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Kept {
    document: String,
    compiler: &'static str,
    image: &'static str,
}

static KEEP: LazyLock<Flight<Kept, Arc<Body>>> = LazyLock::new(|| Flight::new(BUDGET, size));

fn size(body: &Arc<Body>) -> usize {
    1024 * (body.nodes.len() + body.edges.len() + body.segments.len() + body.problems.len() + 1)
}

pub fn routes<S: Wps>() -> Router<GateState<S>> {
    Router::new()
        .route("/api/flow/{*location}", get(flow::<S>))
        .route("/api/flow-parameters/{*location}", get(flow_parameters::<S>))
        .route("/api/flow-validate/{*location}", post(flow_validate::<S>))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FlowQuery {
    kin: Option<String>,
    draft: Option<String>,
}

/// A site's flow scope at a pin, the cut of the site's scopes that read resolved (an open draft's own), that cut's
/// flagged export (none where the cut holds no flow sub-scope), and the draft the read names.
struct Read {
    location: Location,
    root: SitePath,
    cut: UnitCut,
    export: Option<CutExport>,
    draft: Option<Draft>,
}

/// The location a flow route names, a site's flow scope (`customer/{c}/site/{s}/flow`) at `@main` or a pin, and the
/// site's root, whose scopes the read resolves whether or not the flow sub-scope exists.
fn flow_location(
    path: Result<Path<String>, PathRejection>,
    instance: &str,
) -> Result<(Location, SitePath), ProblemAnswer> {
    let Ok(Path(text)) = path else { return Err(validation_failed("the path names no flow location", instance)) };
    let location: Location = text.parse().map_err(|fault| validation_failed(fault, instance))?;
    let slugs: Vec<&str> = location.scope().slugs().collect();
    let ["customer", customer, "site", site, "flow"] = slugs.as_slice() else {
        let why = format!("{} is no site's flow scope, customer/{{customer}}/site/{{site}}/flow", location.scope());
        return Err(validation_failed(why, instance));
    };
    let root =
        format!("customer/{customer}/site/{site}").parse().map_err(|fault| validation_failed(fault, instance))?;
    Ok((location, root))
}

/// Resolve the location to one cut of the site's scopes and export it flagged, as a run reads it; a cut holding no
/// flow sub-scope exports nothing. Kin pins, which stand only beside a changeset location, wait for reads at a
/// version (spec 04's `reads-at-version`).
async fn read<S: Wps>(
    gate: &GateState<S>,
    (location, root): (Location, SitePath),
    query: &FlowQuery,
    instance: &str,
) -> Result<Read, ProblemAnswer> {
    if query.kin.is_some() {
        let why = "kin pins wait for world-proto-server's reads at a version; read the flow at its branch head";
        return Err(validation_failed(why, instance));
    }
    let world = &gate.world;
    let draft = match &query.draft {
        Some(id) => Some(drawn(gate, &location, id, instance).await?),
        None => None,
    };
    let cut = match &draft {
        Some(open) if on_view(open.state) => open.cut.clone(),
        _ => world.resolve_cut(root.as_scope(), location.pin()).await.map_err(|e| wps_problem(&e, instance))?,
    };
    let export = match cut.seqs.contains_key(location.scope()) {
        true => Some(world.export(&cut).await.map_err(|error| wps_problem(&error, instance))?),
        false => None,
    };
    Ok(Read { location, root, cut, export, draft })
}

/// Whether a draft is drawn over its cut: while it is open, and while a press applies it.
fn on_view(state: DraftState) -> bool {
    state.is_open() || state == DraftState::Applying
}

/// The draft `id` names, which must be one on the location's site.
async fn drawn<S: Wps>(
    gate: &GateState<S>,
    location: &Location,
    id: &str,
    instance: &str,
) -> Result<Draft, ProblemAnswer> {
    let missing =
        || ProblemAnswer::from(Problem::new(ProblemType::NotFound, format!("There is no draft {id}.")).at(instance));
    let parsed: Ulid = id.parse().map_err(|_| missing())?;
    let draft = (load_draft(&gate.app.pool, &parsed).await)
        .map_err(|fault| internal("read the draft", fault, instance))?
        .ok_or_else(missing)?;
    if !draft.site.as_scope().holds(location.scope()) {
        return Err(validation_failed(format!("draft {id} is a draft of {}, not of this site", draft.site), instance));
    }
    Ok(draft)
}

/// What each entity a draft patches was in the cut and becomes under it, field by field in the draft's order.
fn diffs(base: &Value, ops: &[ChangesetOp]) -> FxHashMap<EntityId, FlowDiff> {
    let mut found: FxHashMap<EntityId, FlowDiff> = FxHashMap::default();
    for op in ops {
        let ChangesetOp::Patch { target, pointer, value } = op;
        let was = value_at(base, target, pointer).unwrap_or(Value::Null);
        let field = FlowFieldDiff { pointer: pointer.clone(), was, becomes: value.clone() };
        let diff = found.entry(target.clone()).or_insert_with(|| FlowDiff { op: "changed".into(), fields: Vec::new() });
        diff.fields.push(field);
    }
    found
}

/// The body of a read under an open draft: the cut's export with the draft's operations applied in memory, compiled
/// afresh and never kept, each patched entity carrying its diff.
async fn overlaid<S: Wps>(
    gate: &GateState<S>,
    read: &Read,
    export: &CutExport,
    ops: &[ChangesetOp],
    instance: &str,
) -> Result<Body, ProblemAnswer> {
    let base: Value = serde_json::from_slice(&export.bytes).map_err(|fault| validation_failed(fault, instance))?;
    let drawn = patched(&base, ops).map_err(|why| validation_failed(why, instance))?;
    let bytes = Bytes::from(serde_json::to_vec(&drawn).map_err(|fault| validation_failed(fault, instance))?);
    let flow_scope = read.location.scope().clone();
    let compute = on_pool(&*gate.app.compute, move || compiled_body(&bytes, &flow_scope));
    let mut body = compute.await.map_err(|error| wps_problem(&error, instance))?;
    let diffs = diffs(&base, ops);
    body.nodes.iter_mut().for_each(|node| node.diff = diffs.get(&node.ref_).cloned());
    body.edges.iter_mut().for_each(|edge| edge.diff = diffs.get(&edge.ref_).cloned());
    body.segments.iter_mut().for_each(|segment| segment.diff = diffs.get(&segment.ref_).cloned());
    body.return_rules.iter_mut().for_each(|rule| rule.diff = diffs.get(&rule.ref_).cloned());
    Ok(body)
}

/// What a read read, as its answer states it: every scope the export holds entities from with the changeset it read,
/// and whether the export is exact; a read that exported nothing answers its cut's.
fn provenance(read: &Read) -> (BTreeMap<ScopePath, Seq>, bool) {
    match &read.export {
        Some(export) => (export.scopes.clone(), keepable(export)),
        None => (read.cut.seqs.clone(), matches!(read.cut.exact, Exactness::Exact)),
    }
}

/// Whether a compile of `export` may be kept: only an exact export's, since an inexact one carries a head's labels
/// or prologue that a later read at the same changesets may not.
fn keepable(export: &CutExport) -> bool {
    matches!(export.exact, Exactness::Exact)
}

/// The empty flow: what a site with no flow sub-scope reads as (spec 06 section 3.4).
fn no_flow() -> Body {
    alone("flow-empty", Severity::Warning, "This site has no flow graph at this read.")
}

/// What a document compiles to without a scenario; a document dessim's reader refuses compiles to nothing, and says
/// why as one error.
fn compiled_body(document: &Bytes, flow_scope: &ScopePath) -> Body {
    match load_from_bytes(document) {
        Ok(world) => {
            let flows = read_flows(&world.world);
            let compiled = compile(&world, &flows, &Request::validation());
            answer::body(&world, &flows, &compiled, flow_scope)
        }
        Err(_) if !states_a_flow(document) => no_flow(),
        Err(why) => {
            let text = format!("The site will not load in the simulator's reader: {why}");
            alone("world-unreadable", Severity::Error, &text)
        }
    }
}

/// Whether any entity of a document carries a `flow.` component, read without the simulator's reader, so a site
/// with no flow says so even where its map does not load.
fn states_a_flow(document: &Bytes) -> bool {
    let read: Option<Value> = serde_json::from_slice(document).ok();
    let entities = read.as_ref().and_then(|document| document.get("entities")?.as_array().cloned()).unwrap_or_default();
    entities.iter().any(|entity| {
        entity.get("components").and_then(Value::as_object).is_some_and(|c| c.keys().any(|k| k.starts_with(DOMAIN)))
    })
}

/// The read's export where it states a flow: none for a site with no flow sub-scope at the cut, or an empty one.
fn flowing(read: &Read) -> Option<&CutExport> {
    read.export.as_ref().filter(|export| states_a_flow(&export.bytes))
}

/// A body with no graph and one problem.
fn alone(code: &str, severity: Severity, text: &str) -> Body {
    let opening = if severity == Severity::Error { "Error" } else { "Warning" };
    Body {
        nodes: Vec::new(),
        edges: Vec::new(),
        segments: Vec::new(),
        return_rules: Vec::new(),
        plan: None,
        problems: vec![ProblemItem {
            code: code.into(),
            severity,
            ref_: None,
            scope: None,
            field: None,
            knob: None,
            text: format!("{opening}: {text}"),
        }],
    }
}

/// The body of a read: kept under the export's digest when the read is exact, else compiled afresh.
async fn body<S: Wps>(
    gate: &GateState<S>,
    read: &Read,
    export: &CutExport,
    instance: &str,
) -> Result<Arc<Body>, ProblemAnswer> {
    let (bytes, flow_scope) = (export.bytes.clone(), read.location.scope().clone());
    let pool = gate.app.compute.clone();
    let compute = move || async move { on_pool(&*pool, move || Arc::new(compiled_body(&bytes, &flow_scope))).await };
    let key = Kept { document: export.sha256.clone(), compiler: COMPILER, image: IMAGE };
    let body = if keepable(export) { KEEP.get_or_spawn(key, compute).await } else { compute().await };
    body.map_err(|error: WpsError| wps_problem(&error, instance))
}

fn query_of(query: Result<Query<FlowQuery>, QueryRejection>, instance: &str) -> Result<FlowQuery, ProblemAnswer> {
    query.map(|Query(query)| query).map_err(|fault| validation_failed(fault, instance))
}

/// `GET /api/flow/{*location}`: the flow graph at the location's read, compiled in validation mode (spec 06 section
/// 3.4). A site with no flow answers its empty lists, no plan and the warning `flow-empty`.
async fn flow<S: Wps>(
    State(gate): State<GateState<S>>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
    query: Result<Query<FlowQuery>, QueryRejection>,
) -> Result<Json<FlowRead>, ProblemAnswer> {
    let instance = uri.path();
    let (location, query) = (flow_location(path, instance)?, query_of(query, instance)?);
    let read = read(&gate, location, &query, instance).await?;
    let drawn = read.draft.as_ref().filter(|draft| on_view(draft.state));
    let (body, base) = match (&read.export, drawn) {
        (None, Some(_)) => (no_flow(), Some(FlowBase { plan: None })),
        (None, None) => (no_flow(), None),
        (Some(export), Some(draft)) => {
            let base = body(&gate, &read, export, instance).await?.plan.clone();
            (overlaid(&gate, &read, export, &draft.ops, instance).await?, Some(FlowBase { plan: base }))
        }
        (Some(export), None) => ((*body(&gate, &read, export, instance).await?).clone(), None),
    };
    let Body { nodes, edges, segments, return_rules, plan, problems } = body;
    let (seqs, exact) = provenance(&read);
    let draft = (read.draft.as_ref()).map(|draft| FlowDraft {
        ref_: draft.id.clone(),
        state: draft.state,
        revision: draft.revision,
        base: base.clone(),
    });
    Ok(Json(FlowRead {
        scope: read.location.scope().clone(),
        pin: read.location.pin().clone(),
        seqs,
        exact,
        draft,
        nodes,
        edges,
        segments,
        return_rules,
        plan,
        problems,
        made_from: made_from(&gate.app.pool, &read.root)
            .await
            .map_err(|fault| internal("read the site's made-from", fault, instance))?,
    }))
}

/// The site's newest made-from record as a made-from line names it (spec 06 section 5): the template, its tag and the
/// changeset of its root that act read; none for a site no template made.
async fn made_from(pool: &PgPool, site: &SitePath) -> Result<Option<FlowMadeFrom>, SqlxError> {
    let row: Option<(String, Option<String>, Option<Value>)> = sqlx::query_as(
        "SELECT template, tag, template_seqs FROM site_made_from \
         WHERE site = $1 AND state IN ('made', 'updated') AND template IS NOT NULL ORDER BY at DESC, id DESC LIMIT 1",
    )
    .bind(site.as_str())
    .fetch_optional(pool)
    .await?;
    let Some((template, tag, seqs)) = row else { return Ok(None) };
    let template: ScopePath = template.parse().map_err(decode)?;
    let seqs: BTreeMap<ScopePath, Seq> =
        seqs.map(serde_json::from_value).transpose().map_err(decode)?.unwrap_or_default();
    let tag = tag.map(|tag| tag.parse()).transpose().map_err(decode)?;
    Ok(Some(FlowMadeFrom { seq: seqs.get(&template).copied(), template, tag }))
}

/// `GET /api/flow-parameters/{*location}`: every scalar field of the flow at the location's read, labelled from the
/// one field catalogue, with what a sweep may vary (spec 06 section 3.4).
async fn flow_parameters<S: Wps>(
    State(gate): State<GateState<S>>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
    query: Result<Query<FlowQuery>, QueryRejection>,
) -> Result<Json<FlowParameters>, ProblemAnswer> {
    let instance = uri.path();
    let (location, query) = (flow_location(path, instance)?, query_of(query, instance)?);
    let read = read(&gate, location, &query, instance).await?;
    let Some(export) = flowing(&read) else {
        return Ok(Json(FlowParameters { seqs: provenance(&read).0, parameters: Vec::new() }));
    };
    let bytes = export.bytes.clone();
    let listed = on_pool(&*gate.app.compute, move || {
        let world = load_from_bytes(&bytes).map_err(|why| why.to_string())?;
        let flows = read_flows(&world.world);
        let compiled = compile(&world, &flows, &Request::validation());
        Ok::<_, String>(list_parameters(&flows, &compiled, &Accepts::this_build()))
    })
    .await
    .map_err(|error| wps_problem(&error, instance))?
    .map_err(|why| validation_failed(format!("the site will not load in the simulator's reader: {why}"), instance))?;
    let parameters = listed.into_iter().map(answer::parameter).collect();
    Ok(Json(FlowParameters { seqs: provenance(&read).0, parameters }))
}

/// A protocol value as dessim's own type, through the JSON both spell alike.
fn as_compiler<T: DeserializeOwned>(value: &impl Serialize, instance: &str) -> Result<T, ProblemAnswer> {
    let json: Value = serde_json::to_value(value).map_err(|fault| validation_failed(fault, instance))?;
    serde_json::from_value(json).map_err(|fault| validation_failed(fault, instance))
}

/// `POST /api/flow-validate/{*location}`: the flow at the location's read compiled with the patches applied in
/// memory, in run mode when the body gives a setup; a patch at a field the catalogue does not mark sweepable is
/// refused as `not-sweepable` (spec 06 section 3.4).
async fn flow_validate<S: Wps>(
    State(gate): State<GateState<S>>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
    body: Bytes,
) -> Result<Json<FlowValidated>, ProblemAnswer> {
    let instance = uri.path();
    let location = flow_location(path, instance)?;
    let asked: FlowValidate = json_body(&body, instance)?;
    let variant: Vec<Patch> = as_compiler(&asked.patches, instance)?;
    let mut request = match &asked.setup {
        None => Request::validation(),
        Some(setup) => {
            let setup: RunSetup = as_compiler(setup, instance)?;
            let kinds: Vec<OrderKind> = as_compiler(&asked.order_kinds.clone().unwrap_or_default(), instance)?;
            Request::run(setup, kinds)
        }
    };
    request.variant = variant;
    let read = read(&gate, location, &FlowQuery::default(), instance).await?;
    let Some(export) = flowing(&read) else {
        let Body { plan, problems, .. } = no_flow();
        return Ok(Json(FlowValidated { plan, problems }));
    };
    let (bytes, flow_scope) = (export.bytes.clone(), read.location.scope().clone());
    let validated = on_pool(&*gate.app.compute, move || {
        let world = load_from_bytes(&bytes).map_err(|why| why.to_string())?;
        let compiled = compile(&world, &read_flows(&world.world), &request);
        let problems = compiled.problems.iter().map(|finding| answer::item(finding, &flow_scope)).collect();
        Ok::<_, String>(FlowValidated { plan: compiled.plan.as_ref().map(answer::plan), problems })
    })
    .await
    .map_err(|error| wps_problem(&error, instance))?
    .map_err(|why| validation_failed(format!("the site will not load in the simulator's reader: {why}"), instance))?;
    Ok(Json(validated))
}
