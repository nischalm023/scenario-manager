//! RelayInventoryIndex — shared inventory membership and relay-specific costs.
//!
//! Tote and rack-slot queries, all per-relay:
//! 1. Cheapest empty tote (for putaway retrieval)
//! 2. Cheapest tote containing SKU X (for pick retrieval)
//! 3. Cheapest empty rack slot (for tote storage)
//! 4. A named order tote (for buffered-order retrieval, including filled totes)
//!
//! SKU membership is stored once per aisle side, with each tote's rack slot
//! and last accepted write. Queries compute the cost for each eligible relay.
//! Empty pools, order totes and slots retain their per-relay cost frontiers.
//! Every frontier ignores stale writes using the same per-key timestamps.
//! Destination claims exclude a rack slot across all its serving relays until
//! physical storage has been recorded.
//!
//! Cost is computed via RelayZone::{forward,return}_cost_ms — the VTM cycle
//! model, on the run's VTM figures. The index delegates to those methods.

use std::collections::HashSet;
use std::hash::BuildHasher;
use std::sync::{Arc, Mutex};

use rustc_hash::{FxHashMap, FxHashSet};

use crate::error::ReservationError;

use desgraph::{RelayId, WorldPos, ZoneId};
use desmap::BlockSide;

use crate::model::ids::{RackSlot, ToteId};
use crate::model::inventory::InventoryService;
use crate::model::inventory::events::{InventoryEvent, InventoryObserver, InventoryObserverError};
use crate::model::inventory::snapshot::SnapshotEvent;
use crate::model::location::ToteDesignation;
use crate::model::location::ToteLocation;
use crate::model::{RackId, SimDuration, SimTime, SkuId};
use crate::robots::vtm::motion::CycleBreakdown;
use crate::scenario_gen::RackUse;
use crate::utils::frontier_sorted_set::FrontierSortedSet;

#[cfg(test)]
mod tests;

use super::ToteSearchCriteria;
use super::config::{Relay, RelayZone, RelayZones};

/// An index candidate: the cheapest item for a given relay.
/// `R` is the recommendation type — `ToteId` (retrieve) or `RackSlot` (deposit).
#[derive(Clone, Debug)]
pub struct IndexCandidate<R> {
    pub item: R,
    pub relay: Relay,
    pub vtm_cost: SimDuration,
    /// When this item became available (FrontierSortedSet last_write_at).
    /// For totes: when VtmReturn deposited it InRack (or initial seed time).
    /// For slots: when the preceding tote physically leaves, or zero for
    /// initially empty racking.
    pub available_at: SimTime,
}

/// Convenience aliases.
pub type ToteCandidate = IndexCandidate<ToteId>;
pub type SlotCandidate = IndexCandidate<RackSlot>;

/// Per-relay tote and slot frontiers.
///
/// `relay` is co-located so frontier queries don't need a second probe into
/// `Inner::relays` per candidate.
struct PerRelayState {
    relay: Relay,
    /// Empty totes per pool. Separate sets, because a search for one must
    /// never return the other.
    empty_inventory_totes: FrontierSortedSet<ToteId, SimDuration>,
    empty_order_totes: FrontierSortedSet<ToteId, SimDuration>,
    /// Named retrieval includes filled order totes waiting in buffer racking.
    order_totes: FrontierSortedSet<ToteId, SimDuration>,
    /// Empty rack slots per RACKING, the same way empty totes are kept per
    /// pool. Split rather than filtered at the frontier because the frontier
    /// answers only with its cheapest entry: on DFW6 one bay in twenty-one of
    /// every run is order racking, so a deposit hunting for the other kind
    /// would walk thousands of slots to find its first candidate.
    empty_slots: [FrontierSortedSet<RackSlot, SimDuration>; 3],
}

impl PerRelayState {
    /// The empty-tote frontier for one pool.
    fn empty_pool(&self, d: ToteDesignation) -> &FrontierSortedSet<ToteId, SimDuration> {
        match d {
            ToteDesignation::Inventory => &self.empty_inventory_totes,
            ToteDesignation::Order => &self.empty_order_totes,
        }
    }

