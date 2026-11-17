//! Resolved cells belonging to one conveyor machine. These are immutable
//! equipment facts for tasks that reserve waiting locations and presentations;
//! occupancy, admission and operator timing belong to runtime services.

use serde::{Deserialize, Serialize};

use crate::{CellId, HandoffQueue, StationId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConveyorMachine {
    /// Matches the conveyor_group on every station served by this machine.
    pub conveyor_group: u32,
    /// Declared machine membership, in stable cell order.
    pub cells: Vec<CellId>,
    /// Standing locations explicitly marked waiting, in stable cell order.
    /// Presentation-specific membership is recorded on each presentation; these
    /// are separate from the inventory HTM's one-way holding queue.
    pub waiting_cells: Vec<CellId>,
    pub dropoff_cell: Option<CellId>,
    pub collection_cell: Option<CellId>,
    /// Entry-first holding cells, followed by its named collection terminus.
    /// The terminus and entry_from are not included in path.
    pub inventory_queue: Option<HandoffQueue>,
    pub presentations: Vec<MachinePresentation>,
}

impl ConveyorMachine {
    pub fn inventory_holding_capacity(&self) -> usize {
        self.inventory_queue.as_ref().map_or(0, |queue| queue.path.len())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MachinePresentation {
    pub station_id: StationId,
    /// Conveyor seat occupied by the inventory tote during the paired pick.
    pub inventory_cell: CellId,
    pub cell: CellId,
    /// Waiting berths whose machine-internal directed routes reach this
    /// presentation and return from it without leaving the machine.
    #[serde(default)]
    pub waiting_cells: Vec<CellId>,
    /// The independently admitted order-HTM circulation area served here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_area: Option<OrderHtmArea>,
    /// The directed curved edge into the presentation starts here. This is
    /// the turn's start, not the machine's doorway on the surrounding floor.
    pub approach_cell: Option<CellId>,
    /// The directed curved edge out of the presentation ends here. The order
    /// HTM can then return to a waiting location or leave the machine.
    pub departure_cell: Option<CellId>,
}

/// The dedicated entry and egress holds lie outside the resident circulation
/// area. Inventory HTM routes do not participate in this membership limit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderHtmArea {
    pub entry: CellId,
    pub egress: CellId,
    pub resident_cells: Vec<CellId>,
}
