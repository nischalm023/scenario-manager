//! VTM scheduling for assigned relay-zone aisles. A Cencora chassis rides
//! the racking; a Walmart chassis drives on the floor. Each robot retains its
//! catalog motion model. This service evaluates straight aisle cycles and
//! tracks endpoints in world coordinates; it does not reserve free-floor
//! paths or model floor turns/collisions.
//!
//! Each VTM owns a model, a zone assignment, its scheduled endpoint, and the
//! `available_at` frontier: earliest simtime at which its next cycle may begin.
//!
//! Selection rule for a forward (rack→relay) or return (relay→rack)
//! cycle, given a relay reservation pinned at `relay_start`. The anchor
//! offset is the leg up to the relay-side fork: the full cycle for a forward
//! (the fork is the final deposit at the relay) but only the first travel
//! leg for a return (the fork is the pickup, after which the VTM still hauls
//! to the rack). Anchoring the full cycle for a return would schedule the
//! pickup a whole cycle before the tote was deposited.
//!
//! ```text
//! for each VTM:
//!     anchor      = forward ? cycle_duration : first_travel
//!     vtm_start   = max(available_at, relay_start - anchor)
//!     vtm_arrival = vtm_start + cycle_duration       // always ≥ relay_start
//!
//! pick min by (vtm_arrival, avail_key, vtm_id), where the `avail_key`
//! tie-break is DIRECTION-DEPENDENT:
//!   - earliest arrival     → smallest gap inside the reservation
//!   - avail_key            → Forward: oldest `available_at` (load-balance,
//!                            keep cold VTMs warm); Return: SMALLEST gap
//!                            (largest `available_at` still ≤ the JIT start)
//!                            so a return grabs a just-freed VTM instead of
//!                            stranding a long-idle one, leaving early-free
//!                            VTMs for pick forwards
//!   - lowest id            → determinism on full ties
//! ```
//!
//! The selector never declines: every non-empty zone produces a valid
//! assignment. Reservation `start` was already chosen by the demand
//! module to match relay-slot availability; this service only finds the
//! VTM that fills that slot with the smallest arrival gap.
//!
//! Cycles are claimed through candidacies, not grabs: `VtmForward` /
//! `VtmReturn` propose a [`ProposedVtmCycle`] holding a selection and
//! the chosen VTM's frontier generation as its identity premise. A
//! rival commit on that VTM bumps the generation, invalidating the
//! candidacy (the scheduler evicts it and the owner re-selects);
//! acceptance — when the barrier reaches the cycle's start — commits
//! the cycle in place.

mod acceptance_stats;

use crate::model::RackId;
use acceptance_stats::VtmAcceptanceStats;
use desgraph::CellId;
use rustc_hash::FxHashMap;
use std::sync::{Arc, Mutex as StdMutex, OnceLock, RwLock};

use desgraph::{RelayId, WorldPos, ZoneId};

use std::fmt;

use super::config::RelayZones;
use crate::config::VtmConfig;
use crate::config::VtmIsolationMode;
use crate::events::VtmEndpoint;
use crate::model::ids::{RackSlot, RobotId};
use crate::model::{SimDuration, SimTime};
use crate::robots::vtm::model::VtmMotion;
use crate::robots::vtm::motion::CycleBreakdown;
use crate::scenario_gen::ScenarioData;
use crate::scheduler::task::proposal::{AcceptError, ProposedReservation};
use crate::signals::Mut;

/// Snapshot of a single VTM's frontier — id, when it next becomes free,
/// and where it'll be at that moment.
#[derive(Clone, Debug)]
pub struct VtmFrontier {
    pub id: RobotId,
    pub available_at: SimTime,
    pub endpoint: VtmEndpoint,
    /// Frontier generation: bumped on every commit. The identity premise
    /// a tentative VTM reservation records at craft time — a promoted
    /// rival cycle on the same VTM bumps it, invalidating the tentative
    /// (task-model-v2: identity premises are claim dimensions, not
    /// ambient reads). Nothing consumes it until proposals arrive; the
    /// counter exists so they have a premise to record.
    pub generation: u64,
}

/// Result of selecting a VTM for a forward or return cycle.
///
/// `vtm_start` is the time the VTM begins its cycle. It is at least
/// `vtm.available_at` and may be later than that to align the relay-side
/// fork with `relay_start` — for a forward the fork is the deposit at
/// completion; for a return it is the pickup after the first travel leg.
/// `vtm_arrival = vtm_start +
/// cycle_duration`, always `≥ relay_start`. `from_endpoint` is the
/// VTM's current position at `vtm_start` (used by the caller to
/// emit the `RailMove` FROM event). `available_at` is exposed so the
/// caller can classify the pre-cycle idle gap: `vtm_start == available_at`
/// → `Continuous`; otherwise the caller attributes `gap = vtm_start −
/// available_at` to its constraining cause (forward floor or HTM deposit).
#[derive(Clone, Debug)]
pub struct VtmAssignment {
    pub vtm_id: RobotId,
    pub vtm_start: SimTime,
    pub vtm_arrival: SimTime,
    pub cycle: CycleBreakdown,
    /// Geometric cycle for traces/xls; equals `cycle` except under Teleport
    /// (instant stub). Never drives scheduling. Emit sites derive divergence
    /// as `geo_cycle != cycle`.
    pub geo_cycle: CycleBreakdown,
    pub cycle_duration: SimDuration,
    pub from_endpoint: VtmEndpoint,
    /// Interpolated rack-slot position (`rack_world_pos[slot]`) and relay
    /// board-0 exchange position used for this cycle's distances. The task
    /// docks the VTM at these exact positions so the next cycle's first-leg
    /// distance is measured from where this one actually ended — not the
    /// grid cell center.
    pub slot_pos: WorldPos,
    pub relay_pos: WorldPos,
    /// The VTM's `available_at` at the moment of selection.
    pub available_at: SimTime,
    /// The VTM's frontier generation at selection — the identity premise a
    /// [`ProposedVtmCycle`] records (see [`VtmFrontier::generation`]).
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    Forward,
    Return,
}

