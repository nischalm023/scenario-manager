//! Within `finality`, `physical_inventory` reconstructs physical storage and
//! relay occupancy from timed location and content changes, and stamps each
//! slice's stock with them as the slice is made final. Facts report changes
//! out of simulation-time order, so the replay holds each change until the
//! slice it falls in is made final, applies the slice's changes in
//! simulation-time order, and lets each go once applied.

use std::collections::BTreeMap;

use rustc_hash::FxHashSet;

use crate::model::{RackId, SimDuration, SimTime, ToteLocation};
use crate::observer::state::SimObserverState;
use crate::observer::time_slice::{MetricsConfig, ToteVisualState};

/// Several racks may share a drawing cell. Only legacy streams without
/// inventory custody use that cell as their storage-slot identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::observer) enum OccupancySlot {
    Rack(RackId, u16),
    VisualCell(u32, u16),
}

impl OccupancySlot {
    pub(in crate::observer) fn zone(self, config: &MetricsConfig) -> Option<u32> {
        match self {
            Self::Rack(rack, _) => config.rack_id_zone.get(&rack.0).copied(),
            Self::VisualCell(cell, _) => config.rack_cell_zone.get(&cell).copied(),
        }
    }
}

pub(in crate::observer) enum PhysicalChange {
    Location(u32, ToteVisualState),
    Custody(u32, ToteLocation),
    Contents(u32, u64),
    InventoryDelta(u32, i32),
}

impl PhysicalChange {
    fn tote(&self) -> u32 {
        match self {
            Self::Location(tote, _)
            | Self::Custody(tote, _)
            | Self::Contents(tote, _)
            | Self::InventoryDelta(tote, _) => *tote,
        }
    }
}

/// The recorded changes the replay has not applied yet, and where it stands.
#[derive(Default)]
pub(in crate::observer) struct PhysicalReplay {
    /// How many leading slices it has stamped.
    stamped: usize,
    /// The changes not yet applied, by instant, each instant's in the order
    /// they were recorded.
    // ordered: each slice applies the changes before its end, earliest first.
    pending: BTreeMap<SimTime, Vec<PhysicalChange>>,
    /// Totes whose payload an explicit contents fact has stated: after one,
    /// stock deltas no longer change the tote's payload.
    explicit_contents: FxHashSet<u32>,
}

impl SimObserverState {
    /// Record a tote change at `at`, for the replay to apply when the slice it
    /// falls in is made final, and make that slice exist. A change in a slice
    /// B has passed is a write into it.
    pub(in crate::observer) fn record_physical_change(&mut self, at: SimTime, change: PhysicalChange) {
        let slice = (at.as_ms() / self.config.slice_width_ms) as usize;
        self.note_write_into(slice);
        self.extend_slices_through(slice);
        self.physical_replay.pending.entry(at).or_default().push(change);
    }

    /// Stamp slice `slice`, the next one the replay has not stamped, with its
    /// stock: apply the pending changes before the slice's end, or, in the
    /// run's last slice, which its `stop` cuts or ends, up to and including
    /// the stop, in simulation-time order, every change at one instant before
    /// the instant's snapshot is taken. A change past the stop stays pending
    /// and is never applied.
    pub(super) fn stamp_physical_replay(&mut self, slice: usize, stop: Option<SimTime>) {
        debug_assert_eq!(slice, self.physical_replay.stamped, "the replay stamps slices in order, once each");
        let width = self.config.slice_width();
        let start = SimTime::ZERO + width * slice as u64;
        let end = start + width;
        let due_by = |at: SimTime| match stop {
            Some(stop) if stop <= end => at <= stop,
            _ => at < end,
        };
        let data = self.slice_to_finalize(slice);
        data.scalars.rack.occupancy_peak = 0;
        data.tote_state.contents.clear();
        if self.physical_replay.pending.first_key_value().is_none_or(|(&at, _)| at != start) {
            self.stamp_rack_fill(slice);
        }
        while let Some(due) = self.physical_replay.pending.first_entry()
            && due_by(*due.key())
        {
            let (at, changes) = due.remove_entry();
            let mut affected = FxHashSet::default();
            for change in changes {
                self.apply_physical_change(slice, at - start, change, &mut affected);
            }
            self.stamp_rack_fill(slice);
            let mut affected: Vec<_> = affected.into_iter().collect();
            affected.sort_unstable();
            for zone in affected {
                self.stamp_aisle_state(at.as_ms(), zone);
            }
        }
        self.slice_to_finalize(slice).scalars.inventory_units = self.inventory_units;
        self.physical_replay.stamped = slice + 1;
    }

