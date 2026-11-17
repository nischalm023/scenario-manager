//! Everything inventory keeps about a tote, and the `Tote` handle callers
//! read and mutate it through.
//!
//! All tote state is temporal — time-sliced step functions. Each slice
//! records the state from `from_ms` until the next slice's `from_ms`.
//! The last slice is implicitly "until further notice" (no end time stored).
//! Point query = binary search for latest slice where `from_ms <= T`.
//!
//! TODO(overstuffed): "types" also collects the service's own private
//! `Inner` (locks, spatial maps, observer registry, event sink) and the
//! query-result structs the service returns. Three different nouns.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};

use crate::events::{EventSink, InventoryDeltaEvent, InventoryMoveEvent, SimEvent, ToteContentsChangedEvent};
use crate::model::bot::BotService;
use crate::model::location::{ToteDesignation, ToteLocation, ToteType};
use crate::model::{RackId, RobotId, SimTime, SkuId, ToteId};
use crate::signals::Version;

use super::error::InventoryError;
use super::events::{InventoryEvent, InventoryObserver};
use super::sku_ledger::SkuLedger;

// ---------------------------------------------------------------------------
// Query results
// ---------------------------------------------------------------------------

/// Where a tote with a needed SKU can be found, with temporal availability.
/// Returned by `find_sku_availability` — the forward-looking tote search
/// that enables picks to target inventory deposited by future putaway.
#[derive(Clone, Debug)]
pub enum SkuAvailability {
    /// Tote in rack — needs full retrieval chain (VtmForward → HtmPickup → ...).
    InRack { tote_id: ToteId, rack_id: RackId, quantity: u32, available_at: SimTime },
    /// Tote on a robot — direct delivery possible if the robot is an HTM.
    WithRobot { tote_id: ToteId, robot_id: RobotId, quantity: u32, available_at: SimTime },
}

impl SkuAvailability {
    pub fn tote_id(&self) -> ToteId {
        match self {
            Self::InRack { tote_id, .. } | Self::WithRobot { tote_id, .. } => *tote_id,
        }
    }

    pub fn available_at(&self) -> SimTime {
        match self {
            Self::InRack { available_at, .. } | Self::WithRobot { available_at, .. } => *available_at,
        }
    }

    pub fn quantity(&self) -> u32 {
        match self {
            Self::InRack { quantity, .. } | Self::WithRobot { quantity, .. } => *quantity,
        }
    }
}

/// A content snapshot valid from `from_ms` until the next slice.
/// Most totes hold 0 or 1 SKU, so a small vec is optimal.
#[derive(Clone, Debug)]
pub(super) struct ContentSlice {
    pub from: SimTime,
    pub sequence: u64,
    pub changed_sku: Option<SkuId>,
    /// Empty vec = tote is empty. Typically 0-1 entries.
    pub contents: Vec<(SkuId, u32)>,
}

/// A location snapshot valid from `from_ms` until the next slice.
#[derive(Clone, Debug)]
pub(super) struct LocationSlice {
    pub from: SimTime,
    pub sequence: u64,
    pub location: ToteLocation,
}

/// Per-tote data. Not exposed outside inventory module.
pub(super) struct ToteData {
    pub created_at: SimTime,
    pub tote_type: ToteType,
    /// Which pool this tote belongs to. Set once at registration; never
    /// changes.
    pub designation: ToteDesignation,
    /// Time-sliced location history. Sorted by from_ms, append-only.
    pub location_log: Vec<LocationSlice>,
    /// Time-sliced content history. Sorted by from_ms, append-only.
    pub content_log: Vec<ContentSlice>,
}

impl ToteData {
    pub fn new(
        tote_type: ToteType,
        designation: ToteDesignation,
        location: ToteLocation,
        at: SimTime,
        sequence: u64,
    ) -> Self {
        Self {
            created_at: at,
            tote_type,
            designation,
            location_log: vec![LocationSlice { from: at, location, sequence }],
            content_log: vec![ContentSlice { from: at, contents: vec![], sequence, changed_sku: None }],
        }
    }