#[derive(Clone, Debug)]
struct VtmState {
    motion: Arc<VtmMotion>,
    endpoint: VtmEndpoint,
    available_at: SimTime,
    /// See [`VtmFrontier::generation`].
    generation: u64,
}

/// One published cycle candidacy, as the service sees it: which VTM it
/// named and the frontier generation it recorded. Registered when the
/// candidacy is crafted and dropped when it is evicted or accepted, so
/// counting the ones whose premise still holds gives the live choice depth
/// on a VTM.
struct CycleTentative {
    id: u64,
    vtm_id: RobotId,
    generation: u64,
}

struct State {
    vtms: FxHashMap<RobotId, VtmState>,
    /// Zone → RobotIds in registration order.
    zone_index: FxHashMap<ZoneId, Vec<RobotId>>,
    tentatives: Vec<CycleTentative>,
    next_tentative_id: u64,
}

struct Inner {
    zones: Arc<RelayZones>,
    rack_cells: FxHashMap<RackId, CellId>,
    relay_cells: FxHashMap<RelayId, CellId>,
    state: RwLock<State>,
    /// The scheduler's barrier clock, when wired: cycle starts floor at
    /// the settled frontier — a cycle is a new admission, and filling a
    /// VTM's idle gap behind B would be gap-filling, which we don't do.
    admission_floor: OnceLock<Mut<SimTime>>,
    /// VTM isolation mode (diagnostic lever; `Off` in production).
    isolation: VtmIsolationMode,
    /// Flat per-cycle time used when `isolation == IdealRate`. Derived from the
    /// motion calibration (or the `vtm.ideal_cycle_secs` override) at build.
    /// Unused for `Off`/`Teleport`.
    ideal_cycle_secs: f64,
    accept_stats: StdMutex<VtmAcceptanceStats>,
}

/// Synthesize a flat [`CycleBreakdown`] of `total_secs`, with the empty first
/// leg zeroed and all time charged to the loaded leg. Keeps the downstream
/// leg-time reads (forward deposit at completion, return pickup after the
/// first leg) internally consistent under an isolation mode. See
/// [`VtmIsolationMode`].
fn flat_cycle(total_secs: f64) -> CycleBreakdown {
    CycleBreakdown {
        first_travel_secs: 0.0,
        second_travel_secs: total_secs,
        travel_secs: total_secs,
        h_travel_secs: total_secs,
        v_travel_secs: 0.0,
        rack_alignment_secs: 0.0,
        rack_transfer_secs: 0.0,
        relay_alignment_secs: 0.0,
        relay_transfer_secs: 0.0,
        total_secs,
    }
}

/// VTM scheduling service. Cheap to clone (Arc bump).
#[derive(Clone)]
pub struct VtmService(Arc<Inner>);

impl VtmService {
    pub fn rack_cell(&self, rack: RackId) -> Option<CellId> {
        self.0.rack_cells.get(&rack).copied()
    }
    pub fn relay_cell(&self, relay: RelayId) -> Option<CellId> {
        self.0.relay_cells.get(&relay).copied()
    }

    /// Build a fully-populated VtmService from scenario data: zones come
    /// from `data.relay_zones`, and one VTM is registered per entry in
    /// `data.vtm_zone_assignments` using `data.vtm_initial_positions` and its
    /// validated runtime model binding.
    /// Initial endpoints are `Aisle`, before any slot or relay operation.
    pub fn new(data: &ScenarioData, vtm_cfg: &VtmConfig) -> Self {
        let zones = Arc::new(data.relay_zones.clone());
        let ideal_cycle_secs = if vtm_cfg.isolation == VtmIsolationMode::IdealRate {
            vtm_cfg.ideal_cycle_secs.unwrap_or_else(|| {
                zones.mean_forward_cycle_secs().unwrap_or_else(|| {
                    panic!(
                        "vtm.isolation=ideal_rate but no zone yields a representative cycle; set vtm.ideal_cycle_secs"
                    )
                })
            })
        } else {
            0.0
        };
        if vtm_cfg.isolation != VtmIsolationMode::Off {
            let detail = match vtm_cfg.isolation {
                VtmIsolationMode::IdealRate => {
                    let src =
                        if vtm_cfg.ideal_cycle_secs.is_some() { "override" } else { "derived fleet-mean forward" };
                    format!("ideal_cycle={ideal_cycle_secs:.1}s ({src})")
                }
                VtmIsolationMode::Teleport => "totes arrive on demand, capacity ignored".to_string(),
                VtmIsolationMode::Off => unreachable!(),
            };
            eprintln!("[vtm] isolation={:?} — {detail}", vtm_cfg.isolation);
        }
        let svc = Self::with_zones_isolation(
            zones,
            vtm_cfg.isolation,
            ideal_cycle_secs,
            data.rack_cells.iter().map(|(&id, &cell)| (id, cell)).collect(),
            data.relay_data.iter().map(|relay| (relay.relay_id, relay.vtm_access)).collect(),
        );
        for (i, zones_set) in data.vtm_zone_assignments.iter().enumerate() {
            let start = data.vtm_initial_positions[i];
            let cell = start.anchor();
            let pos = start
                .world_pos(&data.map.graph)
                .unwrap_or_else(|| panic!("vtm {i} initial position {start:?} has no world position"));
            assert_eq!(zones_set.len(), 1, "vtm {i} requires exactly one assigned aisle zone");
            let zone_id = *zones_set.first().expect("single zone checked above");
            let motion = data.robot_models.vtm_for_robot(i).unwrap_or_else(|error| panic!("{error}"));
            svc.register_with_model(RobotId(i as u32), zone_id, VtmEndpoint::Aisle { cell, pos }, motion);
        }
        svc
    }

