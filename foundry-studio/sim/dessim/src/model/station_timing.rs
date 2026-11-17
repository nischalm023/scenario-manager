use std::collections::BTreeMap;

use desgraph::{CellId, StationId};
use desmap::WarehouseMap;

use crate::error::PlanError;
use crate::model::station::Station;
use crate::model::{SimDuration, SimTime};
use crate::pathfinding::estimate::{cell_point_mm, manhattan_mm, travel_estimate};

/// Max wait policy for station-bound HTM delivery searches.
///
/// This is intentionally shared by station-bound quotes and `HtmDeliver`
/// commits. A quote that allows a larger wait than the commit can realize will
/// select proposals that later fail; a quote that is stricter than the commit
/// leaves usable station slots idle.
///
/// Strict (tripwires=true): 900s. The original diagnostic value — large
/// enough for normal cencora_phase0 / minimal scenarios but not the deeper
/// queue-saturation cases on cencora_phase2.
pub const STATION_DELIVERY_MAX_WAIT_STRICT: SimDuration = SimDuration::seconds(900);

/// Loose (tripwires=false, default): 24h. Effectively unbounded — lets
/// cencora-scale runs find paths past long station-presentation queues
/// even when throughput is poor. Re-enable strictness with `--tripwires`
/// when investigating timing drift.
pub const STATION_DELIVERY_MAX_WAIT_LOOSE: SimDuration = SimDuration::seconds(86_400);

pub fn station_delivery_max_wait(tripwires: bool) -> SimDuration {
    if tripwires { STATION_DELIVERY_MAX_WAIT_STRICT } else { STATION_DELIVERY_MAX_WAIT_LOOSE }
}

/// Per-station conservative fetch bounds.
///
/// This is the active-planner equivalent of the quarantine-era `F_max`
/// calculation. It is intentionally Manhattan/grid based: callers use it as a
/// control lookback bound, not as a precise path quote. Precise proposal
/// selection still goes through the pathfinder/cache.
#[derive(Clone, Debug, Default)]
pub struct StationFetchBounds {
    global_f_max: SimDuration,
    per_station_f_max: BTreeMap<StationId, SimDuration>,
}

impl StationFetchBounds {
    /// Compute per-station `F_max` from a set of station-bound source cells.
    ///
    /// `source_cells` are floor cells where an HTM can begin the final
    /// station-bound travel leg. For putaway from-rack work, these are relay
    /// HTM-access cells: the VTM places the tote at a relay slot above that
    /// floor cell, then an HTM picks it up and drives from that cell to the
    /// putaway station. They are not rack slots, VTM rail cells, or parking
    /// cells.
    ///
    /// `fixed_offset` is caller-provided upstream work that is not represented
    /// by the HTM floor leg itself, such as VTM rack->relay cycle time plus a
    /// conservative HTM pickup/relay-handoff allowance.
    pub fn from_source_cells<'a>(
        stations: impl IntoIterator<Item = &'a Station>,
        source_cells: impl IntoIterator<Item = CellId>,
        map: &WarehouseMap,
        traverse: SimDuration,
        fixed_offset: SimDuration,
    ) -> Result<Self, PlanError> {
        let cell_size_mm = map.cell_size_mm();
        let sources = source_cells
            .into_iter()
            .map(|cell| {
                let pos = cell_point_mm(map, cell).ok_or_else(|| PlanError::TaskGraphInvariant {
                    context: "StationFetchBounds",
                    detail: format!("source cell {:?} has no position of any kind", cell),
                })?;
                Ok((cell, pos))
            })
            .collect::<Result<Vec<_>, PlanError>>()?;

        let mut global_f_max = SimDuration::ZERO;
        let mut per_station_f_max = BTreeMap::new();

        for station in stations {
            let station_pos =
                cell_point_mm(map, station.presentation_cell).ok_or_else(|| PlanError::TaskGraphInvariant {
                    context: "StationFetchBounds",
                    detail: format!(
                        "station {:?} presentation cell {:?} has no position of any kind",
                        station.id, station.presentation_cell
                    ),
                })?;

            let mut max_manhattan = SimDuration::ZERO;
            for &(_, source_pos) in &sources {
                let estimate = travel_estimate(manhattan_mm(source_pos, station_pos), cell_size_mm, traverse);
                max_manhattan = max_manhattan.max(estimate);
            }
            let f_max = max_manhattan + fixed_offset;

            global_f_max = global_f_max.max(f_max);
            per_station_f_max.insert(station.id, f_max);
        }

        Ok(Self { global_f_max, per_station_f_max })
    }

    pub fn f_max(&self, station_id: StationId) -> SimDuration {
        self.per_station_f_max.get(&station_id).copied().unwrap_or(self.global_f_max)
    }

    pub fn lookback_start(&self, station_id: StationId, target: SimTime) -> SimTime {
        target.saturating_sub(self.f_max(station_id))
    }
}

#[cfg(test)]
mod tests {
    use desgraph::{
        CardinalDirection, Cell, CellId, CellKind, GridPos, StationId, StationType, WarehouseGraphBuilder, ZoneId,
    };
    use desmap::{SystemType, WarehouseMapBuilder};

    use super::*;
    use crate::model::station::Station;

    #[test]
    fn computes_per_station_f_max_with_fixed_offset() {
        let stations = vec![
            Station::new(StationId(1), StationType::Putaway, CellId(10), vec![], CardinalDirection::North, 1),
            Station::new(StationId(2), StationType::Putaway, CellId(20), vec![], CardinalDirection::North, 1),
        ];
        let mut builder = WarehouseGraphBuilder::new();
        builder.add_cell(Cell {
            id: CellId(10),
            kind: CellKind::Open,
            zone: ZoneId(0),
            sector: None,
            grid_pos: Some(GridPos { x: 0, y: 0 }),
            world_pos: None,
            barcode: None,
            htm_rotation_forbidden: false,
            rack_orientation: None,
            station_id: None,
        });
        builder.add_cell(Cell {
            id: CellId(20),
            kind: CellKind::Open,
            zone: ZoneId(0),
            sector: None,
            grid_pos: Some(GridPos { x: 10, y: 0 }),
            world_pos: None,
            barcode: None,
            htm_rotation_forbidden: false,
            rack_orientation: None,
            station_id: None,
        });
        builder.add_cell(Cell {
            id: CellId(100),
            kind: CellKind::Open,
            zone: ZoneId(0),
            sector: None,
            grid_pos: Some(GridPos { x: 3, y: 0 }),
            world_pos: None,
            barcode: None,
            htm_rotation_forbidden: false,
            rack_orientation: None,
            station_id: None,
        });
        builder.add_cell(Cell {
            id: CellId(200),
            kind: CellKind::Open,
            zone: ZoneId(0),
            sector: None,
            grid_pos: Some(GridPos { x: 20, y: 0 }),
            world_pos: None,
            barcode: None,
            htm_rotation_forbidden: false,
            rack_orientation: None,
            station_id: None,
        });
        let graph = builder.build();
        let map = WarehouseMapBuilder::new(SystemType::Relay).graph(graph).build();

        let bounds = StationFetchBounds::from_source_cells(
            stations.iter(),
            [CellId(100), CellId(200)],
            &map,
            SimDuration::ms(1000),
            SimDuration::seconds(5),
        )
        .unwrap();

        assert_eq!(bounds.f_max(StationId(1)), SimDuration::seconds(25));
        assert_eq!(bounds.f_max(StationId(2)), SimDuration::seconds(15));
        assert_eq!(bounds.lookback_start(StationId(1), SimTime::seconds(30)), SimTime::seconds(5));
    }
}
