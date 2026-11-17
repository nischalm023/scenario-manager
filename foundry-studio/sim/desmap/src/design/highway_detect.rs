//! Detect highway layout from a compiled `WarehouseMap`.

use rustc_hash::FxHashSet;

use crate::map::WarehouseMap;
use crate::types::*;

use super::types::*;

/// Detect highway configuration by scanning rows and columns.
///
/// Station access cells (StationArea) on highway lanes are treated as highway
/// for detection — they were Highway before station stamping.
pub(super) fn detect_highways(map: &WarehouseMap, w: u32, d: u32) -> Vec<HighwayDefinition> {
    let station_cells: FxHashSet<u32> =
        map.stations.iter().flat_map(|s| s.presentation_and_queue_cells()).map(|cell| cell.0).collect();

    let is_hw = |id: CellId| -> bool {
        map.graph
            .cell(id)
            .map(|c| {
                c.kind == CellKind::Highway
                    || (matches!(c.kind, CellKind::StationArea | CellKind::Presentation | CellKind::QueueCell)
                        && station_cells.contains(&id.0))
            })
            .unwrap_or(false)
    };
    let full_row = |row: u32| (0..w).all(|col| is_hw(CellId(row * w + col)));
    let full_col = |col: u32| (0..d).all(|row| is_hw(CellId(row * w + col)));

    let mut defs = Vec::new();
    scan_rows(map, w, d, &full_row, &mut defs);
    scan_cols(map, w, d, &full_col, &mut defs);
    defs
}

fn scan_rows(map: &WarehouseMap, w: u32, d: u32, full_row: &dyn Fn(u32) -> bool, defs: &mut Vec<HighwayDefinition>) {
    let mut cur: Option<(u32, LaneDirection, u32)> = None;
    for row in 0..d {
        if full_row(row) {
            let dir = row_direction(map, row, w);
            match &mut cur {
                Some((_, cd, h)) if *cd == dir => *h += 1,
                _ => {
                    if let Some((s, d, h)) = cur.take() {
                        defs.push(HighwayDefinition { origin: [0, s], width: w, height: h, direction: d });
                    }
                    cur = Some((row, dir, 1));
                }
            }
        } else if let Some((s, d, h)) = cur.take() {
            defs.push(HighwayDefinition { origin: [0, s], width: w, height: h, direction: d });
        }
    }
    if let Some((s, d, h)) = cur {
        defs.push(HighwayDefinition { origin: [0, s], width: w, height: h, direction: d });
    }
}

fn scan_cols(map: &WarehouseMap, w: u32, d: u32, full_col: &dyn Fn(u32) -> bool, defs: &mut Vec<HighwayDefinition>) {
    let mut cur: Option<(u32, LaneDirection, u32)> = None;
    for col in 0..w {
        if full_col(col) {
            let dir = col_direction(map, col, w, d);
            match &mut cur {
                Some((_, cd, ww)) if *cd == dir => *ww += 1,
                _ => {
                    if let Some((s, dr, ww)) = cur.take() {
                        defs.push(HighwayDefinition { origin: [s, 0], width: ww, height: d, direction: dr });
                    }
                    cur = Some((col, dir, 1));
                }
            }
        } else if let Some((s, dr, ww)) = cur.take() {
            defs.push(HighwayDefinition { origin: [s, 0], width: ww, height: d, direction: dr });
        }
    }
    if let Some((s, dr, ww)) = cur {
        defs.push(HighwayDefinition { origin: [s, 0], width: ww, height: d, direction: dr });
    }
}

/// Detect lane direction for a highway row by sampling horizontal edge traversability.
fn row_direction(map: &WarehouseMap, row: u32, w: u32) -> LaneDirection {
    let (mut eb, mut et, mut wb, mut wt) = (0u32, 0u32, 0u32, 0u32);
    for col in 0..w.saturating_sub(1) {
        let l = CellId(row * w + col);
        let r = CellId(row * w + col + 1);
        if let Some(e) = map.graph.edge_between(l, r) {
            et += 1;
            if e.traversability.htm_traversal == HtmTraversal::None {
                eb += 1;
            }
        }
        if let Some(e) = map.graph.edge_between(r, l) {
            wt += 1;
            if e.traversability.htm_traversal == HtmTraversal::None {
                wb += 1;
            }
        }
    }
    match (et > 0 && eb == et, wt > 0 && wb == wt) {
        (true, true) => LaneDirection::NorthSouthOnly,
        (true, false) => LaneDirection::Westbound,
        (false, true) => LaneDirection::Eastbound,
        _ => LaneDirection::Bidirectional,
    }
}

/// Detect lane direction for a highway column by sampling vertical edge traversability.
fn col_direction(map: &WarehouseMap, col: u32, w: u32, d: u32) -> LaneDirection {
    let (mut sb, mut st, mut nb, mut nt) = (0u32, 0u32, 0u32, 0u32);
    for row in 0..d.saturating_sub(1) {
        let t = CellId(row * w + col);
        let b = CellId((row + 1) * w + col);
        if let Some(e) = map.graph.edge_between(t, b) {
            st += 1;
            if e.traversability.htm_traversal == HtmTraversal::None {
                sb += 1;
            }
        }
        if let Some(e) = map.graph.edge_between(b, t) {
            nt += 1;
            if e.traversability.htm_traversal == HtmTraversal::None {
                nb += 1;
            }
        }
    }
    match (st > 0 && sb == st, nt > 0 && nb == nt) {
        (true, true) => LaneDirection::EastWestOnly,
        (true, false) => LaneDirection::Northbound,
        (false, true) => LaneDirection::Southbound,
        _ => LaneDirection::Bidirectional,
    }
}
