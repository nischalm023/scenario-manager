//! Relay-zone geometry and its eligible VTM models. Inventory ranking takes
//! the minimum model-specific cycle; conservative lead-time sizing takes the
//! maximum. Execution always supplies the selected robot's own motion.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, OnceLock};

use desgraph::{CellId, RelayId, WorldPos, ZoneId};
use desmap::BlockSide;
use rustc_hash::FxHashMap;

use crate::error::InvalidScenario;
use crate::model::ids::RackSlot;
use crate::model::{RackId, SimDuration};
use crate::robots::vtm::model::VtmMotion;
use crate::robots::vtm::motion;
use crate::robots::vtm::motion::CycleBreakdown;
#[cfg(test)]
use crate::robots::vtm::motion::CycleTimes;
use crate::scenario_gen::ScenarioData;

/// Site geometry, not a robot figure: the relay (diving board) exchange height,
/// in meters, of a zone whose map states no diving boards. Every fixture world
/// states its boards; a hand-built test map does not, and runs at this height,
/// the first-tier buffer height of the A5 spec sheet's layout (sheet 1, row 11).
pub const DEFAULT_RELAY_HEIGHT_M: f64 = 0.39;

/// Immutable per-relay config.
///
/// `side` records which side of the aisle the relay's rack block sits on.
/// A VTM cycle is side-bound: source and destination must share a side, so
/// the relay index pairs a relay only with rack slots that share its `side`.
#[derive(Debug, Clone, Copy)]
pub struct Relay {
    pub id: RelayId,
    pub cell_id: CellId,
    pub zone_id: ZoneId,
    pub world_pos: WorldPos,
    pub side: BlockSide,
}

/// A relay zone: the rack+VTM+relay subsystem.
#[derive(Debug)]
pub struct RelayZone {
    pub zone_id: ZoneId,
    pub relays: Vec<Relay>,
    /// Vertical tote capacity (slots per rack column).
    pub vtc: u16,
    /// Physical racking height in meters.
    pub zone_height_m: f64,
    /// Physical VTM aisle length in meters.
    pub zone_width_m: f64,
    /// Actual physical shelf count per rack column.
    pub real_shelf_count: u16,
    /// Diving-board (relay exchange) floor heights in meters, bottom-up —
    /// the exchange stack present at each relay cell. The sim currently
    /// exchanges on the lowest; the full stack is carried for the two-lane
    /// relay model.
    pub diving_board_floors_m: Vec<f64>,
    /// Relay (buffer/diving-board) exchange height in meters. Board 0 of
    /// the zone's diving-board stack when the zone has explicit slots, else
    /// [`DEFAULT_RELAY_HEIGHT_M`].
    pub relay_height_m: f64,
    /// Rack → fork-interface floor height in meters, for zones where every
    /// slot is its own directly encoded rack (explicit `slots` maps).
    pub rack_floor_m: HashMap<RackId, f64>,
    /// Rack → world position in mm (racks in this zone).
    pub rack_world_pos: HashMap<RackId, WorldPos>,
    /// One immutable motion per deployed VTM, in deterministic binding order.
    /// Repeated models retain robot multiplicity for diagnostic fleet averages.
    pub vtm_models: Vec<Arc<VtmMotion>>,
    /// Rack → side of the aisle (Left or Right). Derived from relay column
    /// geometry: every rack column in a zone shares its x_mm with the relay
    /// column on the same side. A VTM cycle is side-bound, so this drives
    /// same-side pairing in the relay index.
    pub rack_side: HashMap<RackId, BlockSide>,
    /// Axis projections, built lazily on first distance query — see
    /// [`AxisFrame`]. Valid because zone geometry (`relays`,
    /// `rack_world_pos`, `zone_width_m`) is immutable once queries begin:
    /// `from_scenario` fully populates a zone before anything reads it, and
    /// tests that build zones by literal mutate before their first query.
    /// Each holder pays for its own frame: production clones a zone into
    /// `RelayInventoryIndex` and `VtmService`, and every copy lazily builds
    /// its own maps (one entry per rack — the dominant cost on
    /// placed-geometry maps, where every slot is a rack).
    pub(super) axis: OnceLock<AxisFrame>,
}

