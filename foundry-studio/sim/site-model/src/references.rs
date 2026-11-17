//! Which parts of which components point at another entity, written out once.
//!
//! SPEC §2 says references are UUIDs inside component fields and that which
//! fields those are is part of each component's schema, "not guessable from
//! shape". So somebody has to write the list by hand, and this module is the
//! one place it is written: the loader reads it to refuse a dangling reference
//! (SPEC §8 step 6), and the structure report reads it to draw the arrows a
//! person follows. Two hand-kept copies of a list like this drift apart
//! quietly — the report stops drawing an arrow the loader still checks — and a
//! single list is the only thing that rules that out.
//!
//! The format has two kinds of pointer and they are kept apart here because the
//! loader treats them differently. A plain reference is a UUID in a field and a
//! dangling one is one error. A *cell address* (SPEC §3) is any of three
//! spellings — a bare UUID, a derivation address, a template-local address —
//! and the loader checks those where it can say something better than "does not
//! exist": a queue step that lands on a cell an exclusion removed is a
//! different mistake from a queue step that was never a cell at all. Both kinds
//! are arrows a reader wants to follow, so both are here; only the first is the
//! loader's list.

// Documents still carry the deprecated `robot_class`, so its references are still read.
#[allow(deprecated)]
use crate::components::fleet::{Carrier, CarrierPool, Cohort, Robot, RobotClass};
use crate::components::product::{CellAddress, Port, Queue, Sector, Stamp};
use crate::components::spatial::{Frame, Pose, Region};
use crate::components::structure::{Access, AccessCell, Relay, Slot, Stitch, Storage};
use crate::entity::Entity;
use crate::error::LoadError;
use crate::ids::EntityId;

/// One entity pointing at another through a named component field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub component: &'static str,
    /// The field as its component's schema spells it, dotted where the
    /// reference sits inside a nested object (`assignment.to`).
    pub field: &'static str,
    pub target: EntityId,
    /// Whether the target has to be a frame and not merely an entity that
    /// exists. It is part of the field's schema rather than a check somebody
    /// remembered to add, so it travels with the field.
    pub must_be_frame: bool,
}

impl Reference {
    fn plain(component: &'static str, field: &'static str, target: EntityId) -> Reference {
        Reference {
            component,
            field,
            target,
            must_be_frame: false,
        }
    }

    fn frame(component: &'static str, field: &'static str, target: EntityId) -> Reference {
        Reference {
            component,
            field,
            target,
            must_be_frame: true,
        }
    }
}

