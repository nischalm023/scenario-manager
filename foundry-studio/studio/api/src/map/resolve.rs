//! Within `map`, `resolve` is `POST /api/resolve` (spec 03 section 3.5): each reference a panel or a tool names, read
//! at the panel's location and expanded to the marks `marks_for` gives it there, or the whole call refused, so a panel
//! never shows a highlight the API could not resolve: a string off the grammar (`ref-malformed`), a pin the pin rule
//! refuses (`ref-pin-conflicts`, `ref-needs-version`), a reference naming nothing there (`ref-unresolved`, naming
//! each), a location the panel does not show (`panel-mismatch`). The `flow` panel's rows are spec 06's (brief I1.11).

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::Uri;
use runstore::RunStorage;
use rustc_hash::FxHashMap;
use studio_protocol::{
    EntityId, GrammarError, Key, Kinship, Mark, Mm, ObjectKind, Panel, PanelTarget, Problem, ProblemItem, ProblemType,
    Ref, RefMarks, Resolution, ResolveRequest, ScopeSeq, Severity, Ulid,
};
use worldview_render::{FloorAndRoll, RollEntry};
use wps_client::wps::Wps;

use super::marks::{in_mark_order, itself, marks_for_map};
use super::read::{SiteAt, site_at};
use crate::GateState;
use crate::problem::{ProblemAnswer, internal, json_body, validation_failed};
use crate::templates::PlanFile;

/// `POST /api/resolve`.
pub async fn resolve<S: Wps>(
    State(state): State<GateState<S>>,
    uri: Uri,
    body: Bytes,
) -> Result<Json<Resolution>, ProblemAnswer> {
    let instance = uri.path();
    let request: ResolveRequest = json_body(&body, instance)?;
    let mut refs = Vec::new();
    for text in &request.refs {
        let parsed: Ref = text.parse().map_err(|fault: GrammarError| {
            ProblemAnswer::from(Problem::new(ProblemType::RefMalformed, fault.to_string()).at(instance))
        })?;
        refs.push(parsed);
    }
    if refs.is_empty() {
        return Err(validation_failed("`refs` names at least one reference", instance));
    }
    let target: Option<PanelTarget> =
        request.location.as_deref().map(str::parse).transpose().map_err(|fault: GrammarError| {
            ProblemAnswer::from(Problem::new(ProblemType::RefMalformed, fault.to_string()).at(instance))
        })?;
    match request.panel {
        Panel::Map => on_map(&state, target, request.kin.as_deref(), request.draft.is_some(), refs, instance).await,
        Panel::Template => on_template(&state, target, refs, instance).await,
        Panel::Scenario => Ok(Json(Resolution { resolved: Vec::new(), refs: refs.into_iter().map(keyed).collect() })),
        Panel::Runs => on_runs(target, refs, instance),
        Panel::Flow => {
            Err(mismatch("The Flow panel's marks are spec 06's rows, which this build does not hold yet.", instance))
        }
    }
}

/// The Map panel: entities of the site at the location's cut, expanded by the map rows.
async fn on_map<S: Wps>(
    state: &GateState<S>,
    target: Option<PanelTarget>,
    kin: Option<&str>,
    drafted: bool,
    refs: Vec<Ref>,
    instance: &str,
) -> Result<Json<Resolution>, ProblemAnswer> {
    if drafted {
        return Err(mismatch("The Map route takes no draft; a flow draft is drawn on the Flow panel.", instance));
    }
    let Some(PanelTarget::Location(location)) = target else {
        return Err(validation_failed("the Map panel resolves at a location of a site's map: name one", instance));
    };
    let at = site_at(state, &location.to_string(), kin, instance).await?;
    map_marks(&at, refs, instance).map(Json)
}