/// Manual impl so a clone starts with an EMPTY cache. The derived impl would
/// copy a filled `OnceLock`, and a clone that is then geometry-tweaked (the
/// natural test-fixture pattern) would keep serving the donor's stale frame.
/// Rebuilding costs one extent scan on the clone's first query.
impl Clone for RelayZone {
    fn clone(&self) -> Self {
        Self {
            zone_id: self.zone_id,
            relays: self.relays.clone(),
            vtc: self.vtc,
            zone_height_m: self.zone_height_m,
            zone_width_m: self.zone_width_m,
            real_shelf_count: self.real_shelf_count,
            diving_board_floors_m: self.diving_board_floors_m.clone(),
            relay_height_m: self.relay_height_m,
            rack_floor_m: self.rack_floor_m.clone(),
            rack_world_pos: self.rack_world_pos.clone(),
            vtm_models: self.vtm_models.clone(),
            rack_side: self.rack_side.clone(),
            axis: OnceLock::new(),
        }
    }
}

/// The zone's rail axis, projected once: which world axis the rack run
/// spans, its extent, the zone width the projection scales onto, and every
/// relay's / rack's position along it in meters. Replaces recomputing the
/// extent by scanning every rack and relay on each `h_dist_m` call, which
/// dominated solver time on placed-geometry maps (every slot its own rack).
#[derive(Debug, Clone)]
pub(super) struct AxisFrame {
    /// Rail axis selection: true = world X, false = world Y (the longer
    /// span of the zone's rack+relay extent, as before).
    use_x: bool,
    min_mm: f64,
    span_mm: f64,
    zone_width_m: f64,
    relay_axis_m: FxHashMap<RelayId, f64>,
    rack_axis_m: FxHashMap<RackId, f64>,
}

impl RelayZone {
    /// Fork-interface height in meters for a slot: the rack's directly
    /// encoded floor when present (explicit-slot maps, one rack per slot),
    /// else the uniform-pitch model.
    pub fn slot_height_m(&self, slot: RackSlot) -> f64 {
        match self.rack_floor_m.get(&slot.rack_id) {
            Some(&floor) => floor,
            None => motion::shelf_height_uniform(slot.level, self.real_shelf_count, self.zone_height_m),
        }
    }

    /// Physical cycle for the selected VTM, using an explicit first leg.
    pub fn forward_cycle_for_model(
        &self,
        relay_id: RelayId,
        slot: RackSlot,
        first_leg_m: f64,
        model: &VtmMotion,
    ) -> CycleBreakdown {
        model.forward_cycle(
            first_leg_m,
            self.h_dist_m(relay_id, slot.rack_id),
            self.slot_height_m(slot),
            self.relay_height_m,
        )
    }

    pub fn return_cycle_for_model(
        &self,
        relay_id: RelayId,
        slot: RackSlot,
        first_leg_m: f64,
        model: &VtmMotion,
    ) -> CycleBreakdown {
        model.return_cycle(
            first_leg_m,
            self.h_dist_m(relay_id, slot.rack_id),
            self.slot_height_m(slot),
            self.relay_height_m,
        )
    }

    /// Optimistic inventory-ranking cycle, ignoring robot availability. It
    /// chooses among complete eligible models, never mixes their fastest axes.
    pub fn min_forward_cycle_from(
        &self,
        relay_id: RelayId,
        slot: RackSlot,
        first_leg_m: f64,
    ) -> Option<CycleBreakdown> {
        self.vtm_models
            .iter()
            .map(|model| self.forward_cycle_for_model(relay_id, slot, first_leg_m, model))
            .min_by(|a, b| a.total_secs.total_cmp(&b.total_secs))
    }

    pub fn min_return_cycle_from(&self, relay_id: RelayId, slot: RackSlot, first_leg_m: f64) -> Option<CycleBreakdown> {
        self.vtm_models
            .iter()
            .map(|model| self.return_cycle_for_model(relay_id, slot, first_leg_m, model))
            .min_by(|a, b| a.total_secs.total_cmp(&b.total_secs))
    }

    /// Inventory rank from a relay start. An unserved zone has no finite price.
    pub fn min_forward_cycle_cost(&self, relay_id: RelayId, slot: RackSlot) -> SimDuration {
        self.min_forward_cycle_from(relay_id, slot, self.h_dist_m(relay_id, slot.rack_id))
            .map_or(SimDuration::MAX, |cycle| cycle.total())
    }