/// Every reference one entity makes through a component field — the list the
/// loader refuses a dangling one from.
///
/// The order is the order of the component vocabulary below, and it is
/// load-bearing: it is the order a refusal reports in, so a document with two
/// dangling references fails on the same one every time.
///
/// This can only fail on a component that is present and the wrong shape —
/// parsing a typed view is what reading the reference costs (SPEC §9 keeps
/// unknown *fields* legal, so nothing here refuses a newer producer's addition).
#[allow(deprecated)]
pub fn of(entity: &Entity) -> Result<Vec<Reference>, LoadError> {
    let mut out = Vec::new();
    if let Some(frame) = entity.component::<Frame>("frame")? {
        if let Some(parent) = frame.parent {
            out.push(Reference::frame("frame", "parent", parent));
        }
    }
    if let Some(pose) = entity.component::<Pose>("pose")? {
        if let Some(frame) = pose.frame {
            out.push(Reference::frame("pose", "frame", frame));
        }
    }
    if let Some(region) = entity.component::<Region>("region")? {
        if let Some(frame) = region.frame {
            out.push(Reference::frame("region", "frame", frame));
        }
    }
    if let Some(stamp) = entity.component::<Stamp>("stamp")? {
        out.push(Reference::plain("stamp", "template", stamp.template));
    }
    if let Some(access) = entity.component::<Access>("access")? {
        out.push(Reference::plain("access", "rack", access.rack));
        out.push(Reference::plain("access", "along", access.along));
        for class in access.serves.iter().flatten() {
            out.push(Reference::plain("access", "serves", class.clone()));
        }
    }
    if let Some(stitch) = entity.component::<Stitch>("stitch")? {
        out.push(Reference::plain("stitch", "a", stitch.a));
        out.push(Reference::plain("stitch", "b", stitch.b));
    }
    if let Some(sector) = entity.component::<Sector>("sector")? {
        for member in &sector.members {
            out.push(Reference::plain("sector", "members", member.clone()));
        }
    }
    if let Some(storage) = entity.component::<Storage>("storage")? {
        if let Some(carrier_type) = storage.carrier_type {
            out.push(Reference::plain("storage", "carrier_type", carrier_type));
        }
    }
    if let Some(cohort) = entity.component::<Cohort>("cohort")? {
        out.push(Reference::plain(
            "cohort",
            "robot_class",
            cohort.robot_class,
        ));
        if let Some(assignment) = &cohort.assignment {
            for target in &assignment.to {
                out.push(Reference::plain("cohort", "assignment.to", target.clone()));
            }
        }
    }
    if let Some(pool) = entity.component::<CarrierPool>("carrier_pool")? {
        out.push(Reference::plain(
            "carrier_pool",
            "carrier_type",
            pool.carrier_type,
        ));
        if let Some(placement) = &pool.placement {
            if let Some(sector) = &placement.sector {
                out.push(Reference::plain(
                    "carrier_pool",
                    "placement.sector",
                    sector.clone(),
                ));
            }
        }
    }
    if let Some(class) = entity.component::<RobotClass>("robot_class")? {
        if let Some(frame) = class.native_frame {
            // Existence only, unlike `pose.frame` and `region.frame` above —
            // see SPEC-GAPS: nothing decides yet whether a class's native frame
            // has to be a frame.
            out.push(Reference::plain("robot_class", "native_frame", frame));
        }
    }
    if let Some(port) = entity.component::<Port>("port")? {
        if let Some(into) = port.into {
            out.push(Reference::plain("port", "into", into));
        }
        if let Some(from) = port.from {
            out.push(Reference::plain("port", "from", from));
        }
    }
    if let Some(relay) = entity.component::<Relay>("relay")? {
        out.push(Reference::plain("relay", "rack", relay.rack));
    }
    if let Some(slot) = entity.component::<Slot>("slot")? {
        out.push(Reference::plain("slot", "rack", slot.rack));
    }
    if let Some(robot) = entity.component::<Robot>("robot")? {
        out.push(Reference::plain("robot", "robot_class", robot.robot_class));
        out.push(Reference::plain("robot", "cohort", robot.cohort));
        if let Some(assignment) = &robot.assignment {
            for target in &assignment.to {
                out.push(Reference::plain("robot", "assignment.to", target.clone()));
            }
        }
    }
    if let Some(carrier) = entity.component::<Carrier>("carrier")? {
        out.push(Reference::plain(
            "carrier",
            "carrier_type",
            carrier.carrier_type,
        ));
        out.push(Reference::plain("carrier", "pool", carrier.pool));
        if let Some(placement) = &carrier.placement {
            if let Some(sector) = &placement.sector {
                out.push(Reference::plain(
                    "carrier",
                    "placement.sector",
                    sector.clone(),
                ));
            }
        }
    }
    Ok(out)
}

/// Every cell one entity addresses: the steps of a queue's path, the ends of a
/// stitch's explicit pairs, and the cell a relay is bound to, in the order they
/// are written.
///
/// A derivation address resolves by hashing its chain (SPEC §3), so this needs
/// nothing but the entity itself — and a template-local address needs a stamp,
/// which means it has already been rewritten by the time anything calls this
/// on a loaded world. Nothing here is checked against the world: the loader
/// does that in [`crate::pipeline::check`], where it can tell a step that was
/// never a cell from a step whose cell an exclusion removed.
pub fn cells_addressed(entity: &Entity) -> Result<Vec<Reference>, LoadError> {
    let mut out = Vec::new();
    if let Some(queue) = entity.component::<Queue>("queue")? {
        for step in &queue.path {
            if let Some(cell) = addressed(step)? {
                out.push(Reference::plain("queue", "path", cell));
            }
        }
    }
    if let Some(stitch) = entity.component::<Stitch>("stitch")? {
        for pair in stitch.pairs.iter().flatten() {
            for end in pair {
                if let Some(cell) = addressed(end)? {
                    out.push(Reference::plain("stitch", "pairs", cell));
                }
            }
        }
    }
    // A relay's binding is a cell address like any other, whether the search
    // found it or an override stated it (SPEC §5).
    if let Some(binding) = entity.component::<AccessCell>("access_cell")? {
        if let Some(cell) = addressed(&binding.cell)? {
            out.push(Reference::plain("access_cell", "cell", cell));
        }
    }
    Ok(out)
}

