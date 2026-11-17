//! Reconstruct index initialization from authoritative tote histories. Sequence
//! stamps merge content and location writes in commit order, including equal
//! timestamps and out-of-time-order planning; no second mutation log is kept.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::InventoryService;
use super::error::InventoryError;
use super::events::{InventoryEvent, InventoryObserver, InventoryObserverError};
use super::types::{ContentSlice, LocationSlice};
use crate::model::{SimTime, SkuId, ToteLocation};

pub(crate) struct SnapshotEvent {
    pub event: InventoryEvent,
    /// Contents immediately after this write, not the later state at the same
    /// simulation time that a normal live inventory query would now return.
    pub contents: Vec<(SkuId, u32)>,
}

#[derive(Clone, Copy)]
enum Write<'a> {
    Location(&'a LocationSlice),
    Contents(&'a ContentSlice),
}

impl Write<'_> {
    fn sequence(&self) -> u64 {
        match self {
            Self::Location(slice) => slice.sequence,
            Self::Contents(slice) => slice.sequence,
        }
    }
}

impl InventoryService {
    pub(crate) fn add_observer_with_snapshot(
        &self,
        observer: Arc<dyn InventoryObserver>,
        initialize: impl FnOnce(&[SnapshotEvent]) -> Result<(), InventoryObserverError>,
    ) -> Result<Arc<dyn InventoryObserver>, InventoryError> {
        let _mutation = self.0.mutation_sequence.lock().unwrap();
        let snapshot = self.snapshot_events();
        initialize(&snapshot)?;
        self.0.observers.write().unwrap().push(Arc::downgrade(&observer));
        Ok(observer)
    }

    fn snapshot_events(&self) -> Vec<SnapshotEvent> {
        let totes = self.0.totes.read().unwrap();
        let mut events = Vec::new();
        for (&tote_id, data) in &totes.data {
            let mut writes: Vec<_> = data
                .location_log
                .iter()
                .map(Write::Location)
                .chain(data.content_log.iter().filter(|slice| slice.changed_sku.is_some()).map(Write::Contents))
                .collect();
            writes.sort_by_key(Write::sequence);
            // ordered: the predecessor at each simulation time is queried as
            // commit order reconstructs the timeline incrementally.
            let mut locations: BTreeMap<SimTime, ToteLocation> = BTreeMap::new();
            // ordered: same temporal predecessor lookup for contents.
            let mut contents: BTreeMap<SimTime, &[(SkuId, u32)]> = BTreeMap::new();
            for write in writes {
                let (event, after) = match write {
                    Write::Location(slice) => {
                        let old_location = locations
                            .range(..=slice.from)
                            .next_back()
                            .map_or(ToteLocation::Uninitialized, |(_, &location)| location);
                        let held = contents.range(..=slice.from).next_back().map_or(&[][..], |(_, &items)| items);
                        locations.insert(slice.from, slice.location);
                        (
                            InventoryEvent::LocationChanged {
                                tote_id,
                                designation: data.designation,
                                old_location,
                                new_location: slice.location,
                                is_empty: held.is_empty(),
                                at: slice.from,
                            },
                            held.to_vec(),
                        )
                    }
                    Write::Contents(slice) => {
                        let sku = slice.changed_sku.expect("initial empty content slice was excluded");
                        let before = contents.range(..=slice.from).next_back().map_or(&[][..], |(_, &items)| items);
                        let old_qty = before.iter().find(|(item, _)| *item == sku).map_or(0, |(_, qty)| *qty);
                        let new_qty = slice.contents.iter().find(|(item, _)| *item == sku).map_or(0, |(_, qty)| *qty);
                        let event = InventoryEvent::ContentsChanged {
                            tote_id,
                            designation: data.designation,
                            sku,
                            delta: (new_qty as i64 - old_qty as i64) as i32,
                            was_empty: before.is_empty(),
                            is_empty: slice.contents.is_empty(),
                            location: locations
                                .range(..=slice.from)
                                .next_back()
                                .map_or(ToteLocation::Uninitialized, |(_, &location)| location),
                            at: slice.from,
                        };
                        contents.insert(slice.from, &slice.contents);
                        (event, slice.contents.clone())
                    }
                };
                events.push((write.sequence(), SnapshotEvent { event, contents: after }));
            }
        }
        events.sort_by_key(|(sequence, _)| *sequence);
        events.into_iter().map(|(_, event)| event).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Barrier, Mutex};
    use std::thread;

    use crate::model::{RackId, ToteDesignation, ToteId, ToteType};

    #[derive(Default)]
    struct Recorder(Mutex<Vec<InventoryEvent>>);

    impl InventoryObserver for Recorder {
        fn on_inventory_event(&self, event: &InventoryEvent) -> Result<(), InventoryObserverError> {
            self.0.lock().unwrap().push(event.clone());
            Ok(())
        }
    }

    #[test]
    fn subscription_has_no_gap_or_duplicate_while_inventory_changes() {
        let inventory = InventoryService::new();
        inventory
            .register_tote(
                ToteId(1),
                ToteType::Tote,
                ToteDesignation::Inventory,
                ToteLocation::InRack { rack_id: RackId(1), z_index: 0 },
            )
            .unwrap();
        let rendezvous = Arc::new(Barrier::new(2));
        let worker_inventory = inventory.clone();
        let worker_barrier = rendezvous.clone();
        let worker = thread::spawn(move || {
            worker_barrier.wait();
            for at in 1..=100 {
                worker_inventory.tote(ToteId(1)).unwrap().add_item(SkuId(7), 1, at).unwrap();
            }
        });
        let recorder = Arc::new(Recorder::default());
        let _observer = inventory
            .add_observer_with_snapshot(recorder.clone(), |snapshot| {
                rendezvous.wait();
                for entry in snapshot {
                    recorder.on_inventory_event(&entry.event)?;
                }
                Ok(())
            })
            .unwrap();
        worker.join().unwrap();
        let events = recorder.0.lock().unwrap();
        assert_eq!(events.len(), 101);
        assert!(matches!(events[0], InventoryEvent::LocationChanged { .. }));
        let deltas: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                InventoryEvent::ContentsChanged { delta, at, .. } => Some((*delta, at.as_ms())),
                _ => None,
            })
            .collect();
        assert_eq!(deltas, (1..=100).map(|at| (1, at)).collect::<Vec<_>>());
    }
}