    fn check_time(&self, at: SimTime) -> Result<(), InventoryError> {
        if at < self.created_at {
            Err(InventoryError::BeforeCreation { at, created_at: self.created_at })
        } else {
            Ok(())
        }
    }

    pub fn location_at(&self, t: SimTime) -> ToteLocation {
        match self.location_log.partition_point(|s| s.from <= t) {
            0 => ToteLocation::Uninitialized,
            i => self.location_log[i - 1].location,
        }
    }

    pub fn contents_at(&self, t: SimTime) -> &[(SkuId, u32)] {
        match self.content_log.partition_point(|s| s.from <= t) {
            0 => &[], // before first entry
            i => &self.content_log[i - 1].contents,
        }
    }

    /// Earliest time >= `after` when this tote has `sku` AND is InRack.
    /// Returns None if the tote never satisfies both conditions after `after`.
    pub fn earliest_available_with_sku(&self, sku: SkuId, after: SimTime) -> Option<SimTime> {
        // Walk content slices from `after` forward to find when the SKU appears.
        // For each content slice with the SKU, check if the tote is InRack at that time.
        for slice in &self.content_log {
            if slice.from < after {
                continue;
            }
            let has_sku = slice.contents.iter().any(|&(s, q)| s == sku && q > 0);
            if !has_sku {
                continue;
            }
            // Found a time with the SKU. Check location at this time.
            if matches!(self.location_at(slice.from), ToteLocation::InRack { .. }) {
                return Some(slice.from);
            }
            // SKU present but not in rack. Check forward in location log.
            for loc_slice in &self.location_log {
                if loc_slice.from < slice.from {
                    continue;
                }
                if matches!(loc_slice.location, ToteLocation::InRack { .. }) {
                    // Verify SKU still present at this location time.
                    if self.sku_qty_at(sku, loc_slice.from) > 0 {
                        return Some(loc_slice.from);
                    }
                }
            }
        }
        // Also check: tote might already have the SKU at `after` (content set before `after`).
        if self.sku_qty_at(sku, after) > 0 && matches!(self.location_at(after), ToteLocation::InRack { .. }) {
            return Some(after);
        }
        None
    }

    /// Enumerate all (content × location) phases where this tote has `sku`
    /// at or after `at`. Pushes one `SkuAvailability` per intersection.
    pub fn sku_availability_windows(&self, tote_id: ToteId, sku: SkuId, at: SimTime, out: &mut Vec<SkuAvailability>) {
        // Find the content slice active at `at` (or the first one after).
        let start = self.content_log.partition_point(|s| s.from <= at);
        let check_from = if start > 0 { start - 1 } else { 0 };

        for i in check_from..self.content_log.len() {
            let slice = &self.content_log[i];
            let qty = match slice.contents.iter().find(|&&(s, q)| s == sku && q > 0) {
                Some(&(_, q)) => q,
                None => continue,
            };
            let content_from = slice.from.max(at);
            let content_until = self.content_log.get(i + 1).map(|next| next.from).unwrap_or(SimTime::EOT);
            if content_from >= content_until {
                continue;
            }

            // Walk location phases within this content window.
            let loc_start = self.location_log.partition_point(|s| s.from <= content_from);
            let loc_check = if loc_start > 0 { loc_start - 1 } else { 0 };
            for j in loc_check..self.location_log.len() {
                let loc = &self.location_log[j];
                if loc.from >= content_until {
                    break;
                }
                let loc_until = self.location_log.get(j + 1).map(|next| next.from).unwrap_or(SimTime::EOT);
                let phase_start = content_from.max(loc.from);
                let phase_end = content_until.min(loc_until);
                if phase_start >= phase_end {
                    continue;
                }
                match loc.location {
                    ToteLocation::InRack { rack_id, .. } => {
                        out.push(SkuAvailability::InRack {
                            tote_id,
                            rack_id,
                            quantity: qty,
                            available_at: phase_start,
                        });
                    }
                    ToteLocation::WithRobot { robot_id } => {
                        out.push(SkuAvailability::WithRobot {
                            tote_id,
                            robot_id,
                            quantity: qty,
                            available_at: phase_start,
                        });
                    }
                    // AtRelay is transient — tote is in transit, not retrievable.
                    _ => {}
                }
            }
        }
    }

