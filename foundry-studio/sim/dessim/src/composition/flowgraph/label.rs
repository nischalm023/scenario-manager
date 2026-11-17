//! Within `flowgraph`, a `label` is the words a problem uses for a flow entity (spec 06 section 3.1): its `name`
//! where it states one, which every node must, else a label derived from what it is ("Inventory storage to Pick
//! stations: stock tote"; "HTM fleet: relay stand to Pick stations"). Labels never reach the plan.

use rustc_hash::FxHashMap;
use site_model::ids::EntityId;

use super::components::{Executor, Moves, Place, Port};
use super::graph::Graph;
use super::site::Site;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Labels(FxHashMap<EntityId, String>);

impl Labels {
    pub(super) fn build(graph: &Graph, site: &Site) -> Self {
        let mut labels = FxHashMap::default();
        for node in &graph.nodes {
            labels
                .insert(node.id.clone(), node.name.clone().unwrap_or_else(|| format!("the unnamed node {}", node.id)));
        }
        let node = |id: &EntityId| labels.get(id).cloned().unwrap_or_else(|| id.to_string());
        let mut derived = FxHashMap::default();
        for edge in &graph.edges {
            let text =
                || format!("{} to {}: {}", node(&edge.spec.from.id), node(&edge.spec.to.id), moves(edge.spec.moves));
            derived.insert(edge.id.clone(), edge.name.clone().unwrap_or_else(text));
        }
        for segment in &graph.segments {
            let place = |place: &Place| match place.port {
                Port::Relay => "relay stand".to_owned(),
                Port::Slot => "slot".to_owned(),
                Port::Presentation | Port::Boundary => node(&place.node.id),
                Port::Dropoff => format!("{} drop-off", node(&place.node.id)),
                Port::Collect => format!("{} collection", node(&place.node.id)),
                Port::OrderPresentation => format!("{} order presentation", node(&place.node.id)),
            };
            let executor = match &segment.spec.executor {
                Executor::Cohort(cohort) => site.name(&cohort.id).unwrap_or("a cohort").to_owned(),
                Executor::Assigned(_) => "the robots assigned there".to_owned(),
                Executor::Conveyance(belt) => format!("{} belt", node(&belt.id)),
                Executor::External => "outside the site".to_owned(),
            };
            let text = || format!("{executor}: {} to {}", place(&segment.spec.from), place(&segment.spec.to));
            derived.insert(segment.id.clone(), segment.name.clone().unwrap_or_else(text));
        }
        for rule in &graph.rules {
            derived.insert(rule.id.clone(), rule.name.clone().unwrap_or_else(|| "the return rule".to_owned()));
        }
        labels.extend(derived);
        Self(labels)
    }

    /// The entity's label, or its identity where the graph does not hold it.
    pub fn of(&self, id: &EntityId) -> String {
        self.0.get(id).cloned().unwrap_or_else(|| id.to_string())
    }
}

/// What an edge moves, in words.
pub fn moves(moves: Moves) -> &'static str {
    match moves {
        Moves::Units => "units",
        Moves::StockTote => "stock tote",
        Moves::EmptyTote => "empty tote",
        Moves::ToteAfterPick => "tote after the pick",
        Moves::EmptyOrderContainer => "empty order container",
        Moves::NewOrderContainer => "new order container",
        Moves::ClosedOrder => "closed order",
        Moves::OrderInBin => "order in its bin",
    }
}
