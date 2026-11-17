//! InventoryService — system of record for totes, items, and spatial stock.
//!
//! All state is temporal. Tote contents and locations are time-sliced step
//! functions: each entry records "state from T onward" until the next entry.
//! Point queries at arbitrary sim time T use binary search.
//!
//! `Tote` is a read/write handle into InventoryService state —
//! constructed on demand, not stored. Mutations append temporal slices
//! (append-only, no back-mutation).
//!
//! The stock index (`sku_totes`) is a "totes that ever contained this SKU"
//! reverse index. Temporal content checks at query time filter to "contains
//! at time T." This avoids maintaining a separate temporal stock ledger.

pub mod error;
pub mod events;
pub mod sku_ledger;
pub(crate) mod snapshot;
pub mod types;

#[cfg(test)]
mod bench_variants;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use rustc_hash::{FxBuildHasher, FxHashMap};

use crate::events::EventSink;
use crate::model::bot::BotService;
use crate::model::location::{ToteDesignation, ToteLocation, ToteType};
use crate::model::{RackId, RobotId, SimTime, SkuId, ToteId};
use crate::signals::Version;

use error::InventoryError;
use events::{InventoryEvent, InventoryObserver};
use sku_ledger::{CountersByMap, SkuLedger};
use types::{Inner, ToteData, ToteState};
pub use types::{SkuAvailability, Tote};

// ---------------------------------------------------------------------------
// InventoryService
// ---------------------------------------------------------------------------

/// System of record for totes, item locations, and spatial stock queries.
///
/// Clone is a cheap Arc bump.
#[derive(Clone)]
pub struct InventoryService(Arc<Inner>);

impl InventoryService {
    /// Build inventory service.
    ///
    /// Totes are nomadic: a tote's location is temporal, resolved from its own
    /// timeline rather than stored as a position here. Spatial *queries* live
    /// in `model::relay::inventory_index`, which is zone-aware and is what the
    /// solver actually asks.
    pub fn new() -> Self {
        Self::with_event_sink(None)
    }

    /// Same as [`new`] but with a `SimEvent` sink attached so every
    /// `ContentsChanged` mutation also fires `SimEvent::InventoryDelta`
    /// on the metrics stream. Uses the same sink as EventJournal so task
    /// events and inventory changes reach the same observer.
    pub fn with_event_sink(event_sink: Option<Arc<dyn EventSink>>) -> Self {
        Self(Arc::new(Inner {
            bot_service: OnceLock::new(),
            mutation_sequence: Mutex::new(0),
            totes: RwLock::new(ToteState { data: BTreeMap::new(), sku_totes: BTreeMap::new() }),
            sku_ledger: RwLock::new(SkuLedger::new()),
            version: Version::new(),
            observers: RwLock::new(Vec::new()),
            event_sink,
        }))
    }

    /// Wire bot service for WithRobot position resolution.
    /// Called once post-construction (circular dependency between services).
    pub fn set_bot_service(&self, bot_service: BotService) {
        let _ = self.0.bot_service.set(bot_service);
    }

    /// Register an observer for inventory mutation events.
    ///
    /// Observers are held by a `Weak` reference; the returned `Arc` is the
    /// only strong handle and **must be retained by the caller** for the
    /// observer to receive events. Returning the input Arc makes that
    /// requirement self-documenting at the call site (typically the field
    /// in the caller's own Inner struct).
    ///
    /// This weak-registry design exists to break Arc cycles for observers
    /// like `RelayInventoryIndex` that hold a back-reference to
    /// `InventoryService`. A strong registry plus a back-reference would
    /// leak the inventory subgraph — and once `event_sink` lives inside
    /// `Inner`, that leak holds an `mpsc::Sender<SimEvent>` clone open and
    /// hangs the observer thread at sim shutdown.
    #[must_use = "InventoryService holds observers weakly; caller must retain the returned Arc"]
    pub fn add_observer(&self, observer: Arc<dyn InventoryObserver>) -> Arc<dyn InventoryObserver> {
        let _mutation = self.0.mutation_sequence.lock().unwrap();
        self.0.observers.write().unwrap().push(Arc::downgrade(&observer));
        observer
    }

    /// Emit an event to all registered observers, and additionally bridge
    /// `ContentsChanged` mutations to the `SimEvent` stream when an
    /// `event_sink` is wired up (see [`with_event_sink`]). The metrics
    /// observer uses this to maintain the live `inventory_units` counter.
    ///
    /// Delegates to `Inner::dispatch_event` so this path stays in lock-step
    /// with `Tote::emit` (the hot path for `ContentsChanged`).
    pub(crate) fn emit(&self, event: InventoryEvent) -> Result<(), InventoryError> {
        Inner::dispatch_event(&self.0, event)
    }

