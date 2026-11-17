//! Within `observer`, `handlers` CONSUMES the live event stream: each
//! `SimEvent` mutates counters, journals, and timelines — the sole producer
//! of the captured fact records (leg journals, operation log, aisle state
//! stamps, duty overlays).
//!
//! TODO(overstuffed): every fact family's capture logic shares this one
//! file; when issue #300's tenant migration touches capture, split by fact
//! family the way `reporting/` splits by report family.
//!
//! Each `handle_*` method processes one variant of `SimEvent`, updating the
//! slices its facts fall in and the run's journals; the run's totals are
//! carried from each slice as it is made final (`finality::run_totals`).

use std::collections::{BTreeMap, BTreeSet};
use std::hash::Hash;

use desgraph::{CellId, RelayId, RobotType};

use desgraph::StationType;

use rustc_hash::FxHashMap;
use worldview::frames::robot_state::{RobotState, WaitCause};

use crate::events::{
    CycleCompleteEvent, CycleKind, DivingBoardSlotWaitEvent, HtmAssignmentEvent, HtmRelayDepositEvent,
    HtmRelayPickupEvent, InitialPositionsEvent, InventoryDeltaEvent, InventoryMoveEvent, OrderCompleteEvent,
    PipelineSnapshotEvent, RelayStateChangeEvent, SimEvent, ToteContentsChangedEvent, ToteCreatedEvent,
    ToteInitialStatesEvent, ToteMovedEvent, ToteSource, ToteTransitEvent, TripEndpoint, TripPurpose, TripStartEvent,
    VtmCycleDirection, VtmCycleEvent, VtmIdleReasonEvent, VtmIdleReasonKind, VtmOperationEvent,
};
use crate::model::location::ToteLocation;
use crate::model::{RobotId, SimDuration, SimTime, StationArrivalSource};
use crate::observer::activity::{ActivityKind, Milestone};
use crate::observer::duty::StationBusyInterval;
use crate::observer::output::{
    AisleFluxRecord, AisleStateRecord, CompletedOrderRecord, ExtractionRecord, HtmWaitCause, PickRecord, PutawayRecord,
    VtmCycleRecord, VtmOperationRecord,
};
use crate::observer::robot_state::{Claim, role_of};
use crate::observer::station_diag::PipelineDepthRecord;
use crate::observer::time_slice::{
    BotLocation, ScalarCounters, ToteCreationRecord, ToteTransitRecord, ToteVisualState, standing_height_mm,
};

use super::activity::{DwellContainment, PendingDwell};
use super::finality::physical_inventory::{OccupancySlot, PhysicalChange};
use super::state::{SimObserverState, TripContext};

impl SimObserverState {
    /// Dispatch a single event to the appropriate handler.
    pub(super) fn dispatch_event(&mut self, event: &SimEvent) {
        match event {
            // A journey's nav is the only fact of where it ends, so the run's
            // horizon runs to the end of every journey: a run that completes
            // on a parking drive ends where the drive does.
            SimEvent::Nav(data) => {
                let arrival = data.waypoints.last().map_or(data.time, |waypoint| waypoint.end_time());
                self.latest_sim_time = self.latest_sim_time.max(data.time).max(arrival);
                self.handle_nav(data);
            }
            SimEvent::VtmNav(data) => {
                self.latest_sim_time = self.latest_sim_time.max(data.kind.end());
                self.handle_vtm_nav(data);
            }
            SimEvent::CycleComplete(data) => {
                self.latest_sim_time = self.latest_sim_time.max(SimTime::ms(cycle_busy_interval(data).1));
                self.handle_cycle_complete(data);
            }
            SimEvent::OrderStationPhase(data) => {
                self.latest_sim_time = self.latest_sim_time.max(data.at);
                self.handle_order_station_phase(data);
            }
            SimEvent::PresentationReadiness(data) => {
                self.latest_sim_time = self.latest_sim_time.max(data.ready_at);
                self.handle_presentation_readiness(data);
            }
            SimEvent::InventoryVisitPhase(data) => {
                self.latest_sim_time = self.latest_sim_time.max(data.at);
                self.handle_inventory_visit_phase(data);
            }
            SimEvent::Resource(data) => {
                self.latest_sim_time = self.latest_sim_time.max(SimTime::ms(data.sim_time_ms));
                self.handle_resource(data);
            }
            SimEvent::InitialPositions(data) => {
                self.handle_initial_positions(data);
            }
            SimEvent::OrderComplete(data) => {
                self.latest_sim_time = self.latest_sim_time.max(SimTime::ms(data.sim_time_ms));
                self.handle_order_complete(data);
            }
            SimEvent::RelayStateChange(data) => {
                self.latest_sim_time = self.latest_sim_time.max(data.time);
                self.handle_relay_state_change(data);
            }
            SimEvent::ToteCreated(data) => {
                self.latest_sim_time = self.latest_sim_time.max(data.time);
                self.handle_tote_created(data);
            }
            SimEvent::ToteContentsChanged(data) => {
                self.latest_sim_time = self.latest_sim_time.max(data.at);
                self.handle_tote_contents_changed(data);
            }
            SimEvent::ToteInitialStates(data) => {
                self.handle_tote_initial_states(data);
            }
            SimEvent::StripePlan(data) => {
                self.output.stripe_plan_log.push(data.clone());
            }
            SimEvent::ToteMoved(data) => {
                self.latest_sim_time = self.latest_sim_time.max(data.time);
                self.handle_tote_moved(data);
            }
            SimEvent::ToteTransit(data) => {
                self.latest_sim_time = self.latest_sim_time.max(data.end);
                self.handle_tote_transit(data);
            }
            SimEvent::TripStart(data) => {
                self.latest_sim_time = self.latest_sim_time.max(SimTime::ms(data.sim_time_ms));
                self.handle_trip_start(data);
            }
            SimEvent::PipelineSnapshot(data) => {
                self.latest_sim_time = self.latest_sim_time.max(SimTime::ms(data.sim_time_ms));
                self.handle_pipeline_snapshot(data);
            }
            SimEvent::OrderDispatch(data) => {
                self.order_station.insert(data.order_id, data.station_id);
            }
            SimEvent::VtmIdleReason(data) => {
                self.latest_sim_time = self.latest_sim_time.max(data.at);
                self.handle_vtm_idle_reason(data);
            }
            SimEvent::VtmCycle(data) => {
                self.latest_sim_time = self.latest_sim_time.max(SimTime::ms(data.sim_time_ms));
                self.handle_vtm_cycle(data);
            }
            SimEvent::VtmOperation(data) => {
                self.latest_sim_time = self.latest_sim_time.max(SimTime::ms(data.sim_time_ms));
                self.handle_vtm_operation(data);
            }
            SimEvent::InventoryDelta(data) => {
                self.latest_sim_time = self.latest_sim_time.max(data.at);
                self.handle_inventory_delta(data);
            }
            SimEvent::InventoryMove(data) => {
                self.latest_sim_time = self.latest_sim_time.max(data.at);
                self.handle_inventory_move(data);
            }
            SimEvent::HtmRelayPickup(data) => {
                self.latest_sim_time = self.latest_sim_time.max(data.end);
                self.handle_htm_relay_pickup(data);
            }
            SimEvent::HtmRelayDeposit(data) => {
                self.latest_sim_time = self.latest_sim_time.max(data.end);
                self.handle_htm_relay_deposit(data);
            }
            SimEvent::DivingBoardSlotWait(data) => {
                self.latest_sim_time = self.latest_sim_time.max(SimTime::ms(data.sim_time_ms));
                self.handle_diving_board_slot_wait(data);
            }
            SimEvent::HtmAssignment(data) => {
                self.latest_sim_time = self.latest_sim_time.max(data.dispatch_time);
                accumulate_dispatch(&mut self.slice_at(data.dispatch_time).scalars, data);
                self.handle_htm_assignment(data);
            }
            SimEvent::HtmArrival(data) => {
                self.latest_sim_time = self.latest_sim_time.max(data.arrival);
                self.handle_htm_arrival(data);
            }
            SimEvent::DispatchRejection(data) => {
                self.latest_sim_time = self.latest_sim_time.max(data.at);
                self.handle_dispatch_rejection(data);
            }
            SimEvent::BotPlanSnapshot(data) => {
                self.handle_bot_plan_snapshot(data);
            }
            // Navigation's holds give each slice its relay busy time as the
            // slice is made final; they move no robot.
            SimEvent::CellHeld(data) => self.note_cell_held(data),
            SimEvent::CellReleased(data) => self.note_cell_released(data),
            // Not a fact: `SimObserver::run` settles the work B has passed.
            SimEvent::BarrierAdvanced(_) => {}
            // Stub: not emitted until putaway station logic is available.
            SimEvent::Induction(_) => {}
            // Forensic audit events — recorded but not aggregated into metrics.
            SimEvent::HtmDispatch(_) | SimEvent::ArrivalError(_) | SimEvent::WatchdogTimeout(_) => {}
        }
    }