/// Each of `refs` resolved and expanded to its Map marks in `at`, the one cut its caller read, so a panel command's
/// marks and the route it records name the same cut (spec 03 section 3.3, steps 2, 4 and 6).
pub(crate) fn map_marks(at: &SiteAt, refs: Vec<Ref>, instance: &str) -> Result<Resolution, ProblemAnswer> {
    let Ok(loaded) = at.loaded.as_ref() else {
        let detail = "This map's references cannot be read: the site's document will not load.";
        return Err(Problem::new(ProblemType::FloorCompileFailed, detail).at(instance).into());
    };
    let (mut answered, mut unresolved) = (Vec::new(), Vec::new());
    for reference in refs {
        let Ref::Entity(entity) = &reference else {
            answered.push(keyed(reference));
            continue;
        };
        let (Some(owner), Some(entry)) = (loaded.owners.get(&entity.id().declared()), loaded.entities.get(entity.id()))
        else {
            unresolved.push(reference.key());
            continue;
        };
        let kinship = if at.site.as_scope().holds(owner) { Kinship::Kin } else { Kinship::Lateral };
        if let Err(breach) = entity.clone().under_pin_rule(kinship) {
            let problem = Problem::new(breach.breach.problem_type(), breach.to_string()).at(instance);
            return Err(problem.into());
        }
        answered.push(RefMarks {
            key: reference.key(),
            scope: Some(owner.clone()),
            label: entry.label.clone(),
            components: entry.components.clone(),
            marks: marks_for_map(loaded, entity.id()),
            ref_: reference,
        });
    }
    if !unresolved.is_empty() {
        return Err(unresolved_at(&unresolved, &at.location.to_string(), instance));
    }
    let resolved = at.cut.seqs.iter().map(|(scope, seq)| ScopeSeq { scope: scope.clone(), seq: *seq }).collect();
    Ok(Resolution { resolved, refs: answered })
}

/// The Template panels: a key marks itself; on a template draft, an entity of its newest ready evaluation marks
/// itself on that evaluation's floor roll, so the agent can highlight a would-be entity before the site exists.
async fn on_template<S: Wps>(
    state: &GateState<S>,
    target: Option<PanelTarget>,
    refs: Vec<Ref>,
    instance: &str,
) -> Result<Json<Resolution>, ProblemAnswer> {
    let draft = match target {
        Some(PanelTarget::Object(object)) if object.kind() == ObjectKind::TemplateDraft => object.id().clone(),
        Some(PanelTarget::Object(object)) => {
            return Err(mismatch(&format!("The Template panel shows no {object}."), instance));
        }
        _ => return Ok(Json(Resolution { resolved: Vec::new(), refs: refs.into_iter().map(keyed).collect() })),
    };
    let shown = evaluation_floor(state, &draft, instance).await?;
    let (mut answered, mut unresolved) = (Vec::new(), Vec::new());
    for reference in refs {
        let Ref::Entity(entity) = &reference else {
            answered.push(keyed(reference));
            continue;
        };
        let id = entity.id().to_string();
        let declared = shown.names.get(&id);
        let centre = shown.centres.get(&id).copied();
        if declared.is_none() && centre.is_none() {
            unresolved.push(reference.key());
            continue;
        }
        let (label, components) = match declared {
            Some((name, components)) => (name.clone().unwrap_or_else(|| short(entity.id())), components.clone()),
            None => (short(entity.id()), Vec::new()),
        };
        let marks = centre
            .map(|centre| Mark {
                key: reference.key(),
                label: label.clone(),
                components: components.clone(),
                centre_mm: Some(centre),
                on_floor: true,
            })
            .into_iter()
            .collect();
        answered.push(RefMarks { key: reference.key(), scope: None, label, components, marks, ref_: reference });
    }
    if !unresolved.is_empty() {
        return Err(unresolved_at(&unresolved, &format!("template draft {draft}'s newest evaluation"), instance));
    }
    Ok(Json(Resolution { resolved: Vec::new(), refs: answered }))
}

/// What a draft's newest ready evaluation shows: each entity's centre on its floor, and each declared one's name and
/// components from its plan.
struct Shown {
    centres: FxHashMap<String, [Mm; 2]>,
    names: FxHashMap<String, (Option<String>, Vec<String>)>,
}

