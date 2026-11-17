//! Within `flowgraph`, a `patch` sets one field of one flow entity in memory before the compile (spec 06 section
//! 3.4's `flow-validate` patches; spec 07's sweep variants, which the check and the runner both apply, ruling D77).
//! Only a field the static field catalogue (`parameters`) marks sweepable may be patched, so no structural field
//! enters through a variant (the Astra review); any other is refused as `not-sweepable`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use site_model::ids::EntityId;

use super::parameters::lookup;
use super::problem::{Code, Problem};
use crate::worldmap::flows::FlowScope;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Patch {
    pub target: EntityId,
    /// The component's name and a JSON pointer into it: `/flow.material_edge/until_sim_ms`.
    pub pointer: String,
    pub value: Value,
}

/// Whether a sweep may set the field at `pointer`: the catalogue's word (`parameters`), so the parameter list and the
/// patches never disagree.
pub fn sweepable(pointer: &str) -> bool {
    lookup(pointer).is_some_and(|field| field.sweepable)
}

/// Apply `patches` to `flows` in order, refusing each that is not sweepable or names no such field.
pub fn apply(flows: &mut FlowScope, patches: &[Patch]) -> Vec<Problem> {
    let mut problems = Vec::new();
    for patch in patches {
        let field = Some(patch.pointer.clone());
        if !sweepable(&patch.pointer) {
            let text = format!(
                "{} is not a field a sweep may vary: it is a reference, a structural field or one no run reads.",
                patch.pointer
            );
            problems.push(Problem::error(Code::NotSweepable, &patch.target, field, text));
            continue;
        }
        let mut steps = patch.pointer.trim_start_matches('/').split('/');
        let component = steps.next().unwrap_or_default();
        let steps: Vec<&str> = steps.collect();
        let target = flows.get_mut(&patch.target).and_then(|entity| entity.components.get_mut(component));
        if !target.is_some_and(|value| set(value, &steps, patch.value.clone())) {
            let text = format!("the patch names {}{}, which this read does not hold.", patch.target, patch.pointer);
            problems.push(Problem::error(Code::RefUnresolved, &patch.target, field, text));
        }
    }
    problems
}

/// Set the value at `steps` below `at`, making absent objects on the way; a list index must already exist.
fn set(at: &mut Value, steps: &[&str], value: Value) -> bool {
    let Some((last, path)) = steps.split_last() else { return false };
    let mut here = at;
    for step in path {
        here = match here {
            Value::Object(fields) => fields.entry(*step).or_insert_with(|| Value::Object(Map::new())),
            Value::Array(items) => match step.parse::<usize>().ok().and_then(|at| items.get_mut(at)) {
                Some(item) => item,
                None => return false,
            },
            _ => return false,
        };
    }
    match here {
        Value::Object(fields) => {
            fields.insert((*last).to_owned(), value);
            true
        }
        _ => false,
    }
}