    pub fn min_return_cycle_cost(&self, relay_id: RelayId, slot: RackSlot) -> SimDuration {
        self.min_return_cycle_from(relay_id, slot, self.h_dist_m(relay_id, slot.rack_id))
            .map_or(SimDuration::MAX, |cycle| cycle.total())
    }

    /// Conservative lead-time sizing across every eligible model; this is not
    /// an execution quote. None means that no VTM serves the zone.
    pub fn max_forward_cycle_cost(&self, relay_id: RelayId, slot: RackSlot) -> Option<SimDuration> {
        self.vtm_models
            .iter()
            .map(|model| {
                SimDuration::from_secs(
                    self.forward_cycle_for_model(relay_id, slot, self.h_dist_m(relay_id, slot.rack_id), model)
                        .total_secs,
                )
            })
            .max()
    }

    pub fn max_return_cycle_cost(&self, relay_id: RelayId, slot: RackSlot) -> Option<SimDuration> {
        self.vtm_models
            .iter()
            .map(|model| {
                SimDuration::from_secs(
                    self.return_cycle_for_model(relay_id, slot, self.h_dist_m(relay_id, slot.rack_id), model)
                        .total_secs,
                )
            })
            .max()
    }

    /// Representative *loaded* forward-cycle time (seconds) for this zone: a
    /// mid-shelf forward cycle averaged over every (robot, relay, rack) combination, with
    /// the empty rack→relay repositioning leg (`first_travel`) dropped. Used to
    /// derive the `VtmIsolationMode::IdealRate` timer from the same motion
    /// calibration the live scheduler uses, so the isolation mode is
    /// self-consistent rather than a hand-picked constant.
    ///
    /// We drop `first_travel` deliberately: it's the schedulable slack an ideal
    /// scheduler hides (pre-positioning / overlapping one VTM's empty leg with
    /// another's loaded work). Keeping it would make the derived timer slower
    /// than the live scheduler's *realized* cycles — inventory is isochrone-
    /// filled near relays and VTMs often start docked at the relay — which
    /// would push `IdealRate` throughput *below* baseline (not "ideal" at all).
    /// The irreducible work per tote is fork + loaded travel + buffer.
    /// `None` if the zone has no relays or racks.
    pub fn representative_forward_cycle_secs(&self) -> Option<f64> {
        // Mid-rack representative height: median of the directly encoded
        // slot floors when present, else the uniform model's midpoint.
        let mid_height = if self.rack_floor_m.is_empty() {
            self.slot_height_m(RackSlot { rack_id: RackId(u32::MAX), level: self.real_shelf_count / 2 })
        } else {
            let mut floors: Vec<f64> = self.rack_floor_m.values().copied().collect();
            floors.sort_by(|a, b| a.partial_cmp(b).unwrap());
            floors[floors.len() / 2]
        };
        let mut sum = 0.0;
        let mut n: u32 = 0;
        for relay in &self.relays {
            for &rack_id in self.rack_world_pos.keys() {
                let h = self.h_dist_m(relay.id, rack_id);
                for model in &self.vtm_models {
                    let c = model.forward_cycle(h, h, mid_height, self.relay_height_m);
                    sum += c.total_secs - c.first_travel_secs;
                    n += 1;
                }
            }
        }
        (n > 0).then(|| sum / n as f64)
    }

    fn h_dist_m(&self, relay_id: RelayId, rack_id: RackId) -> f64 {
        let frame = self.axis_frame();
        let relay_axis = *frame
            .relay_axis_m
            .get(&relay_id)
            .unwrap_or_else(|| panic!("relay {:?} not in zone {:?}", relay_id, self.zone_id));
        let rack_axis = *frame
            .rack_axis_m
            .get(&rack_id)
            .unwrap_or_else(|| panic!("rack {:?} not in zone {:?}", rack_id, self.zone_id));
        (relay_axis - rack_axis).abs()
    }

    pub fn h_dist_between_cells_m(&self, from: WorldPos, to: WorldPos) -> f64 {
        let frame = self.axis_frame();
        (frame.project_m(from) - frame.project_m(to)).abs()
    }