    fn handle_cycle_complete(&mut self, data: &CycleCompleteEvent) {
        let (busy_start_ms, busy_end_ms) = cycle_busy_interval(data);
        let busy_end = SimTime::ms(busy_end_ms);

        // Kind and station type together determine every counter touched.
        let stype = self.config.station_types.get(&data.station_id).copied();
        accumulate_cycle(&mut self.slice_at(busy_end).scalars, data, stype);

        // KPI #6: operator pick time at pick stations only (legacy `None`
        // station-type falls into Pick, matching `accumulate_cycle`). Booked
        // through the shared overlay helper — NOT in `accumulate_cycle` — so
        // the dwell window [sim_time_ms, sim_time_ms + dwell_ms) is clipped
        // per slice exactly like sibling overlays #4/#5 (`sim_time_ms` is the
        // dwell START; a lump-sum booking would let one slice's #6 exceed its
        // clipped Processing parent).
        if matches!((data.kind, stype), (CycleKind::Pick { .. }, Some(StationType::Pick) | None)) {
            self.add_htm_duty_overlay(
                SimTime::ms(data.sim_time_ms),
                SimTime::ms(data.sim_time_ms + data.dwell_ms),
                |s| &mut s.htm_duty_pick_at_station_ms,
            );
        }

        // Physical busy interval for this cycle. Pick fires at the dwell START
        // (span `[t, t+dwell)`); putaway's CycleComplete fires at `fill_at`,
        // the dwell END (the empty presented `[fill_at-dwell, fill_at)`). One
        // kind-aware interval, reused for the Gantt timeline, the inspector
        // Serving-vs-Idle span, and the hold journal's dwell bounds, so all
        // three count putaway HTM dwell where it physically happened.
        self.mark_dwell(
            data.robot_id,
            PendingDwell {
                presented: (SimTime::ms(busy_start_ms), SimTime::ms(busy_end_ms)),
                containment: match data.kind {
                    CycleKind::Pick { .. } | CycleKind::OrderLoad { .. } | CycleKind::OrderExtract { .. } => {
                        DwellContainment::FullWindow
                    }
                    CycleKind::Putaway { .. } => DwellContainment::EndOnly,
                },
                presentation_id: data.presentation_id,
            },
        );

        // Flag any pick that lands before the embargo lifts. The dispatch path
        // floors every admission at the embargo, so this must be 0; counting it
        // (qty-weighted, mirroring `picks_completed`) keeps a violation out of
        // the throughput numerator instead of letting it silently inflate
        // picks/hr (greymatter-foundry#208). Legacy `None` station-type counts
        // as Pick, matching `accumulate_cycle`.
        if let CycleKind::Pick { qty, .. } = data.kind
            && matches!(stype, Some(StationType::Pick) | None)
            && data.sim_time_ms < self.config.pick_embargo_ms
        {
            debug_assert!(
                false,
                "pick dwell started before embargo: station={:?} t={}ms embargo={}ms",
                data.station_id, data.sim_time_ms, self.config.pick_embargo_ms
            );
            let qty = qty as u64;
            self.slice_at(busy_end).scalars.pre_embargo_picks += qty;
        }

        // Per-station cycle span for the inspector's Serving-vs-Idle decision.
        self.record_station_cycle(data, busy_start_ms, busy_end_ms);

        // Per-event row for xlsx export. Kept in event order — no sort/dedup.
        match data.kind {
            CycleKind::Pick { sku, qty } => {
                if let Some(context) = data.tote_presentation {
                    // `order_station` is shared with pallet counting and is
                    // populated by `OrderDispatch` first; `or_insert` only
                    // backfills never-dispatched orders and must not overwrite
                    // the dispatch attribution.
                    self.order_station.entry(context.order_id).or_insert(data.station_id);
                    let order_dc = self.order_dc.entry(context.order_id).or_insert_with(|| data.dc_id.clone());
                    debug_assert_eq!(*order_dc, data.dc_id, "picks for order {:?} disagree on DC", context.order_id);
                }
                self.output.pick_log.push(PickRecord {
                    sim_time_ms: busy_end_ms,
                    station_id: data.station_id,
                    sku,
                    quantity: qty,
                    dc_id: data.dc_id.clone(),
                    order_id: data.tote_presentation.map(|context| context.order_id),
                    tote_id: data.tote_presentation.map(|context| context.tote_id.0),
                    source_zone_id: data.tote_presentation.map(|context| context.source_zone_id.0),
                    source_relay_id: data.tote_presentation.map(|context| context.source_relay_id),
                    presentation_id: data
                        .tote_presentation
                        .map(|context| context.presentation_id.0)
                        .or_else(|| data.presentation_id.map(|id| id.0)),
                });
            }
            CycleKind::Putaway { sku, qty } => {
                self.output.putaway_log.push(PutawayRecord {
                    sim_time_ms: data.sim_time_ms,
                    station_id: data.station_id,
                    sku,
                    quantity: qty,
                    presentation_id: data.presentation_id.map(|id| id.0),
                });
            }
            CycleKind::OrderExtract { qty } => {
                self.output.extraction_log.push(ExtractionRecord {
                    sim_time_ms: busy_end_ms,
                    station_id: data.station_id,
                    robot_id: data.robot_id,
                    quantity: qty,
                    presentation_id: data.presentation_id.map(|id| id.0),
                });
            }
            CycleKind::OrderLoad { .. } => {}
        }

        // Per-station operator-busy accumulation. Counted for both station
        // types — utilization is the binding-resource diagnostic, type
        // filtering is done at the consumer.
        *self.output.station_busy_ms.entry(data.station_id).or_insert(0) += data.dwell_ms;

        // Per-station timeline: one busy interval per cycle (kind-aware span).
        // Gaps between intervals are operator-cell idle and drive the Gantt
        // view's starvation diagnosis.
        self.output
            .station_timelines
            .entry(data.station_id)
            .or_default()
            .push(StationBusyInterval { start: SimTime::ms(busy_start_ms), end: SimTime::ms(busy_end_ms) });
        // Lazily mirror the config's station_type into output so renderers
        // can color lanes without needing access to MetricsConfig.
        if let Some(stype) = self.config.station_types.get(&data.station_id).copied() {
            self.output.station_types.entry(data.station_id).or_insert(stype);
        }

        // First-seen for active-window utilization. Pegged at the start of
        // the first dwell (kind-aware busy_start), not the end — so a
        // single-cycle station shows 100% over its own window rather than 0%.
        let start = SimTime::ms(busy_start_ms);
        self.output
            .station_first_seen
            .entry(data.station_id)
            .and_modify(|first| *first = (*first).min(start))
            .or_insert(start);
    }