    pub fn has_sku_at(&self, sku: SkuId, t: SimTime) -> bool {
        self.contents_at(t).iter().any(|&(s, q)| s == sku && q > 0)
    }

    pub fn sku_qty_at(&self, sku: SkuId, t: SimTime) -> u32 {
        self.contents_at(t).iter().find(|&&(s, _)| s == sku).map(|&(_, q)| q).unwrap_or(0)
    }

    pub fn is_empty_at(&self, t: SimTime) -> bool {
        self.contents_at(t).is_empty()
    }

    pub fn is_empty_in_rack_at(&self, t: SimTime) -> bool {
        self.is_empty_at(t) && matches!(self.location_at(t), ToteLocation::InRack { .. })
    }

    /// Earliest time >= `after` when this tote is empty AND InRack and will
    /// stay that way (no subsequent events). For the offline solver, this
    /// means checking the tote's latest stable state — a tote that was
    /// dispatched at T=5000 is not "available at T=0" even though its
    /// initial state at T=0 was empty+InRack.
    pub fn earliest_available_empty(&self, after: SimTime) -> Option<SimTime> {
        let last_loc = self.location_log.last().map(|s| s.from).unwrap_or(SimTime::ZERO);
        let last_content = self.content_log.last().map(|s| s.from).unwrap_or(SimTime::ZERO);
        let last_event = last_loc.max(last_content);

        let check_from = after.max(last_event);
        if self.is_empty_at(check_from) && matches!(self.location_at(check_from), ToteLocation::InRack { .. }) {
            return Some(check_from);
        }
        None
    }

    pub fn current_location(&self) -> ToteLocation {
        self.location_log.last().map(|s| s.location).unwrap_or(ToteLocation::Uninitialized)
    }

    /// Current contents at time `t` (latest content slice at or before `t`),
    /// as owned pairs. Used when bridging a `LocationChanged` into a
    /// `ToteMoved` SimEvent, to value the aisle crossing by what the tote is
    /// carrying at the moment it crosses.
    pub fn owned_contents_at(&self, t: SimTime) -> Vec<(SkuId, u32)> {
        self.contents_at(t).to_vec()
    }

    pub fn current_contents(&self) -> &[(SkuId, u32)] {
        self.content_log.last().map(|s| s.contents.as_slice()).unwrap_or(&[])
    }
}

fn insert_content_sorted(log: &mut Vec<ContentSlice>, slice: ContentSlice) {
    let pos = log.partition_point(|s| s.from <= slice.from);
    log.insert(pos, slice);
}

fn insert_location_sorted(log: &mut Vec<LocationSlice>, slice: LocationSlice) {
    let pos = log.partition_point(|s| s.from <= slice.from);
    log.insert(pos, slice);
}

/// Tote collection + sku reverse index. Behind a single RwLock.
pub(super) struct ToteState {
    pub data: BTreeMap<ToteId, ToteData>,
    /// Reverse index: SKU → totes that have ever contained this SKU.
    /// Membership grows monotonically. Temporal content check filters at query time.
    pub sku_totes: BTreeMap<SkuId, BTreeSet<ToteId>>,
}

/// Service inner. Immutable spatial data + single RwLock for all tote state.
pub(super) struct Inner {
    pub(super) version: Version,
    /// Serializes a mutation through observer delivery against snapshot and
    /// subscription; the counter merges location/content histories in commit order.
    pub mutation_sequence: Mutex<u64>,
    /// Bot service for resolving WithRobot tote positions (temporal).
    /// Set once post-construction via set_bot_service (circular dep).
    pub bot_service: OnceLock<BotService>,
    /// All tote state behind one lock.
    pub totes: RwLock<ToteState>,
    /// Unpicked SKU stock and reservation counters (temporal).
    pub sku_ledger: RwLock<SkuLedger>,