    fn axis_frame(&self) -> &AxisFrame {
        self.axis.get_or_init(|| {
            let mut min_x = f64::INFINITY;
            let mut max_x = f64::NEG_INFINITY;
            let mut min_y = f64::INFINITY;
            let mut max_y = f64::NEG_INFINITY;

            for p in self.rack_world_pos.values().copied().chain(self.relays.iter().map(|r| r.world_pos)) {
                min_x = min_x.min(p.x_mm);
                max_x = max_x.max(p.x_mm);
                min_y = min_y.min(p.y_mm);
                max_y = max_y.max(p.y_mm);
            }

            let span_x = max_x - min_x;
            let span_y = max_y - min_y;
            let (use_x, min_mm, span_mm) =
                if span_x >= span_y { (true, min_x, span_x) } else { (false, min_y, span_y) };

            let project = |pos| project_axis_m(use_x, min_mm, span_mm, self.zone_width_m, pos);
            // `.or_insert` keeps the FIRST entry for a duplicated RelayId,
            // matching the `.find()` the per-call path used. from_scenario
            // never produces duplicates; this pins the tie-break anyway.
            let mut relay_axis_m = FxHashMap::default();
            for r in &self.relays {
                relay_axis_m.entry(r.id).or_insert_with(|| project(r.world_pos));
            }
            AxisFrame {
                use_x,
                min_mm,
                span_mm,
                zone_width_m: self.zone_width_m,
                relay_axis_m,
                rack_axis_m: self.rack_world_pos.iter().map(|(&id, &pos)| (id, project(pos))).collect(),
            }
        })
    }
}

impl AxisFrame {
    /// Position along the rail axis in meters.
    fn project_m(&self, pos: WorldPos) -> f64 {
        project_axis_m(self.use_x, self.min_mm, self.span_mm, self.zone_width_m, pos)
    }
}

/// The projection itself — the same normalization the per-call scan used to
/// produce: fraction of the zone extent, clamped, scaled onto the zone's
/// authored width. A free function so frame construction and [`AxisFrame`]
/// queries share one implementation without a half-built placeholder frame.
fn project_axis_m(use_x: bool, min_mm: f64, span_mm: f64, zone_width_m: f64, pos: WorldPos) -> f64 {
    let coord = if use_x { pos.x_mm } else { pos.y_mm };
    if !span_mm.is_finite() || span_mm <= 0.0 {
        0.0
    } else {
        ((coord - min_mm) / span_mm).clamp(0.0, 1.0) * zone_width_m
    }
}

/// All relay zones in the warehouse.
#[derive(Debug, Default, Clone)]
pub struct RelayZones(Vec<RelayZone>);

impl RelayZones {
    pub fn zones(&self) -> &[RelayZone] {
        &self.0
    }

    /// Construct directly from a list of zone configs. Used internally
    /// (and by sibling-module tests) to build small fixtures without
    /// going through `from_scenario`.
    #[cfg(test)]
    pub(super) fn from_zones(zones: Vec<RelayZone>) -> Self {
        Self(zones)
    }

    /// Flat index: relay_id → Relay.
    pub(super) fn relay_index(&self) -> BTreeMap<RelayId, Relay> {
        self.0.iter().flat_map(|z| z.relays.iter().map(|r| (r.id, *r))).collect()
    }

    /// Zone config by zone_id.
    pub(super) fn zone_map(&self) -> HashMap<ZoneId, &RelayZone> {
        self.0.iter().map(|z| (z.zone_id, z)).collect()
    }

    /// Relay count per zone.
    pub(super) fn zone_capacities(&self) -> HashMap<ZoneId, u32> {
        self.0.iter().map(|z| (z.zone_id, z.relays.len() as u32)).collect()
    }

    /// Fleet-mean representative forward-cycle time (seconds), equal-weighted
    /// across zones that have relays. Default timer for
    /// [`crate::config::VtmIsolationMode::IdealRate`]. `None` if no zone yields a
    /// representative cycle.
    pub fn mean_forward_cycle_secs(&self) -> Option<f64> {
        let vals: Vec<f64> = self.0.iter().filter_map(|z| z.representative_forward_cycle_secs()).collect();
        (!vals.is_empty()).then(|| vals.iter().sum::<f64>() / vals.len() as f64)
    }