    fn empty_pool_mut(&mut self, d: ToteDesignation) -> &mut FrontierSortedSet<ToteId, SimDuration> {
        match d {
            ToteDesignation::Inventory => &mut self.empty_inventory_totes,
            ToteDesignation::Order => &mut self.empty_order_totes,
        }
    }

    /// The empty-slot frontier for one racking.
    fn slots(&self, uses: RackUse) -> &FrontierSortedSet<RackSlot, SimDuration> {
        &self.empty_slots[Self::slot_index(uses)]
    }

    fn slots_mut(&mut self, uses: RackUse) -> &mut FrontierSortedSet<RackSlot, SimDuration> {
        &mut self.empty_slots[Self::slot_index(uses)]
    }

    fn slot_index(uses: RackUse) -> usize {
        match uses {
            RackUse::Inventory => 0,
            RackUse::OrderDepot => 1,
            RackUse::OrderBuffer => 2,
        }
    }

    fn new(relay: Relay) -> Self {
        Self {
            relay,
            empty_inventory_totes: FrontierSortedSet::new(),
            empty_order_totes: FrontierSortedSet::new(),
            order_totes: FrontierSortedSet::new(),
            empty_slots: std::array::from_fn(|_| FrontierSortedSet::new()),
        }
    }
}

struct Inner {
    /// Rack → (zone ID, side of aisle).
    ///
    /// A VTM cycle is side-bound (source and destination share an aisle side),
    /// so the index pairs a rack only with relays whose `side` matches.
    rack_meta: FxHashMap<RackId, (ZoneId, BlockSide)>,
    /// Rack → what its steel is FOR. A rack the map says nothing about is
    /// inventory, which is what every map said before order racking existed.
    rack_use: FxHashMap<RackId, RackUse>,
    /// Per-relay immutable config (includes `side`).
    relays: FxHashMap<RelayId, Relay>,
    /// Zone → relay IDs in that zone.
    zone_relays: FxHashMap<ZoneId, Vec<RelayId>>,
    /// Zone configs (owns cost computation + rack/relay positions).
    zone_configs: FxHashMap<ZoneId, RelayZone>,
    /// Inventory service (for querying tote contents at sim-time).
    inventory: InventoryService,

    state: Mutex<State>,
}

struct State {
    relays: FxHashMap<RelayId, PerRelayState>,
    /// Accepted deposits retain exclusive destination ownership while the
    /// HTM and VTM approach. Inventory custody changes only at physical arrival.
    reserved_slots: FxHashSet<RackSlot>,
    /// All same-side relays serve the same totes, but their travel costs
    /// differ. Store membership and temporal safety once, then rank actual
    /// slots for the requested relay instead of copying stock per relay.
    sku_totes: FxHashMap<(ZoneId, BlockSide, SkuId), FrontierSortedSet<ToteId, RackSlot>>,
}

/// Zone-aware inventory index for RelayService queries.
/// Clone is a cheap Arc bump.
#[derive(Clone)]
pub(super) struct RelayInventoryIndex(Arc<Inner>);

/// An exclusive destination shared by every relay serving this rack slot.
/// Keep it until inventory records the deposit; dropping an abandoned claim
/// exposes the existing vacancy without fabricating a tote movement.
pub(super) struct RackSlotReservation {
    index: RelayInventoryIndex,
    slot: RackSlot,
}

impl Drop for RackSlotReservation {
    fn drop(&mut self) {
        self.index.0.state.lock().unwrap().reserved_slots.remove(&self.slot);
    }
}