    /// Construct an empty service backed by the given zone configs. VTMs
    /// must be registered separately via [`Self::register`]. Used by
    /// [`Self::new`] and by unit tests that build their own zone fixtures.
    #[cfg(test)]
    pub(super) fn with_zones(zones: Arc<RelayZones>) -> Self {
        Self::with_zones_isolation(zones, VtmIsolationMode::Off, 0.0, FxHashMap::default(), FxHashMap::default())
    }

    fn with_zones_isolation(
        zones: Arc<RelayZones>,
        isolation: VtmIsolationMode,
        ideal_cycle_secs: f64,
        rack_cells: FxHashMap<RackId, CellId>,
        relay_cells: FxHashMap<RelayId, CellId>,
    ) -> Self {
        Self(Arc::new(Inner {
            zones,
            rack_cells,
            relay_cells,
            state: RwLock::new(State {
                vtms: FxHashMap::default(),
                zone_index: FxHashMap::default(),
                tentatives: Vec::new(),
                next_tentative_id: 0,
            }),
            admission_floor: OnceLock::new(),
            isolation,
            ideal_cycle_secs,
            accept_stats: StdMutex::new(VtmAcceptanceStats::default()),
        }))
    }

    /// Bind a robot's immutable model to its scheduled endpoint. Catalog
    /// construction validates these bindings before this service is created.
    pub fn register_with_model(
        &self,
        id: RobotId,
        zone_id: ZoneId,
        initial_endpoint: VtmEndpoint,
        motion: Arc<VtmMotion>,
    ) {
        let mut st = self.0.state.write().unwrap();
        assert!(!st.vtms.contains_key(&id), "VTM {id:?} registered twice");
        st.vtms.insert(id, VtmState { motion, endpoint: initial_endpoint, available_at: SimTime::ZERO, generation: 0 });
        st.zone_index.entry(zone_id).or_default().push(id);
    }

    /// Small legacy fixtures explicitly use their zone's sole model. Runtime
    /// construction always passes the robot binding to `register_with_model`.
    #[cfg(test)]
    fn register(&self, id: RobotId, zone_id: ZoneId, initial_endpoint: VtmEndpoint) {
        let zones = self.0.zones.zone_map();
        let models = &zones[&zone_id].vtm_models;
        assert_eq!(models.len(), 1, "legacy test registration needs exactly one model");
        self.register_with_model(id, zone_id, initial_endpoint, models[0].clone());
    }

    /// Frontier snapshot for every VTM in a zone, in registration order.
    pub fn frontier_in_zone(&self, zone: ZoneId) -> Vec<VtmFrontier> {
        let st = self.0.state.read().unwrap();
        let Some(ids) = st.zone_index.get(&zone) else {
            return Vec::new();
        };
        ids.iter()
            .filter_map(|id| {
                st.vtms.get(id).map(|s| VtmFrontier {
                    id: *id,
                    available_at: s.available_at,
                    endpoint: s.endpoint.clone(),
                    generation: s.generation,
                })
            })
            .collect()
    }

    /// Pick the best VTM to forward-cycle (rack → relay) for a relay
    /// reservation pinned at `relay_start`. Returns `None` if the zone
    /// has no registered VTMs.
    pub fn select_for_forward(
        &self,
        zone_id: ZoneId,
        relay_id: RelayId,
        rack_slot: RackSlot,
        relay_start: SimTime,
    ) -> Option<VtmAssignment> {
        self.select(Direction::Forward, zone_id, relay_id, rack_slot, relay_start)
    }

    /// Pick the best VTM to return-cycle (relay → rack). Same selection
    /// rule as [`Self::select_for_forward`].
    pub fn select_for_return(
        &self,
        zone_id: ZoneId,
        relay_id: RelayId,
        rack_slot: RackSlot,
        relay_start: SimTime,
    ) -> Option<VtmAssignment> {
        self.select(Direction::Return, zone_id, relay_id, rack_slot, relay_start)
    }

    /// Advance a VTM's frontier after it commits to a cycle. The caller
    /// (VtmForward / VtmReturn task) owns the cycle commitment and ends
    /// at `complete_endpoint` (typically `Slot` for Return, `Relay` for
    /// Forward). VtmService records the new frontier and refuses to move
    /// it backwards: the frontier is a committed fact, and a commit
    /// landing before it means a caller planned against a stale
    /// snapshot — a bug the old blind overwrite would have silently
    /// laundered into overlapping cycles (task-model-v2 temporal model:
    /// committed times are authoritative, one open reservation per
    /// resource). Both refusals are the sim harness's crash disposition
    /// of an invariant violation.
    pub fn commit(&self, vtm_id: RobotId, complete_at: SimTime, complete_endpoint: VtmEndpoint) {
        let mut st = self.0.state.write().unwrap();
        let s = st.vtms.get_mut(&vtm_id).unwrap_or_else(|| panic!("VtmService::commit for unregistered {vtm_id:?}"));
        // Teleport declares infinite capacity: selection starts every cycle at
        // relay_start regardless of the frontier, so commits land in arbitrary
        // time order and the monotonic-frontier premise does not apply. Record
        // the endpoint for the geometric traces and skip the refusal.
        if self.0.isolation == VtmIsolationMode::Teleport {
            s.endpoint = complete_endpoint;
            s.generation += 1;
            return;
        }
        assert!(
            complete_at >= s.available_at,
            "VtmService::commit moves {vtm_id:?} frontier backwards: {} -> {} (endpoint {:?})",
            s.available_at,
            complete_at,
            complete_endpoint,
        );
        s.available_at = complete_at;
        s.endpoint = complete_endpoint;
        s.generation += 1;
    }

    /// Wire the scheduler's barrier clock (see `Inner::admission_floor`).
    pub fn set_admission_floor(&self, clock: Mut<SimTime>) {
        let _ = self.0.admission_floor.set(clock);
    }