    /// Observers for inventory mutation events. Registered by external indices
    /// (e.g., RelayInventoryIndex). Called on every content/location mutation.
    ///
    /// Held weakly: observers that back-reference InventoryService (such as
    /// RelayInventoryIndex via its `inventory: InventoryService` field) would
    /// form an Arc cycle if stored strongly here, leaking `event_sink` and
    /// hanging the observer thread at sim shutdown. Callers of `add_observer`
    /// must keep their own strong Arc alive (see [`InventoryService::add_observer`]).
    pub observers: RwLock<Vec<Weak<dyn InventoryObserver>>>,

    /// Optional SimEvent sink for bridging `ContentsChanged` mutations to
    /// `SimEvent::InventoryDelta` (drives the desvis CurrentInventory
    /// chart). In a run it is the sink the EventJournal hands out, so
    /// inventory changes reach the same observer as task events, checked
    /// against B as they are.
    pub event_sink: Option<Arc<dyn EventSink>>,
}

impl Inner {
    /// Notify all observers and bridge inventory mutations to the SimEvent
    /// stream. Single source of truth for inventory event dispatch — invoked
    /// from both `InventoryService::emit` (init / location changes) and
    /// `Tote::emit` (per-mutation `ContentsChanged`).
    ///
    /// Stock deltas remain distinct from absolute physical payload snapshots:
    /// - `ContentsChanged` → `SimEvent::InventoryDelta` — the signed
    ///   available inventory ledger. Order contents never enter this stock total.
    /// - `ContentsChanged` → `ToteContentsChanged` — the physical payload of
    ///   either inventory or order containers, including filled order buffers.
    /// - `LocationChanged` → `SimEvent::InventoryMove` — carries the tote's
    ///   contents at the move instant so the observer can compute per-aisle
    ///   inventory flux (units a tote carries across an aisle boundary).
    pub(super) fn dispatch_event(self: &Arc<Self>, event: InventoryEvent) -> Result<(), InventoryError> {
        let observers = self.observers.read().unwrap();
        for weak in observers.iter() {
            if let Some(observer) = weak.upgrade() {
                observer.on_inventory_event(&event)?;
            }
        }
        let Some(sink) = &self.event_sink else { return Ok(()) };
        match &event {
            InventoryEvent::ContentsChanged { tote_id, designation, sku, delta, at, .. } => {
                if *designation == ToteDesignation::Inventory {
                    sink.send(SimEvent::InventoryDelta(InventoryDeltaEvent {
                        delta: *delta,
                        sku: *sku,
                        tote_id: *tote_id,
                        at: *at,
                    }));
                }
                let units = self
                    .totes
                    .read()
                    .unwrap()
                    .data
                    .get(tote_id)
                    .ok_or(InventoryError::NotFound)?
                    .owned_contents_at(*at)
                    .iter()
                    .map(|(_, qty)| u64::from(*qty))
                    .sum();
                sink.send(SimEvent::ToteContentsChanged(ToteContentsChangedEvent {
                    tote_id: *tote_id,
                    units,
                    at: *at,
                }));
            }
            InventoryEvent::LocationChanged { tote_id, designation, old_location, new_location, is_empty, at } => {
                // Value the crossing by what the tote carries now — but an
                // empty tote carries nothing, so skip the contents read-lock +
                // clone and send an empty payload. The observer still needs
                // every move to keep each tote's last-settled aisle current (an
                // empty tote can reposition between aisles before being
                // refilled); it just logs no flux for an empty one.
                let contents = if *is_empty || *designation == ToteDesignation::Order {
                    Vec::new()
                } else {
                    self.totes.read().unwrap().data.get(tote_id).map(|d| d.owned_contents_at(*at)).unwrap_or_default()
                };
                sink.send(SimEvent::InventoryMove(InventoryMoveEvent {
                    tote_id: *tote_id,
                    from: *old_location,
                    to: *new_location,
                    contents,
                    at: *at,
                }));
            }
        }
        Ok(())
    }
}

/// Handle to a specific tote. Constructed on demand, not stored.
///
/// Read methods take read locks. Mutation methods (add_contents, set_location)
/// take write locks and append temporal slices.
pub struct Tote {
    pub id: ToteId,
    pub(super) inner: Arc<Inner>,
}

impl Tote {
    fn emit(&self, event: InventoryEvent) -> Result<(), InventoryError> {
        Inner::dispatch_event(&self.inner, event)
    }

