//! Station placement tests: edge layouts, custom configs, access positions.

use std::collections::HashSet;

use super::super::*;

#[test]
fn station_count_matches_config() {
    let config =
        GeneratorConfig { station_count: 3, station_layout: StationLayout::SingleBottom, ..Default::default() };
    assert_eq!(generate(&config).unwrap().stations.len(), 3);
}

#[test]
fn stations_on_correct_edge() {
    let config = GeneratorConfig { station_count: 2, station_layout: StationLayout::SingleTop, ..Default::default() };
    let map = generate(&config).unwrap();

    for station in &map.stations {
        assert_eq!(station.edge, Some(StationEdge::Top));
        let gp = map.graph.cell(station.position).unwrap().grid_pos.unwrap();
        assert_eq!(gp.y, config.grid_depth - 1);
    }
}

#[test]
fn opposite_vertical_splits_stations() {
    let config =
        GeneratorConfig { station_count: 4, station_layout: StationLayout::OppositeVertical, ..Default::default() };
    let map = generate(&config).unwrap();

    let bottom = map.stations.iter().filter(|s| s.edge == Some(StationEdge::Bottom)).count();
    let top = map.stations.iter().filter(|s| s.edge == Some(StationEdge::Top)).count();
    assert_eq!(bottom, 2);
    assert_eq!(top, 2);
    assert_eq!(map.stations.len(), 4);
}

#[test]
fn station_presentation_cells_are_valid_cells() {
    for layout in [
        StationLayout::SingleBottom,
        StationLayout::SingleTop,
        StationLayout::SingleLeft,
        StationLayout::SingleRight,
        StationLayout::OppositeVertical,
        StationLayout::OppositeHorizontal,
    ] {
        let config = GeneratorConfig { station_count: 2, station_layout: layout, ..Default::default() };
        let map = generate(&config).unwrap();

        for station in &map.stations {
            assert!(
                map.graph.cell(station.position).is_some(),
                "station {:?} position {:?} is not a valid cell",
                station.id,
                station.position
            );
            let [point] = station.presentation_points.as_slice() else {
                panic!("station {:?} presents at one cell", station.id)
            };
            assert!(
                map.graph.cell(point.presentation_cell).is_some(),
                "station {:?} presentation_cell {:?} is not a valid cell (edge: {:?})",
                station.id,
                point.presentation_cell,
                station.edge
            );
        }
    }
}

#[test]
fn four_edge_layout_works() {
    let config = GeneratorConfig {
        grid_width: 30,
        grid_depth: 30,
        station_count: 8,
        station_layout: StationLayout::FourEdge,
        ..Default::default()
    };
    let map = generate(&config).unwrap();

    assert_eq!(map.stations.len(), 8);
    let bottom = map.stations.iter().filter(|s| s.edge == Some(StationEdge::Bottom)).count();
    let top = map.stations.iter().filter(|s| s.edge == Some(StationEdge::Top)).count();
    let left = map.stations.iter().filter(|s| s.edge == Some(StationEdge::Left)).count();
    let right = map.stations.iter().filter(|s| s.edge == Some(StationEdge::Right)).count();
    assert_eq!(bottom, 2);
    assert_eq!(top, 2);
    assert_eq!(left, 2);
    assert_eq!(right, 2);

    for station in &map.stations {
        let [point] = station.presentation_points.as_slice() else {
            panic!("station {:?} presents at one cell", station.id)
        };
        assert!(
            map.graph.cell(point.presentation_cell).is_some(),
            "station {:?} presentation_cell {:?} is not a valid cell (edge: {:?})",
            station.id,
            point.presentation_cell,
            station.edge
        );
    }
}

#[test]
fn custom_station_layout_works() {
    let config = GeneratorConfig {
        grid_width: 30,
        grid_depth: 30,
        station_count: 5,
        station_layout: StationLayout::Custom(vec![
            StationEdgeSpec { edge: StationEdge::Bottom, count: 3, spacing: 5 },
            StationEdgeSpec { edge: StationEdge::Right, count: 2, spacing: 8 },
        ]),
        ..Default::default()
    };
    let map = generate(&config).unwrap();

    assert_eq!(map.stations.len(), 5);
    let bottom = map.stations.iter().filter(|s| s.edge == Some(StationEdge::Bottom)).count();
    let right = map.stations.iter().filter(|s| s.edge == Some(StationEdge::Right)).count();
    assert_eq!(bottom, 3);
    assert_eq!(right, 2);

    for station in &map.stations {
        let [point] = station.presentation_points.as_slice() else {
            panic!("station {:?} presents at one cell", station.id)
        };
        assert!(map.graph.cell(point.presentation_cell).is_some());
    }
    for cell in map.graph.cells() {
        assert!(map.zone(cell.zone).is_some(), "cell {:?} zone {:?} not found", cell.id, cell.zone);
    }
}

#[test]
fn no_duplicate_stations() {
    let config = GeneratorConfig {
        station_count: 8,
        station_layout: StationLayout::FourEdge,
        grid_width: 30,
        grid_depth: 30,
        ..Default::default()
    };
    let map = generate(&config).unwrap();
    let mut positions: HashSet<CellId> = HashSet::new();
    for station in &map.stations {
        assert!(positions.insert(station.position), "duplicate station position: {:?}", station.position);
    }
}
