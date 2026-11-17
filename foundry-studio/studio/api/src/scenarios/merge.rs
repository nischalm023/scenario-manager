//! Within `scenarios`, `merge` is spec 07 section 3.5's field merge, pure: an edit names the revision it was based on
//! and the pointers it sets; two pointers overlap when they are equal or one is the other's ancestor, and a field
//! changed after the edit's base that overlaps one it sets refuses the edit (`scenario-changed`). Otherwise the edit
//! applies to the latest content, and the whole merged content is checked, so two disjoint edits cannot together
//! break a rule that joins fields. `set_by` keeps each field's last change; setting a pointer replaces the entries
//! beneath it.

use std::collections::BTreeMap;

use serde_json::Value;
use studio_protocol::{FieldConflict, FieldSet, Revision, ScenarioContent, SetBy};

use scenario_spec::{Violation, set as set_field, validate};

use super::fields::settable;

/// Each field's last change, by pointer.
// ordered: answered in pointer order.
pub type SetByMap = BTreeMap<String, SetBy>;

/// Whether `a` and `b` name one field, or one holds the other.
pub fn overlaps(a: &str, b: &str) -> bool {
    a == b || holds(a, b) || holds(b, a)
}

/// Whether the field `inner` lies beneath the group `outer`.
fn holds(outer: &str, inner: &str) -> bool {
    inner.strip_prefix(outer).is_some_and(|rest| rest.starts_with('/'))
}

/// Every field changed after `base` that overlaps a pointer the edit sets, with its value now.
pub fn conflicts(content: &ScenarioContent, set_by: &SetByMap, base: Revision, set: &[FieldSet]) -> Vec<FieldConflict> {
    let now = serde_json::to_value(content).expect("a scenario's content is JSON");
    set_by
        .iter()
        .filter(|(changed, last)| last.revision > base && set.iter().any(|field| overlaps(&field.pointer, changed)))
        .map(|(changed, last)| FieldConflict {
            pointer: changed.clone(),
            value: now.pointer(changed).cloned().unwrap_or(Value::Null),
            set_by: Some(last.clone()),
        })
        .collect()
}

/// The content with every field of `set` applied in order, each one read back strictly, then the whole of it
/// checked; and `set_by` with each set pointer's entry replacing those beneath it.
pub fn apply(
    content: &ScenarioContent,
    set_by: &SetByMap,
    set: &[FieldSet],
    change: &SetBy,
) -> Result<(ScenarioContent, SetByMap), Vec<Violation>> {
    let mut json = serde_json::to_value(content).expect("a scenario's content is JSON");
    let mut merged = content.clone();
    for field in set {
        let refuse = |rule: String| vec![Violation { pointer: field.pointer.clone(), rule }];
        if !settable(&field.pointer) || set_field(&mut json, &field.pointer, field.value.clone()).is_none() {
            return Err(refuse("is no field of a scenario".to_owned()));
        }
        merged = serde_json::from_value(json.clone()).map_err(|fault| refuse(format!("does not read: {fault}")))?;
    }
    let found = validate(&merged);
    if !found.is_empty() {
        return Err(found);
    }
    let mut set_by = set_by.clone();
    for field in set {
        set_by.retain(|pointer, _| pointer != &field.pointer && !holds(&field.pointer, pointer));
        set_by.insert(field.pointer.clone(), change.clone());
    }
    Ok((merged, set_by))
}

/// Every field of `content` set by `change`: a new version's record.
pub fn every_field(change: &SetBy) -> SetByMap {
    super::fields::catalogue().items.iter().map(|entry| (entry.pointer.clone(), change.clone())).collect()
}