    // The read getters below panic on an unregistered id, which no caller can
    // reach: `InventoryService::tote` errs unless the id is in the map, and
    // nothing ever removes from that map. TODO(zero-panic): the ruling wants
    // these to be `Result<_, InventoryError>` anyway; the conversion also
    // touches `model::relay` (`relay::mod`, `relay::inventory_index`), the TTP
    // task and demand modules, and `model::inventory::tests`.
    pub fn designation(&self) -> ToteDesignation {
        let totes = self.inner.totes.read().unwrap();
        totes.data.get(&self.id).unwrap_or_else(|| panic!("tote {:?} not registered", self.id)).designation
    }

    pub fn tote_type(&self) -> ToteType {
        let totes = self.inner.totes.read().unwrap();
        totes.data.get(&self.id).unwrap_or_else(|| panic!("tote {:?} not registered", self.id)).tote_type
    }

    pub fn location(&self) -> ToteLocation {
        let totes = self.inner.totes.read().unwrap();
        totes.data.get(&self.id).unwrap_or_else(|| panic!("tote {:?} not registered", self.id)).current_location()
    }

    pub fn location_at(&self, t: SimTime) -> ToteLocation {
        let totes = self.inner.totes.read().unwrap();
        totes.data.get(&self.id).unwrap_or_else(|| panic!("tote {:?} not registered", self.id)).location_at(t)
    }

    /// True only when the tote stays in this custody throughout [start, end].
    /// Inspect every recorded transition, so departure and return between the
    /// endpoints cannot masquerade as continuous presentation.
    pub fn location_is_through(&self, start: SimTime, end: SimTime, expected: ToteLocation) -> bool {
        if end < start {
            return false;
        }
        let totes = self.inner.totes.read().unwrap();
        let data = &totes.data[&self.id];
        data.location_at(start) == expected
            && data
                .location_log
                .iter()
                .filter(|slice| slice.from > start && slice.from <= end)
                .all(|slice| slice.location == expected)
    }

    pub fn contents(&self) -> HashMap<SkuId, u32> {
        let totes = self.inner.totes.read().unwrap();
        let data = totes.data.get(&self.id).unwrap_or_else(|| panic!("tote {:?} not registered", self.id));
        data.current_contents().iter().cloned().collect()
    }

    pub fn contents_at(&self, t: SimTime) -> Vec<(SkuId, u32)> {
        let totes = self.inner.totes.read().unwrap();
        totes.data.get(&self.id).unwrap_or_else(|| panic!("tote {:?} not registered", self.id)).contents_at(t).to_vec()
    }

    pub fn is_empty(&self) -> bool {
        let totes = self.inner.totes.read().unwrap();
        totes
            .data
            .get(&self.id)
            .unwrap_or_else(|| panic!("tote {:?} not registered", self.id))
            .current_contents()
            .is_empty()
    }

    pub fn is_empty_at(&self, t: SimTime) -> bool {
        let totes = self.inner.totes.read().unwrap();
        totes.data.get(&self.id).unwrap_or_else(|| panic!("tote {:?} not registered", self.id)).is_empty_at(t)
    }

    pub fn has_sku(&self, sku: SkuId) -> bool {
        let totes = self.inner.totes.read().unwrap();
        let data = totes.data.get(&self.id).unwrap_or_else(|| panic!("tote {:?} not registered", self.id));
        data.current_contents().iter().any(|&(s, q)| s == sku && q > 0)
    }

    /// Quantity of a specific SKU in this tote at time T.
    /// Read-only — takes a read lock. Use in check phase before remove_item.
    pub fn sku_qty_at(&self, sku: SkuId, t: SimTime) -> u32 {
        let totes = self.inner.totes.read().unwrap();
        totes.data.get(&self.id).unwrap_or_else(|| panic!("tote {:?} not registered", self.id)).sku_qty_at(sku, t)
    }

    pub fn has_sku_at(&self, sku: SkuId, t: SimTime) -> bool {
        let totes = self.inner.totes.read().unwrap();
        totes.data.get(&self.id).unwrap_or_else(|| panic!("tote {:?} not registered", self.id)).has_sku_at(sku, t)
    }

