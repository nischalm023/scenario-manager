//! Within `flowgraph`, `roles` names what each node is for and which registered root serves each material edge
//! (spec 06 section 3.2, root inference): the patterns today's roots execute and nothing else, so an edge no root
//! serves is refused by name rather than run as something else (lean 10).

use super::components::{Demand, Holds, Moves, NodeKind, Operation};
use super::graph::{Edge, Graph, Node};

/// The roots root inference may emit, in the table's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Root {
    InitializeFleet,
    FillInventory,
    LoadOrders,
    FleetConservation,
    FulfillOrders,
    PutawaySupplyDemand,
    PutawayInduction,
    PickDemand,
}

impl Root {
    pub const ORDER: [Root; 8] = [
        Root::InitializeFleet,
        Root::FillInventory,
        Root::LoadOrders,
        Root::FleetConservation,
        Root::FulfillOrders,
        Root::PutawaySupplyDemand,
        Root::PutawayInduction,
        Root::PickDemand,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Root::InitializeFleet => "InitializeFleet",
            Root::FillInventory => "FillInventory",
            Root::LoadOrders => "LoadOrders",
            Root::FleetConservation => "FleetConservation",
            Root::FulfillOrders => "FulfillOrders",
            Root::PutawaySupplyDemand => "PutawaySupplyDemand",
            Root::PutawayInduction => "PutawayInduction",
            Root::PickDemand => "PickDemand",
        }
    }

    /// The roots each waits for, as `compositions/cencora.json` and `walmart.json` list them.
    pub fn after(self) -> &'static [Root] {
        match self {
            Root::InitializeFleet => &[],
            Root::FillInventory => &[Root::InitializeFleet],
            Root::LoadOrders | Root::FleetConservation => &[Root::FillInventory],
            _ => &[Root::LoadOrders],
        }
    }
}

/// What serves an edge: a root, an abstract completion with no transport (an order leaving in its bin, spec 06
/// section 6.1), the clearance each finished presentation spawns (`ClearPresentation`: a tote's return from its
/// station into storage, which no root runs), or nothing registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Served {
    Root(Root),
    Completion,
    Clearance,
    Nothing,
}

pub fn operation(node: &Node) -> Option<Operation> {
    node.spec.operation()
}

pub fn is_stations(node: &Node, op: Operation) -> bool {
    node.spec.kind == NodeKind::Stations && operation(node) == Some(op)
}

/// A pick conveyance node: a group of belt machines picking into order containers, which FulfillOrders runs.
pub fn is_pick_loop(node: &Node) -> bool {
    node.spec.kind == NodeKind::Conveyance && operation(node) == Some(Operation::Pick)
}

pub fn is_pick(node: &Node) -> bool {
    operation(node) == Some(Operation::Pick)
}

pub fn is_storage(node: &Node, holds: Holds) -> bool {
    node.spec.kind == NodeKind::Storage && node.spec.holds == Some(holds)
}

pub fn is_putaway_source(node: &Node) -> bool {
    node.spec.kind == NodeKind::Source && node.spec.demand == Some(Demand::Putaway)
}

/// Which root serves `edge`, by the shape of its two ends and what it moves.
pub fn served_by(graph: &Graph, edge: &Edge) -> Served {
    let Some((from, to)) = graph.ends(edge) else { return Served::Nothing };
    let put = |node| is_stations(node, Operation::Put);
    let inventory = |node| is_storage(node, Holds::InventoryTote);
    let root = match edge.spec.moves {
        Moves::Units if is_putaway_source(from) && put(to) => Root::PutawayInduction,
        Moves::EmptyTote if put(to) && (inventory(from) || is_pick(from) && edge.spec.alternative_to.is_some()) => {
            Root::PutawaySupplyDemand
        }
        Moves::StockTote if inventory(from) && is_stations(to, Operation::Pick) => Root::PickDemand,
        moves if moves.is_inventory_tote() && from.spec.kind == NodeKind::Stations && inventory(to) => {
            return Served::Clearance;
        }
        Moves::StockTote if inventory(from) && is_pick_loop(to) => Root::FulfillOrders,
        Moves::ToteAfterPick if is_pick_loop(from) && inventory(to) => Root::FulfillOrders,
        Moves::EmptyOrderContainer if is_storage(from, Holds::OrderContainer) && is_pick_loop(to) => {
            Root::FulfillOrders
        }
        Moves::NewOrderContainer if is_stations(from, Operation::CreateContainer) && is_pick_loop(to) => {
            Root::FulfillOrders
        }
        Moves::ClosedOrder
            if (is_pick_loop(from) || is_storage(from, Holds::ClosedOrder))
                && (is_storage(to, Holds::ClosedOrder) || is_stations(to, Operation::Extract)) =>
        {
            Root::FulfillOrders
        }
        Moves::EmptyOrderContainer
            if is_stations(from, Operation::Extract) && is_storage(to, Holds::OrderContainer) =>
        {
            Root::FulfillOrders
        }
        Moves::OrderInBin if is_stations(from, Operation::Pick) && to.spec.kind == NodeKind::Sink => {
            return Served::Completion;
        }
        _ => return Served::Nothing,
    };
    Served::Root(root)
}
