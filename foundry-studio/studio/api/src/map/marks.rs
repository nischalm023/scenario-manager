//! Within `map`, `marks` is spec 03 section 3.5's rule table, `marks_for`: what one entity covers on the panel in
//! view, so a highlight outlines a sector's stations rather than nothing, and every reader meets the marks in one order
//! along the floor. The `map` rows are this spec's: an entity on floor cells marks itself; a sector marks its members,
//! down to what stands somewhere; anything else that stands somewhere marks itself off the floor (racks, slots and
//! regions are listed, never outlined); what stands nowhere marks nothing. Spec 06's rows for flow components join
//! here with task 21 (brief I1.11).

use rustc_hash::FxHashSet;
use studio_protocol::{EntityId, Key, Mark};

use super::loaded::{Entry, Loaded, Place};

/// The marks of `id` on the Map panel, in mark order; empty for what stands nowhere.
pub fn marks_for_map(loaded: &Loaded, id: &EntityId) -> Vec<Mark> {
    let mut marks = Vec::new();
    let mut seen = FxHashSet::default();
    walk(loaded, id, &mut seen, &mut marks);
    in_mark_order(&mut marks);
    marks
}

fn walk(loaded: &Loaded, id: &EntityId, seen: &mut FxHashSet<EntityId>, marks: &mut Vec<Mark>) {
    if !seen.insert(id.clone()) {
        return;
    }
    let Some(entry) = loaded.entities.get(id) else { return };
    match entry.place {
        Place::Cells(_) | Place::Elsewhere(_) => marks.push(mark(id, entry)),
        Place::Nowhere => {
            for member in &entry.members {
                walk(loaded, member, seen, marks);
            }
        }
    }
}

/// An entity's own mark: its key, label and components, its centre if it stands somewhere, on the floor if on cells.
pub fn mark(id: &EntityId, entry: &Entry) -> Mark {
    let (centre_mm, on_floor) = match entry.place {
        Place::Cells(centre) => (Some(centre), true),
        Place::Elsewhere(centre) => (Some(centre), false),
        Place::Nowhere => (None, false),
    };
    Mark {
        key: Key::Entity(id.clone()),
        label: entry.label.clone(),
        components: entry.components.clone(),
        centre_mm,
        on_floor,
    }
}

/// A key that marks itself where no floor places it (a template card, a scenario's field, a run's row).
pub fn itself(key: Key) -> Mark {
    Mark { label: key.to_string(), key, components: Vec::new(), centre_mm: None, on_floor: false }
}

/// Spec 03 section 3.5's order: by centre, x then y, then by key; marks with no centre after every placed one.
pub fn in_mark_order(marks: &mut [Mark]) {
    marks.sort_by_cached_key(|mark| {
        (mark.centre_mm.is_none(), mark.centre_mm.map(|[x, y]| (x, y)), mark.key.to_string())
    });
}
