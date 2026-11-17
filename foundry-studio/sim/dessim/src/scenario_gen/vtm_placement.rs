//! VTM zone assignment and initial position computation.

use std::collections::{BTreeMap, BTreeSet};

use desgraph::{CellId, CellKind, ZoneId};
use desmap::{VtmPosition, WarehouseMap, Zone};
use rustc_hash::FxHashMap;

/// Assign VTMs to zones using per-zone vtm_count from the compiled map.
///
/// Each zone gets exactly [`Zone::vtms_deployed`] VTMs (vtm_count for relay
/// zones, default 2 for A5; zero for non-relay zones). Total VTM count is
/// fully determined by the map definition — there is no fleet-size config
/// parameter for VTMs, and `WarehouseMap::vtm_count` reports the same sum.
pub(super) fn assign_vtm_zones(zones: &[Zone]) -> Vec<BTreeSet<ZoneId>> {
    let mut assignments = Vec::new();
    for zone in zones {
        for _ in 0..zone.vtms_deployed() {
            let mut set = BTreeSet::new();
            set.insert(zone.id);
            assignments.push(set);
        }
    }
    assignments
}

/// Compute initial position for each VTM by distributing evenly across
/// the stated rail positions, or existing grid aisle cells, in their zone.
pub(super) fn compute_vtm_positions(
    assignments: &[BTreeSet<ZoneId>],
    map: &WarehouseMap,
    stated: &FxHashMap<ZoneId, Vec<VtmPosition>>,
) -> Vec<VtmPosition> {
    let mut zone_vtm_count: BTreeMap<ZoneId, usize> = BTreeMap::new();
    for zset in assignments {
        if let Some(&zid) = zset.iter().next() {
            *zone_vtm_count.entry(zid).or_insert(0) += 1;
        }
    }

    let mut zone_vtm_index: BTreeMap<ZoneId, usize> = BTreeMap::new();
    let mut positions_by_zone = stated.clone();

    assignments
        .iter()
        .map(|zone_ids| {
            let Some(&zone_id) = zone_ids.iter().next() else {
                return VtmPosition::Cell(CellId(0));
            };

            let candidates = positions_by_zone
                .entry(zone_id)
                .or_insert_with(|| collect_zone_aisle_cells(zone_id, map).into_iter().map(VtmPosition::Cell).collect());

            if candidates.is_empty() {
                return map
                    .zones
                    .iter()
                    .find(|z| z.id == zone_id)
                    .and_then(|z| z.cells.first().copied())
                    .map(VtmPosition::Cell)
                    .unwrap_or(VtmPosition::Cell(CellId(0)));
            }

            let total_in_zone = zone_vtm_count.get(&zone_id).copied().unwrap_or(1);
            let idx = zone_vtm_index.entry(zone_id).or_insert(0);
            let pick = *idx * candidates.len() / total_in_zone;
            *idx += 1;
            candidates[pick]
        })
        .collect()
}

/// Collect sorted aisle cells for a given zone.
fn collect_zone_aisle_cells(zone_id: ZoneId, map: &WarehouseMap) -> Vec<CellId> {
    let Some(zone) = map.zones.iter().find(|z| z.id == zone_id) else {
        return Vec::new();
    };
    let mut aisle_cells: Vec<CellId> = zone
        .cells
        .iter()
        .copied()
        .filter(|&cid| map.graph.cell(cid).map(|c| c.kind == CellKind::Aisle).unwrap_or(false))
        .collect();
    aisle_cells.sort();
    aisle_cells
}