    pub(super) fn finalize_station_gaps(&mut self) {
        self.output.station_gaps_ms.clear();
        for (&station, intervals) in &self.output.station_timelines {
            let mut ordered: Vec<_> = intervals.iter().collect();
            ordered.sort_unstable_by_key(|interval| (interval.start, interval.end));
            let Some(first) = ordered.first() else { continue };
            let mut busy_end = first.end;
            let mut gaps = Vec::new();
            for interval in ordered.into_iter().skip(1) {
                if interval.start >= busy_end {
                    gaps.push((interval.start - busy_end).as_ms());
                }
                // Nested or overlapping work cannot expose idle time before
                // the latest occupied end; keep the raw intervals for audits.
                busy_end = busy_end.max(interval.end);
            }
            if !gaps.is_empty() {
                self.output.station_gaps_ms.insert(station, gaps);
            }
        }
    }

    // handle_resource is in resource_handler.rs

    fn handle_initial_positions(&mut self, data: &InitialPositionsEvent) {
        let headings: FxHashMap<_, _> = data.htm_headings_rad.iter().copied().collect();
        let mut standing = Vec::with_capacity(data.positions.len());
        for &(robot_id, cell, robot_type, world_pos) in &data.positions {
            let heading = headings.get(&robot_id).map(|radians| *radians as f32);
            self.bot_positions.insert(robot_id.0, cell.0);
            self.bot_world_pos.insert(robot_id.0, world_pos);
            self.bot_types.insert(robot_id.0, robot_type);
            self.output.robot_types.insert(robot_id, robot_type);
            self.initial_bot_state.push((robot_id.0, cell.0, world_pos, robot_type, heading));
            if robot_type != RobotType::Vtm {
                let role = role_of(self.hold_position(cell));
                self.robot_state_log.stand(robot_id, SimTime::ZERO, role);
            }
            standing.push((
                robot_id.0,
                BotLocation {
                    motion: Some(Vec::new()),
                    start_cell: cell.0,
                    start_pos: world_pos,
                    start_z_mm: standing_height_mm(robot_type),
                    start_toward: None,
                    start_heading_rad: heading,
                    robot_type,
                    steps: Default::default(),
                },
            ));
        }
        self.slice_to_write(0).bot_locations.extend(standing);
    }

    fn handle_order_complete(&mut self, data: &OrderCompleteEvent) {
        let completed = SimTime::ms(data.sim_time_ms);
        self.slice_at(completed).scalars.orders_completed += 1;

        self.output.completed_order_log.push(CompletedOrderRecord {
            sim_time_ms: data.sim_time_ms,
            order_id: data.order_id,
            station_id: self.order_station.get(&data.order_id).copied(),
            dc_id: self.order_dc.get(&data.order_id).cloned().flatten(),
        });

        // Pallet count: order completed at a pick station = one pallet.
        if let Some(&station_id) = self.order_station.get(&data.order_id) {
            let stype = self.config.station_types.get(&station_id).copied();
            if matches!(stype, Some(StationType::Pick) | None) {
                self.slice_at(completed).scalars.throughput.pick_station_orders += 1;
            }
        }
    }

    fn handle_relay_state_change(&mut self, data: &RelayStateChangeEvent) {
        // Before the running set changes: a slice this makes exist starts
        // from the cells reserved before the change.
        let slice = self.slice_at(data.time);
        let offset_ms = data.time.as_ms() - slice.start_ms;
        slice.relay_state.transitions.push((data.cell_id, offset_ms, data.reserved));

        if data.reserved {
            self.reserved_relay_cells.insert(data.cell_id);
        } else {
            self.reserved_relay_cells.remove(&data.cell_id);
        }
    }