    fn admission_floor(&self) -> SimTime {
        self.0.admission_floor.get().map_or(SimTime::ZERO, |clock| clock.value())
    }

    /// Whether a cycle candidacy's identity premise still holds: the
    /// chosen VTM's frontier has not moved since selection. Teleport
    /// declares infinite capacity — selection ignores frontiers there,
    /// so a rival commit invalidates nothing.
    pub fn cycle_premise_holds(&self, vtm_id: RobotId, generation: u64) -> bool {
        if self.0.isolation == VtmIsolationMode::Teleport {
            return true;
        }
        let st = self.0.state.read().unwrap();
        st.vtms.get(&vtm_id).is_some_and(|s| s.generation == generation)
    }

    /// Register a published cycle candidacy so the family can count the
    /// live choice depth on its VTM. Returns the handle that deregisters it.
    fn register_cycle_tentative(&self, vtm_id: RobotId, generation: u64) -> u64 {
        let mut st = self.0.state.write().unwrap();
        let id = st.next_tentative_id;
        st.next_tentative_id += 1;
        st.tentatives.push(CycleTentative { id, vtm_id, generation });
        id
    }

    fn deregister_cycle_tentative(&self, id: u64) {
        let mut st = self.0.state.write().unwrap();
        st.tentatives.retain(|t| t.id != id);
    }

    /// How many candidacies naming `vtm_id` still hold their premise — the
    /// asking one included, so an uncontended acceptance reads 1.
    fn live_cycle_depth(&self, vtm_id: RobotId) -> usize {
        let teleport = self.0.isolation == VtmIsolationMode::Teleport;
        let st = self.0.state.read().unwrap();
        st.tentatives
            .iter()
            .filter(|t| t.vtm_id == vtm_id)
            .filter(|t| teleport || st.vtms.get(&t.vtm_id).is_some_and(|s| s.generation == t.generation))
            .count()
    }

    fn record_cycle_accept(&self, depth: usize) {
        if let Ok(mut stats) = self.0.accept_stats.lock() {
            stats.record_accept(depth);
        }
    }

    /// The VTM family's end-of-run laziness audit; `None` when no cycle was
    /// accepted.
    pub fn acceptance_report(&self) -> Option<String> {
        self.0.accept_stats.lock().ok().and_then(|stats| stats.report())
    }

    fn select(
        &self,
        direction: Direction,
        zone_id: ZoneId,
        relay_id: RelayId,
        rack_slot: RackSlot,
        relay_start: SimTime,
    ) -> Option<VtmAssignment> {
        let st = self.0.state.read().unwrap();
        let zone_map = self.0.zones.zone_map();
        let zone = zone_map.get(&zone_id)?;
        let ids = st.zone_index.get(&zone_id)?;
        if ids.is_empty() {
            return None;
        }

        // Tripwire: VTM cycles are side-bound. The inventory index pairs only
        // same-side (relay, slot) candidates by construction; this asserts no
        // caller has constructed a cross-aisle pair some other way.
        debug_assert!(
            zone.relays.iter().find(|r| r.id == relay_id).map(|r| r.side)
                == zone.rack_side.get(&rack_slot.rack_id).copied(),
            "VTM cycle pair crosses the aisle: relay {relay_id:?} side {:?}, rack {:?} side {:?}",
            zone.relays.iter().find(|r| r.id == relay_id).map(|r| r.side),
            rack_slot.rack_id,
            zone.rack_side.get(&rack_slot.rack_id),
        );

        // Geometric endpoints for this (relay, slot) pair — the same positions
        // the cycle distances use. The task docks the VTM at these, so the
        // next cycle measures its first leg from where this one ended.
        let slot_pos = *zone.rack_world_pos.get(&rack_slot.rack_id)?;
        let relay_pos = zone.relays.iter().find(|r| r.id == relay_id)?.world_pos;
        let target_pos = match direction {
            // Forward starts with the VTM travelling to the rack to fork
            // up the tote, so the first leg's distance is VTM→rack.
            Direction::Forward => slot_pos,
            // Return starts with the VTM travelling to the relay to fork
            // up the tote, so the first leg's distance is VTM→relay.
            Direction::Return => relay_pos,
        };

        ids.iter()
            .filter_map(|id| st.vtms.get(id).map(|s| (*id, s)))
            .map(|(id, s)| {
                let h_dist = zone.h_dist_between_cells_m(s.endpoint.pos(), target_pos);
                // Real geometric cycle — the physical geometric timing, computed
                // regardless of isolation mode. It is the committed cycle in Off
                // mode and the `geo_cycle` (geometric traces) under Teleport.
                let geometric = match direction {
                    Direction::Forward => zone.forward_cycle_for_model(relay_id, rack_slot, h_dist, &s.motion),
                    Direction::Return => zone.return_cycle_for_model(relay_id, rack_slot, h_dist, &s.motion),
                };
                let cycle = match self.0.isolation {
                    VtmIsolationMode::Off => geometric.clone(),
                    VtmIsolationMode::IdealRate => flat_cycle(self.0.ideal_cycle_secs),
                    // 1ms floor: from_secs/travel scheduling rejects zero.
                    VtmIsolationMode::Teleport => flat_cycle(0.001),
                };
                // Traces show the geometric geometry; scheduling acts on `cycle`.
                // They differ only under Teleport, where `cycle` is the instant
                // stub but the trace should still show a real cycle's shape.
                let geo_cycle = match self.0.isolation {
                    VtmIsolationMode::Teleport => geometric,
                    _ => cycle.clone(),
                };
                let cycle_duration = SimDuration::from_secs(cycle.total_secs);
                // Anchor so the relay-side fork lands no earlier than relay_start.
                // Forward's relay event is the LAST phase (deposit), so completion
                // (= vtm_arrival) anchors to relay_start. Return's relay event is the
                // PICKUP after the first travel leg, so the VTM must ARRIVE at the
                // relay (after first_travel) no earlier than relay_start (=
                // deposit_time); anchoring the full cycle would schedule the pickup a
                // whole cycle early — grabbing a tote before the HTM deposited it.
                // Flat cycles have first_travel = 0, so the return anchor collapses
                // to relay_start under IdealRate — the pickup still lands ≥ deposit.
                let anchor_offset = match direction {
                    Direction::Forward => cycle_duration,
                    Direction::Return => SimDuration::from_secs(cycle.first_travel_secs),
                };
                // Teleport ignores the per-VTM frontier (infinite capacity): the
                // tote lands at the relay exactly when demand pulls it.
                let floor = self.admission_floor();
                let vtm_start = match self.0.isolation {
                    VtmIsolationMode::Teleport => relay_start.max(floor),
                    _ => s.available_at.max(relay_start.saturating_sub(anchor_offset)).max(floor),
                };
                let vtm_arrival = vtm_start + cycle_duration;
                (
                    s.available_at,
                    VtmAssignment {
                        vtm_id: id,
                        vtm_start,
                        vtm_arrival,
                        cycle,
                        geo_cycle,
                        cycle_duration,
                        from_endpoint: s.endpoint.clone(),
                        slot_pos,
                        relay_pos,
                        available_at: s.available_at,
                        generation: s.generation,
                    },
                )
            })
            // Tie-break among equal-arrival VTMs. Forward: oldest `available_at`
            // (load-balance, keep cold VTMs warm). Return: SMALLEST gap
            // (largest `available_at` still ≤ the JIT start) so a return grabs
            // a just-freed VTM instead of stranding a long-idle one across the
            // AwaitingHtmDeposit wait — leaving early-free VTMs for pick forwards.
            .min_by_key(|(avail, a)| {
                let avail_ms = avail.as_ms() as i64;
                let avail_key = match direction {
                    Direction::Forward => avail_ms,
                    Direction::Return => -avail_ms,
                };
                (a.vtm_arrival, avail_key, a.vtm_id)
            })
            .map(|(_, a)| a)
    }
}