async fn evaluation_floor<S: Wps>(state: &GateState<S>, draft: &Ulid, instance: &str) -> Result<Shown, ProblemAnswer> {
    let fault = |fault| internal("read the draft's evaluation", fault, instance);
    let keys: Option<(String, String)> = sqlx::query_as(
        "SELECT floor_key, plan_key FROM template_evaluations WHERE draft = $1 AND state = 'ready' \
         AND floor_key IS NOT NULL AND plan_key IS NOT NULL ORDER BY n DESC LIMIT 1",
    )
    .bind(draft.as_str())
    .fetch_optional(&state.app.pool)
    .await
    .map_err(fault)?;
    let Some((floor_key, plan_key)) = keys else {
        let detail = format!("Template draft {draft} has no evaluation drawn yet.");
        return Err(Problem::new(ProblemType::LocationUnresolved, detail).at(instance).into());
    };
    let store = |fault| internal("read the evaluation from the run-object store", fault, instance);
    let floor = state.app.runstore.get(&floor_key).await.map_err(store)?;
    let plan = state.app.runstore.get(&plan_key).await.map_err(store)?;
    let unreadable = |fault: String| internal("read the evaluation's floor and plan", fault, instance);
    let (floor, roll) = FloorAndRoll::read(&floor).map_err(|fault| unreadable(fault.to_string()))?;
    let plan: PlanFile = serde_json::from_slice(&plan).map_err(|fault| unreadable(fault.to_string()))?;
    let whole = |value: f64| Mm::new(value.round() as i64);
    let mut centres = FxHashMap::default();
    for RollEntry { key, cells, .. } in roll {
        let bounds: Vec<[f64; 4]> = cells
            .iter()
            .filter_map(|&cell| floor.cells.get(cell as usize)?.as_ref().map(|drawn| drawn.bounds_mm()))
            .collect();
        let Some(first) = bounds.first() else { continue };
        let (lo, hi) = bounds.iter().fold(([first[0], first[1]], [first[0], first[1]]), |(lo, hi), [x, y, w, h]| {
            ([lo[0].min(*x), lo[1].min(*y)], [hi[0].max(x + w), hi[1].max(y + h)])
        });
        centres.insert(key, [whole((lo[0] + hi[0]) / 2.0), whole((lo[1] + hi[1]) / 2.0)]);
    }
    let names = plan
        .scopes
        .iter()
        .flat_map(|scope| &scope.declarations)
        .map(|made| (made.id.to_string(), (made.name.clone(), made.components.keys().cloned().collect())))
        .collect();
    Ok(Shown { centres, names })
}

/// The Runs panel: a run's row marks itself; an entity resolves only on a run's playback, at the changesets that run
/// read, and no run is recorded on this build yet.
fn on_runs(target: Option<PanelTarget>, refs: Vec<Ref>, instance: &str) -> Result<Json<Resolution>, ProblemAnswer> {
    if let Some(target) = target {
        let detail = format!("There is no run {target} on this build: runs are recorded once the run chain lands.");
        return Err(Problem::new(ProblemType::LocationUnresolved, detail).at(instance).into());
    }
    let entities: Vec<Key> =
        refs.iter().filter(|reference| matches!(reference, Ref::Entity(_))).map(Ref::key).collect();
    if !entities.is_empty() {
        return Err(unresolved_at(
            &entities,
            "the Runs list, which places entities only on one run's playback",
            instance,
        ));
    }
    Ok(Json(Resolution { resolved: Vec::new(), refs: refs.into_iter().map(keyed).collect() }))
}

/// A reference that marks its own key, as a scope, an object or a field does wherever no floor places it.
fn keyed(reference: Ref) -> RefMarks {
    let key = reference.key();
    let scope = match &reference {
        Ref::Scope(location) => Some(location.scope().clone()),
        _ => None,
    };
    let mut marks = vec![itself(key.clone())];
    in_mark_order(&mut marks);
    RefMarks { label: key.to_string(), key, scope, components: Vec::new(), marks, ref_: reference }
}

fn short(id: &EntityId) -> String {
    format!("entity {}", &id.to_string()[..8])
}

fn mismatch(detail: &str, instance: &str) -> ProblemAnswer {
    Problem::new(ProblemType::PanelMismatch, detail).at(instance).into()
}

/// `ref-unresolved`, naming each reference that names nothing `at`.
fn unresolved_at(keys: &[Key], at: &str, instance: &str) -> ProblemAnswer {
    let named: Vec<String> = keys.iter().map(ToString::to_string).collect();
    let detail = format!("Nothing at {at} is named {}.", named.join(", "));
    let mut problem = Problem::new(ProblemType::RefUnresolved, detail).at(instance);
    problem.problems = keys
        .iter()
        .map(|key| ProblemItem {
            code: "ref-unresolved".to_owned(),
            severity: Severity::Error,
            ref_: Some(key.clone()),
            scope: None,
            field: None,
            knob: None,
            text: format!("{key} names nothing at {at}"),
        })
        .collect();
    problem.into()
}