    /// Reservations retain physical rack custody until pickup. Drawing cells
    /// are a fallback only before an authoritative custody event is available.
    fn tote_rack_slot(&self, tote_id: u32) -> Option<OccupancySlot> {
        match self.tote_custody.get(&tote_id) {
            Some(ToteLocation::InRack { rack_id, z_index } | ToteLocation::Reserved { rack_id, z_index }) => {
                Some(OccupancySlot::Rack(*rack_id, *z_index))
            }
            Some(_) => None,
            None => match self.tote_states.get(&tote_id) {
                Some(ToteVisualState::InRack { cell_id, level }) => Some(OccupancySlot::VisualCell(*cell_id, *level)),
                _ => None,
            },
        }
    }

    /// Rack slot holding a laden tote; empty stored totes do not count as fill.
    pub(super) fn tote_fill_slot(&self, tote_id: u32) -> Option<OccupancySlot> {
        (self.tote_units.get(&tote_id).copied().unwrap_or(0) > 0).then(|| self.tote_rack_slot(tote_id)).flatten()
    }

    /// Move one tote's fill contribution from `old` slot to `new` slot. Slots
    /// are refcounted (`slot_laden`) so a transient double-book counts the
    /// slot once — fill can never exceed capacity.
    pub(super) fn move_fill(&mut self, old: Option<OccupancySlot>, new: Option<OccupancySlot>) {
        if old == new {
            return;
        }
        if let Some(slot) = old {
            if let Some(cnt) = self.slot_laden.get_mut(&slot) {
                *cnt -= 1;
                if *cnt == 0 {
                    self.slot_laden.remove(&slot);
                    self.rack_laden_slots = self.rack_laden_slots.saturating_sub(1);
                    if let Some(z) = slot.zone(&self.config) {
                        if let Some(n) = self.laden_slots_by_zone.get_mut(&z) {
                            *n = n.saturating_sub(1);
                        }
                    }
                }
            }
        }
        if let Some(slot) = new {
            let cnt = self.slot_laden.entry(slot).or_insert(0);
            *cnt += 1;
            if *cnt == 1 {
                self.rack_laden_slots += 1;
                if let Some(z) = slot.zone(&self.config) {
                    *self.laden_slots_by_zone.entry(z).or_insert(0) += 1;
                }
            }
        }
    }

    /// Rack slot plus empty/non-empty classification for one tote. Live tote
    /// contents are authoritative at the event boundary; a tote with no
    /// positive units is empty.
    pub(super) fn tote_storage_slot(&self, tote_id: u32) -> Option<(OccupancySlot, bool)> {
        self.tote_rack_slot(tote_id).map(|slot| (slot, self.tote_units.get(&tote_id).copied().unwrap_or(0) <= 0))
    }

    fn increment_refcount<K: Hash + Eq + Copy>(map: &mut FxHashMap<K, u32>, key: K) -> bool {
        let count = map.entry(key).or_default();
        *count += 1;
        *count == 1
    }

    fn decrement_refcount<K: Hash + Eq + Copy>(map: &mut FxHashMap<K, u32>, key: K) -> bool {
        let Some(count) = map.get_mut(&key) else {
            return false;
        };
        *count = count.saturating_sub(1);
        if *count == 0 {
            map.remove(&key);
            true
        } else {
            false
        }
    }

    fn increment_zone(map: &mut FxHashMap<u32, u32>, zone_id: u32) {
        *map.entry(zone_id).or_default() += 1;
    }

    fn decrement_zone(map: &mut FxHashMap<u32, u32>, zone_id: u32) {
        if let Some(count) = map.get_mut(&zone_id) {
            *count = count.saturating_sub(1);
        }
    }

    /// Move one tote's occupied-slot contribution and return the aisles whose
    /// state changed. Empty slots are derived as occupied minus non-empty,
    /// giving non-empty precedence during a transient mixed double-book.
    pub(super) fn move_storage_state(
        &mut self,
        old: Option<(OccupancySlot, bool)>,
        new: Option<(OccupancySlot, bool)>,
    ) -> BTreeSet<u32> {
        let mut affected = BTreeSet::new();
        let old_slot = old.map(|(slot, _)| slot);
        let new_slot = new.map(|(slot, _)| slot);
        if old_slot != new_slot {
            if let Some(slot) = old_slot {
                if Self::decrement_refcount(&mut self.slot_occupied, slot) {
                    if let Some(zone_id) = slot.zone(&self.config) {
                        Self::decrement_zone(&mut self.occupied_slots_by_zone, zone_id);
                        affected.insert(zone_id);
                    }
                }
            }
            if let Some(slot) = new_slot {
                if Self::increment_refcount(&mut self.slot_occupied, slot) {
                    if let Some(zone_id) = slot.zone(&self.config) {
                        Self::increment_zone(&mut self.occupied_slots_by_zone, zone_id);
                        affected.insert(zone_id);
                    }
                }
            }
        }

        if old != new {
            for slot in old_slot.into_iter().chain(new_slot) {
                if let Some(zone_id) = slot.zone(&self.config) {
                    affected.insert(zone_id);
                }
            }
        }
        affected
    }

    pub(super) fn relay_cell(state: Option<&ToteVisualState>) -> Option<u32> {
        match state {
            Some(ToteVisualState::AtRelay { cell_id }) => Some(*cell_id),
            _ => None,
        }
    }

    /// Move physical diving-board occupancy independently of reservation
    /// state, returning the affected aisles.
    pub(super) fn move_relay_tote(&mut self, old_cell: Option<u32>, new_cell: Option<u32>) -> BTreeSet<u32> {
        let mut affected = BTreeSet::new();
        if old_cell == new_cell {
            return affected;
        }
        if let Some(cell) = old_cell {
            if Self::decrement_refcount(&mut self.relay_totes, cell) {
                if let Some(&zone_id) = self.config.relay_cell_zone.get(&cell) {
                    Self::decrement_zone(&mut self.occupied_relays_by_zone, zone_id);
                    affected.insert(zone_id);
                }
            }
        }
        if let Some(cell) = new_cell {
            if Self::increment_refcount(&mut self.relay_totes, cell) {
                if let Some(&zone_id) = self.config.relay_cell_zone.get(&cell) {
                    Self::increment_zone(&mut self.occupied_relays_by_zone, zone_id);
                    affected.insert(zone_id);
                }
            }
        }
        affected
    }