// A revocable candidacy for one VTM cycle: the selection plus both cycle
// endpoints, held with the chosen VTM's frontier generation as its
// identity premise. Nothing is spent until acceptance, which commits the
// cycle in place; the accepted assignment rides back into `schedule`
// through the proposal's own fields.
pub struct ProposedVtmCycle {
    service: VtmService,
    pub assignment: VtmAssignment,
    pub slot_endpoint: VtmEndpoint,
    pub relay_endpoint: VtmEndpoint,
    // Where the VTM ends this cycle: the relay for a forward, the slot
    // for a return.
    complete_endpoint: VtmEndpoint,
    /// Registry handle, taken when acceptance counts this candidacy and
    /// again by `Drop` on eviction — whichever comes first.
    tentative: Option<u64>,
}

impl ProposedVtmCycle {
    pub fn new(
        service: VtmService,
        assignment: VtmAssignment,
        slot_endpoint: VtmEndpoint,
        relay_endpoint: VtmEndpoint,
        complete_endpoint: VtmEndpoint,
    ) -> Self {
        let tentative = Some(service.register_cycle_tentative(assignment.vtm_id, assignment.generation));
        Self { service, assignment, slot_endpoint, relay_endpoint, complete_endpoint, tentative }
    }
}

// Eviction drops the candidacy, which is the only signal the family gets
// that it is gone; an accepted one deregistered itself already.
impl Drop for ProposedVtmCycle {
    fn drop(&mut self) {
        if let Some(id) = self.tentative.take() {
            self.service.deregister_cycle_tentative(id);
        }
    }
}

impl fmt::Display for ProposedVtmCycle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "vtm-cycle{{vtm={:?} start={} gen={}}}",
            self.assignment.vtm_id, self.assignment.vtm_start, self.assignment.generation
        )
    }
}

impl ProposedReservation for ProposedVtmCycle {
    // Scored on the movement's START time: the 100%-utilization objective,
    // not arrival (that way lies sequencing optimization we are not doing).
    fn start_time(&self) -> Option<SimTime> {
        Some(self.assignment.vtm_start)
    }

    fn poisoned(&self) -> bool {
        !self.service.cycle_premise_holds(self.assignment.vtm_id, self.assignment.generation)
    }

