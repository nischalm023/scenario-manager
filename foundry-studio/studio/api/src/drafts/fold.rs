//! Within `drafts`, the `fold` is spec 01 section 3.11's rule for what a draft keeps of the operations added to it: a
//! patch replaces every earlier patch of its target at the same pointer or below it, and drops out itself when it
//! sets the field back to its value in the cut while no earlier patch above its pointer remains. A tool call and a
//! PATCH fold alike, so a field edited twice is one operation and a field set back is none (spec 01 statement 5).

use serde_json::Value;
use studio_protocol::{ChangesetOp, EntityId};

/// Whether `pointer` lies strictly below `of`.
fn below(pointer: &str, of: &str) -> bool {
    pointer.len() > of.len() && pointer.starts_with(of) && pointer.as_bytes()[of.len()] == b'/'
}

/// `held` with `added` folded in, in order; `in_cut` answers a target's value at a pointer in the cut the draft was
/// built on, `None` where the field is absent there.
pub fn fold(
    mut held: Vec<ChangesetOp>,
    added: Vec<ChangesetOp>,
    in_cut: impl Fn(&EntityId, &str) -> Option<Value>,
) -> Vec<ChangesetOp> {
    for op in added {
        let ChangesetOp::Patch { target, pointer, value } = &op;
        held.retain(|kept| {
            let ChangesetOp::Patch { target: other, pointer: at, .. } = kept;
            !(other == target && (at == pointer || below(at, pointer)))
        });
        let above = held.iter().any(|kept| {
            let ChangesetOp::Patch { target: other, pointer: at, .. } = kept;
            other == target && below(pointer, at)
        });
        if above || in_cut(target, pointer).as_ref() != Some(value) {
            held.push(op);
        }
    }
    held
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn patch(target: &EntityId, pointer: &str, value: Value) -> ChangesetOp {
        ChangesetOp::Patch { target: target.clone(), pointer: pointer.into(), value }
    }

    /// Spec 01 statement 5's draft half: a field edited twice is one operation, a field set back is none.
    #[test]
    fn a_field_edited_twice_is_one_operation_and_one_set_back_is_none() {
        let node: EntityId = "b6a9a763-687c-4067-909b-460e986e2229".parse().unwrap();
        let factor = "/flow.node/intake/0/admission_factor";
        let in_cut = |_: &EntityId, at: &str| (at == factor).then(|| json!(4.0));
        let once = fold(Vec::new(), vec![patch(&node, factor, json!(5.0))], in_cut);
        let twice = fold(once, vec![patch(&node, factor, json!(6.0))], in_cut);
        assert_eq!(twice, [patch(&node, factor, json!(6.0))]);
        assert_eq!(fold(twice, vec![patch(&node, factor, json!(4.0))], in_cut), []);
    }

    /// A patch above replaces the patches below it; a patch below one above stays even when it restores the cut.
    #[test]
    fn a_patch_replaces_those_below_it_and_stays_below_one_above() {
        let node: EntityId = "b6a9a763-687c-4067-909b-460e986e2229".parse().unwrap();
        let in_cut = |_: &EntityId, _: &str| Some(json!(7000));
        let held = vec![patch(&node, "/flow.node/work/pick_sim_ms", json!(6500))];
        let whole = fold(held, vec![patch(&node, "/flow.node/work", json!({ "pick_sim_ms": 6000 }))], in_cut);
        assert_eq!(whole, [patch(&node, "/flow.node/work", json!({ "pick_sim_ms": 6000 }))]);
        let restored = fold(whole.clone(), vec![patch(&node, "/flow.node/work/pick_sim_ms", json!(7000))], in_cut);
        assert_eq!(restored.len(), 2, "the patch above still sets the field, so the restoring patch is kept");
    }
}
