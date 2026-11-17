//! The station resource family's reservation service.
//!
//! Within `model`, `station_service` owns the station family's keys
//! (station → presentation cell) and answers the family's admissibility
//! and diagnostic questions about presentation-cell occupancy. Per the
//! task-model-v2 temporal model (issue #423), the physical cell row
//! STAYS in the reservation oracle — this service is the family's read
//! surface over that lane (and, as tentative reservations arrive in
//! later cuts, becomes the family's sole authoring path into it, which
//! is what makes the poisoning sweep structurally unskippable).
//!
//! Distinct from [`StationSet`](super::station::StationSet), which holds
//! demand-side dispatch bookkeeping (pipeline depth, operator state):
//! this service is about the CELL as a contended resource.

use std::collections::HashMap;
use std::sync::Arc;

use desgraph::{CellId, StationId};

use crate::model::RobotId;
use crate::model::reservation::cell_reservation::CellReservation;
use crate::model::reservation::oracle::{CellStatus, ReservationOracle};
use crate::model::{SimDuration, SimTime};
use crate::signals::Version;

/// Whether a robot may schedule an arrival at a station's presentation
/// cell right now — the family's reading of the oracle's open-ended-hold
/// state.
pub enum PresentationAdmissibility {
    /// No open-ended hold on the cell: path scheduling decides.
    Schedulable,
    /// Held open-ended by the asking robot itself (its own predecessor
    /// hold) — it may proceed against it.
    HeldBySelf,
    /// Held open-ended by another robot: the incumbent's continuation
    /// hasn't been scheduled, so an arrival cannot be planned yet.
    HeldByOther(RobotId),
}

pub struct StationService {
    /// StationId → presentation cell (where HTMs deliver totes).
    presentation: HashMap<StationId, CellId>,
    oracle: Arc<dyn ReservationOracle>,
    version: Version,
}

impl StationService {
    pub fn new(presentation: HashMap<StationId, CellId>, oracle: Arc<dyn ReservationOracle>) -> Self {
        Self { presentation, oracle, version: Version::new() }
    }

    pub fn version(&self) -> &Version {
        &self.version
    }

    /// The family key lookup: a station's presentation cell.
    pub fn presentation_cell(&self, station: StationId) -> Option<CellId> {
        self.presentation.get(&station).copied()
    }

    /// Reverse key lookup: the station whose presentation cell is `cell`.
    /// At most one station owns a given cell, so map order cannot leak.
    pub fn station_at_cell(&self, cell: CellId) -> Option<StationId> {
        self.presentation.iter().find_map(|(&station_id, &station_cell)| (station_cell == cell).then_some(station_id))
    }

    /// May `robot` schedule an arrival at `station`'s presentation cell?
    /// One oracle `cell_status` read, classified with the family's
    /// holder rule.
    pub fn presentation_admissibility(&self, station: StationId, robot: RobotId) -> PresentationAdmissibility {
        let Some(cell) = self.presentation_cell(station) else {
            // Unknown station: no cell to hold. Callers that require the
            // key use `presentation_cell` and error there.
            return PresentationAdmissibility::Schedulable;
        };
        match self.oracle.cell_status(cell) {
            CellStatus::Schedulable => PresentationAdmissibility::Schedulable,
            CellStatus::PartiallyPlanned { robot: holder } if holder == robot => PresentationAdmissibility::HeldBySelf,
            CellStatus::PartiallyPlanned { robot: holder } => PresentationAdmissibility::HeldByOther(holder),
        }
    }

    /// Diagnostic: the idle gap a just-committed presentation-cell
    /// reservation leaves behind the latest earlier reservation on the
    /// same cell, plus how many earlier reservations exist. Feeds the
    /// station-presentation gap tripwire (`--tripwires`); `None` when
    /// nothing precedes the new reservation.
    pub fn gap_before(
        &self,
        station: StationId,
        station_reservation: &CellReservation,
    ) -> Option<(SimTime, SimDuration, usize)> {
        let cell = self.presentation_cell(station)?;
        let start = station_reservation.window().start();
        let previous_reservations = self
            .oracle
            .reservations_for_cell(cell)
            .into_iter()
            .filter(|reservation| reservation.id() != station_reservation.id())
            .filter_map(|reservation| reservation.window().end())
            .filter(|end| *end <= start)
            .collect::<Vec<_>>();
        let previous_end = previous_reservations.iter().copied().max()?;

        Some((previous_end, start - previous_end, previous_reservations.len()))
    }
}