    // -- Tote lifecycle --

    /// Register a tote at init time (T=0).
    pub fn register_tote(
        &self,
        id: ToteId,
        tote_type: ToteType,
        designation: ToteDesignation,
        location: ToteLocation,
    ) -> Result<(), InventoryError> {
        let mut sequence = self.0.mutation_sequence.lock().unwrap();
        let order = *sequence;
        *sequence += 1;
        let mut totes = self.0.totes.write().unwrap();
        totes.data.insert(id, ToteData::new(tote_type, designation, location, SimTime::ZERO, order));
        drop(totes);
        // Emit initial state event. All totes start empty.
        self.emit(InventoryEvent::LocationChanged {
            designation,
            tote_id: id,
            old_location: ToteLocation::Uninitialized,
            new_location: location,
            is_empty: true,
            at: SimTime::ZERO,
        })
    }

    /// Create a new container at its actual birth time. IDs never reuse an
    /// existing container, including one already extracted from the system.
    pub fn create_tote(
        &self,
        tote_type: ToteType,
        designation: ToteDesignation,
        location: ToteLocation,
        at: SimTime,
    ) -> Result<ToteId, InventoryError> {
        let mut sequence = self.0.mutation_sequence.lock().unwrap();
        let mut totes = self.0.totes.write().unwrap();
        let id = match totes.data.last_key_value() {
            Some((&ToteId(id), _)) => ToteId(id.checked_add(1).ok_or(InventoryError::IdExhausted)?),
            None => ToteId(0),
        };
        let order = *sequence;
        *sequence += 1;
        totes.data.insert(id, ToteData::new(tote_type, designation, location, at, order));
        drop(totes);
        self.emit(InventoryEvent::LocationChanged {
            designation,
            tote_id: id,
            old_location: ToteLocation::Uninitialized,
            new_location: location,
            is_empty: true,
            at,
        })?;
        Ok(id)
    }

    /// Get a read/write handle for a specific tote.
    pub fn tote(&self, id: ToteId) -> Result<Tote, InventoryError> {
        let totes = self.0.totes.read().unwrap();
        if totes.data.contains_key(&id) {
            Ok(Tote { id, inner: Arc::clone(&self.0) })
        } else {
            Err(InventoryError::NotFound)
        }
    }

