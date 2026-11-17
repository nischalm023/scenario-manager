//! Top-level warehouse map — the desmap format.

mod navigation_wait;
pub mod queue_edges;
pub mod relay_access;
mod types;

pub use types::{FleetConfig, GridBounds};

use serde::{Deserialize, Serialize};

use crate::graph::WarehouseGraph;
use crate::machine::ConveyorMachine;
use crate::station::{RelayPosition, Station};
use crate::types::*;
use crate::zone::{RackBlock, Sector, Zone};

/// Top-level warehouse map -- the desmap format.
///
/// This is the root serialization target. `serde_json::to_string(&map)` produces
/// the desmap JSON. `serde_json::from_str(&json)` reads it back.
///
/// The graph is immutable after construction. Build with [`WarehouseMapBuilder`].
///
/// Collections are stored as `Vec` (serialized as JSON arrays) for human-readable
/// output. Use the lookup methods for indexed access.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WarehouseMap {
    /// Format version (for forward compatibility).
    pub version: u32,
    /// System type.
    pub system_type: SystemType,
    /// Grid dimensions (only for parametric maps; describes the bounding rectangle).
    pub grid_bounds: Option<GridBounds>,
    /// The navigation graph (cells + edges).
    pub graph: WarehouseGraph,
    /// PPS stations.
    pub stations: Vec<Station>,
    /// Resolved conveyor assemblies, when the source declares their membership.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conveyor_machines: Vec<ConveyorMachine>,
    /// Relay positions (RELAY system only).
    pub relays: Vec<RelayPosition>,
    /// Zones.
    pub zones: Vec<Zone>,
    /// Sectors (optional secondary partitioning).
    pub sectors: Vec<Sector>,
    /// Rack block definitions.
    pub blocks: Vec<RackBlock>,
    /// Fleet configuration (optional; defaults based on system type).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fleet: Option<FleetConfig>,
}

impl WarehouseMap {
    /// Grid position for a cell, when the map encodes one.
    pub fn cell_grid_pos(&self, id: CellId) -> Option<GridPos> {
        // TODO: remove the WarehouseGraph dependency from WarehouseMap. WarehouseGraph should be focused on traversal,
        // and WarehouseMap should be focused on the map topology / spatial layout.
        self.graph.cell(id).and_then(|cell| cell.grid_pos)
    }

    /// World position for a cell, when the map encodes one.
    pub fn cell_world_pos(&self, id: CellId) -> Option<WorldPos> {
        self.graph.cell(id).and_then(|cell| cell.world_pos)
    }

    /// Configured cell size in millimeters. Older/non-parametric maps may not
    /// carry grid bounds; those maps have historically used 1000 mm cells.
    pub fn cell_size_mm(&self) -> u64 {
        self.grid_bounds.as_ref().map(|bounds| bounds.cell_size_mm as u64).unwrap_or(1000)
    }

    /// Find a station by ID.
    pub fn station(&self, id: StationId) -> Option<&Station> {
        self.stations.iter().find(|s| s.id == id)
    }

    /// All stations of a given operational type.
    pub fn stations_of_type(&self, t: StationType) -> Vec<&Station> {
        self.stations.iter().filter(|s| s.station_type == t).collect()
    }

    /// Find a relay position by ID.
    pub fn relay(&self, id: RelayId) -> Option<&RelayPosition> {
        self.relays.iter().find(|r| r.id == id)
    }

    /// Find a zone by ID.
    pub fn zone(&self, id: ZoneId) -> Option<&Zone> {
        self.zones.iter().find(|z| z.id == id)
    }

    /// Find a rack block by ID.
    pub fn block(&self, id: BlockId) -> Option<&RackBlock> {
        self.blocks.iter().find(|b| b.id == id)
    }