impl RelayInventoryIndex {
    /// Build the index over a world's relays.
    ///
    /// `rack_use` says what each rack's steel is for, which decides both which
    /// frontier its slots belong to and which slots start EMPTY.
    ///
    /// Order-buffer slots start with no tote in them, and everywhere else
    /// scenario generation puts a tote in every slot, so the index can learn
    /// an empty slot the way it learns everything else: from the
    /// `LocationChanged` event of the tote that left it. A slot that never
    /// held a tote produces no such event, so without seeding the buffer's
    /// capacity would be invisible for the whole run and no deposit could
    /// ever be scored against it.
    ///
    /// Order-DEPOT slots left spare by a tote pool smaller than the row are
    /// deliberately NOT seeded: an order tote can only be deposited after
    /// being retrieved, and retrieval frees the slot it came from, so the
    /// pool's own slots are all the landing space the pool can ever need.
    pub(super) fn new(zones: &RelayZones, inventory: InventoryService, rack_use: &FxHashMap<RackId, RackUse>) -> Self {
        let zone_map = zones.zone_map();

        let mut rack_meta: FxHashMap<RackId, (ZoneId, BlockSide)> = FxHashMap::default();
        let mut relays: FxHashMap<RelayId, Relay> = FxHashMap::default();
        let mut relay_state: FxHashMap<RelayId, PerRelayState> = FxHashMap::default();
        let mut zone_relays: FxHashMap<ZoneId, Vec<RelayId>> = FxHashMap::default();
        let mut zone_configs: FxHashMap<ZoneId, RelayZone> = FxHashMap::default();

        for (&zid, &rz) in &zone_map {
            for (&rack_id, &side) in &rz.rack_side {
                rack_meta.insert(rack_id, (zid, side));
            }
            let relay_ids: Vec<RelayId> = rz
                .relays
                .iter()
                .map(|r| {
                    relays.insert(r.id, *r);
                    relay_state.insert(r.id, PerRelayState::new(*r));
                    r.id
                })
                .collect();
            zone_relays.insert(zid, relay_ids.clone());
            zone_configs.insert(zid, rz.clone());

            for (&rack_id, &side) in &rz.rack_side {
                if rack_use.get(&rack_id).copied().unwrap_or_default() != RackUse::OrderBuffer {
                    continue;
                }
                for level in 0..rz.vtc {
                    let slot = RackSlot { rack_id, level };
                    for relay_id in relay_ids.iter().filter(|rid| relays[rid].side == side) {
                        let cost = rz.min_return_cycle_cost(*relay_id, slot);
                        relay_state
                            .get_mut(relay_id)
                            .expect("relay state was just inserted")
                            .slots_mut(RackUse::OrderBuffer)
                            .add(slot, cost, SimTime::ZERO)
                            .expect("seeding at time zero cannot be stale");
                    }
                }
            }
        }

        Self(Arc::new(Inner {
            rack_meta,
            rack_use: rack_use.clone(),
            relays,
            zone_relays,
            zone_configs,
            inventory,
            state: Mutex::new(State {
                relays: relay_state,
                reserved_slots: FxHashSet::default(),
                sku_totes: FxHashMap::default(),
            }),
        }))
    }

