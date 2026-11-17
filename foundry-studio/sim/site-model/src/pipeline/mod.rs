//! The pipeline: one total order from a scenario file to a derived world, so
//! there is exactly one answer to what a composition means.
//!
//! SPEC §8 numbers six steps and this module is their sequence, one submodule
//! each — except step 5, which has two halves and two: `graph` compiles the
//! arcs and `bind` gives each relay its cell, both over the same
//! post-suppression cell set. The order is the design, not an implementation
//! detail: merging before expanding is what lets an overlay change a
//! generator's input rather than patch up its output, overriding after
//! expanding is what lets a record reach a cell nobody wrote, and binding after
//! both is what lets a hand-stated binding name a cell a later layer declared.
//! Running the steps in any other order would answer the same question
//! differently.

pub mod apply;
pub mod bind;
pub mod check;
pub mod expand;
pub mod graph;
pub mod layers;
pub mod merge;

use crate::document::Expect;
use crate::entity::Entity;
use crate::error::{LoadError, Warning};
use crate::frames::FrameGraph;
use crate::ids::{DeclaredEntityId, EntityId};
use apply::{Applied, SuppressedTarget};
use graph::{CellGeometry, Edge};
use layers::LayerSet;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Everything a scenario means once it has been read: the entities that exist,
/// the arcs between their cells, and the records that produced them in the
/// resolved form a bake writes back out.
#[derive(Debug, Clone)]
pub struct World {
    /// Declared and derived entities that survived, keyed by identity.
    pub entities: BTreeMap<EntityId, Entity>,
    pub edges: Vec<Edge>,
    pub warnings: Vec<Warning>,
    pub frames: FrameGraph,
    /// The merged declarations — one statement per declared entity, after every
    /// layer has had its say.
    pub declared: BTreeMap<EntityId, Entity>,
    /// One entry per overridden derivation address, components folded in layer
    /// order.
    pub overrides: BTreeMap<EntityId, BTreeMap<String, Value>>,
    pub suppressions: Vec<SuppressedTarget>,
    /// The `expect` pin each override or suppression carried.
    pub expects: BTreeMap<EntityId, Expect>,
    /// Surviving derived entities per declared generator.
    pub counts: BTreeMap<DeclaredEntityId, usize>,
    /// Every cell's centre and extent in the primary frame.
    pub cells: BTreeMap<EntityId, CellGeometry>,
}

impl World {
    pub fn derived_count(&self) -> usize {
        self.entities.values().filter(|e| e.is_derived()).count()
    }

    /// How many entities carry each component name — the shape of the world in
    /// one table.
    pub fn component_census(&self) -> BTreeMap<String, usize> {
        let mut out = BTreeMap::new();
        for entity in self.entities.values() {
            for name in entity.components.keys() {
                *out.entry(name.clone()).or_default() += 1;
            }
        }
        out
    }
}

/// Run all six steps over an already-resolved layer set.
pub fn derive(set: &LayerSet) -> Result<World, LoadError> {
    let merged = merge::merge(set)?;
    let expansion = expand::expand(&merged)?;
    let applied = apply::apply(&merged, &expansion)?;
    let mut world = finish(applied, expansion.frames, &expansion.produced, merged.entities)?;
    world.warnings.extend(check::required_components_gaps(&set.documents().collect::<Vec<_>>()));
    // Last, because a cache can only be checked against a finished world: any
    // document that carries its expansion inline must agree with what the
    // records derive, or the load fails naming where they part.
    crate::expanded::verify(set, &world)?;
    Ok(world)
}

/// Steps 5 and 6 over what steps 1 to 4 left, and the world they make: each
/// cell's geometry, the arcs and the relays' binding, then the checks. Both of
/// this crate's readers end here, so one set of entities makes one world
/// whoever generated it: `derive`, which runs steps 1 to 4 itself, and the
/// flagged export's reader, whose file WPS ran them for. `frames` is the frame
/// graph the entities stand in, `produced` every address a generation produced
/// (what a queue path may name), and `declared` the declarations they came from.
pub fn finish(
    applied: Applied,
    frames: FrameGraph,
    produced: &BTreeSet<EntityId>,
    declared: BTreeMap<EntityId, Entity>,
) -> Result<World, LoadError> {
    let Applied { mut entities, overrides, suppressions, expects, mut warnings } = applied;
    let cells = graph::cell_geometry(&entities, &frames)?;
    let contributions = graph::cell_contributions(&entities, &cells);
    let edges = graph::compile(&entities, &cells, &contributions, &frames)?;
    // Step 5 has two halves and this is the second: the relays bind over the
    // same post-suppression cell set the arcs were compiled from.
    warnings.extend(bind::relays(&mut entities, &cells, &frames)?);
    warnings.extend(check::check(&entities, &frames, &cells, &contributions, produced, &edges)?);
    let mut counts: BTreeMap<DeclaredEntityId, usize> = BTreeMap::new();
    for entity in entities.values().filter(|entity| entity.is_derived()) {
        *counts.entry(entity.id.root()).or_default() += 1;
    }
    Ok(World { entities, edges, warnings, frames, declared, overrides, suppressions, expects, counts, cells })
}