    pub(super) fn stamp_aisle_state(&mut self, sim_time_ms: u64, zone_id: u32) {
        let storage_occupied = self.occupied_slots_by_zone.get(&zone_id).copied().unwrap_or(0);
        let storage_non_empty = self.laden_slots_by_zone.get(&zone_id).copied().unwrap_or(0);
        let storage_empty = storage_occupied.saturating_sub(storage_non_empty);
        let record = AisleStateRecord {
            sim_time_ms,
            zone_id,
            storage_capacity: self.config.zone_capacity.get(&zone_id).copied().unwrap_or(0),
            storage_occupied,
            storage_empty,
            storage_non_empty,
            storage_unknown: 0,
            diving_board_capacity: self.config.relay_zone_capacity.get(&zone_id).copied().unwrap_or(0),
            diving_board_occupied: self.occupied_relays_by_zone.get(&zone_id).copied().unwrap_or(0),
        };
        let unchanged = self.last_aisle_state.get(&zone_id).is_some_and(|prior| {
            prior.storage_capacity == record.storage_capacity
                && prior.storage_occupied == record.storage_occupied
                && prior.storage_empty == record.storage_empty
                && prior.storage_non_empty == record.storage_non_empty
                && prior.storage_unknown == record.storage_unknown
                && prior.diving_board_capacity == record.diving_board_capacity
                && prior.diving_board_occupied == record.diving_board_occupied
        });
        if !unchanged {
            self.last_aisle_state.insert(zone_id, record);
            self.output.aisle_state_log.push(record);
        }
    }

    /// Stamp every configured zone's aisle state at `sim_time_ms`. Also the
    /// t=0 baseline: the constructor calls this so handlers.rs is the ONLY
    /// producer of `AisleStateRecord`s.
    pub(super) fn stamp_all_aisles(&mut self, sim_time_ms: u64) {
        let zones: BTreeSet<_> =
            self.config.zone_capacity.keys().chain(self.config.relay_zone_capacity.keys()).copied().collect();
        for zone_id in zones {
            self.stamp_aisle_state(sim_time_ms, zone_id);
        }
    }

    /// Spread of fill % across all rack aisles right now: `(min, max, stddev)`
    /// in percentage points. Every aisle is included (an aisle with no filled
    /// slots contributes 0%), so a single drained aisle pulls `min` down even
    /// while the warehouse mean stays high. `(0, 0, 0)` when there are no
    /// aisles.
    fn aisle_fill_spread(&self) -> (f64, f64, f64) {
        let fills: Vec<f64> = self
            .config
            .zone_capacity
            .iter()
            .filter_map(|(&z, &cap)| {
                (cap > 0).then(|| 100.0 * self.laden_slots_by_zone.get(&z).copied().unwrap_or(0) as f64 / cap as f64)
            })
            .collect();
        if fills.is_empty() {
            return (0.0, 0.0, 0.0);
        }
        let min = fills.iter().copied().fold(f64::INFINITY, f64::min);
        let max = fills.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let mean = fills.iter().sum::<f64>() / fills.len() as f64;
        let variance = fills.iter().map(|f| (f - mean).powi(2)).sum::<f64>() / fills.len() as f64;
        (min, max, variance.sqrt())
    }

    /// Stamp the running rack-fill snapshot onto a slice. Called after any
    /// event that can change fill.
    pub(super) fn stamp_rack_fill(&mut self, slice_idx: usize) {
        // Keep touched zones even at 0 so a drain to empty is visible in the
        // per-aisle series (rather than the column silently disappearing).
        let per_zone: BTreeMap<u32, u32> = self.laden_slots_by_zone.iter().map(|(&z, &n)| (z, n)).collect();
        let filled = self.rack_laden_slots;
        let (min_pct, max_pct, stddev_pct) = self.aisle_fill_spread();
        let capacity = self.config.total_rack_slots;
        let slice_rack = &mut self.slice_to_finalize(slice_idx).scalars.rack;
        slice_rack.occupancy_total = filled;
        slice_rack.occupancy_peak = slice_rack.occupancy_peak.max(filled);
        slice_rack.capacity_total = capacity;
        slice_rack.per_zone_occupancy = per_zone;
        slice_rack.fill_min_pct = min_pct;
        slice_rack.fill_max_pct = max_pct;
        slice_rack.fill_stddev_pct = stddev_pct;
    }

    fn handle_tote_initial_states(&mut self, data: &ToteInitialStatesEvent) {
        for &(tote_id, ref state) in &data.totes {
            self.record_physical_change(SimTime::ZERO, PhysicalChange::Location(tote_id, state.clone()));
        }
        let tote_state = &mut self.slice_to_write(0).tote_state;
        tote_state.start_states = data.totes.iter().map(|(id, s)| (*id, s.clone())).collect();
        tote_state.order_totes = data.order_totes.clone();
    }

    fn handle_tote_created(&mut self, data: &ToteCreatedEvent) {
        let slice = self.slice_at(data.time);
        slice.tote_state.creations.push(ToteCreationRecord {
            offset_ms: data.time.as_ms() - slice.start_ms,
            tote_id: data.tote_id.0,
            tote_type: data.tote_type,
            designation: data.designation,
        });
        self.handle_tote_moved(&ToteMovedEvent {
            tote_id: data.tote_id,
            new_state: data.initial_state.clone(),
            time: data.time,
        });
    }

    fn handle_tote_moved(&mut self, data: &ToteMovedEvent) {
        let tid = data.tote_id.0;
        self.record_physical_change(data.time, PhysicalChange::Location(tid, data.new_state.clone()));

        let slice = self.slice_at(data.time);
        let offset_ms = data.time.as_ms() - slice.start_ms;
        slice.tote_state.transitions.push((offset_ms, tid, data.new_state.clone()));
    }

    fn handle_tote_transit(&mut self, data: &ToteTransitEvent) {
        let start_ms = data.start.as_ms();
        let end_ms = data.end.as_ms();
        if end_ms <= start_ms {
            return;
        }
        let record = ToteTransitRecord {
            tote_id: data.tote_id.0,
            from_cell: data.from_cell.0,
            to_cell: data.to_cell.0,
            start_ms,
            end_ms,
        };
        // The journey is reported once it has landed, so it is filed in the
        // slice holding its last travelling moment and in no earlier one:
        // those slices closed while the tote was still in the air. The record
        // states absolute instants, so a reader holding it interpolates over
        // whatever part of `start..end` its cursor sits in.
        let arrival_slice = ((end_ms - 1) / self.config.slice_width_ms) as usize;
        self.slice_to_write(arrival_slice).tote_state.transits.push(record);
    }

