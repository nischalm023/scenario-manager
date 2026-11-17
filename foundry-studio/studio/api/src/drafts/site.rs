//! Within `drafts`, the `site` a draft is checked against is spec 01 section 3.11's complete would-be site: every
//! scope of the draft's cut, those it does not write included, as the flagged export a run reads, with the draft's
//! patches applied in memory and compiled by spec 06's flow compiler in validation mode; and the changeset the draft
//! writes, built by spec 04's draft builder against the cut and checked by the client against the registry.

use bytes::Bytes;
use dessim::composition::flowgraph::{Request, compile};
use dessim::worldmap::flows::read as read_flows;
use dessim::worldmap::load_from_bytes;
use serde_json::{Value, json};
use studio_protocol::{ChangesetOp, ChangesetPart, DeclaredId, EntityId, Key, ProblemItem, ScopePath, Severity};
use wps_client::compute::run as on_pool;
use wps_client::draft::set_pointer;
use wps_client::wps::Wps;
use wps_client::{Author, ChangesetDraft, DraftBuilder, Problem as WpsProblem, UnitCut, UnitSnapshot, WorldStore};

use crate::GateState;
use crate::flows::answer::item;
use crate::problem::{ProblemAnswer, validation_failed, wps_problem};

/// What a draft's operation patches: a declared entity, the component named first in the pointer, and the pointer
/// inside that component (empty for the whole component).
struct Target {
    id: DeclaredId,
    component: String,
    inner: String,
}

fn target(op: &ChangesetOp) -> Result<Target, String> {
    let ChangesetOp::Patch { target, pointer, .. } = op;
    let id: DeclaredId = (target.to_string().parse()).map_err(|_| format!("{target} is no declared entity"))?;
    let rest = pointer.strip_prefix('/').ok_or_else(|| format!("{pointer} opens with `/` and a component's name"))?;
    let (component, inner) = rest.split_once('/').map_or((rest, String::new()), |(c, i)| (c, format!("/{i}")));
    if !(component.starts_with("flow.") || component == "sector") {
        return Err(format!(
            "{pointer} is in neither a flow component nor a map sector, the two a draft edits (lean 8)"
        ));
    }
    Ok(Target { id, component: component.to_owned(), inner })
}

fn entity<'a>(document: &'a Value, id: &EntityId) -> Option<&'a Value> {
    let id = json!(id.to_string());
    document.get("entities")?.as_array()?.iter().find(|entity| entity.get("id") == Some(&id))
}

/// The value at `pointer` (component first) of an entity in a flagged export's file; `None` where it is absent.
pub fn value_at(document: &Value, id: &EntityId, pointer: &str) -> Option<Value> {
    entity(document, id)?.get("components")?.pointer(pointer).cloned()
}

/// A flagged export's file with `ops` applied in memory.
pub fn patched(document: &Value, ops: &[ChangesetOp]) -> Result<Value, String> {
    let mut out = document.clone();
    for op in ops {
        let ChangesetOp::Patch { target: id, value, .. } = op;
        let Target { component, inner, .. } = target(op)?;
        let wanted = json!(id.to_string());
        let list = out.get_mut("entities").and_then(Value::as_array_mut).ok_or("the export lists no entities")?;
        let found = list
            .iter_mut()
            .find(|entity| entity.get("id") == Some(&wanted))
            .ok_or_else(|| format!("{id} is not in the cut"))?;
        let components =
            found.get_mut("components").and_then(Value::as_object_mut).ok_or("an entity without components")?;
        set_pointer(components.entry(component).or_insert_with(|| json!({})), &inner, value.clone())?;
    }
    Ok(out)
}

/// A finding of the client's check, or WPS's refusal, as spec 01's item: each stops the write, so each is an error.
pub(super) fn client_item(problem: &WpsProblem) -> ProblemItem {
    ProblemItem {
        code: problem.kind.to_string(),
        severity: Severity::Error,
        ref_: problem.entity.clone().map(Key::Entity),
        scope: problem.scope.clone(),
        field: problem.field(),
        knob: None,
        text: problem.text.clone(),
    }
}

/// What one check found: every problem of the complete would-be site, the scopes the draft writes with their
/// operations counted, and the changeset an apply would send.
pub struct Checked {
    pub problems: Vec<ProblemItem>,
    pub parts: Vec<ChangesetPart>,
    pub changeset: ChangesetDraft,
}

