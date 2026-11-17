//! Phases 2-4: Highway placement, occupied positions, rack block tiling.

use std::collections::BTreeSet;

use crate::types::*;

use super::stations::edge_positions;
use super::types::*;

pub(super) fn place_highways(
    station_edges: &BTreeSet<StationEdge>,
    highways_per_edge: u32,
    w: u32,
    d: u32,
) -> Vec<HighwaySegment> {
    let mut highways = Vec::new();
    for &edge in station_edges {
        for i in 0..highways_per_edge {
            let seg = match edge {
                StationEdge::Bottom => {
                    let row = (1 + i).min(d - 1);
                    HighwaySegment { start: RC { row, col: 0 }, end: RC { row, col: w - 1 } }
                }
                StationEdge::Top => {
                    let row = d.saturating_sub(2 + i);
                    HighwaySegment { start: RC { row, col: 0 }, end: RC { row, col: w - 1 } }
                }
                StationEdge::Left => {
                    let col = (1 + i).min(w - 1);
                    HighwaySegment { start: RC { row: 0, col }, end: RC { row: d - 1, col } }
                }
                StationEdge::Right => {
                    let col = w.saturating_sub(2 + i);
                    HighwaySegment { start: RC { row: 0, col }, end: RC { row: d - 1, col } }
                }
            };
            highways.push(seg);
        }
    }
    highways
}

fn highway_positions(highways: &[HighwaySegment]) -> BTreeSet<RC> {
    let mut set = BTreeSet::new();
    for seg in highways {
        let (min_row, max_row) = (seg.start.row.min(seg.end.row), seg.start.row.max(seg.end.row));
        let (min_col, max_col) = (seg.start.col.min(seg.end.col), seg.start.col.max(seg.end.col));
        for row in min_row..=max_row {
            for col in min_col..=max_col {
                set.insert(RC { row, col });
            }
        }
    }
    set
}

pub(super) fn calculate_occupied(
    stations: &[StationPlacement],
    highways: &[HighwaySegment],
    station_edges_used: &BTreeSet<StationEdge>,
    w: u32,
    d: u32,
) -> BTreeSet<RC> {
    let mut occupied = BTreeSet::new();

    for s in stations {
        occupied.insert(s.position);
    }

    for &edge in station_edges_used {
        for pos in edge_positions(edge, w, d) {
            occupied.insert(pos);
        }
    }

    // Border buffer: edges WITHOUT stations are also blocked for rack placement.
    if !station_edges_used.contains(&StationEdge::Bottom) {
        for col in 0..w {
            occupied.insert(RC { row: 0, col });
        }
    }
    if !station_edges_used.contains(&StationEdge::Top) {
        for col in 0..w {
            occupied.insert(RC { row: d - 1, col });
        }
    }
    if !station_edges_used.contains(&StationEdge::Left) {
        for row in 0..d {
            occupied.insert(RC { row, col: 0 });
        }
    }
    if !station_edges_used.contains(&StationEdge::Right) {
        for row in 0..d {
            occupied.insert(RC { row, col: w - 1 });
        }
    }

    for pos in highway_positions(highways) {
        occupied.insert(pos);
    }
    occupied
}

pub(super) fn place_rack_blocks(
    occupied: &BTreeSet<RC>,
    rows_per_block: u32,
    rack_columns: u32,
    cross_aisle_width: u32,
    inter_block_gap: u32,
    w: u32,
    d: u32,
) -> Vec<BlockPlacement> {
    let mut available = BTreeSet::new();
    for row in 0..d {
        for col in 0..w {
            let rc = RC { row, col };
            if !occupied.contains(&rc) {
                available.insert(rc);
            }
        }
    }

    if available.is_empty() {
        return Vec::new();
    }

    let min_row = available.iter().map(|rc| rc.row).min().unwrap();
    let max_row = available.iter().map(|rc| rc.row).max().unwrap();
    let min_col = available.iter().map(|rc| rc.col).min().unwrap();
    let max_col = available.iter().map(|rc| rc.col).max().unwrap();

    let mut blocks = Vec::new();
    let mut block_id = 1u32;
    let mut current_row = min_row;

    while current_row <= max_row {
        let block_end_row = (current_row + rows_per_block - 1).min(max_row);
        let mut current_col = min_col;

        while current_col <= max_col {
            let block_end_col = (current_col + rack_columns - 1).min(max_col);
            if block_end_row > max_row || block_end_col > max_col {
                break;
            }

            let mut all_available = true;
            let mut count = 0u32;
            'outer: for row in current_row..=block_end_row {
                for col in current_col..=block_end_col {
                    if !available.contains(&RC { row, col }) {
                        all_available = false;
                        break 'outer;
                    }
                    count += 1;
                }
            }

            if all_available && count == rack_columns * rows_per_block {
                blocks.push(BlockPlacement {
                    block_id,
                    top_left: RC { row: current_row, col: current_col },
                    bottom_right: RC { row: block_end_row, col: block_end_col },
                });
                block_id += 1;
            }

            current_col = block_end_col + 1 + inter_block_gap;
        }

        current_row = block_end_row + 1 + cross_aisle_width;
    }

    blocks
}