    /// Effective fleet configuration, falling back to system-type defaults.
    pub fn fleet_config(&self) -> FleetConfig {
        self.fleet.unwrap_or_else(|| match self.system_type {
            SystemType::Relay => FleetConfig::default_relay(),
            SystemType::Rtp => FleetConfig::default_rtp(),
        })
    }

    /// Total VTM fleet size. The VTM count is fully determined by the map:
    /// each zone deploys [`Zone::vtms_deployed`] VTMs (the same predicate
    /// dessim's `assign_vtm_zones` spawns from). This is the authoritative
    /// number — there is no separate fleet parameter for VTMs.
    pub fn vtm_count(&self) -> usize {
        self.zones.iter().map(|z| z.vtms_deployed() as usize).sum()
    }
}

/// Builder for constructing a [`WarehouseMap`].
pub struct WarehouseMapBuilder {
    version: u32,
    system_type: SystemType,
    grid_bounds: Option<GridBounds>,
    graph: Option<WarehouseGraph>,
    stations: Vec<Station>,
    relays: Vec<RelayPosition>,
    zones: Vec<Zone>,
    sectors: Vec<Sector>,
    blocks: Vec<RackBlock>,
    fleet: Option<FleetConfig>,
}

impl WarehouseMapBuilder {
    pub fn new(system_type: SystemType) -> Self {
        Self {
            version: 1,
            system_type,
            grid_bounds: None,
            graph: None,
            stations: Vec::new(),
            relays: Vec::new(),
            zones: Vec::new(),
            sectors: Vec::new(),
            blocks: Vec::new(),
            fleet: None,
        }
    }

    pub fn fleet(mut self, fleet: FleetConfig) -> Self {
        self.fleet = Some(fleet);
        self
    }

    pub fn grid_bounds(mut self, bounds: GridBounds) -> Self {
        self.grid_bounds = Some(bounds);
        self
    }

    pub fn graph(mut self, graph: WarehouseGraph) -> Self {
        self.graph = Some(graph);
        self
    }

    pub fn add_station(mut self, station: Station) -> Self {
        self.stations.push(station);
        self
    }

    pub fn add_relay(mut self, relay: RelayPosition) -> Self {
        self.relays.push(relay);
        self
    }

    pub fn add_zone(mut self, zone: Zone) -> Self {
        self.zones.push(zone);
        self
    }

    pub fn add_sector(mut self, sector: Sector) -> Self {
        self.sectors.push(sector);
        self
    }

    pub fn add_block(mut self, block: RackBlock) -> Self {
        self.blocks.push(block);
        self
    }