    /// Build geometry and eligible models from validated per-robot bindings.
    pub fn from_scenario(data: &ScenarioData) -> Result<Self, InvalidScenario> {
        let mut models = FxHashMap::<ZoneId, Vec<Arc<VtmMotion>>>::default();
        for (index, zones) in data.vtm_zone_assignments.iter().enumerate() {
            if zones.len() != 1 {
                return Err(InvalidScenario::RobotModels(format!("VTM {index} needs exactly one assigned aisle zone")));
            }
            let zone = *zones.first().expect("singleton checked above");
            let model = data.robot_models.vtm_for_robot(index).map_err(InvalidScenario::RobotModels)?;
            models.entry(zone).or_default().push(model);
        }
        Self::build(data, models)
    }

    /// Explicit compatibility input for small unit fixtures without a catalog.
    #[cfg(test)]
    pub fn from_scenario_legacy(data: &ScenarioData, times: CycleTimes) -> Result<Self, InvalidScenario> {
        let model = Arc::new(VtmMotion::from(times));
        let models = data.zone_data.iter().map(|zone| (zone.zone_id, vec![model.clone()])).collect();
        Self::build(data, models)
    }

    fn build(data: &ScenarioData, mut models: FxHashMap<ZoneId, Vec<Arc<VtmMotion>>>) -> Result<Self, InvalidScenario> {
        let graph = &data.map.graph;
        let vtc = data.vertical_tote_capacity as u16;

        // desmap::Zone carries zone_height_mm from the design.
        let zone_height: HashMap<ZoneId, f64> =
            data.map.zones.iter().map(|z| (z.id, z.zone_height_mm as f64 / 1000.0)).collect();
        let zone_width: HashMap<ZoneId, f64> =
            data.map.zones.iter().filter_map(|z| Some((z.id, z.zone_width_mm? as f64 / 1000.0))).collect();
        let real_shelf_count: HashMap<ZoneId, u16> =
            data.map.zones.iter().map(|z| (z.id, z.real_shelf_count)).collect();

        // Diving-board (relay exchange) stacks, bottom-up, as the map's own
        // layout states them. A map that states none leaves the default relay
        // height standing.
        let diving_boards = &data.board_floors_m;

        let relay_zone_index: BTreeMap<RelayId, ZoneId> =
            data.zone_data.iter().flat_map(|zd| zd.relay_ids.iter().map(move |&rid| (rid, zd.zone_id))).collect();

        // Rack → zone (via rack_cells → zone_storage).
        let cell_to_zone: HashMap<CellId, ZoneId> =
            data.zone_storage.iter().flat_map(|(&zid, cells)| cells.iter().map(move |&cid| (cid, zid))).collect();

        // Zones that have relay columns at all. A zone without any is not a
        // relay zone -- an order-buffer RTP block, say -- and its racks have no
        // side to derive; VTMs simply never serve them. Refusing the whole
        // scenario for that turned a deliberately inert block into a load
        // error, which is how the Walmart v5 fixture stopped opening.
        let zones_with_relays: HashSet<ZoneId> =
            data.zone_data.iter().filter(|zd| !zd.relay_ids.is_empty()).map(|zd| zd.zone_id).collect();

        // Per-zone rack geometry. Side comes from `data.rack_side`, derived
        // upstream from the map's relay columns (see `topology::build_rack_side`).
        let mut rack_pos_by_zone: HashMap<ZoneId, HashMap<RackId, WorldPos>> = HashMap::new();
        let mut rack_side_by_zone: HashMap<ZoneId, HashMap<RackId, BlockSide>> = HashMap::new();
        let mut rack_floor_by_zone: HashMap<ZoneId, HashMap<RackId, f64>> = HashMap::new();
        for (&rack_id, &cell_id) in &data.rack_cells {
            if let Some(&zone_id) = cell_to_zone.get(&cell_id) {
                if !zones_with_relays.contains(&zone_id) {
                    continue;
                }
                // Prefer the interpolated column position (zones that decouple
                // slots_per_row from the grid); fall back to the cell center.
                let wp = data
                    .rack_world_pos
                    .get(&rack_id)
                    .copied()
                    .or_else(|| graph.cell(cell_id).and_then(|c| c.world_pos))
                    .unwrap_or_default();
                rack_pos_by_zone.entry(zone_id).or_default().insert(rack_id, wp);
                if let Some(&floor) = data.rack_floor_m.get(&rack_id) {
                    rack_floor_by_zone.entry(zone_id).or_default().insert(rack_id, floor);
                }
                let side = data.rack_side.get(&rack_id).copied().ok_or(InvalidScenario::RackWithoutSide {
                    rack_id: rack_id.0,
                    zone_id: zone_id.0,
                    // Whole-millimetre x for ordering; a positionless cell sorts last.
                    x_mm: graph
                        .cell(cell_id)
                        .and_then(|c| c.world_pos.map(|w| w.x_mm).or(c.grid_pos.map(|g| g.x as f64 * 1000.0)))
                        .map(|x| x.round() as i64)
                        .unwrap_or(i64::MAX),
                })?;
                rack_side_by_zone.entry(zone_id).or_default().insert(rack_id, side);
            }
        }

        let mut by_zone: BTreeMap<ZoneId, Vec<Relay>> = BTreeMap::new();
        for r in &data.relay_data {
            let zone_id = *relay_zone_index
                .get(&r.relay_id)
                .ok_or(InvalidScenario::RelayWithoutZone { relay_id: r.relay_id.0 })?;
            let cell =
                graph.cell(r.htm_access).ok_or(InvalidScenario::CellWithoutPosition { cell_id: r.htm_access.0 })?;
            // Prefer the diving-board exchange position (explicit-slot zones)
            // over the relay cell center, so the VTM's relay-leg distance and
            // docked position match where the board physically sits.
            let world_pos = data.relay_exchange_pos.get(&r.relay_id).copied().or(cell.world_pos).unwrap_or_default();
            by_zone.entry(zone_id).or_default().push(Relay {
                id: r.relay_id,
                cell_id: r.htm_access,
                zone_id,
                world_pos,
                side: r.side,
            });
        }

        Ok(Self(
            by_zone
                .into_iter()
                .map(|(zone_id, relays)| RelayZone {
                    zone_id,
                    relays,
                    vtc,
                    zone_height_m: zone_height.get(&zone_id).copied().unwrap_or(9.0),
                    zone_width_m: zone_width.get(&zone_id).copied().unwrap_or(24.0),
                    real_shelf_count: real_shelf_count.get(&zone_id).copied().unwrap_or(vtc),
                    vtm_models: models.remove(&zone_id).unwrap_or_default(),
                    diving_board_floors_m: diving_boards.get(&zone_id).cloned().unwrap_or_default(),
                    relay_height_m: diving_boards
                        .get(&zone_id)
                        .and_then(|boards| boards.first().copied())
                        .unwrap_or(DEFAULT_RELAY_HEIGHT_M),
                    rack_floor_m: rack_floor_by_zone.remove(&zone_id).unwrap_or_default(),
                    rack_world_pos: rack_pos_by_zone.remove(&zone_id).unwrap_or_default(),
                    rack_side: rack_side_by_zone.remove(&zone_id).unwrap_or_default(),
                    axis: OnceLock::new(),
                })
                .collect(),
        ))
    }
}