impl Checked {
    pub fn has_errors(&self) -> bool {
        self.problems.iter().any(|problem| problem.severity == Severity::Error)
    }
}

/// The cut's flagged export as JSON, which the fold reads values in the cut from.
pub async fn base<S: Wps>(gate: &GateState<S>, cut: &UnitCut, instance: &str) -> Result<Value, ProblemAnswer> {
    let export = gate.world.export(cut).await.map_err(|error| wps_problem(&error, instance))?;
    serde_json::from_slice(&export.bytes).map_err(|fault| validation_failed(fault, instance))
}

/// The changeset `ops` write against `cut`, built by spec 04's draft builder as `operation` by `author`, and the
/// snapshot of the cut it was built on.
#[allow(clippy::too_many_arguments)]
pub async fn build<S: Wps>(
    gate: &GateState<S>,
    site: &ScopePath,
    cut: &UnitCut,
    summary: &str,
    ops: &[ChangesetOp],
    operation: &str,
    author: &str,
    instance: &str,
) -> Result<(ChangesetDraft, UnitSnapshot), ProblemAnswer> {
    let world = &gate.world;
    let base = world.read_unit(cut).await.map_err(|error| wps_problem(&error, instance))?;
    let author = Author::new(author).map_err(|fault| validation_failed(fault, instance))?;
    let mut builder = DraftBuilder::new(&base, site.clone(), author, summary, operation);
    for op in ops {
        let ChangesetOp::Patch { target: entity, value, .. } = op;
        let Target { id, component, inner } = target(op).map_err(|why| validation_failed(why, instance))?;
        let (wanted, family) = if component == "sector" { ("map", "map sector") } else { ("flow", "flow entity") };
        if base.owner_of(entity).and_then(|scope| scope.slugs().last()) != Some(wanted) {
            let why =
                format!("{entity} is no {family} of this site's cut, so its `{component}` is not a draft's to edit");
            return Err(validation_failed(why, instance));
        }
        builder.patch_pointer(id, &component, &inner, value.clone()).map_err(|error| wps_problem(&error, instance))?;
    }
    let changeset = builder.build();
    Ok((changeset, base))
}

/// Build the changeset `ops` write against `cut` (as `operation`, by `author`), check it with the client, and compile
/// the complete would-be site in validation mode.
#[allow(clippy::too_many_arguments)]
pub async fn check<S: Wps>(
    gate: &GateState<S>,
    site: &ScopePath,
    cut: &UnitCut,
    document: &Value,
    summary: &str,
    ops: &[ChangesetOp],
    operation: &str,
    author: &str,
    instance: &str,
) -> Result<Checked, ProblemAnswer> {
    let (changeset, base) = build(gate, site, cut, summary, ops, operation, author, instance).await?;
    let mut problems: Vec<ProblemItem> = (gate.world.check(&changeset, &base).await)
        .map_err(|error| wps_problem(&error, instance))?
        .iter()
        .map(client_item)
        .collect();
    let would_be = patched(document, ops).map_err(|why| validation_failed(why, instance))?;
    let bytes = Bytes::from(serde_json::to_vec(&would_be).map_err(|fault| validation_failed(fault, instance))?);
    let flow_scope = site.join(&"flow".parse().expect("a slug")).map_err(|fault| validation_failed(fault, instance))?;
    let compiled = on_pool(&*gate.app.compute, move || {
        let world = load_from_bytes(&bytes).map_err(|why| why.to_string())?;
        let compiled = compile(&world, &read_flows(&world.world), &Request::validation());
        Ok::<_, String>(compiled.problems.iter().map(|finding| item(finding, &flow_scope)).collect::<Vec<_>>())
    })
    .await
    .map_err(|error| wps_problem(&error, instance))?
    .map_err(|why| {
        validation_failed(format!("the would-be site will not load in the simulator's reader: {why}"), instance)
    })?;
    problems.extend(compiled);
    // WPS reads a changeset naming no operation as one writing its initiating scope; a draft holding none writes none.
    let parts = (changeset.written().into_iter())
        .map(|scope| ChangesetPart { ops: changeset.ops_of(&scope).count() as u32, scope })
        .filter(|part| part.ops > 0)
        .collect();
    Ok(Checked { problems, parts, changeset })
}
