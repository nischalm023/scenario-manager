//! Record order progress at committed simulation timestamps. Each order is
//! identified by its fixed presentation; physical residency is independent
//! of logical assignment and ends only when its outbound route clears.

use desgraph::{CellId, StationId};
use desmap::OrderHtmArea;

use crate::error::PlanError;
use crate::events::{
    EventJournal, OrderStationPhase, OrderStationPhaseEvent, PresentationReadinessEvent, PresentationStream,
};
use crate::flows::transport::windows::RouteWindow;
use crate::model::conveyor::ConveyorId;
use crate::model::{OrderId, SimTime};
use crate::pathfinding::{Path, WaypointAction};

#[derive(Clone)]
pub(super) struct OrderProgress {
    order_id: OrderId,
    conveyor_id: ConveyorId,
    presentation_id: StationId,
    waiting_cell: Option<CellId>,
    events: EventJournal,
}

impl OrderProgress {
    pub fn new(
        order_id: OrderId,
        conveyor_id: ConveyorId,
        presentation_id: StationId,
        waiting_cell: Option<CellId>,
        events: EventJournal,
    ) -> Self {
        Self { order_id, conveyor_id, presentation_id, waiting_cell, events }
    }

    pub fn phase(&self, phase: OrderStationPhase, at: SimTime) {
        self.events.emit(OrderStationPhaseEvent {
            order_id: self.order_id,
            conveyor_id: self.conveyor_id,
            presentation_id: self.presentation_id,
            waiting_cell: self.waiting_cell,
            at,
            phase,
        });
    }

    pub fn set_waiting_cell(&mut self, cell: CellId) {
        self.waiting_cell = Some(cell);
    }

    pub fn dispatched(&self, path: &Path) {
        if let Some(at) = path.waypoints.iter().find_map(|waypoint| match waypoint.action {
            WaypointAction::Move { start, .. } => Some(start),
            _ => None,
        }) {
            self.phase(OrderStationPhase::Dispatched, at);
        }
    }

    pub fn readiness(&self, pick_index: u32, stream: PresentationStream, ready_at: SimTime) {
        self.events.emit(PresentationReadinessEvent {
            order_id: self.order_id,
            pick_index,
            conveyor_id: self.conveyor_id,
            presentation_id: self.presentation_id,
            stream,
            ready_at,
        });
    }
}

/// One route's single crossing of an order area's boundary, watched window
/// by window as the route commits, so the crossing is acted on as the window
/// that makes it commits, never after the barrier has passed it.
pub(super) struct BoundaryCrossing {
    area: OrderHtmArea,
    entering: bool,
    crossed: bool,
}

impl BoundaryCrossing {
    pub(super) fn entry(area: OrderHtmArea) -> Self {
        Self { area, entering: true, crossed: false }
    }

    pub(super) fn exit(area: OrderHtmArea) -> Self {
        Self { area, entering: false, crossed: false }
    }

    /// The crossing's time when `window` makes it. An error when the route
    /// crosses twice, or its last window commits without a crossing.
    pub(super) fn in_window(&mut self, window: &Path, which: RouteWindow) -> Result<Option<SimTime>, PlanError> {
        let crossing = crossing_time(window, &self.area, self.entering)?;
        if crossing.is_some() && self.crossed {
            return Err(super::invalid("order area route crosses its boundary in the wrong direction or twice"));
        }
        self.crossed |= crossing.is_some();
        if which == RouteWindow::Last && !self.crossed {
            return Err(super::invalid("committed order route did not cross its area boundary"));
        }
        Ok(crossing)
    }
}

fn crossing_time(path: &Path, area: &OrderHtmArea, entering: bool) -> Result<Option<SimTime>, PlanError> {
    let mut crossing = None;
    for pair in path.waypoints.windows(2) {
        let from_inside = area.resident_cells.contains(&pair[0].cell);
        let to_inside = area.resident_cells.contains(&pair[1].cell);
        if from_inside == to_inside {
            continue;
        }
        let WaypointAction::Move { start, end, .. } = pair[1].action else {
            return Err(super::invalid("order area boundary crossed without a movement"));
        };
        if to_inside != entering || crossing.is_some() {
            return Err(super::invalid("order area route crosses its boundary in the wrong direction or twice"));
        }
        let doorway = if entering { pair[0].cell } else { pair[1].cell };
        if doorway != if entering { area.entry } else { area.egress } {
            return Err(super::invalid("order area route bypasses its declared doorway"));
        }
        // Count entry from the start of crossing, exit only once the HTM
        // has finished crossing. Neither timestamp is the final pick dwell.
        crossing = Some(if entering { start } else { end });
    }
    Ok(crossing)
}

#[cfg(test)]
#[path = "progress_tests.rs"]
mod tests;
