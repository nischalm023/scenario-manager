//! Cells omitted from navigational waiting statistics. Membership follows the
//! compiled map's operational roles, including queues drawn as Open cells, so
//! the visualizer and offline reports apply the same definition.

use desgraph::CellKind;
use rustc_hash::FxHashMap;

use super::WarehouseMap;

impl WarehouseMap {
    /// Fixed work, holding and equipment areas are not ordinary navigation.
    /// This classifies locations; waiting elsewhere still need not be caused
    /// by traffic contention (it can be idle time or a task/resource delay).
    pub fn navigation_wait_exclusions(&self) -> FxHashMap<u32, &'static str> {
        let mut excluded = FxHashMap::default();
        for cell in self.graph.cells() {
            let reason = match cell.kind {
                CellKind::QueueCell => "queue / waiting cell",
                CellKind::Presentation | CellKind::Exchange | CellKind::Relay => "presentation / handoff cell",
                CellKind::Charging => "charging cell",
                _ => continue,
            };
            excluded.insert(cell.id.0, reason);
        }
        for relay in &self.relays {
            for cell in [relay.position, relay.htm_access, relay.vtm_access.anchor()] {
                excluded.insert(cell.0, "relay presentation / access cell");
            }
        }
        for station in &self.stations {
            for cell in
                [Some(station.position), station.order_presentation_cell, station.dropoff_cell, station.collection_cell]
                    .into_iter()
                    .flatten()
                    .chain(station.presentation_and_queue_cells())
            {
                excluded.insert(cell.0, "station presentation / queue cell");
            }
            for queue in &station.handoff_queues {
                for cell in queue.path.iter().copied().chain(std::iter::once(queue.into)) {
                    excluded.insert(cell.0, "handoff queue / presentation cell");
                }
            }
        }
        for machine in &self.conveyor_machines {
            for cell in machine.cells.iter().chain(&machine.waiting_cells) {
                excluded.insert(cell.0, "conveyor equipment area (including EasyLoop)");
            }
            if let Some(queue) = &machine.inventory_queue {
                for cell in queue.path.iter().copied().chain(std::iter::once(queue.into)) {
                    excluded.insert(cell.0, "inventory HTM holding cell");
                }
            }
        }
        excluded
    }
}
