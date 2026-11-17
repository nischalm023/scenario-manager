//! Within `templates`, `list` is the Template list and one template as their routes serve them (spec 05 sections 3.8
//! and 4.1): every site template under `template/site/` read at each scope's head on main, in the POC's order, each
//! with its knobs, its material edges' names and its thumbnail, the default evaluation computed once per template
//! state and kept, its card marked evaluating until then.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use axum::Json;
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, State};
use axum::http::Uri;
use serde_json::{Value, json};
use site_template::{DocId, Evaluation, KnobValues, Mint, Prior, RegistrySnapshot, TemplateRead, evaluate};
use sqlx::{Error as SqlxError, Row};
use studio_protocol::{
    BranchName, Census, EvaluationProblem, Location, Pin, Problem, ProblemType, ScopePath, Seq, TemplateCard,
    TemplateDetail, TemplateList, Thumbnail, ThumbnailState,
};
use wps_client::wps::Wps;
use wps_client::{GeneratorSet, WorldStore};

use super::evaluation::{Inputs, compute, keep, pooled};
use super::read::{Read, read};
use crate::GateState;
use crate::db::{decode, mint_ulid, read_word};
use crate::problem::{ProblemAnswer, internal, validation_failed, wps_problem};

/// The five shells in the POC's order (spec 05 section 6: Square, Rectangle, L-Shape, U-Shape, I-Shape); any other
/// template follows them by path.
const POC_ORDER: [&str; 5] = ["square", "rectangle", "l-shape", "u-shape", "i-shape"];

/// `GET /api/templates`.
pub async fn list<S: Wps>(State(gate): State<GateState<S>>, uri: Uri) -> Result<Json<TemplateList>, ProblemAnswer> {
    let instance = uri.path();
    let mut items = Vec::new();
    for (root, seqs) in templates(&gate, instance).await? {
        let read = read(&gate.app.templates, &gate.world, &root, &seqs).await;
        let read = match read {
            Ok(read) => read,
            Err(fault) => {
                eprintln!("studio-api: the template {root} cannot be read for the list: {fault}");
                continue;
            }
        };
        let thumbnail = thumbnail(&gate, &root, &seqs, &read, instance).await?;
        items.push(card(&root, seqs, &read, thumbnail));
    }
    Ok(Json(TemplateList { items }))
}

/// `GET /api/templates/{*location}`: one template at `@main`.
pub async fn one<S: Wps>(
    State(gate): State<GateState<S>>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<TemplateDetail>, ProblemAnswer> {
    let instance = uri.path();
    let Ok(Path(location)) = path else { return Err(validation_failed("the path names no template", instance)) };
    let location: Location = location.parse().map_err(|fault| validation_failed(fault, instance))?;
    if location.pin() != &Pin::Branch(BranchName::main()) {
        return Err(validation_failed("a template is read at @main until world-proto-server has tags", instance));
    }
    let root = location.scope().clone();
    let Some((_, seqs)) = templates(&gate, instance).await?.into_iter().find(|(held, _)| *held == root) else {
        let detail = format!("There is no site template {root}.");
        return Err(Problem::new(ProblemType::NotFound, detail).at(instance).into());
    };
    let read = read(&gate.app.templates, &gate.world, &root, &seqs)
        .await
        .map_err(|fault| super::drafts::read_problem(&fault, instance))?;
    let default = thumbnail(&gate, &root, &seqs, &read, instance).await?;
    Ok(Json(TemplateDetail {
        ref_: root,
        tag: None,
        seqs,
        description: read.description.clone(),
        knobs: read.knobs.clone(),
        flows: read.flows.clone(),
        default,
    }))
}

/// Every site template's root, each with its scopes' heads on main, in the POC's order.
async fn templates<S: Wps>(
    gate: &GateState<S>,
    instance: &str,
) -> Result<Vec<(ScopePath, BTreeMap<ScopePath, Seq>)>, ProblemAnswer> {
    let under: ScopePath = "template/site".parse().expect("a scope path");
    let listing = gate.world.list_scopes(&under).await.map_err(|error| wps_problem(&error, instance))?;
    let mut found: Vec<(ScopePath, BTreeMap<ScopePath, Seq>)> = listing
        .iter()
        .filter(|scope| scope.boundary && scope.path.slugs().count() == 3)
        .map(|root| {
            let seqs =
                listing.iter().filter(|scope| scope.unit == root.path).map(|scope| (scope.path.clone(), scope.head));
            (root.path.clone(), seqs.collect())
        })
        .filter(|(_, seqs): &(ScopePath, BTreeMap<ScopePath, Seq>)| seqs.values().all(|seq| *seq > Seq::new(0)))
        .collect();
    let rank = |root: &ScopePath| {
        POC_ORDER.iter().position(|slug| root.slugs().nth(2) == Some(*slug)).unwrap_or(POC_ORDER.len())
    };
    found.sort_by(|a, b| rank(&a.0).cmp(&rank(&b.0)).then_with(|| a.0.cmp(&b.0)));
    Ok(found)
}

fn card(root: &ScopePath, seqs: BTreeMap<ScopePath, Seq>, read: &Read, thumbnail: Thumbnail) -> TemplateCard {
    TemplateCard {
        ref_: root.clone(),
        title: read.description.title.clone(),
        text: read.description.text.clone(),
        built_from: read.description.built_from.clone(),
        tag: None,
        seqs,
        knobs: read.knobs.clone(),
        flows: read.flows.clone(),
        thumbnail,
    }
}

/// The version of the engine this API links, which keys a thumbnail: the stamp an evaluation of nothing carries,
/// since the engine stamps every evaluation before it reads the template.
fn engine_version() -> &'static str {
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION.get_or_init(|| {
        let registry = RegistrySnapshot::new(Vec::new(), None).expect("an empty registry compiles");
        let nothing = TemplateRead {
            template: "template/site/none".parse().expect("a scope path"),
            tag: None,
            scopes: Vec::new(),
            catalog: Vec::new(),
            registry,
        };
        let stamped: Evaluation = evaluate(&nothing, &KnobValues::new(), &Prior::new(DocId::mint()), &mut Mint);
        stamped.engine.version
    })
}

