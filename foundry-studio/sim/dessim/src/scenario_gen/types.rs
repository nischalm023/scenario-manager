//! Output types for scenario generation.

use std::collections::{BTreeMap, BTreeSet};

use desgraph::{CellId, RelayId, StationId, StationType, WorldPos, ZoneId};
use desmap::VtmPosition;
use desmap::{BlockSide, WarehouseMap};
use serde::{Deserialize, Serialize};

use crate::model::SimDuration;
use crate::model::relay::config::RelayZones;
use crate::model::{OrderSpec, SkuId, StorageSpaceId};
use crate::model::{RackId, ToteDesignation, ToteId, ToteLocation};
use crate::robots::runtime::RuntimeRobots;

use super::demand_pool::DemandPool;
use super::topology::RackUse;

/// Complete generated scenario data, ready to initialize all actors.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScenarioData {
    /// Rebuilt from the source world and settings during replay, never inferred
    /// from positions after deserialization. Missing runtime bindings refuse.
    #[serde(skip)]
    pub robot_models: RuntimeRobots,
    pub map: WarehouseMap,
    pub skus: Vec<SkuInfo>,
    pub orders: Vec<OrderSpec>,
    pub inventory: BTreeMap<(SkuId, StorageSpaceId), u32>,
    pub zone_data: Vec<ZoneSetupData>,
    pub relay_data: Vec<RelaySetupData>,
    pub station_data: Vec<StationSetupData>,
    pub zone_storage: BTreeMap<ZoneId, BTreeSet<CellId>>,
    pub tote_placements: Vec<TotePlacement>,
    pub rack_cells: BTreeMap<RackId, CellId>,
    /// Rack → physical world position override. Populated only for racks in
    /// zones that decouple `slots_per_row` from the grid footprint; racks
    /// absent here sit at their anchor cell's world position.
    #[serde(default)]
    pub rack_world_pos: BTreeMap<RackId, WorldPos>,
    /// Rack → fork-interface floor height in meters. Populated only for
    /// zones with an explicit `rack_levels` table, where every slot is its
    /// own rack with a directly encoded (x, floor) position. Racks absent
    /// here use the zone's level/uniform vertical model.
    #[serde(default)]
    pub rack_floor_m: BTreeMap<RackId, f64>,
    /// Relay → the board-0 exchange position along the row. The VTM meets the
    /// diving board here rather than at the standing cell's centre; drives both
    /// the relay-leg travel distance and the docked position. Absent → falls
    /// back to the standing cell's centre.
    #[serde(default)]
    pub relay_exchange_pos: BTreeMap<RelayId, WorldPos>,
    /// Rack → side of the aisle. Derived during pipeline assembly from the
    /// map's relay columns; consumed by `RelayZones::from_scenario` and the
    /// relay inventory index to enforce same-side VTM cycles.
    #[serde(default)]
    pub rack_side: BTreeMap<RackId, BlockSide>,
    /// Rack → what its steel is FOR: stock, the row of empty order totes, or
    /// the order buffer. Read off the map, not inferred; a rack absent here
    /// is inventory. This is what keeps a deposit landing in racking of the
    /// kind the tote belongs to.
    // ordered: RackId-keyed like every other rack map here, and this one is
    // serialized with the scenario.
    #[serde(default)]
    pub rack_use: BTreeMap<RackId, RackUse>,
    /// Zone → the heights, bottom-up, of the stack of handoff boards each
    /// standing position carries. The lowest is where the exchange happens;
    /// the rest are carried for the two-lane relay model. Empty for a map
    /// that states no board heights, which leaves the VTM model its own
    /// buffer constant.
    #[serde(default)]
    pub board_floors_m: BTreeMap<ZoneId, Vec<f64>>,
    pub rack_placements: Vec<RackPlacement>,
    pub vtm_zone_assignments: Vec<BTreeSet<ZoneId>>,
    pub vtm_initial_positions: Vec<VtmPosition>,
    pub htm_initial_positions: Vec<CellId>,
    pub rack_to_vtm_access: BTreeMap<CellId, CellId>,
    pub vertical_tote_capacity: u16,
    /// Relay zone topology — built from zone_data + relay_data + map graph.
    /// TODO: make this serializable
    #[serde(skip)]
    pub relay_zones: RelayZones,
    /// Bulk demand pool for late-binding order construction. `None` for
    /// pre-formed-order scenarios (Generated/Inline/Csv). Not serialized
    /// — profile replay re-runs `resolve_bulk` from the original config
    /// to rebuild a fresh pool. Pallet-size bounds live on the pool
    /// itself ([`DemandPool::min_pallet_units`] / [`DemandPool::max_pallet_units`])
    /// so they're carried alongside the demand they constrain.
    #[serde(skip)]
    pub demand_pool: Option<DemandPool>,
}

/// SKU metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkuInfo {
    pub id: SkuId,
    pub is_popular: bool,
    pub inventory_units: u32,
    /// Weight in kg (used for pick sequencing; 0.0 for generated scenarios).
    #[serde(default)]
    pub weight_kg: f32,
}

/// Zone setup for Dispatcher.register_zone().
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ZoneSetupData {
    pub zone_id: ZoneId,
    pub relay_ids: BTreeSet<RelayId>,
}

/// Relay setup for Dispatcher.register_relay().
///
/// `side` records which side of the aisle this relay's rack block sits on.
/// A VTM cycle is side-bound: source and destination must share a side.
/// The relay index uses `side` to pair relays only with same-side racks.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RelaySetupData {
    pub relay_id: RelayId,
    pub vtm_access: CellId,
    pub htm_access: CellId,
    pub side: BlockSide,
}

/// Station setup for Dispatcher.register_station().
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StationSetupData {
    pub station_id: StationId,
    /// The cell the flows deliver to (`desmap::Station::delivery_cell`). A
    /// station may present at others, and the simulator knows it by this one
    /// alone.
    pub presentation_cell: CellId,
    pub bins: Option<usize>,
    pub station_type: StationType,
    /// One circuit of this station's fixed conveyance, from the document's
    /// `go.easyloop.loop_transit_secs`. `None` for a station that presents to
    /// an operator directly, which is every station outside DFW6.
    #[serde(default)]
    pub conveyor_transit: Option<SimDuration>,
    /// Where this station presents an ORDER carrier, when that differs from
    /// `presentation_cell`.
    #[serde(default)]
    pub order_presentation_cell: Option<CellId>,
    /// Which machine's conveyance this station feeds. Two stations of one
    /// EasyLoop share it.
    #[serde(default)]
    pub conveyor: Option<u32>,
}

/// Tote placement for Dispatcher.place_tote().
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TotePlacement {
    pub tote_id: ToteId,
    pub location: ToteLocation,
    pub has_tasks: bool,
    /// Which pool this tote belongs to, for its whole life.
    pub designation: ToteDesignation,
}

/// RTP rack placement for Dispatcher.register_rtp_rack().
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RackPlacement {
    pub rack_id: RackId,
    pub pos: CellId,
    pub tasks_by_station: BTreeMap<StationId, u32>,
}

/// Describes the scenario source for display purposes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ScenarioSourceKind {
    Generated,
    Loaded { dir_name: String },
}

/// Summary of a scenario for sidebar display.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScenarioSummary {
    pub source: ScenarioSourceKind,
    pub num_skus: usize,
    pub num_popular_skus: usize,
    pub total_inventory_units: u64,
    pub num_orders: usize,
    pub total_picks: usize,
    pub picks_dropped: usize,
    pub completable: bool,
}