    /// Relays serving a rack — restricted to those on the same aisle side as
    /// the rack. A VTM cycle is side-bound; cross-side pairs are physically
    /// invalid and are excluded from the index.
    fn relays_for_rack(&self, rack_id: RackId) -> impl Iterator<Item = RelayId> + '_ {
        // A rack with no zone/side mapping sits in a zone with no relay
        // columns -- an order-buffer block -- and is served by no relay at
        // all. Inventory placed there is unreachable by VTM, which is a fact
        // about the map, not a broken invariant to panic over.
        let meta = self.0.rack_meta.get(&rack_id).copied();
        meta.into_iter().flat_map(move |(zone, side)| {
            self.0
                .zone_relays
                .get(&zone)
                .into_iter()
                .flatten()
                .copied()
                .filter(move |rid| self.0.relays[rid].side == side)
        })
    }

    /// What a rack's steel is for. A rack the map says nothing about is
    /// inventory racking.
    fn use_of(&self, rack_id: RackId) -> RackUse {
        self.0.rack_use.get(&rack_id).copied().unwrap_or_default()
    }

    /// Zone config for a rack.
    fn zone_config_for_rack(&self, rack_id: RackId) -> Option<&RelayZone> {
        self.0.rack_meta.get(&rack_id).and_then(|(zid, _)| self.0.zone_configs.get(zid))
    }

    pub(super) fn forward_cycle(
        &self,
        relay_id: RelayId,
        slot: RackSlot,
        from_pos: WorldPos,
    ) -> Option<CycleBreakdown> {
        let zone = self.zone_config_for_rack(slot.rack_id)?;
        let rack_pos = *zone.rack_world_pos.get(&slot.rack_id)?;
        let h_dist_to_rack = zone.h_dist_between_cells_m(from_pos, rack_pos);
        zone.min_forward_cycle_from(relay_id, slot, h_dist_to_rack)
    }

    pub(super) fn return_cycle(&self, relay_id: RelayId, slot: RackSlot, from_pos: WorldPos) -> Option<CycleBreakdown> {
        let zone = self.zone_config_for_rack(slot.rack_id)?;
        let relay_pos = zone.relays.iter().find(|r| r.id == relay_id)?.world_pos;
        let h_dist_to_relay = zone.h_dist_between_cells_m(from_pos, relay_pos);
        zone.min_return_cycle_from(relay_id, slot, h_dist_to_relay)
    }

    /// Cheapest tote per relay on the current frontier, excluding skip_zones.
    pub(super) fn best_tote_per_relay<S: BuildHasher>(
        &self,
        criteria: ToteSearchCriteria,
        skip_zones: &HashSet<ZoneId, S>,
    ) -> Result<Vec<ToteCandidate>, ReservationError> {
        let state = self.0.state.lock().unwrap();
        let mut results: Vec<ToteCandidate> = Vec::with_capacity(state.relays.len());
        for prs in state.relays.values() {
            let relay = prs.relay;
            if skip_zones.contains(&relay.zone_id) {
                continue;
            }
            let Some((&item, vtm_cost, available_at)) = (match &criteria {
                ToteSearchCriteria::Empty { designation } => prs.empty_pool(*designation).first_with_time(),
                ToteSearchCriteria::EmptyOfType { designation, tote_type } => {
                    let pool = prs.empty_pool(*designation);
                    pool.find(|id, _, _| self.0.inventory.tote(*id).is_ok_and(|tote| tote.tote_type() == *tote_type))
                        .and_then(|(id, cost)| pool.get(id).map(|(_, at)| (id, cost, at)))
                }
                ToteSearchCriteria::ContainsSku { sku } => {
                    let zone = &self.0.zone_configs[&relay.zone_id];
                    state.sku_totes.get(&(relay.zone_id, relay.side, *sku)).and_then(|set| {
                        set.iter()
                            .map(|(tote, slot, at)| (tote, zone.min_forward_cycle_cost(relay.id, slot), at))
                            .min_by_key(|&(tote, cost, _)| (cost, *tote))
                    })
                }
                // A named tote, not a cheapest one: this relay is a candidate
                // only if it serves the rack that tote sits in.
                ToteSearchCriteria::BufferedOrder { tote_id } => {
                    prs.order_totes.get(tote_id).map(|(vtm_cost, at)| (tote_id, vtm_cost, at))
                }
            }) else {
                continue;
            };
            results.push(ToteCandidate { item, relay, vtm_cost, available_at });
        }
        if results.is_empty() {
            Err(ReservationError::NoCandidates {
                operation: "retrieve",
                reason: "no matching totes in any eligible zone",
            })
        } else {
            Ok(results)
        }
    }

    /// Cheapest empty rack slot per relay on the current frontier, excluding skip_zones.
    pub(super) fn best_empty_slot_per_relay<S: BuildHasher>(
        &self,
        uses: RackUse,
        skip_zones: &HashSet<ZoneId, S>,
    ) -> Result<Vec<SlotCandidate>, ReservationError> {
        let state = self.0.state.lock().unwrap();
        let mut results: Vec<SlotCandidate> = Vec::with_capacity(state.relays.len());
        for prs in state.relays.values() {
            let relay = prs.relay;
            if skip_zones.contains(&relay.zone_id) {
                continue;
            }
            let Some((&item, vtm_cost, available_at)) =
                prs.slots(uses).iter().find(|(slot, _, _)| !state.reserved_slots.contains(slot))
            else {
                continue;
            };
            results.push(SlotCandidate { item, relay, vtm_cost, available_at });
        }
        if results.is_empty() {
            Err(ReservationError::NoCandidates { operation: "deposit", reason: "no empty slots in any eligible zone" })
        } else {
            Ok(results)
        }
    }

    pub(super) fn reserve_slot(&self, slot: RackSlot, at: SimTime) -> Result<RackSlotReservation, ReservationError> {
        let mut state = self.0.state.lock().unwrap();
        let vacant = self
            .relays_for_rack(slot.rack_id)
            .next()
            .and_then(|relay| state.relays[&relay].slots(self.use_of(slot.rack_id)).get(&slot));
        if vacant.is_none_or(|(_, available_at)| available_at > at) || !state.reserved_slots.insert(slot) {
            return Err(ReservationError::NoCandidates {
                operation: "deposit",
                reason: "rack slot is occupied, already reserved, or not yet vacant",
            });
        }
        Ok(RackSlotReservation { index: self.clone(), slot })
    }
}

