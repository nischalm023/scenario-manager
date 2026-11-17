//! Initial HTM positions are sampled without replacement from usable relay
//! access and charging cells. Relay capacity alone must exceed the fleet so
//! charging cells are not required as idle destinations after startup.

use desgraph::{CellId, CellKind};
use desmap::WarehouseMap;

use crate::model::parking::{ParkingRegistry, usable_cells};
use crate::utils::rng::Rng;

pub(crate) fn initial_positions(map: &WarehouseMap, count: usize, seed: u64) -> Result<Vec<CellId>, String> {
    if count == 0 {
        return Ok(Vec::new());
    }

    let relays = ParkingRegistry::from_map(map);
    let relay_count = relays.candidate_count();
    if relay_count <= count {
        return Err(format!(
            "HTM startup requires more usable relay presentation cells than HTMs: \
             {relay_count} usable relay cells for {count} HTMs. Charging cells do not increase idle relay capacity."
        ));
    }

    let usable = usable_cells(map);
    let mut candidates: Vec<_> = relays
        .candidate_cells()
        .chain(
            map.graph
                .cells()
                .filter(|cell| cell.kind == CellKind::Charging && usable.contains(&cell.id))
                .map(|cell| cell.id),
        )
        .collect();
    candidates.sort_unstable();
    candidates.dedup();
    let mut rng = Rng::new(seed);
    for index in (1..candidates.len()).rev() {
        candidates.swap(index, rng.index(index + 1));
    }
    candidates.truncate(count);
    Ok(candidates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use desgraph::{BlockId, Cell, Edge, RelayId, SystemType, WarehouseGraphBuilder, WorldPos, ZoneId};
    use desmap::{BlockSide, RelayPosition, VtmPosition, WarehouseMapBuilder};
    use rustc_hash::FxHashSet;

    fn map(relay_access: &[u32], charging: &[u32], disconnected: &[u32]) -> WarehouseMap {
        let mut graph = WarehouseGraphBuilder::new();
        for id in 0..8 {
            graph.add_cell(Cell {
                id: CellId(id),
                kind: if charging.contains(&id) { CellKind::Charging } else { CellKind::Open },
                zone: ZoneId(0),
                sector: None,
                grid_pos: None,
                world_pos: Some(WorldPos { x_mm: f64::from(id) * 1000.0, y_mm: 0.0 }),
                barcode: None,
                htm_rotation_forbidden: false,
                rack_orientation: None,
                station_id: None,
            });
            if id > 0 && !disconnected.contains(&id) {
                graph.add_edge_bidi(CellId(0), CellId(id), Edge::uniform(1000));
            }
        }
        let mut map = WarehouseMapBuilder::new(SystemType::Relay).graph(graph.build());
        for (id, &cell) in relay_access.iter().enumerate() {
            map = map.add_relay(RelayPosition {
                id: RelayId(id as u32),
                position: CellId(7),
                htm_access: CellId(cell),
                vtm_access: VtmPosition::Cell(CellId(0)),
                block: BlockId(0),
                side: BlockSide::Left,
            });
        }
        map.build()
    }

    #[test]
    fn relay_capacity_counts_distinct_access_cells_and_requires_a_spare() {
        let map = map(&[1, 2, 2], &[3, 4, 5], &[]);
        let error = initial_positions(&map, 2, 42).unwrap_err();
        assert!(error.contains("2 usable relay cells for 2 HTMs"), "{error}");
        assert_eq!(initial_positions(&map, 1, 42).unwrap().len(), 1);
    }

    #[test]
    fn charging_cells_do_not_replace_relay_capacity() {
        let map = map(&[], &[1, 2, 3], &[]);
        let error = initial_positions(&map, 1, 42).unwrap_err();
        assert!(error.contains("0 usable relay cells for 1 HTMs"), "{error}");
        assert!(initial_positions(&map, 0, 42).unwrap().is_empty());
    }

    #[test]
    fn seeded_assignment_is_distinct_and_can_start_at_chargers() {
        let map = map(&[1, 2, 3, 4], &[5, 6], &[]);
        let first = initial_positions(&map, 3, 42).unwrap();
        assert_eq!(first, initial_positions(&map, 3, 42).unwrap());
        assert_eq!(first.iter().collect::<FxHashSet<_>>().len(), 3);
        assert!(first.iter().all(|cell| (1..=6).contains(&cell.0)));
        let sampled: FxHashSet<_> = (1..=32).flat_map(|seed| initial_positions(&map, 3, seed).unwrap()).collect();
        assert!(sampled.contains(&CellId(5)) && sampled.contains(&CellId(6)));
        assert_ne!(first, initial_positions(&map, 3, 1234).unwrap());
    }

    #[test]
    fn unusable_relays_reduce_capacity_and_unusable_chargers_are_not_selected() {
        let sparse = map(&[1, 2, 3], &[], &[3]);
        let error = initial_positions(&sparse, 2, 42).unwrap_err();
        assert!(error.contains("2 usable relay cells for 2 HTMs"), "{error}");

        let map = map(&[1, 2, 3], &[4], &[4]);
        for seed in 1..=32 {
            assert!(initial_positions(&map, 2, seed).unwrap().iter().all(|cell| (1..=3).contains(&cell.0)));
        }
    }
}
