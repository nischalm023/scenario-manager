//! Within `worldmap`, `flows` is the reading of a loaded world's flow sub-scope (spec 06 section 3.1; BACKLOG task
//! 18): every entity carrying a `flow.` component, kept as the raw components it arrived with. The simulator reads
//! none of them; the flow compiler (`composition::flowgraph`) types them, and a draft or a sweep variant patches
//! them in memory before it does, so nothing here interprets a field.
//!
//! The prefix is the four components' domain, `flow.`, with the dot: the protocol's own `flow` component, traffic
//! direction on a cell grid, is map data and stays out.

use std::collections::BTreeMap;

use serde_json::Value;
use site_model::ids::EntityId;
use site_model::pipeline::World;

/// The domain every flow component's name opens with.
pub const DOMAIN: &str = "flow.";

/// One flow entity as the document states it: its identity, its label, and every component it carries (its
/// `external_ids` included, which say whether a template made it).
#[derive(Debug, Clone, PartialEq)]
pub struct FlowEntity {
    pub id: EntityId,
    pub name: Option<String>,
    pub components: BTreeMap<String, Value>, // ordered: as `site_model::entity::Entity` keeps them
}

impl FlowEntity {
    /// The `flow.` components this entity carries, by name.
    pub fn flow_components(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.components.iter().filter(|(name, _)| name.starts_with(DOMAIN)).map(|(name, value)| (name.as_str(), value))
    }
}

/// A world's flow entities, ordered by identity.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FlowScope {
    pub entities: Vec<FlowEntity>,
}

impl FlowScope {
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    pub fn get(&self, id: &EntityId) -> Option<&FlowEntity> {
        self.entities.binary_search_by(|entity| entity.id.cmp(id)).ok().map(|at| &self.entities[at])
    }

    pub fn get_mut(&mut self, id: &EntityId) -> Option<&mut FlowEntity> {
        self.entities.binary_search_by(|entity| entity.id.cmp(id)).ok().map(|at| &mut self.entities[at])
    }
}

/// Every entity of `world` carrying a `flow.` component. `World::entities` is keyed by identity, so the result is
/// in identity order without a sort.
pub fn read(world: &World) -> FlowScope {
    let entities = world
        .entities
        .values()
        .filter(|entity| entity.components.keys().any(|name| name.starts_with(DOMAIN)))
        .map(|entity| FlowEntity {
            id: entity.id.clone(),
            name: entity.name.clone(),
            components: entity.components.clone(),
        })
        .collect();
    FlowScope { entities }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::json;
    use site_model::entity::Entity;

    use super::*;

    fn l_shape() -> World {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/snapshots/worlds/flowgraph/l_shape_6_aisles.json");
        site_model::load(&path).expect("the L-Shape fixture loads").1
    }

    /// The made L-Shape carries section 6.1's flow: 5 nodes, 8 edges, 13 segments and 1 return rule, every one kept
    /// whole and in identity order.
    #[test]
    fn the_made_l_shape_reads_as_its_flow_entities() {
        let flows = read(&l_shape());
        let count = |component: &str| flows.entities.iter().filter(|e| e.components.contains_key(component)).count();
        let census = ["flow.node", "flow.material_edge", "flow.agent_segment", "flow.return_rule"].map(count);
        assert_eq!(census, [5, 8, 13, 1]);
        assert_eq!(flows.entities.len(), 27);
        assert!(flows.entities.windows(2).all(|pair| pair[0].id < pair[1].id), "ordered by identity");
        let node = flows.entities.iter().find(|e| e.name.as_deref() == Some("Pick stations")).unwrap();
        assert_eq!(node.components["flow.node"]["work"]["pick_sim_ms"], 7000);
        assert!(node.components.contains_key("external_ids"), "the template's source key rides along");
        assert_eq!(flows.get(&node.id), Some(node));
    }

    /// The protocol's own `flow` component, traffic direction on a cell grid, is map data: an entity carrying it is
    /// not a flow entity.
    #[test]
    fn traffic_direction_is_not_a_flow_entity() {
        let mut world = l_shape();
        let mut lane = Entity::new("5d1e0c3a-8f4b-4e2a-9c6d-1b7f3e9a2c40".parse::<EntityId>().unwrap());
        lane.components.insert("flow".into(), json!({ "direction": "+x" }));
        world.entities.insert(lane.id.clone(), lane.clone());
        let flows = read(&world);
        assert_eq!(flows.entities.len(), 27);
        assert!(flows.get(&lane.id).is_none());
    }
}