    /// Tote population bucketed by (location variant, empty/full) at `at`,
    /// for the planner's deadlock dump.
    pub fn debug_census(&self, at: SimTime) -> String {
        use crate::model::location::ToteLocation;
        use std::collections::BTreeMap;
        let totes = self.0.totes.read().unwrap();
        let mut buckets: BTreeMap<(&'static str, bool), u32> = BTreeMap::new(); // ordered: stable dump lines
        for record in totes.data.values() {
            let variant = match record.location_at(at) {
                ToteLocation::Uninitialized => "uninitialized",
                ToteLocation::InRack { .. } => "in_rack",
                ToteLocation::Reserved { .. } => "reserved",
                ToteLocation::WithRobot { .. } => "with_robot",
                ToteLocation::AtRelay { .. } => "at_relay",
                ToteLocation::AtCell { .. } => "at_cell",
                ToteLocation::Extracted => "extracted",
                ToteLocation::OnConveyor { .. } | ToteLocation::OnConveyance { .. } => "on_conveyor",
            };
            let empty = record.is_empty_at(at);
            *buckets.entry((variant, empty)).or_insert(0) += 1;
        }
        buckets
            .into_iter()
            .map(|((variant, empty), n)| format!("{variant}/{}={n}", if empty { "empty" } else { "full" }))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Find a free rack slot at time T for a specific tote to return to.
    /// Excludes the tote itself from occupancy check (it's currently traveling).
    /// Returns the first (rack_id, z_index) that no OTHER tote occupies at that time.
    #[cfg_attr(feature = "perf", tracing::instrument(skip_all))]
    pub fn find_free_rack_slot(&self, at: SimTime) -> Option<(RackId, u16)> {
        use crate::model::location::ToteLocation;
        let totes = self.0.totes.read().unwrap();

        let mut occupied: HashSet<(u32, u16)> = HashSet::new();
        let mut all_slots: Vec<(RackId, u16)> = Vec::new();

        for data in totes.data.values() {
            if let ToteLocation::InRack { rack_id, z_index } = data.location_at(SimTime::ZERO) {
                all_slots.push((rack_id, z_index));
            }
            if let ToteLocation::InRack { rack_id, z_index } = data.location_at(at) {
                occupied.insert((rack_id.0, z_index));
            }
        }

        all_slots.into_iter().find(|(rid, z)| !occupied.contains(&(rid.0, *z)))
    }

    /// Find the tote carried by a robot at time T, if any.
    /// Returns (tote_id, contents). O(totes) scan — fine for infrequent calls.
    pub fn tote_on_robot(&self, robot_id: RobotId, at: SimTime) -> Option<(ToteId, Vec<(SkuId, u32)>)> {
        let totes = self.0.totes.read().unwrap();
        for (&tote_id, data) in &totes.data {
            if let ToteLocation::WithRobot { robot_id: rid } = data.location_at(at) {
                if rid == robot_id {
                    let contents = data.contents_at(at).to_vec();
                    return Some((tote_id, contents));
                }
            }
        }
        None
    }

    /// Total registered totes.
    pub fn tote_count(&self) -> usize {
        self.0.totes.read().unwrap().data.len()
    }

    // -- Temporal queries --

    // -- SKU ledger: unpicked stock + reservation counters --

    /// Record that a unit of SKU entered the warehouse.
    pub fn record_inventory_added(&self, sku: SkuId, at: SimTime) {
        let mut ledger = self.0.sku_ledger.write().unwrap();
        ledger.inventory.entry(sku).or_default().record(at, 1);
        drop(ledger);
        self.0.version.bump();
    }

    /// Remove one reserved unit from pickable stock at physical pick completion.
    /// The unit may still be physically present inside an order-designated tote.
    pub fn record_pick_consumed(&self, sku: SkuId, at: SimTime) {
        let mut ledger = self.0.sku_ledger.write().unwrap();
        ledger.inventory.entry(sku).or_default().record(at, -1);
        ledger.reserved.entry(sku).or_default().record(at, -1);
        debug_assert!(
            ledger.inventory.get(&sku).map(|c| c.count_at(at)).unwrap_or(0) >= 0,
            "sku_inventory went negative for {:?} at {:?}",
            sku,
            at,
        );
        debug_assert!(
            ledger.reserved.get(&sku).map(|c| c.count_at(at)).unwrap_or(0) >= 0,
            "sku_reserved went negative for {:?} at {:?}",
            sku,
            at,
        );
    }

    /// Reserve SKU units for an order entering the picking state. Refused
    /// unless that many units stay unreserved at `at` and at every later
    /// simtime, since a reservation holds until its pick.
    pub fn reserve_for_order(&self, lines: &[(SkuId, u32)], at: SimTime) -> bool {
        let mut ledger = self.0.sku_ledger.write().unwrap();
        let mut demand: FxHashMap<SkuId, u32> = FxHashMap::default();
        for &(sku, qty) in lines {
            *demand.entry(sku).or_insert(0) += qty;
        }
        for (&sku, &qty) in &demand {
            if !ledger.holds_from(&sku, at, qty) {
                return false;
            }
        }
        for (&sku, &qty) in &demand {
            ledger.reserved.entry(sku).or_default().record(at, qty as isize);
        }
        true
    }

    /// Unpicked stock count at sim time T, excluding contents of order totes.
    pub fn sku_inventory_at(&self, sku: SkuId, at: SimTime) -> isize {
        let ledger = self.0.sku_ledger.read().unwrap();
        ledger.inventory.get(&sku).map(|c| c.count_at(at)).unwrap_or(0)
    }

    /// Reserved count at sim time T.
    pub fn sku_reserved_at(&self, sku: SkuId, at: SimTime) -> isize {
        let ledger = self.0.sku_ledger.read().unwrap();
        ledger.reserved.get(&sku).map(|c| c.count_at(at)).unwrap_or(0)
    }

    /// True when any unreserved unit of `sku` exists at or after `at`.
    pub fn has_sku_at_or_after(&self, sku: SkuId, at: SimTime) -> bool {
        let ledger = self.0.sku_ledger.read().unwrap();
        ledger.earliest_available(&sku, at).is_some()
    }

    /// Earliest time >= `at` when unreserved inventory of `sku` exists.
    pub fn earliest_sku_available(&self, sku: SkuId, at: SimTime) -> Option<SimTime> {
        let ledger = self.0.sku_ledger.read().unwrap();
        ledger.earliest_available(&sku, at)
    }

    /// Earliest time >= `at` when at least `qty` unreserved units of `sku` exist.
    #[cfg_attr(feature = "perf", tracing::instrument(skip_all))]
    pub fn earliest_sku_qty_available(&self, sku: SkuId, qty: u32, at: SimTime) -> Option<SimTime> {
        let ledger = self.0.sku_ledger.read().unwrap();
        ledger.earliest_available_qty(&sku, at, qty)
    }

    /// Earliest simtime >= `at` from which at least `qty` units of `sku` stay
    /// unreserved through every later simtime the ledger records: the
    /// earliest simtime at which `reserve_for_order` accepts `qty` of `sku`.
    pub fn earliest_sku_qty_lasting(&self, sku: SkuId, qty: u32, at: SimTime) -> Option<SimTime> {
        let ledger = self.0.sku_ledger.read().unwrap();
        ledger.earliest_lasting_qty(&sku, at, qty)
    }

    /// True when at least `qty` unreserved units of `sku` exist at or after `at`.
    #[cfg_attr(feature = "perf", tracing::instrument(skip_all))]
    pub fn has_sku_qty_at_or_after(&self, sku: SkuId, qty: u32, at: SimTime) -> bool {
        self.earliest_sku_qty_available(sku, qty, at).is_some()
    }

    /// Find all totes with `sku` at or after `at`, regardless of location.
    ///
    /// Returns one entry per (content × location) phase intersection.
    /// Each entry's `available_at` is the earliest time within that phase
    /// when the tote has the SKU at that location. This is the primary
    /// tote search for the offline solver — it finds inventory that will
    /// exist in the future (deposited by putaway chains).
    #[cfg_attr(feature = "perf", tracing::instrument(skip_all))]
    pub fn find_sku_availability(&self, sku: SkuId, exclude: &HashSet<ToteId>, at: SimTime) -> Vec<SkuAvailability> {
        // Use sku_live temporal index for candidates. Fall back to sku_totes
        // (ever-contained) for totes that had the SKU before `at` — the
        // temporal index only tracks point-in-time membership.
        let totes = self.0.totes.read().unwrap();
        let candidate_totes = match totes.sku_totes.get(&sku) {
            Some(v) => v,
            None => return Vec::new(),
        };
        let mut results = Vec::new();
        for &tote_id in candidate_totes {
            if exclude.contains(&tote_id) {
                continue;
            }
            let data = match totes.data.get(&tote_id) {
                Some(d) => d,
                None => continue,
            };
            if data.designation == ToteDesignation::Inventory {
                data.sku_availability_windows(tote_id, sku, at, &mut results);
            }
        }
        results
    }

    /// Bumped only by `record_inventory_added` (putaway delivery).
    /// FIXME: `reserve_for_order`/`record_pick_consumed` mutate `reserved`
    /// without bumping, so a lost reservation race re-fails each sweep
    /// until the next putaway lands (order_queue Gotcha 3); per-SKU dirty
    /// tracking would fix both that and the blocked-order rescan cost.
    pub fn version(&self) -> &Version {
        &self.0.version
    }

    /// Available for new orders at sim time T = inventory - reserved.
    pub fn sku_available_at(&self, sku: SkuId, at: SimTime) -> isize {
        let ledger = self.0.sku_ledger.read().unwrap();
        let inv = ledger.inventory.get(&sku).map(|c| c.count_at(at)).unwrap_or(0);
        let res = ledger.reserved.get(&sku).map(|c| c.count_at(at)).unwrap_or(0);
        inv - res
    }

    // TODO(module): lends the ledger to one consumer by name; replace it with a counter view any reader can borrow.
    /// Run a closure with shared-read access to the inv + reserved
    /// TemporalCounter maps. The closure receives two
    /// [`CountersByMap`](sku_ledger::CountersByMap)
    /// views — read-only for the duration of the lock. Used by the bulk
    /// order builder's pallet search so it can sweep events across many
    /// SKUs under a single read lock (rather than acquiring/releasing
    /// once per SKU which would dominate cost at scale).
    pub fn with_ledger_for_pallet_search<R, F>(&self, f: F) -> R
    where
        F: FnOnce(&CountersByMap<'_, FxBuildHasher>, &CountersByMap<'_, FxBuildHasher>) -> R,
    {
        let ledger = self.0.sku_ledger.read().unwrap();
        let inv_view = CountersByMap { map: &ledger.inventory };
        let res_view = CountersByMap { map: &ledger.reserved };
        f(&inv_view, &res_view)
    }

    /// Seed the inventory counter from pre-populated tote contents at T=0.
    /// Called once during construction when putaway is disabled (totes
    /// already have SKUs). With putaway enabled, this is a no-op (all
    /// counters start at 0, putaway increments them as cycles complete).
    pub fn seed_inventory_from_totes(&self) {
        let totes = self.0.totes.read().unwrap();
        let mut ledger = self.0.sku_ledger.write().unwrap();
        for data in totes.data.values().filter(|data| data.designation == ToteDesignation::Inventory) {
            for &(sku, qty) in data.contents_at(SimTime::ZERO) {
                if qty > 0 {
                    ledger.inventory.entry(sku).or_default().record(SimTime::ZERO, qty as isize);
                }
            }
        }
    }
}