#[cfg(test)]
mod slot_height_tests {
    use super::*;
    use crate::robots::vtm::model::{AxisMotion, FloorVtmMotionModel, HandlingTimes, VtmChassis};

    fn zone_with(rack_floor_m: HashMap<RackId, f64>) -> RelayZone {
        RelayZone {
            zone_id: ZoneId(1),
            relays: vec![],
            vtc: 4,
            zone_height_m: 9.0,
            zone_width_m: 12.0,
            real_shelf_count: 10,
            vtm_models: vec![Arc::new(CycleTimes::default().into())],
            diving_board_floors_m: vec![0.394, 0.952],
            relay_height_m: 0.394,
            rack_floor_m,
            rack_world_pos: HashMap::new(),
            rack_side: HashMap::new(),
            axis: OnceLock::new(),
        }
    }

    fn slot(rack: u32, level: u16) -> RackSlot {
        RackSlot { rack_id: RackId(rack), level }
    }

    #[test]
    fn explicit_floors_win_over_uniform_model() {
        let zone = zone_with(HashMap::from([(RackId(0), 1.580), (RackId(1), 2.697)]));
        assert_eq!(zone.slot_height_m(slot(0, 0)), 1.580);
        assert_eq!(zone.slot_height_m(slot(1, 0)), 2.697);
        // Racks without an encoded floor fall back to the uniform model.
        assert_eq!(zone.slot_height_m(slot(9, 9)), 9.0);
    }