    fn accept(&mut self) -> Result<(), AcceptError> {
        if self.poisoned() {
            return Err(AcceptError::Poisoned {
                detail: format!(
                    "vtm cycle on {:?} lost its frontier premise (gen {})",
                    self.assignment.vtm_id, self.assignment.generation
                ),
            });
        }
        // Depth is read before the commit and counts this candidacy: the
        // commit bumps the frontier generation, which invalidates every
        // rival on this VTM and would leave nothing to count.
        let depth = self.service.live_cycle_depth(self.assignment.vtm_id);
        if let Some(id) = self.tentative.take() {
            self.service.deregister_cycle_tentative(id);
        }
        self.service.commit(self.assignment.vtm_id, self.assignment.vtm_arrival, self.complete_endpoint.clone());
        self.service.record_cycle_accept(depth);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::OnceLock;

    use super::*;
    use crate::model::RackId;
    use crate::robots::vtm::motion::CycleTimes;
    use desgraph::{CellId, GridPos, WorldPos};
    use desmap::BlockSide;

    use crate::model::relay::config::{DEFAULT_RELAY_HEIGHT_M, Relay, RelayZone};

    const ZONE: ZoneId = ZoneId(1);
    const RELAY: RelayId = RelayId(0);
    const RACK: RackId = RackId(0);
    const SLOT: RackSlot = RackSlot { rack_id: RACK, level: 0 };
    const AISLE_CELL: CellId = CellId(42);
    const RACK_CELL: CellId = CellId(7);
    const RELAY_CELL: CellId = CellId(11);

    /// Single-zone, single-relay, single-rack fixture. Geometry is a tiny
    /// straight-line column: rack at y=0, relay at y=2m, VTMs initialize
    /// in the aisle at y=0 (next to the rack).
    fn one_zone_one_rack() -> Arc<RelayZones> {
        let relay = Relay {
            id: RELAY,
            cell_id: RELAY_CELL,
            zone_id: ZONE,
            world_pos: WorldPos { x_mm: 0.0, y_mm: 2000.0 },
            side: BlockSide::Left,
        };
        let mut rack_world_pos = HashMap::new();
        rack_world_pos.insert(RACK, WorldPos { x_mm: 0.0, y_mm: 0.0 });
        let mut rack_side = HashMap::new();
        rack_side.insert(RACK, BlockSide::Left);
        let zone = RelayZone {
            zone_id: ZONE,
            relays: vec![relay],
            vtc: 10,
            zone_height_m: 5.0,
            zone_width_m: 2.0,
            vtm_models: vec![Arc::new(CycleTimes::default().into())],
            real_shelf_count: 10,
            diving_board_floors_m: vec![],
            relay_height_m: DEFAULT_RELAY_HEIGHT_M,
            rack_floor_m: Default::default(),
            rack_world_pos,
            rack_side,
            axis: OnceLock::new(),
        };
        Arc::new(RelayZones::from_zones(vec![zone]))
    }

    fn aisle_at_rack() -> VtmEndpoint {
        VtmEndpoint::Aisle { cell: AISLE_CELL, pos: WorldPos { x_mm: 0.0, y_mm: 0.0 } }
    }

    fn slot_endpoint() -> VtmEndpoint {
        VtmEndpoint::Slot { rack_id: RACK, cell: RACK_CELL, pos: WorldPos { x_mm: 0.0, y_mm: 0.0 } }
    }

    fn relay_endpoint() -> VtmEndpoint {
        VtmEndpoint::Relay { relay_id: RELAY, cell: RELAY_CELL, pos: WorldPos { x_mm: 0.0, y_mm: 2000.0 } }
    }

    #[test]
    fn empty_zone_returns_none() {
        let svc = VtmService::with_zones(one_zone_one_rack());
        assert!(svc.select_for_forward(ZONE, RELAY, SLOT, SimTime::ZERO).is_none());
    }

    #[test]
    fn shared_zone_selects_and_prices_the_individual_robot_model() {
        use crate::robots::vtm::model::{AxisMotion, FloorVtmMotionModel, VtmChassis};

        let ladder = Arc::new(VtmMotion::from(CycleTimes::default()));
        let floor = Arc::new(VtmMotion {
            chassis: VtmChassis::Floor(FloorVtmMotionModel {
                empty: AxisMotion::Constant { speed_m_s: 1.0 },
                loaded: AxisMotion::Constant { speed_m_s: 0.1 },
            }),
            ..*ladder
        });
        let mut zone = one_zone_one_rack().zones()[0].clone();
        zone.vtm_models = vec![floor.clone(), ladder.clone()];
        let svc = VtmService::with_zones(Arc::new(RelayZones::from_zones(vec![zone])));
        // The slow robot has the lower ID. Equal starts must still choose the
        // faster complete model, not the first or a zone-wide motion value.
        svc.register_with_model(RobotId(0), ZONE, aisle_at_rack(), floor.clone());
        svc.register_with_model(RobotId(1), ZONE, aisle_at_rack(), ladder.clone());
        let first = svc.select_for_forward(ZONE, RELAY, SLOT, SimTime::ZERO).unwrap();
        assert_eq!(first.vtm_id, RobotId(1));
        assert_eq!(first.cycle, ladder.forward_cycle(0.0, 2.0, 0.0, DEFAULT_RELAY_HEIGHT_M));

        // Once the ladder robot is booked, the floor robot is selected and
        // its laden 0.1 m/s translation prices the actual cycle.
        svc.commit(RobotId(1), SimTime::seconds(100), relay_endpoint());
        let second = svc.select_for_forward(ZONE, RELAY, SLOT, SimTime::ZERO).unwrap();
        assert_eq!(second.vtm_id, RobotId(0));
        assert_eq!(second.cycle, floor.forward_cycle(0.0, 2.0, 0.0, DEFAULT_RELAY_HEIGHT_M));
        assert_eq!(second.cycle.second_travel_secs, 20.0);
        assert!(second.cycle_duration > first.cycle_duration);
    }

    #[test]
    fn unknown_zone_returns_none() {
        let svc = VtmService::with_zones(one_zone_one_rack());
        svc.register(RobotId(0), ZONE, aisle_at_rack());
        assert!(svc.select_for_forward(ZoneId(99), RELAY, SLOT, SimTime::ZERO).is_none());
    }

    #[test]
    fn registered_vtm_appears_in_frontier() {
        let svc = VtmService::with_zones(one_zone_one_rack());
        svc.register(RobotId(7), ZONE, aisle_at_rack());
        let f = svc.frontier_in_zone(ZONE);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].id, RobotId(7));
        assert_eq!(f[0].available_at, SimTime::ZERO);
        assert!(matches!(f[0].endpoint, VtmEndpoint::Aisle { .. }));
    }

    #[test]
    fn commit_advances_frontier_and_endpoint() {
        let svc = VtmService::with_zones(one_zone_one_rack());
        svc.register(RobotId(0), ZONE, aisle_at_rack());
        svc.commit(RobotId(0), SimTime::seconds(30), relay_endpoint());
        let f = svc.frontier_in_zone(ZONE);
        assert_eq!(f[0].available_at, SimTime::seconds(30));
        assert!(matches!(f[0].endpoint, VtmEndpoint::Relay { relay_id, .. } if relay_id == RELAY));
    }

    /// Every commit bumps the frontier generation — the identity premise
    /// tentative VTM reservations will record.
    #[test]
    fn commit_bumps_generation() {
        let svc = VtmService::with_zones(one_zone_one_rack());
        svc.register(RobotId(0), ZONE, aisle_at_rack());
        assert_eq!(svc.frontier_in_zone(ZONE)[0].generation, 0);
        svc.commit(RobotId(0), SimTime::seconds(30), relay_endpoint());
        assert_eq!(svc.frontier_in_zone(ZONE)[0].generation, 1);
        svc.commit(RobotId(0), SimTime::seconds(60), relay_endpoint());
        assert_eq!(svc.frontier_in_zone(ZONE)[0].generation, 2);
    }

    /// A commit landing before the recorded frontier is a caller planning
    /// against a stale snapshot; the service refuses instead of silently
    /// rewinding the fact.
    #[test]
    #[should_panic(expected = "frontier backwards")]
    fn commit_refuses_backwards_frontier() {
        let svc = VtmService::with_zones(one_zone_one_rack());
        svc.register(RobotId(0), ZONE, aisle_at_rack());
        svc.commit(RobotId(0), SimTime::seconds(30), relay_endpoint());
        svc.commit(RobotId(0), SimTime::seconds(29), relay_endpoint());
    }

    /// An equal-time commit is not backwards motion (a zero-length cycle
    /// completes at the frontier it started from).
    #[test]
    fn commit_at_exact_frontier_is_allowed() {
        let svc = VtmService::with_zones(one_zone_one_rack());
        svc.register(RobotId(0), ZONE, aisle_at_rack());
        svc.commit(RobotId(0), SimTime::seconds(30), relay_endpoint());
        svc.commit(RobotId(0), SimTime::seconds(30), relay_endpoint());
        assert_eq!(svc.frontier_in_zone(ZONE)[0].available_at, SimTime::seconds(30));
    }

    /// A commit for a VTM the service never registered is a caller bug,
    /// not a no-op.
    #[test]
    #[should_panic(expected = "unregistered")]
    fn commit_refuses_unregistered_vtm() {
        let svc = VtmService::with_zones(one_zone_one_rack());
        svc.commit(RobotId(42), SimTime::seconds(1), relay_endpoint());
    }

    #[test]
    fn assignment_carries_from_endpoint() {
        let svc = VtmService::with_zones(one_zone_one_rack());
        svc.register(RobotId(0), ZONE, aisle_at_rack());
        let a = svc.select_for_forward(ZONE, RELAY, SLOT, SimTime::ZERO).expect("non-empty");
        // Fresh VTM is in the aisle.
        assert!(matches!(a.from_endpoint, VtmEndpoint::Aisle { .. }));

        // After committing into a slot, the from_endpoint reflects that.
        svc.commit(RobotId(0), SimTime::seconds(10), slot_endpoint());
        let a = svc.select_for_return(ZONE, RELAY, SLOT, SimTime::seconds(20)).expect("non-empty");
        assert!(matches!(a.from_endpoint, VtmEndpoint::Slot { rack_id, .. } if rack_id == RACK));
    }

    #[test]
    fn first_chain_picks_lowest_id_on_tie() {
        let svc = VtmService::with_zones(one_zone_one_rack());
        svc.register(RobotId(1), ZONE, aisle_at_rack());
        svc.register(RobotId(0), ZONE, aisle_at_rack());
        // Both at avail=0 in the same position -> identical arrival.
        // Tie broken by lowest id (RobotId(0) wins).
        let a = svc.select_for_forward(ZONE, RELAY, SLOT, SimTime::ZERO).expect("non-empty");
        assert_eq!(a.vtm_id, RobotId(0));
    }

    #[test]
    fn cold_vtm_late_starts_to_meet_relay_start() {
        let svc = VtmService::with_zones(one_zone_one_rack());
        svc.register(RobotId(0), ZONE, aisle_at_rack());
        // Relay reservation pinned far enough out that the VTM has slack.
        let relay_start = SimTime::seconds(60);
        let a = svc.select_for_forward(ZONE, RELAY, SLOT, relay_start).expect("non-empty");
        // Arrival must satisfy the reservation (>= relay_start).
        assert!(a.vtm_arrival >= relay_start, "arrival {:?} < relay_start {:?}", a.vtm_arrival, relay_start);
        // VTM started late enough that there's no idle gap on the relay
        // side: vtm_arrival == relay_start within rounding.
        assert_eq!(a.vtm_arrival, relay_start, "expected vtm to land exactly at relay_start");
        // Started after avail=0 (late-started).
        assert!(a.vtm_start > SimTime::ZERO);
        assert_eq!(a.vtm_start + a.cycle_duration, a.vtm_arrival);
    }

    #[test]
    fn return_late_starts_so_pickup_not_cycle_meets_deposit() {
        // Return's relay event is the PICKUP after the first travel leg, not
        // the cycle completion. A cold VTM with slack must late-start so the
        // pickup lands at deposit_time — anchoring the full cycle (the old
        // bug) would schedule the pickup a whole cycle early.
        let svc = VtmService::with_zones(one_zone_one_rack());
        svc.register(RobotId(0), ZONE, aisle_at_rack());
        let deposit = SimTime::seconds(60);
        let a = svc.select_for_return(ZONE, RELAY, SLOT, deposit).expect("non-empty");
        let pickup = a.vtm_start + SimDuration::from_secs(a.cycle.first_travel_secs);
        assert_eq!(pickup, deposit, "pickup must land at deposit_time, not a full cycle early");
        // Completion (tote in rack) is strictly after the deposit.
        assert!(a.vtm_arrival > deposit, "return completes after the deposit");
        assert_eq!(a.vtm_start + a.cycle_duration, a.vtm_arrival);
        assert!(a.vtm_start > SimTime::ZERO, "late-started, not begun at avail=0");
    }

    #[test]
    fn cold_vtm_beats_warm_vtm_when_cold_can_late_start() {
        let svc = VtmService::with_zones(one_zone_one_rack());
        // Two VTMs at the same physical position (so cycle duration is identical).
        svc.register(RobotId(0), ZONE, aisle_at_rack());
        svc.register(RobotId(1), ZONE, aisle_at_rack());
        // Warm r0 by committing it past r1 — leave it at the slot.
        svc.commit(RobotId(0), SimTime::seconds(20), slot_endpoint());
        // r1 is still at avail=0. relay_start is far enough out that r1
        // can late-start to land exactly there.
        let relay_start = SimTime::seconds(40);
        let a = svc.select_for_forward(ZONE, RELAY, SLOT, relay_start).expect("non-empty");
        assert_eq!(a.vtm_id, RobotId(1), "cold r1 should win — arrives no later than warm r0");
        assert_eq!(a.vtm_arrival, relay_start);
    }

    #[test]
    fn warm_vtm_wins_when_cold_is_too_far_back() {
        // Variant: cold VTM cycle exceeds relay_start window -> warm one
        // arrives sooner. With relay_start=0, the warm VTM still loses
        // because both have the same cycle from the same starting pos —
        // see comment in the original test for the geometry argument.
        let svc = VtmService::with_zones(one_zone_one_rack());
        svc.register(RobotId(0), ZONE, aisle_at_rack());
        svc.register(RobotId(1), ZONE, aisle_at_rack());
        svc.commit(RobotId(0), SimTime::seconds(10), slot_endpoint());
        let a = svc.select_for_forward(ZONE, RELAY, SLOT, SimTime::ZERO).expect("non-empty");
        // r1 (cold) wins because lower arrival (no late-start needed since
        // relay_start=0, and r1's avail=0 < r0's avail=10).
        assert_eq!(a.vtm_id, RobotId(1));
    }

    #[test]
    fn commit_alternation_balances_load() {
        // Both VTMs cold; pick #1 -> commit -> pick #2 -> the OTHER VTM wins.
        let svc = VtmService::with_zones(one_zone_one_rack());
        svc.register(RobotId(0), ZONE, aisle_at_rack());
        svc.register(RobotId(1), ZONE, aisle_at_rack());

        let a1 = svc.select_for_forward(ZONE, RELAY, SLOT, SimTime::ZERO).expect("non-empty");
        // First commit completes a cycle for the chosen VTM (lands at relay).
        let complete_at = a1.vtm_arrival + SimDuration::ms(1);
        svc.commit(a1.vtm_id, complete_at, relay_endpoint());

        let a2 = svc.select_for_forward(ZONE, RELAY, SLOT, complete_at).expect("non-empty");
        assert_ne!(a2.vtm_id, a1.vtm_id, "second pick should alternate to the cold VTM");
    }

    fn candidacy(svc: &VtmService, assignment: VtmAssignment) -> ProposedVtmCycle {
        ProposedVtmCycle::new(svc.clone(), assignment, slot_endpoint(), relay_endpoint(), relay_endpoint())
    }

    /// Two candidacies that independently selected the same VTM make a
    /// choice depth of 2 for whichever one is accepted — the winner counts
    /// itself.
    #[test]
    fn choice_depth_counts_the_live_rivals_on_the_accepted_vtm() {
        let svc = VtmService::with_zones(one_zone_one_rack());
        svc.register(RobotId(0), ZONE, aisle_at_rack());
        let first = svc.select_for_forward(ZONE, RELAY, SLOT, SimTime::ZERO).expect("non-empty");
        let rival = svc.select_for_forward(ZONE, RELAY, SLOT, SimTime::ZERO).expect("non-empty");

        let mut winner = candidacy(&svc, first);
        let _rival = candidacy(&svc, rival);
        winner.accept().expect("a live premise accepts");

        assert_eq!(svc.acceptance_report().as_deref(), Some("vtm accepts=1 choice_depth avg=2.00 max=2"));
    }

    /// An evicted candidacy is dropped, and a dropped one is no longer a
    /// choice — the next acceptance must not still be counting it.
    #[test]
    fn an_evicted_candidacy_stops_counting_toward_choice_depth() {
        let svc = VtmService::with_zones(one_zone_one_rack());
        svc.register(RobotId(0), ZONE, aisle_at_rack());
        let first = svc.select_for_forward(ZONE, RELAY, SLOT, SimTime::ZERO).expect("non-empty");
        let rival = svc.select_for_forward(ZONE, RELAY, SLOT, SimTime::ZERO).expect("non-empty");

        let mut winner = candidacy(&svc, first);
        drop(candidacy(&svc, rival));
        winner.accept().expect("a live premise accepts");

        assert_eq!(svc.acceptance_report().as_deref(), Some("vtm accepts=1 choice_depth avg=1.00 max=1"));
    }

    /// A rival whose premise the acceptance invalidated is not a live
    /// choice for the NEXT acceptance, even while its owner still holds it.
    #[test]
    fn a_stale_rival_does_not_inflate_a_later_choice_depth() {
        let svc = VtmService::with_zones(one_zone_one_rack());
        svc.register(RobotId(0), ZONE, aisle_at_rack());
        let first = svc.select_for_forward(ZONE, RELAY, SLOT, SimTime::ZERO).expect("non-empty");
        let stale = svc.select_for_forward(ZONE, RELAY, SLOT, SimTime::ZERO).expect("non-empty");

        let mut winner = candidacy(&svc, first);
        // Held, not dropped: its generation is now behind the committed one.
        let _stale = candidacy(&svc, stale);
        winner.accept().expect("a live premise accepts");

        let fresh = svc.select_for_forward(ZONE, RELAY, SLOT, SimTime::seconds(600)).expect("non-empty");
        let mut second = candidacy(&svc, fresh);
        second.accept().expect("a freshly selected premise accepts");

        assert_eq!(svc.acceptance_report().as_deref(), Some("vtm accepts=2 choice_depth avg=1.50 max=2"));
    }
}
