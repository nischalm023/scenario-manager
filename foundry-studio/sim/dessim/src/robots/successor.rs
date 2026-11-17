//! Within `robots`, `successor` is Agent Studio's agent override (spec 07 section 3.7; ruling D38): an entity carrying
//! `successor` succeeds the catalog item its `of` names, at a version the export carries, and holds only the
//! components it replaces. `resolve_successors` gives each such entity the item's components, each replaced whole by
//! the override's own component of the same name, before any fleet reader reads the world, so the fleet census, the
//! racking assignment of climbing cohorts and the robot figures read a cohort that names an override as they read
//! one that names the item, with one implementation for every consumer. The entity keeps its id and its name.

use serde_json::Value;
use site_model::entity::Entity;
use site_model::ids::EntityId;
use site_model::pipeline::World;

use super::materialize::{Refusal, RobotError};

/// The override's component: its one field, `of`, names the item succeeded.
pub const SUCCESSOR: &str = "successor";

/// The world with every override resolved over the item it succeeds. It refuses an `of` the world does not carry, an
/// override of an override, and an override that replaces no component of its item. A world without `successor`
/// comes back as it went in.
pub fn resolve_successors(mut world: World) -> Result<World, RobotError> {
    let overrides: Vec<EntityId> =
        world.entities.values().filter(|entity| entity.has(SUCCESSOR)).map(|entity| entity.id.clone()).collect();
    for id in overrides {
        let entity = &world.entities[&id];
        let refuse = |refusal: Refusal| RobotError { entity: id.clone(), name: entity.name.clone(), refusal };
        let named = entity.components[SUCCESSOR].get("of").and_then(Value::as_str).unwrap_or_default();
        let missing = || refuse(Refusal::SuccessorOfMissing { of: named.to_owned() });
        let of: EntityId = named.split_once('@').map_or(named, |(bare, _)| bare).parse().map_err(|_| missing())?;
        let item = world.entities.get(&of).ok_or_else(missing)?;
        if item.has(SUCCESSOR) {
            return Err(refuse(Refusal::SuccessorOfSuccessor { of }));
        }
        let replaces = entity.components.keys().any(|name| name != SUCCESSOR && item.has(name));
        if !replaces {
            return Err(refuse(Refusal::SuccessorReplacesNothing { of }));
        }
        let mut components = item.components.clone();
        components.extend(entity.components.iter().map(|(name, value)| (name.clone(), value.clone())));
        let resolved = Entity { components, ..entity.clone() };
        world.entities.insert(id, resolved);
    }
    Ok(world)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use serde_json::{Value, json};
    use site_model::entity::Entity;
    use site_model::ids::EntityId;

    use crate::worldmap::{WorldMap, WorldMapError, load_from_bytes};

    const VTM: &str = "d257c36b-53ea-5ba2-a0f3-6fd0346f00ac";
    const VTM_COHORT: &str = "a0a3e703-695c-5d8a-9577-17cd47521b10";
    const OVERRIDE: &str = "6c7f1f3e-2f39-4a43-9b8f-0d2c5e7a91b4";

    fn fixture() -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal_putaway.json");
        serde_json::from_slice(&std::fs::read(path).expect("the fixture reads")).expect("JSON")
    }

    fn entity<'a>(doc: &'a mut Value, id: &str) -> &'a mut Value {
        let entities = doc["entities"].as_array_mut().expect("entities");
        entities.iter_mut().find(|entity| entity["id"] == id).expect("the entity")
    }

    /// The fixture with an override of its VTM class declared, stating its own `end_effector`, and the VTM cohort
    /// naming it when `named`.
    fn overridden(of: &str, components: Value, named: bool) -> Value {
        let mut doc = fixture();
        let mut parts = components.as_object().cloned().expect("an object");
        parts.insert("successor".to_owned(), json!({ "of": of }));
        let declared = json!({ "id": OVERRIDE, "name": "VTM A5, this site's figures", "components": parts });
        doc["entities"].as_array_mut().expect("entities").push(declared);
        if named {
            entity(&mut doc, VTM_COHORT)["components"]["cohort"]["robot_class"] = json!(OVERRIDE);
        }
        doc
    }

    fn load(doc: &Value) -> Result<WorldMap, WorldMapError> {
        load_from_bytes(&serde_json::to_vec(doc).expect("JSON"))
    }

    const FIGURES: &str = r#"{ "end_effector": { "pick_secs": 7.1, "drop_secs": 5.3,
        "align_before_pick_secs": 1.7, "align_before_drop_secs": 1.7 } }"#;

    /// Spec 07 section 8, statement 8: the override takes the catalog item's components with `end_effector` replaced
    /// whole; a cohort naming it counts in the fleet census, joins its aisle in the racking assignment and gets the
    /// override's figures; naming the item again restores the item's.
    #[test]
    fn a_cohort_naming_an_override_reads_the_items_components_with_the_override_replacing_whole() {
        let catalog = load(&fixture()).expect("the fixture loads");
        let figures: Value = serde_json::from_str(FIGURES).expect("JSON");
        let world = load(&overridden(&format!("{VTM}@12"), figures.clone(), true)).expect("the override loads");
        let resolved = &world.world.entities[&OVERRIDE.parse().expect("an id")];
        assert_eq!(resolved.components["end_effector"], figures["end_effector"], "replaced whole, never merged");
        let item = &catalog.world.entities[&VTM.parse().expect("an id")];
        for name in ["robot_class", "lift", "rack_ladder"] {
            assert_eq!(resolved.components[name], item.components[name], "{name} is the item's");
        }
        assert_eq!(resolved.name.as_deref(), Some("VTM A5, this site's figures"), "the override keeps its name");
        assert_eq!(world.map.vtm_count(), catalog.map.vtm_count(), "the cohort still counts and joins its aisle");
        let vtm = &world.robot_catalog.vtm[&OVERRIDE.parse().unwrap()];
        assert_eq!((vtm.handling.pick_secs, vtm.handling.drop_secs), (7.1, 5.3), "the override's figures");
        let restored = load(&overridden(&format!("{VTM}@12"), figures, false)).expect("loads");
        let vtm = &restored.robot_catalog.vtm[&VTM.parse().unwrap()];
        assert_eq!((vtm.handling.pick_secs, vtm.handling.drop_secs), (4.0, 2.0), "the cohort naming the item reads the item's");
    }

    #[test]
    fn a_world_without_successor_loads_unchanged() {
        let doc = fixture();
        let world = load(&doc).expect("the fixture loads");
        let resolved = super::resolve_successors((*world.world).clone()).expect("nothing to resolve");
        let read = |entities: &BTreeMap<EntityId, Entity>| -> Vec<(String, Option<String>, Value)> {
            let read = |entity: &Entity| (entity.id.to_string(), entity.name.clone(), json!(entity.components));
            entities.values().map(read).collect()
        };
        assert_eq!(read(&resolved.entities), read(&world.world.entities));
    }

    /// Statement 8's refusals: an override of an override, an `of` the world does not carry, and one that replaces
    /// nothing of its item.
    #[test]
    fn an_override_of_an_override_one_of_nothing_and_one_replacing_nothing_are_refused() {
        let figures: Value = serde_json::from_str(FIGURES).expect("JSON");
        let mut chained = overridden(VTM, figures.clone(), true);
        let second = json!({ "id": "0b1d3e5f-7a9c-4b2d-8e6f-1a3c5e7b9d0f",
            "components": { "successor": { "of": OVERRIDE }, "end_effector": figures["end_effector"] } });
        chained["entities"].as_array_mut().expect("entities").push(second);
        let refused = load(&chained).map(drop).expect_err("an override of an override");
        assert!(refused.to_string().contains("itself an override"), "{refused}");
        let nowhere = load(&overridden("7b0c2d4e-6f8a-4b1c-9d3e-5f7a9b1c3d5e", figures, true));
        assert!(nowhere.map(drop).expect_err("of names nothing").to_string().contains("does not carry"));
        let empty = load(&overridden(VTM, json!({}), true));
        assert!(empty.map(drop).expect_err("replaces nothing").to_string().contains("replaces no component"));
    }
}