    /// Add items to this tote at time `at_ms`. Inserts a new content slice
    /// based on contents_at(at_ms) — supports non-monotonic insertion.
    pub fn add_item(&self, sku: SkuId, qty: u32, at_ms: u64) -> Result<(), InventoryError> {
        let mut sequence = self.inner.mutation_sequence.lock().unwrap();
        let order = *sequence;
        *sequence += 1;
        let t = SimTime::ms(at_ms);
        let mut totes = self.inner.totes.write().unwrap();
        totes.data.get(&self.id).ok_or(InventoryError::NotFound)?.check_time(t)?;
        totes.sku_totes.entry(sku).or_default().insert(self.id);

        let data = totes.data.get_mut(&self.id).ok_or(InventoryError::NotFound)?;
        let mut new_contents: Vec<(SkuId, u32)> = data.contents_at(t).to_vec();
        if let Some(entry) = new_contents.iter_mut().find(|(s, _)| *s == sku) {
            entry.1 += qty;
        } else {
            new_contents.push((sku, qty));
        }
        let was_empty = data.is_empty_at(t);
        let location = data.location_at(t);
        insert_content_sorted(
            &mut data.content_log,
            ContentSlice { from: t, contents: new_contents, sequence: order, changed_sku: Some(sku) },
        );
        drop(totes);

        self.emit(InventoryEvent::ContentsChanged {
            tote_id: self.id,
            designation: self.designation(),
            sku,
            delta: qty as i32,
            was_empty,
            is_empty: false,
            location,
            at: t,
        })
    }

    /// Remove items from this tote at time `at_ms`. Errs with
    /// `InsufficientQuantity` when the tote holds less than `qty` of `sku`
    /// then (R4: no negative content); the caller retries a different tote.
    pub fn remove_item(&self, sku: SkuId, qty: u32, at_ms: u64) -> Result<(), InventoryError> {
        let mut sequence = self.inner.mutation_sequence.lock().unwrap();
        let order = *sequence;
        *sequence += 1;
        let t = SimTime::ms(at_ms);
        let mut totes = self.inner.totes.write().unwrap();
        let data = totes.data.get_mut(&self.id).ok_or(InventoryError::NotFound)?;
        data.check_time(t)?;

        let mut new_contents: Vec<(SkuId, u32)> = data.contents_at(t).to_vec();
        if let Some(entry) = new_contents.iter_mut().find(|(s, _)| *s == sku) {
            if entry.1 < qty {
                return Err(InventoryError::InsufficientQuantity);
            }
            entry.1 -= qty;
        } else {
            return Err(InventoryError::InsufficientQuantity);
        }
        new_contents.retain(|&(_, q)| q > 0);
        let now_empty = new_contents.is_empty();
        let location = data.location_at(t);
        insert_content_sorted(
            &mut data.content_log,
            ContentSlice { from: t, contents: new_contents, sequence: order, changed_sku: Some(sku) },
        );
        drop(totes);

        self.emit(InventoryEvent::ContentsChanged {
            tote_id: self.id,
            designation: self.designation(),
            sku,
            delta: -(qty as i32),
            was_empty: false,
            is_empty: now_empty,
            location,
            at: t,
        })?;
        Ok(())
    }

    pub fn move_to(&self, location: ToteLocation, at: SimTime) -> Result<(), InventoryError> {
        let mut sequence = self.inner.mutation_sequence.lock().unwrap();
        let order = *sequence;
        *sequence += 1;
        let mut totes = self.inner.totes.write().unwrap();
        let data = totes.data.get_mut(&self.id).ok_or(InventoryError::NotFound)?;
        data.check_time(at)?;
        let is_empty = data.is_empty_at(at);
        let old_location = data.location_at(at);
        insert_location_sorted(&mut data.location_log, LocationSlice { from: at, location, sequence: order });
        drop(totes);

        self.emit(InventoryEvent::LocationChanged {
            tote_id: self.id,
            designation: self.designation(),
            old_location,
            new_location: location,
            is_empty,
            at,
        })
    }
}
