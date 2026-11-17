//! Phase 1: Station placement along grid edges.

use crate::types::*;

use super::types::*;

pub(super) fn resolve_edge_specs(config: &GeneratorConfig) -> Vec<StationEdgeSpec> {
    match &config.station_layout {
        StationLayout::SingleBottom => {
            vec![StationEdgeSpec { edge: StationEdge::Bottom, count: config.station_count, spacing: 5 }]
        }
        StationLayout::SingleTop => {
            vec![StationEdgeSpec { edge: StationEdge::Top, count: config.station_count, spacing: 5 }]
        }
        StationLayout::SingleLeft => {
            vec![StationEdgeSpec { edge: StationEdge::Left, count: config.station_count, spacing: 5 }]
        }
        StationLayout::SingleRight => {
            vec![StationEdgeSpec { edge: StationEdge::Right, count: config.station_count, spacing: 5 }]
        }
        StationLayout::OppositeVertical => {
            let bottom = config.station_count / 2;
            let top = config.station_count - bottom;
            vec![
                StationEdgeSpec { edge: StationEdge::Bottom, count: bottom, spacing: 5 },
                StationEdgeSpec { edge: StationEdge::Top, count: top, spacing: 5 },
            ]
        }
        StationLayout::OppositeHorizontal => {
            let left = config.station_count / 2;
            let right = config.station_count - left;
            vec![
                StationEdgeSpec { edge: StationEdge::Left, count: left, spacing: 5 },
                StationEdgeSpec { edge: StationEdge::Right, count: right, spacing: 5 },
            ]
        }
        StationLayout::FourEdge => {
            let per_edge = config.station_count / 4;
            let remainder = config.station_count % 4;
            let edges = [StationEdge::Bottom, StationEdge::Top, StationEdge::Left, StationEdge::Right];
            edges
                .iter()
                .enumerate()
                .filter_map(|(i, &edge)| {
                    let count = per_edge + if (i as u32) < remainder { 1 } else { 0 };
                    if count > 0 { Some(StationEdgeSpec { edge, count, spacing: 5 }) } else { None }
                })
                .collect()
        }
        StationLayout::Custom(specs) => specs.clone(),
    }
}

pub(super) fn edge_positions(edge: StationEdge, w: u32, d: u32) -> Vec<RC> {
    match edge {
        StationEdge::Bottom => (0..w).map(|col| RC { row: 0, col }).collect(),
        StationEdge::Top => (0..w).map(|col| RC { row: d - 1, col }).collect(),
        StationEdge::Left => (0..d).map(|row| RC { row, col: 0 }).collect(),
        StationEdge::Right => (0..d).map(|row| RC { row, col: w - 1 }).collect(),
    }
}

/// Access points inward: robots approach stations from the warehouse interior.
/// Java's WarehouseMap.getAccessibilityDirection() has a bug — it uses (1,0) for
/// both BOTTOM and TOP because GridCoordinate rejects negative values. The Java
/// mock (WarehouseBuilder) gets it right with (-1,0). Confirmed correct by Cencora
/// GSB data: queue paths approach from interior, f2 traversability flags encode this
/// directionality. Only SINGLE_BOTTOM is used in practice so the Java bug is never
/// triggered.
pub(super) fn access_direction(edge: StationEdge) -> (i32, i32) {
    match edge {
        StationEdge::Bottom => (1, 0),
        StationEdge::Top => (-1, 0),
        StationEdge::Left => (0, 1),
        StationEdge::Right => (0, -1),
    }
}

pub(super) fn place_stations(specs: &[StationEdgeSpec], w: u32, d: u32) -> Vec<StationPlacement> {
    let mut stations = Vec::new();
    let mut next_id = 1u32;

    for spec in specs {
        let positions = edge_positions(spec.edge, w, d);
        let edge_len = positions.len();
        let count = spec.count as usize;
        let spacing = spec.spacing as usize;

        let chosen: Vec<RC> = if count == 1 {
            vec![positions[edge_len / 2]]
        } else {
            let required = (count - 1) * spacing + 1;
            if required > edge_len {
                let actual_spacing = if count > 1 { (edge_len - 1) / (count - 1) } else { 0 };
                let start = (edge_len - ((count - 1) * actual_spacing + 1)) / 2;
                (0..count).map(|i| positions[start + i * actual_spacing]).collect()
            } else {
                let start = (edge_len - required) / 2;
                (0..count).map(|i| positions[start + i * spacing]).collect()
            }
        };

        let dir = access_direction(spec.edge);
        for pos in chosen {
            stations.push(StationPlacement {
                station_id: next_id,
                position: pos,
                edge: spec.edge,
                access_direction: dir,
            });
            next_id += 1;
        }
    }

    stations
}