    fn handle_trip_start(&mut self, data: &TripStartEvent) {
        // Track the robot's current trip context so `handle_nav` can attribute
        // each nav event's distance/turns to its purpose and stamp the journal
        // row with the Trip ID and semantic endpoints.
        //
        // The leg's origin is the robot's previous leg destination (continuity:
        // the bot resumes from where it last stopped). The event's declared
        // `from` is the fallback — it seeds the first leg, before any
        // destination is recorded. If both are absent (true first leg with an
        // unknown origin, e.g. a direct-putaway chain), default to `Parking`.
        let robot = RobotId(data.robot_id.0);
        let from = self.last_trip_to.get(&robot).copied().or(data.from).unwrap_or(TripEndpoint::Parking);
        // The physical origin, read here because the `Nav` that follows moves
        // `bot_positions` on to the destination.
        let from_cell = self.bot_positions.get(&robot.0).copied().map(CellId);
        self.current_trip.insert(
            robot,
            TripContext { purpose: data.purpose, presentation_id: data.presentation_id, from, to: data.to },
        );
        self.last_trip_to.insert(robot, data.to);

        // Every purpose opens travel, a parking relocation included: driving
        // where the schedule sent the bot is work.
        self.mark_activity(
            robot,
            Milestone::Departure {
                at: SimTime::ms(data.sim_time_ms),
                nav: None,
                leaves_visit: data.purpose == TripPurpose::ToReturnRelay,
                from,
                from_cell,
                known_from: self.current_world_time(),
            },
        );
    }

    fn handle_vtm_cycle(&mut self, data: &VtmCycleEvent) {
        let filled = data
            .presentation_id
            .is_some_and(|id| self.output.putaway_log.iter().any(|fill| fill.presentation_id == Some(id.0)));
        if data.direction == VtmCycleDirection::Return && data.tote_source == ToteSource::Putaway && filled {
            let stored = SimTime::ms(data.sim_time_ms + data.total_cycle_ms);
            self.slice_at(stored).scalars.putaway_totes_stored += 1;
        }
        // Per-cycle log only — the empty/loaded/mechanical split is the raw
        // primitive. Per-slice sums and any hop-vs-penalty classification are
        // derived downstream (no opinionated thresholds baked in here).
        self.output.vtm_cycle_log.push(VtmCycleRecord {
            sim_time_ms: data.sim_time_ms,
            vtm_id: data.vtm_id.0,
            vtm_cycle_id: Some(data.vtm_cycle_id.0),
            zone_id: data.zone_id.0,
            direction: data.direction,
            empty_leg_ms: data.empty_leg_ms,
            loaded_leg_ms: data.loaded_leg_ms,
            fork_ms: data.fork_ms,
            buffer_ms: data.buffer_ms,
            rack_alignment_ms: data.rack_alignment_ms,
            relay_alignment_ms: data.relay_alignment_ms,
            relay_transfer_ms: data.relay_transfer_ms,
            total_cycle_ms: data.total_cycle_ms,
            relay_id: data.relay_id.0,
            rack_id: data.rack_id.0,
            tote_id: data.tote_id.0,
            presentation_id: data.presentation_id.map(|id| id.0),
            pre_cycle_wait_ms: data.pre_cycle_wait_ms,
            wait_reason: data.wait_reason,
            tote_source: data.tote_source,
        });
    }

    fn handle_vtm_operation(&mut self, data: &VtmOperationEvent) {
        self.output.vtm_operation_log.push(VtmOperationRecord {
            sim_time_ms: data.sim_time_ms,
            vtm_id: data.vtm_id.0,
            vtm_cycle_id: data.vtm_cycle_id.0,
            zone_id: data.zone_id.0,
            kind: data.kind,
            rack_id: data.rack_id.0,
            rack_level: Some(data.rack_level),
            relay_id: data.relay_id.0,
            tote_id: data.tote_id.0,
            payload: data.payload,
            presentation_id: data.presentation_id.map(|id| id.0),
        });
    }

    fn handle_pipeline_snapshot(&mut self, data: &PipelineSnapshotEvent) {
        let slice = self.slice_at(SimTime::ms(data.sim_time_ms));
        slice.scalars.pipeline.pipeline_depth_sum += data.scheduled_picks_count as u64;
        slice.scalars.pipeline.pipeline_snapshot_count += 1;
        // Per-station depth for the station inspector.
        slice.station_diag.pipeline_depths.push(PipelineDepthRecord {
            station_id: data.station_id,
            depth: data.scheduled_picks_count,
            at: SimTime::ms(data.sim_time_ms),
        });
    }

    fn handle_vtm_idle_reason(&mut self, data: &VtmIdleReasonEvent) {
        let kind = data.reason.kind();
        *self.slice_at(data.at).scalars.vtm_idle_reason_counts.entry(kind).or_insert(0) += 1;
        *self.output.vtm_idle_reason_counts.entry(kind).or_insert(0) += 1;

        // Per-VTM idle reason for the resource inspector. `Continuous` (no gap)
        // carries no diagnostic — skip it. Resolve the relay's HTM-access cell
        // so the record keys the same way a clicked relay cell does.
        if kind != VtmIdleReasonKind::Continuous {
            if let Some(&relay_cell) = self.config.relay_htm_access_cells.get(&data.relay_id.0) {
                self.record_vtm_idle_reason(data, relay_cell);
            } else {
                // Map miss = relay unknown to the solver-built table (empty in
                // unit tests). The inspector record is dropped; the KPI counts
                // above are unaffected.
                crate::trace_event!(reject, relay_access_cell_unmapped, relay = data.relay_id.0, at = data.at);
            }
        }

        // Journey-leg journal Wait row from the pre-cycle gap (additional output;
        // the counts above are unchanged). `Continuous` carries no gap → no row.
        self.push_vtm_wait_leg(data);
    }