/// Both kinds of arrow, plain references first: what somebody following the
/// structure of a world wants to see.
pub fn all(entity: &Entity) -> Result<Vec<Reference>, LoadError> {
    let mut out = of(entity)?;
    out.extend(cells_addressed(entity)?);
    Ok(out)
}

/// The identity a cell address names, when it names one that can be resolved
/// without a stamp.
fn addressed(address: &CellAddress) -> Result<Option<EntityId>, LoadError> {
    match address {
        CellAddress::Id(id) => Ok(Some(id.clone())),
        CellAddress::Derived(raw) => Ok(Some(raw.parse()?.into())),
        CellAddress::Local(_) | CellAddress::Handle(_) => Ok(None),
        // A position is geometry, not a reference: there is nothing to dangle.
        CellAddress::At(_) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::DeclaredEntityId;
    use serde_json::{json, Value};
    use uuid::Uuid;

    fn entity(components: Value) -> Entity {
        let mut entity = Entity::new(DeclaredEntityId(Uuid::nil()));
        for (name, value) in components.as_object().expect("a component map") {
            entity.components.insert(name.clone(), value.clone());
        }
        entity
    }

    fn generator() -> DeclaredEntityId {
        DeclaredEntityId(
            "b83f97cc-9ff1-49ec-b547-5c1a1ff88411"
                .parse()
                .expect("a literal UUID"),
        )
    }

    /// The identity a generator derives at one key, spelled the way a document
    /// spells it.
    fn at(key: &str) -> EntityId {
        format!("{}/{key}", generator())
            .parse()
            .expect("a literal identity")
    }

    #[test]
    fn a_queue_path_addresses_the_cells_its_keys_hash_to() {
        let station = entity(json!({
            "queue": { "path": [
                { "generator": generator(), "key": "instance:0/local:order_queue/cell:0,0" },
                { "generator": generator(), "key": "instance:0/local:order_queue/cell:1,0" }
            ]}
        }));
        let cells = cells_addressed(&station).expect("the addresses parse");
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].component, "queue");
        assert_eq!(cells[0].field, "path");
        assert_eq!(cells[0].target, at("instance:0/local:order_queue/cell:0,0"));
        assert_eq!(cells[1].target, at("instance:0/local:order_queue/cell:1,0"));
    }

    /// The loader's list stops at UUID-in-a-field on purpose. A queue path in
    /// it would make [`crate::pipeline::check`]'s reference pass fire first and
    /// answer "does not exist" where that module can say which of three things
    /// went wrong — the step was never a cell, or its cell was removed, or the
    /// address reaches nothing at all.
    #[test]
    fn the_loaders_reference_list_leaves_cell_addresses_to_the_checks_that_can_explain_them() {
        let station = entity(json!({
            "queue": { "path": [
                { "generator": generator(), "key": "cell:0,0" }
            ]},
            "stitch": { "a": Uuid::nil(), "b": Uuid::nil(), "rule": "explicit",
                        "pairs": [[{ "generator": generator(), "key": "cell:0,0" },
                                   { "generator": generator(), "key": "cell:1,0" }]] }
        }));
        let fields: Vec<&str> = of(&station)
            .expect("the components parse")
            .iter()
            .map(|reference| reference.field)
            .collect();
        assert_eq!(fields, ["a", "b"], "only the stitch's two endpoints");
        assert_eq!(
            cells_addressed(&station)
                .expect("the addresses parse")
                .len(),
            3
        );
        assert_eq!(all(&station).expect("the addresses parse").len(), 5);
    }
}