    pub fn build(self) -> WarehouseMap {
        WarehouseMap {
            version: self.version,
            system_type: self.system_type,
            grid_bounds: self.grid_bounds,
            graph: self.graph.expect("graph is required"),
            stations: self.stations,
            conveyor_machines: Vec::new(),
            relays: self.relays,
            zones: self.zones,
            sectors: self.sectors,
            blocks: self.blocks,
            fleet: self.fleet,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{Cell, Edge, WarehouseGraphBuilder};
    use crate::station::PresentationPoint;

    #[test]
    fn serde_round_trip_full_map() {
        let mut gb = WarehouseGraphBuilder::new();
        let cell_size_mm = 1000.0;
        let mut id = 0u32;
        for y in 0..2u32 {
            for x in 0..2u32 {
                gb.add_cell(Cell {
                    id: CellId(id),
                    kind: CellKind::Open,
                    zone: ZoneId(0),
                    sector: None,
                    grid_pos: Some(GridPos { x, y }),
                    world_pos: Some(WorldPos {
                        x_mm: x as f64 * cell_size_mm + cell_size_mm / 2.0,
                        y_mm: y as f64 * cell_size_mm + cell_size_mm / 2.0,
                    }),
                    barcode: None,
                    htm_rotation_forbidden: false,
                    rack_orientation: None,
                    station_id: None,
                });
                id += 1;
            }
        }
        gb.add_edge_bidi(CellId(0), CellId(1), Edge::uniform(cell_size_mm as u64));
        gb.add_edge_bidi(CellId(2), CellId(3), Edge::uniform(cell_size_mm as u64));
        gb.add_edge_bidi(CellId(0), CellId(2), Edge::uniform(cell_size_mm as u64));
        gb.add_edge_bidi(CellId(1), CellId(3), Edge::uniform(cell_size_mm as u64));
        let graph = gb.build();

        let station = Station {
            id: StationId(1),
            outbound_role: None,
            position: CellId(0),
            presentation_points: vec![PresentationPoint { presentation_cell: CellId(1), queue_path: vec![] }],
            edge: Some(StationEdge::Bottom),
            handoff_queues: vec![],
            exit_direction: CardinalDirection::East,
            bins: None,
            dropoff_cell: None,
            collection_cell: None,
            order_presentation_cell: None,
            conveyance_accepts: None,
            conveyor_transit_ms: None,
            conveyor_group: None,
            station_type: StationType::Pick,
        };
        let zone = Zone {
            id: ZoneId(0),
            name: Some("default".into()),
            cells: vec![CellId(0), CellId(1), CellId(2), CellId(3)],
            relays: vec![],
            vtm_count: 0,
            holds: Default::default(),
            vtm_configuration: None,
            zone_height_mm: 0,
            zone_width_mm: None,
            real_shelf_count: 0,
            slots: None,
        };

        let map = WarehouseMapBuilder::new(SystemType::Rtp)
            .grid_bounds(GridBounds { width: 2, depth: 2, cell_size_mm })
            .graph(graph)
            .add_station(station)
            .add_zone(zone)
            .build();

        let json = serde_json::to_string_pretty(&map).unwrap();
        let map2: WarehouseMap = serde_json::from_str(&json).unwrap();

        assert_eq!(map2.version, 1);
        assert_eq!(map2.system_type, SystemType::Rtp);
        assert_eq!(map2.graph.cell_count(), 4);
        assert_eq!(map2.graph.edge_count(), 8);
        assert_eq!(map2.stations.len(), 1);
        assert_eq!(map2.station(StationId(1)).unwrap().position, CellId(0));
        assert_eq!(map2.zones.len(), 1);
        assert_eq!(map2.zone(ZoneId(0)).unwrap().cells.len(), 4);
        assert_eq!(map2.grid_bounds.unwrap().width, 2);

        let cell = map2.graph.cell(CellId(0)).unwrap();
        assert_eq!(cell.kind, CellKind::Open);
        assert_eq!(cell.grid_pos, Some(GridPos { x: 0, y: 0 }));
        assert_eq!(map2.graph.neighbors(CellId(0)).len(), 2);
    }

    #[test]
    fn json_is_human_readable() {
        let mut gb = WarehouseGraphBuilder::new();
        gb.add_cell(Cell {
            id: CellId(0),
            kind: CellKind::Storage,
            zone: ZoneId(1),
            sector: Some(SectorId(0)),
            grid_pos: Some(GridPos { x: 5, y: 10 }),
            world_pos: Some(WorldPos { x_mm: 5500.0, y_mm: 10500.0 }),
            barcode: Some("010.005".into()),
            htm_rotation_forbidden: false,
            rack_orientation: None,
            station_id: None,
        });
        let graph = gb.build();

        let map = WarehouseMapBuilder::new(SystemType::Relay).graph(graph).build();
        let json = serde_json::to_string_pretty(&map).unwrap();

        assert!(json.contains("\"version\""));
        assert!(json.contains("\"system_type\""));
        assert!(json.contains("\"grid_pos\""));
        assert!(json.contains("\"world_pos\""));
        assert!(json.contains("\"x_mm\""));
        assert!(json.contains("\"barcode\""));
        assert!(json.contains("\"zone\""));
        assert!(json.contains("\"kind\""));
        assert!(json.contains("\"Storage\""));
    }
}