    /// One fork-Get at a diving board. The KPI counters read its `wait`; the
    /// journal reads the dwell that preceded it; the interval path closes the
    /// leg that brought the bot here and books the operation itself.
    fn handle_htm_relay_pickup(&mut self, data: &HtmRelayPickupEvent) {
        // A resident bot was standing on the access cell when the pickup task
        // chose it, and the task is accepted at the fork-Get's own start, so
        // the stand before it is a gap: no task held the bot. The awaited
        // deposit row below journals that stand, and the gap rows leave it to
        // that row.
        let resident_since = data.resident.then(|| data.start - data.wait);
        self.mark_activity(
            data.robot_id,
            Milestone::Span {
                kind: ActivityKind::ForkOp,
                start: data.start,
                end: data.end,
                carries_after: Some(true),
                resident_since,
                known_from: self.current_world_time(),
            },
        );
        let committed_stand = if data.resident { SimDuration::ZERO } else { data.wait };
        let stand = (committed_stand, WaitCause::AwaitingDeposit);
        self.note_relay_operation(data.robot_id, data.relay_id, (data.start, data.end), RobotState::Pickup, stand);
        let wait_ms = data.wait.as_ms();
        let slice = self.slice_at(data.start);
        slice.scalars.htm_wait_at_relay_ms += wait_ms;
        slice.scalars.htm_relay_pickup_count += 1;

        // Per-relay pickup dwell for the relay inspector, keyed by the relay's
        // HTM-access cell (matching a clicked relay cell).
        if let Some(&relay_cell) = self.config.relay_htm_access_cells.get(&data.relay_id.0) {
            self.record_htm_relay_pickup(data, relay_cell);
        } else {
            // Same drop semantics as the idle-reason miss above.
            crate::trace_event!(reject, relay_access_cell_unmapped, relay = data.relay_id.0, at = data.start.as_ms());
        }

        // Journal the dwell before the fork-Get as an AwaitingDeposit Wait row
        // (additional output; the KPI counters above are unchanged). It spans
        // `[start − wait, start)`. A zero wait (on-time arrival) journals
        // nothing: no simtime passed before the fork-Get.
        if data.wait > SimDuration::ZERO {
            self.push_htm_awaiting_deposit_row(
                data.robot_id,
                data.relay_id,
                data.start - data.wait,
                data.wait,
                data.resident,
            );
        }
    }

    /// State a D30's fork operation at a relay in the robot state channel, and
    /// the stand on the relay's access cell its drive was committed with,
    /// `stand` long and held for its cause. A resident D30's stand before its
    /// pickup is no such stand: the task is committed once B has passed it, so
    /// it is the gap before the task, no task, and its caller passes none.
    fn note_relay_operation(
        &mut self,
        robot: RobotId,
        relay: RelayId,
        (start, end): (SimTime, SimTime),
        operation: RobotState,
        (stand, cause): (SimDuration, WaitCause),
    ) {
        self.robot_state_log.note(robot, (start, end), operation, Claim::Committed);
        let access = self.config.relay_htm_access_cells.get(&relay.0);
        let cell = access.and_then(|&cell| role_of(self.hold_position(CellId(cell))));
        self.robot_state_log.note(robot, (start - stand, start), RobotState::Waiting { cause, cell }, Claim::Committed);
    }

    /// Journal the return-slot dwell as an `AwaitingSlot` `Wait` row. The
    /// event fires at the deposit, so the dwell spans `[end − wait, end)` on
    /// the relay's access cell.
    ///
    /// This is the per-robot fact `DivingBoardSlotWaitEvent` cannot supply:
    /// that one is relay-attributed, carries no robot, and measures from the
    /// D30's readiness to leave the station — travel time included. The
    /// schedule does not delay the departure for a slot, so there is no
    /// origin-side wait to journal; the whole wait is here (#272's open
    /// question, resolved 2026-07-30).
    fn handle_htm_relay_deposit(&mut self, data: &HtmRelayDepositEvent) {
        self.mark_activity(
            data.robot_id,
            Milestone::Span {
                kind: ActivityKind::ForkOp,
                start: data.start,
                end: data.end,
                carries_after: Some(false),
                resident_since: None,
                known_from: self.current_world_time(),
            },
        );
        let stand = (data.wait, WaitCause::AwaitingSlot);
        self.note_relay_operation(data.robot_id, data.relay_id, (data.start, data.end), RobotState::Drop, stand);
        // A zero wait (the slot was valid on arrival) journals nothing: the
        // operation still happened, but no simtime passed before it.
        if data.wait > SimDuration::ZERO {
            self.push_htm_relay_dwell_row(
                data.robot_id,
                data.relay_id,
                data.start - data.wait,
                data.wait,
                HtmWaitCause::AwaitingSlot,
                // Still holding the tote: the deposit is what ends this dwell.
                true,
            );
        }
    }

    fn handle_diving_board_slot_wait(&mut self, data: &DivingBoardSlotWaitEvent) {
        // KPI counter only — no journal row. This wait is relay-attributed
        // (the event carries no robot_id) and measures from the D30's
        // readiness to leave the station, so it is not a robot interval;
        // `HtmRelayDeposit` carries the per-robot dwell instead.
        let slice = self.slice_at(SimTime::ms(data.sim_time_ms));
        slice.scalars.diving_board_slot_wait_ms += data.wait_ms;
        slice.scalars.diving_board_presentation_count += 1;
    }

    fn handle_tote_contents_changed(&mut self, data: &ToteContentsChangedEvent) {
        self.record_physical_change(data.at, PhysicalChange::Contents(data.tote_id.0, data.units));
    }

    fn handle_inventory_delta(&mut self, data: &InventoryDeltaEvent) {
        self.record_physical_change(data.at, PhysicalChange::InventoryDelta(data.tote_id.0, data.delta));
    }

    /// Aisle (relay zone) a tote location belongs to, if any. `InRack`/
    /// `Reserved` resolve via the rack→zone map; `AtRelay` via the relay→zone
    /// map. `WithRobot` (in transit), `OnConveyor` (riding a station's belt,
    /// which stands in no rack zone) and `Uninitialized` belong to no aisle --
    /// which is also what makes a loop ride read as transit to the
    /// aisle-crossing ledger below, rather than as a leave-and-return.
    fn location_aisle(&self, loc: &ToteLocation) -> Option<u32> {
        match loc {
            ToteLocation::InRack { rack_id, .. } | ToteLocation::Reserved { rack_id, .. } => {
                self.config.rack_id_zone.get(&rack_id.0).copied()
            }
            ToteLocation::AtRelay { relay_id } => self.config.relay_zones.get(relay_id).copied(),
            ToteLocation::WithRobot { .. }
            | ToteLocation::OnConveyor { .. }
            | ToteLocation::OnConveyance { .. }
            | ToteLocation::AtCell { .. }
            | ToteLocation::Extracted
            | ToteLocation::Uninitialized => None,
        }
    }