    #[test]
    fn uniform_fallback_matches_motion_model() {
        let zone = zone_with(HashMap::new());
        assert_eq!(zone.slot_height_m(slot(0, 0)), 0.0);
        assert_eq!(zone.slot_height_m(slot(0, 9)), 9.0); // top shelf of 10 spans zone_height
    }

    #[test]
    fn cycle_lifts_to_floor_not_uniform_pitch() {
        let mut zone = zone_with(HashMap::from([(RackId(0), 5.0)]));
        let world_pos = WorldPos { x_mm: 0.0, y_mm: 0.0 };
        zone.relays.push(Relay {
            id: RelayId(0),
            cell_id: CellId(0),
            zone_id: zone.zone_id,
            world_pos,
            side: BlockSide::Left,
        });
        zone.rack_world_pos.insert(RackId(0), world_pos);
        // Vertical leg dominates a zero-horizontal cycle: travel time must
        // reflect the 5.0 m floor, not the uniform model's pitch.
        let slot = RackSlot { rack_id: RackId(0), level: 1 };
        let cycle = zone.forward_cycle_for_model(RelayId(0), slot, 0.0, &zone.vtm_models[0]);
        // At 3 m/s and 2 m/s², acceleration and braking cover 4.5 m in 3 s; cruise adds 0.5/3 s.
        let expected_up = 19.0 / 6.0;
        assert!((cycle.first_travel_secs - expected_up).abs() < 1e-9, "{} vs {}", cycle.first_travel_secs, expected_up);
    }

    #[test]
    fn inventory_minimum_and_sizing_maximum_keep_whole_models() {
        let mut zone = zone_with(HashMap::new());
        zone.zone_width_m = 10.0;
        zone.relay_height_m = 0.0;
        zone.rack_world_pos.insert(RackId(0), WorldPos { x_mm: 0.0, y_mm: 0.0 });
        zone.relays.push(Relay {
            id: RelayId(0),
            cell_id: CellId(0),
            zone_id: zone.zone_id,
            world_pos: WorldPos { x_mm: 10_000.0, y_mm: 0.0 },
            side: BlockSide::Left,
        });
        let model = |speed, handling| {
            Arc::new(VtmMotion {
                chassis: VtmChassis::Floor(FloorVtmMotionModel {
                    empty: AxisMotion::Constant { speed_m_s: speed },
                    loaded: AxisMotion::Constant { speed_m_s: speed },
                }),
                lift: AxisMotion::Trapezoidal { max_speed_m_s: 3.0, accel_m_s2: 2.0 },
                handling: HandlingTimes {
                    pick_secs: handling,
                    drop_secs: handling,
                    pick_alignment_secs: 0.0,
                    drop_alignment_secs: 0.0,
                },
            })
        };
        zone.vtm_models = vec![model(10.0, 10.0), model(1.0, 0.0)];
        assert_eq!(zone.min_forward_cycle_cost(RelayId(0), slot(0, 0)), SimDuration::seconds(20));
        assert_eq!(zone.max_forward_cycle_cost(RelayId(0), slot(0, 0)), Some(SimDuration::seconds(22)));
        assert_eq!(zone.min_return_cycle_cost(RelayId(0), slot(0, 0)), SimDuration::seconds(20));
        assert_eq!(zone.max_return_cycle_cost(RelayId(0), slot(0, 0)), Some(SimDuration::seconds(22)));
        zone.vtm_models.clear();
        assert_eq!(zone.min_forward_cycle_cost(RelayId(0), slot(0, 0)), SimDuration::MAX);
        assert_eq!(zone.max_forward_cycle_cost(RelayId(0), slot(0, 0)), None);
        assert_eq!(zone.min_forward_cycle_from(RelayId(0), slot(0, 0), 0.0), None);
    }
}