    /// Apply one change, `offset` into slice `slice`, adding the aisles it
    /// moved to `affected`.
    fn apply_physical_change(
        &mut self,
        slice: usize,
        offset: SimDuration,
        change: PhysicalChange,
        affected: &mut FxHashSet<u32>,
    ) {
        let tote = change.tote();
        let old = self.tote_fill_slot(tote);
        let old_storage = self.tote_storage_slot(tote);
        let old_relay = Self::relay_cell(self.tote_states.get(&tote));
        let mut contents_changed = false;
        match change {
            PhysicalChange::Location(_, state) => {
                self.tote_states.insert(tote, state);
            }
            PhysicalChange::Custody(_, location) => {
                self.tote_custody.insert(tote, location);
            }
            PhysicalChange::Contents(_, units) => {
                self.physical_replay.explicit_contents.insert(tote);
                self.tote_units.insert(tote, i64::try_from(units).expect("physical tote contents fit i64"));
                contents_changed = true;
            }
            PhysicalChange::InventoryDelta(_, delta) => {
                self.inventory_units = self.inventory_units.saturating_add_signed(i64::from(delta));
                // After the first explicit payload, stock deltas cannot
                // mutate it. Earlier legacy seeding remains available.
                if !self.physical_replay.explicit_contents.contains(&tote) {
                    *self.tote_units.entry(tote).or_default() += i64::from(delta);
                    contents_changed = true;
                }
            }
        }
        if contents_changed {
            let units = self.tote_units[&tote].max(0) as u64;
            self.slice_to_finalize(slice).tote_state.contents.push((offset.as_ms(), tote, units));
        }
        let new = self.tote_fill_slot(tote);
        let new_storage = self.tote_storage_slot(tote);
        let new_relay = Self::relay_cell(self.tote_states.get(&tote));
        self.move_fill(old, new);
        affected.extend(self.move_storage_state(old_storage, new_storage));
        affected.extend(self.move_relay_tote(old_relay, new_relay));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::events::{
        InventoryMoveEvent, SimEvent, ToteContentsChangedEvent, ToteInitialStatesEvent, ToteMovedEvent,
        ToteTransitEvent,
    };
    use crate::model::{RobotId, SimTime, ToteId};

    #[test]
    fn shared_drawing_cell_keeps_distinct_rack_slots_and_reserved_custody() {
        let mut state = SimObserverState::new(MetricsConfig {
            slice_width_ms: 100,
            total_rack_slots: 2,
            rack_cell_zone: Arc::new([(10, 0)].into_iter().collect()),
            rack_id_zone: Arc::new([(40, 0), (41, 0)].into_iter().collect()),
            zone_capacity: Arc::new([(0, 2)].into_iter().collect()),
            ..MetricsConfig::default()
        });
        let rack = |id| ToteLocation::InRack { rack_id: RackId(id), z_index: 0 };
        let reserved = ToteLocation::Reserved { rack_id: RackId(40), z_index: 0 };
        let carried = ToteLocation::WithRobot { robot_id: RobotId(7) };
        let custody = |tote, from, to, at| {
            SimEvent::InventoryMove(InventoryMoveEvent {
                tote_id: ToteId(tote),
                from,
                to,
                // Order-container moves intentionally do not carry stock flux.
                contents: Vec::new(),
                at: SimTime::ms(at),
            })
        };
        let contents = |tote, units, at| {
            SimEvent::ToteContentsChanged(ToteContentsChangedEvent {
                tote_id: ToteId(tote),
                units,
                at: SimTime::ms(at),
            })
        };
        state.dispatch_event(&SimEvent::ToteInitialStates(ToteInitialStatesEvent {
            totes: vec![
                (0, ToteVisualState::InRack { cell_id: 10, level: 0 }),
                (1, ToteVisualState::InRack { cell_id: 10, level: 0 }),
            ],
            order_totes: vec![0, 1],
        }));
        for tote in 0..2 {
            state.dispatch_event(&custody(tote, ToteLocation::Uninitialized, rack(40 + tote), 0));
            state.dispatch_event(&contents(tote, 3, 0));
        }
        // Future-stamped sibling work arrives before this tote's reservation.
        state.dispatch_event(&custody(0, reserved, carried, 200));
        state.dispatch_event(&custody(0, rack(40), reserved, 100));
        state.dispatch_event(&contents(0, 4, 150));
        state.dispatch_event(&SimEvent::ToteMoved(ToteMovedEvent {
            tote_id: ToteId(0),
            new_state: ToteVisualState::OnBot { bot_id: 7 },
            time: SimTime::ms(100),
        }));
        state.dispatch_event(&SimEvent::ToteMoved(ToteMovedEvent {
            tote_id: ToteId(0),
            new_state: ToteVisualState::InRack { cell_id: 10, level: 0 },
            time: SimTime::ms(200),
        }));
        state.dispatch_event(&contents(1, 0, 300));
        stamp_every_slice(&mut state);

        for (slice, filled) in [2, 2, 1, 0].into_iter().enumerate() {
            let rack = &state.slices()[slice].scalars.rack;
            assert_eq!(rack.occupancy_total, filled, "global fill at slice {slice}");
            assert_eq!(rack.per_zone_occupancy.get(&0), Some(&filled), "aisle fill at slice {slice}");
            assert_eq!(rack.occupancy_peak, filled, "same-instant changes settle before the peak");
        }
        for (at, occupied, non_empty, empty) in [(0, 2, 2, 0), (100, 2, 2, 0), (200, 1, 1, 0), (300, 1, 0, 1)] {
            let record = state.output.aisle_state_log.iter().rfind(|row| row.sim_time_ms <= at).unwrap();
            assert_eq!(record.storage_occupied, occupied, "occupied at {at}");
            assert_eq!(record.storage_non_empty, non_empty, "non-empty at {at}");
            assert_eq!(record.storage_empty, empty, "empty at {at}");
        }
        let stock: Vec<u64> = state.slices().iter().map(|slice| slice.scalars.inventory_units).collect();
        assert!(stock.iter().all(|units| *units == 0), "order payloads are not warehouse stock: {stock:?}");
    }

    /// A rack aisle of one slot, holding tote 0 with three units from zero.
    fn one_slot_rack() -> SimObserverState {
        let mut state = SimObserverState::new(MetricsConfig {
            slice_width_ms: 100,
            total_rack_slots: 1,
            rack_cell_zone: Arc::new([(10, 0)].into_iter().collect()),
            zone_capacity: Arc::new([(0, 1)].into_iter().collect()),
            ..MetricsConfig::default()
        });
        state.dispatch_event(&SimEvent::ToteInitialStates(ToteInitialStatesEvent {
            totes: vec![(0, ToteVisualState::InRack { cell_id: 10, level: 0 })],
            order_totes: vec![0],
        }));
        state.dispatch_event(&SimEvent::ToteContentsChanged(ToteContentsChangedEvent {
            tote_id: ToteId(0),
            units: 3,
            at: SimTime::ZERO,
        }));
        state
    }

    fn moved(state: ToteVisualState, at: u64) -> SimEvent {
        SimEvent::ToteMoved(ToteMovedEvent { tote_id: ToteId(0), new_state: state, time: SimTime::ms(at) })
    }

    fn stamp_every_slice(state: &mut SimObserverState) {
        for slice in state.physical_replay.stamped..state.slices().len() {
            state.stamp_physical_replay(slice, None);
        }
    }

    /// Facts read out of simulation-time order, ahead of the slices they fall
    /// in: the replay stamps each slice once, as it is made final, with what
    /// one pass over the finished run stamps.
    #[test]
    fn stamping_a_slice_at_a_time_matches_stamping_the_whole_run() {
        let facts = [
            moved(ToteVisualState::InRack { cell_id: 10, level: 0 }, 200),
            moved(ToteVisualState::OnBot { bot_id: 7 }, 100),
            moved(ToteVisualState::OnBot { bot_id: 7 }, 350),
        ];
        let mut stepwise = one_slot_rack();
        stepwise.dispatch_event(&facts[0]);
        stepwise.dispatch_event(&facts[1]);
        stepwise.stamp_physical_replay(0, None);
        stepwise.dispatch_event(&facts[2]);
        stamp_every_slice(&mut stepwise);
        let mut whole = one_slot_rack();
        for fact in &facts {
            whole.dispatch_event(fact);
        }
        stamp_every_slice(&mut whole);

        let fill = |state: &SimObserverState| -> Vec<(u32, u32)> {
            state.slices().iter().map(|s| (s.scalars.rack.occupancy_total, s.scalars.rack.occupancy_peak)).collect()
        };
        assert_eq!(fill(&stepwise), [(1, 1), (0, 0), (1, 1), (0, 1)]);
        assert_eq!(fill(&stepwise), fill(&whole), "the departure at a slice's start has no duration in it");
        assert_eq!(stepwise.output.aisle_state_log, whole.output.aisle_state_log);
        let stock: Vec<u64> = stepwise.slices().iter().map(|slice| slice.scalars.inventory_units).collect();
        assert!(stock.iter().all(|units| *units == 0), "order payloads are not warehouse stock: {stock:?}");
    }

    /// The slice the run's stop cuts states the stock as the stop left it: a
    /// tote stored before the stop and taken out after it, inside that one
    /// slice, is still in the rack, and no aisle row is filed for its leaving.
    #[test]
    fn the_slice_the_stop_cuts_states_the_stock_at_the_stop() {
        let mut state = one_slot_rack();
        state.dispatch_event(&moved(ToteVisualState::InRack { cell_id: 10, level: 0 }, 120));
        state.dispatch_event(&moved(ToteVisualState::OnBot { bot_id: 7 }, 180));
        state.stamp_physical_replay(0, None);
        state.stamp_physical_replay(1, Some(SimTime::ms(150)));
        let rack = &state.slices()[1].scalars.rack;
        assert_eq!((rack.occupancy_total, rack.occupancy_peak), (1, 1), "the tote stands in the rack at the stop");
        let filed: Vec<u64> = state.output.aisle_state_log.iter().map(|row| row.sim_time_ms).collect();
        assert!(filed.iter().all(|at| *at <= 150), "aisle rows filed by the stop alone: {filed:?}");
    }

    /// The slice the run's stop ends states the stock as the stop left it: a
    /// tote taken out of the rack at the stop, the slice's end, is out of it.
    #[test]
    fn the_slice_the_stop_ends_states_the_stock_at_the_stop() {
        let mut state = one_slot_rack();
        state.dispatch_event(&moved(ToteVisualState::OnBot { bot_id: 7 }, 100));
        state.stamp_physical_replay(0, Some(SimTime::ms(100)));
        let rack = &state.slices()[0].scalars.rack;
        assert_eq!((rack.occupancy_total, rack.occupancy_peak), (0, 1), "the rack is empty at the stop");
    }

    /// A change recorded in a slice B has passed is a write into it, noted as
    /// it is recorded, so the fact that reported it is the one named.
    #[test]
    fn a_change_recorded_in_a_slice_b_has_passed_is_a_write_into_it() {
        let mut state = one_slot_rack();
        state.settle_activity(SimTime::ms(250)).unwrap();
        let carried = || PhysicalChange::Location(0, ToteVisualState::OnBot { bot_id: 7 });
        state.record_physical_change(SimTime::ms(150), carried());
        assert_eq!(state.take_late_write(), Some(SimTime::ms(100)), "the change falls in slice 1, which B has passed");
        state.record_physical_change(SimTime::ms(250), carried());
        assert_eq!(state.take_late_write(), None, "B stands in slice 2");
    }

    #[test]
    fn a_transit_is_filed_where_it_lands_and_states_the_whole_journey() {
        use desgraph::CellId;
        let mut state = SimObserverState::new(MetricsConfig { slice_width_ms: 100, ..Default::default() });
        state.dispatch_event(&SimEvent::ToteTransit(ToteTransitEvent {
            tote_id: ToteId(42),
            from_cell: CellId(5),
            to_cell: CellId(6),
            start: SimTime::ms(75),
            end: SimTime::ms(300),
        }));
        state.extend_slices_through(3);
        let records = &state.slices()[2].tote_state.transits;
        assert_eq!(records.len(), 1, "the journey is filed in the slice holding its last travelling moment");
        assert_eq!((records[0].start_ms, records[0].end_ms), (75, 300));
        assert_eq!((records[0].from_cell, records[0].to_cell), (5, 6));
        assert_eq!(records[0].tote_id, 42);
        for slice in [0, 1] {
            assert!(
                state.slices()[slice].tote_state.transits.is_empty(),
                "slice {slice} closed while the tote was still travelling"
            );
        }
        assert!(state.slices()[3].tote_state.transits.is_empty(), "arrival belongs to the next static state");
    }
}