    /// Per-aisle inventory flux: a row per SKU only when a tote *settles* in a
    /// different aisle (relay zone) than it was last settled in.
    ///
    /// A move is not a crossing: totes traverse `WithRobot` (which maps to no
    /// aisle) between every settled position, so a naïve from/to comparison
    /// reports a leave-and-return for every hop of a same-aisle round trip. We
    /// instead track each tote's last settled aisle (`last_settled_aisle`) and
    /// compare against it: a move *into* an aisle that differs from where the
    /// tote last settled is a genuine crossing; a move into transit is
    /// remembered but not logged; a move back into the same aisle is a no-op.
    ///
    /// The crossing is valued at what the tote carries when it settles, so the
    /// leave (−) and enter (+) balance — units picked mid-transit left via a
    /// station, not into another aisle, and are captured by the pick metrics.
    /// Empty totes and moves to/from no aisle produce no rows.
    fn handle_inventory_move(&mut self, data: &InventoryMoveEvent) {
        let tote_id = data.tote_id.0;
        self.record_physical_change(data.at, PhysicalChange::Custody(tote_id, data.to));
        // Seed the last-settled aisle from the departure location the first
        // time we see this tote, so a crossing that opens with a transit hop
        // still knows which aisle the tote left.
        if let Some(from_aisle) = self.location_aisle(&data.from) {
            self.last_settled_aisle.entry(tote_id).or_insert(from_aisle);
        }
        // A move into transit (or nowhere) is not a settle: keep the last
        // settled aisle so the eventual arrival can be compared against it.
        let Some(to_aisle) = self.location_aisle(&data.to) else {
            return;
        };
        let prev_aisle = self.last_settled_aisle.insert(tote_id, to_aisle);
        if prev_aisle == Some(to_aisle) {
            return; // settled back into the same aisle — no boundary crossed
        }
        let sim_time_ms = data.at.as_ms();
        for &(sku, qty) in &data.contents {
            if qty == 0 {
                continue;
            }
            // Leaving the prior aisle (−qty, only if there was one), entering
            // the new one (+qty).
            for (aisle, delta) in [(prev_aisle, -(qty as i32)), (Some(to_aisle), qty as i32)] {
                if let Some(zone_id) = aisle {
                    self.output.aisle_flux_log.push(AisleFluxRecord { sim_time_ms, zone_id, tote_id, sku, delta });
                }
            }
        }
    }
}

/// The physical busy interval `[start_ms, end_ms)` a cycle occupies its
/// station. Pick's `CycleComplete` fires at the dwell START, so the operator
/// picks over `[t, t+dwell)`. Putaway's fires at `fill_at` (the dwell END —
/// the empty presented from `fill_at-dwell` until the fill), so its span is
/// `[t-dwell, t)`. This is the one place the two conventions are reconciled;
/// completion counts use the resulting physical end.
fn cycle_busy_interval(data: &CycleCompleteEvent) -> (u64, u64) {
    match data.kind {
        CycleKind::Pick { .. } | CycleKind::OrderLoad { .. } | CycleKind::OrderExtract { .. } => {
            (data.sim_time_ms, data.sim_time_ms + data.dwell_ms)
        }
        CycleKind::Putaway { .. } => (data.sim_time_ms.saturating_sub(data.dwell_ms), data.sim_time_ms),
    }
}

/// Apply one CycleComplete event to the counters of the slice it lands in.
/// Every *event-count*
/// counter touched by a cycle lives in one of these match arms so the
/// kind/station pairing is decided in one place. The one exception is the
/// KPI #6 duration overlay (`htm_duty_pick_at_station_ms`), booked in
/// `handle_cycle_complete` via `add_htm_duty_overlay` because durations
/// clip per slice while counts land in the event's slice.
fn accumulate_cycle(scalars: &mut ScalarCounters, data: &CycleCompleteEvent, stype: Option<StationType>) {
    scalars.presentations += 1;
    match (data.kind, stype) {
        // Legacy `None` station-type falls into Pick to preserve prior behavior.
        (CycleKind::Pick { qty, .. }, Some(StationType::Pick) | None) => {
            let qty = qty as u64;
            scalars.picks_completed += qty;
            scalars.pick_presentations += 1;
            *scalars.picks_per_station.entry(data.station_id).or_insert(0) += qty;
            scalars.throughput.pick_station_picks += qty;
            scalars.throughput.pick_station_presentations += 1;
        }
        (CycleKind::Putaway { qty, .. }, Some(StationType::Putaway)) => {
            let qty = qty as u64;
            scalars.putaways_completed += qty;
            scalars.putaway_presentations += 1;
            scalars.throughput.putaway_station_picks += qty;
            scalars.throughput.putaway_station_presentations += 1;
            match data.arrival_source {
                Some(StationArrivalSource::PutawayDirect) => {
                    scalars.throughput.putaway_station_presentations_direct += 1;
                }
                Some(StationArrivalSource::PutawayFromRack) => {
                    scalars.throughput.putaway_station_presentations_from_rack += 1;
                }
                Some(StationArrivalSource::PickFromRack) => {
                    debug_assert!(false, "pick arrival_source on putaway cycle: station={:?}", data.station_id);
                }
                None => {
                    debug_assert!(false, "putaway cycle missing arrival_source: station={:?}", data.station_id);
                }
            }
        }
        // Extraction removes already-picked contents; it never counts as
        // another pick or as inbound stock.
        (CycleKind::OrderLoad { .. }, Some(StationType::Pick) | None) => {
            scalars.order_load_presentations += 1;
        }
        (CycleKind::OrderExtract { qty }, Some(StationType::Extraction)) => {
            scalars.order_extract_presentations += 1;
            scalars.order_extracted_units += u64::from(qty);
        }
        (kind, stype) => {
            debug_assert!(false, "cycle kind/station type mismatch: station={:?} kind={:?}", stype, kind);
        }
    }
}

// Dispatch-side complement of the presentation split above, counted when the
// HTM starts moving (`dispatch_time`): the dispatch and presentation pairs
// bracket the in-flight pipeline per source. Pick-chain assignments are not
// putaway staging and stay out.
fn accumulate_dispatch(scalars: &mut ScalarCounters, data: &HtmAssignmentEvent) {
    match data.source {
        StationArrivalSource::PutawayDirect => scalars.throughput.putaway_dispatches_direct += 1,
        StationArrivalSource::PutawayFromRack => scalars.throughput.putaway_dispatches_from_rack += 1,
        StationArrivalSource::PickFromRack => {}
    }
}