/// `root`'s thumbnail at `seqs`: kept, or computed now in the background and `evaluating` meanwhile. The row and the
/// computation that fills it start together in a task of their own, which a request dropped meanwhile does not stop,
/// since a row written for a request that stopped waiting would otherwise stay `evaluating` with nothing computing it.
async fn thumbnail<S: Wps>(
    gate: &GateState<S>,
    root: &ScopePath,
    seqs: &BTreeMap<ScopePath, Seq>,
    read: &Arc<Read>,
    instance: &str,
) -> Result<Thumbnail, ProblemAnswer> {
    let fault = |fault| internal("read the template's thumbnail", fault, instance);
    let generators = gate.world.registry().snapshot().generators.clone();
    let named = json!(generators);
    let (task, template, at, keyed, read) =
        (gate.clone(), root.as_str().to_owned(), json!(seqs), named.clone(), read.clone());
    let started = tokio::spawn(async move {
        let made: Option<String> = sqlx::query_scalar(
            "INSERT INTO template_thumbnails (id, template, template_seqs, engine, generators) \
             VALUES ($1, $2, $3, $4, $5) ON CONFLICT (template, template_seqs, engine, generators) DO NOTHING RETURNING id",
        )
        .bind(mint_ulid().as_str())
        .bind(&template)
        .bind(&at)
        .bind(engine_version())
        .bind(&keyed)
        .fetch_optional(&task.app.pool)
        .await?;
        if let Some(id) = made.clone() {
            tokio::spawn(compute_thumbnail(task, id, read, generators));
        }
        Ok::<bool, SqlxError>(made.is_some())
    });
    if started.await.map_err(|stopped| fault(decode(stopped)))?.map_err(fault)? {
        return Ok(Thumbnail {
            state: ThumbnailState::Evaluating,
            floor_key: None,
            census: None,
            problems: Vec::new(),
        });
    }
    let row = sqlx::query(
        "SELECT state, census, problems, floor_key FROM template_thumbnails \
         WHERE template = $1 AND template_seqs = $2 AND engine = $3 AND generators = $4",
    )
    .bind(root.as_str())
    .bind(json!(seqs))
    .bind(engine_version())
    .bind(&named)
    .fetch_one(&gate.app.pool)
    .await
    .map_err(fault)?;
    let state: ThumbnailState = read_word(&row.try_get::<String, _>("state").map_err(fault)?).map_err(fault)?;
    let census: Option<Census> = row
        .try_get::<Option<Value>, _>("census")
        .map_err(fault)?
        .map(serde_json::from_value)
        .transpose()
        .map_err(|f| fault(decode(f)))?;
    let problems: Vec<EvaluationProblem> =
        serde_json::from_value(row.try_get("problems").map_err(fault)?).map_err(|f| fault(decode(f)))?;
    let floor_key = row.try_get::<Option<String>, _>("floor_key").map_err(fault)?;
    Ok(Thumbnail { state, floor_key, census, problems })
}

/// Compute thumbnail row `id`: the default evaluation, its plan and floor kept under `template-thumbnails/{id}/`.
/// A store that refused takes the row away, so the next list computes it again.
async fn compute_thumbnail<S: Wps>(gate: GateState<S>, id: String, read: Arc<Read>, generators: GeneratorSet) {
    let (knobs, name) = (BTreeMap::new(), read.description.title.clone());
    let inputs = Arc::new(Inputs { read, knobs, prior: Prior::new(DocId::mint()), name, generators });
    let work = move || compute(&inputs);
    let pool = &gate.app.pool;
    let Ok(mut computed) = pooled(&*gate.app.compute, work).await else {
        let _ = sqlx::query("DELETE FROM template_thumbnails WHERE id = $1").bind(&id).execute(pool).await;
        return;
    };
    let kept = keep(&gate.app.runstore, &format!("template-thumbnails/{id}"), &mut computed).await;
    if kept.plan.is_none() {
        let _ = sqlx::query("DELETE FROM template_thumbnails WHERE id = $1").bind(&id).execute(pool).await;
        return;
    }
    let state = if computed.ready { "ready" } else { "failed" };
    let stored = sqlx::query(
        "UPDATE template_thumbnails SET state = $2, census = $3, problems = $4, floor_key = $5 WHERE id = $1",
    )
    .bind(&id)
    .bind(state)
    .bind(json!(computed.census))
    .bind(json!(computed.problems))
    .bind(kept.floor.as_ref().map(|(key, _)| key.as_str()))
    .execute(pool)
    .await;
    if let Err(fault) = stored {
        eprintln!("studio-api: thumbnail {id} was computed and not recorded: {fault}");
    }
}