impl InventoryObserver for RelayInventoryIndex {
    fn on_inventory_event(&self, event: &InventoryEvent) -> Result<(), InventoryObserverError> {
        self.apply(event, None)
    }
}

impl RelayInventoryIndex {
    pub(super) fn initialize(&self, snapshot: &[SnapshotEvent]) -> Result<(), InventoryObserverError> {
        for entry in snapshot {
            self.apply(&entry.event, Some(&entry.contents))?;
        }
        Ok(())
    }

    fn apply(&self, event: &InventoryEvent, snapshot: Option<&[(SkuId, u32)]>) -> Result<(), InventoryObserverError> {
        crate::perf_span!("inv_index_event");
        let contents_at = |tote_id: ToteId, at: SimTime| -> Result<Vec<(SkuId, u32)>, InventoryObserverError> {
            match snapshot {
                Some(contents) => Ok(contents.to_vec()),
                None => Ok(self.0.inventory.tote(tote_id)?.contents_at(at)),
            }
        };
        let mut state = self.0.state.lock().unwrap();

        match event {
            InventoryEvent::LocationChanged { tote_id, designation, old_location, new_location, is_empty, at } => {
                // -- Departing old rack slot ----------------------------------
                let old_rack = match old_location {
                    ToteLocation::InRack { rack_id, z_index } => Some((*rack_id, *z_index)),
                    ToteLocation::Reserved { rack_id, z_index } => Some((*rack_id, *z_index)),
                    _ => None,
                };
                if let Some((rack_id, z_index)) = old_rack {
                    let slot = RackSlot { rack_id, level: z_index };
                    let zone = self.zone_config_for_rack(rack_id);
                    let tote_stays_in_slot = matches!(new_location, ToteLocation::Reserved { .. });
                    let was_reserved = matches!(old_location, ToteLocation::Reserved { .. });

                    for relay_id in self.relays_for_rack(rack_id) {
                        let prs = state
                            .relays
                            .get_mut(&relay_id)
                            .unwrap_or_else(|| panic!("relay {:?} missing from index state", relay_id));

                        if !tote_stays_in_slot {
                            // Slot becomes vacant — tote physically leaving.
                            let cost = zone
                                .unwrap_or_else(|| panic!("no zone config for rack {:?}", rack_id))
                                .min_return_cycle_cost(relay_id, slot);
                            prs.slots_mut(self.use_of(rack_id)).add(slot, cost, *at)?;
                        }

                        if !was_reserved {
                            // Tote index cleanup only needed on first departure (InRack → *).
                            // Reserved → WithRobot: tote already removed during InRack → Reserved.
                            prs.empty_pool_mut(*designation).remove(tote_id, *at)?;
                            if *designation == ToteDesignation::Order {
                                prs.order_totes.remove(tote_id, *at)?;
                            }
                        }
                    }

                    if !was_reserved && *designation == ToteDesignation::Inventory {
                        // SKU cleanup only on first departure.
                        let skus = contents_at(*tote_id, *at)?;
                        if let Some(&(zone, side)) = self.0.rack_meta.get(&rack_id) {
                            for &(sku, _qty) in &skus {
                                if let Some(set) = state.sku_totes.get_mut(&(zone, side, sku)) {
                                    set.remove(tote_id, *at)?;
                                }
                            }
                        }
                    }
                }

                // -- Arriving at new rack slot --------------------------------
                if let ToteLocation::InRack { rack_id, z_index } = new_location {
                    let slot = RackSlot { rack_id: *rack_id, level: *z_index };
                    let zone = self.zone_config_for_rack(*rack_id);

                    // Query tote contents once for SKU insertion.
                    let skus = contents_at(*tote_id, *at)?;

                    for relay_id in self.relays_for_rack(*rack_id) {
                        let prs = state
                            .relays
                            .get_mut(&relay_id)
                            .unwrap_or_else(|| panic!("relay {:?} missing from index state", relay_id));
                        let zone_cfg = zone.unwrap_or_else(|| panic!("no zone config for rack {:?}", rack_id));

                        // Slot becomes occupied.
                        prs.slots_mut(self.use_of(*rack_id)).remove(&slot, *at)?;
                        if *designation == ToteDesignation::Order {
                            prs.order_totes.add(*tote_id, zone_cfg.min_forward_cycle_cost(relay_id, slot), *at)?;
                        }

                        // Empty tote tracking.
                        if *is_empty {
                            let cost = zone_cfg.min_forward_cycle_cost(relay_id, slot);
                            prs.empty_pool_mut(*designation).add(*tote_id, cost, *at)?;
                        }
                    }
                    // Picked contents in order totes never become pick stock.
                    if *designation == ToteDesignation::Inventory
                        && let Some(&(zone, side)) = self.0.rack_meta.get(rack_id)
                    {
                        for &(sku, _qty) in &skus {
                            state
                                .sku_totes
                                .entry((zone, side, sku))
                                .or_insert_with(FrontierSortedSet::new)
                                .add(*tote_id, slot, *at)?;
                        }
                    }
                }
            }

            InventoryEvent::ContentsChanged { tote_id, designation, sku, delta, was_empty, is_empty, location, at } => {
                // Only affects relay sets if tote is in a rack slot.
                let (rack_id, z_index) = match location {
                    ToteLocation::InRack { rack_id, z_index } => (*rack_id, *z_index),
                    _ => return Ok(()),
                };
                let slot = RackSlot { rack_id, level: z_index };
                let zone = match self.zone_config_for_rack(rack_id) {
                    Some(z) => z,
                    None => return Ok(()),
                };

                if *designation == ToteDesignation::Inventory
                    && let Some(&(zone_id, side)) = self.0.rack_meta.get(&rack_id)
                {
                    let key = (zone_id, side, *sku);
                    if *delta > 0 {
                        state.sku_totes.entry(key).or_insert_with(FrontierSortedSet::new).add(*tote_id, slot, *at)?;
                    } else if *delta < 0 {
                        let still_has = contents_at(*tote_id, *at)?.iter().any(|(item, qty)| item == sku && *qty > 0);
                        if !still_has && let Some(set) = state.sku_totes.get_mut(&key) {
                            set.remove(tote_id, *at)?;
                        }
                    }
                }

                for relay_id in self.relays_for_rack(rack_id) {
                    let prs = state
                        .relays
                        .get_mut(&relay_id)
                        .unwrap_or_else(|| panic!("relay {:?} missing from index state", relay_id));
                    let fwd_cost = zone.min_forward_cycle_cost(relay_id, slot);
                    if *designation == ToteDesignation::Order {
                        prs.order_totes.add(*tote_id, fwd_cost, *at)?;
                    }

                    // Empty tote transitions.
                    if *was_empty && !is_empty {
                        // Was empty, now has content -> leave empty set.
                        prs.empty_pool_mut(*designation).remove(tote_id, *at)?;
                    } else if !was_empty && *is_empty {
                        // Was non-empty, now empty -> enter empty set.
                        prs.empty_pool_mut(*designation).add(*tote_id, fwd_cost, *at)?;
                    }
                }
            }
        }
        Ok(())
    }
}
