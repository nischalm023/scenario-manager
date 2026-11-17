//! One-shot fixture generator.
//!
//! Run with: `cargo test --test generate_fixtures -- --ignored generate_all_fixtures`
//!
//! This generates all JSON fixture files in `tests/fixtures/`. The actual
//! serialization tests live in `fixture_tests.rs` and verify that deserializing
//! each fixture matches what the generator produces.

use desmap::generator::{GeneratorConfig, StationEdgeSpec, StationLayout};
use desmap::types::{StationEdge, SystemType};
use std::fs;
use std::path::Path;

/// Write a fixture file as design JSON (v3 format).
fn write_fixture(dir: &Path, name: &str, config: &GeneratorConfig) {
    let design = desmap::generator::generate_design(config)
        .unwrap_or_else(|e| panic!("generate_design failed for {}: {}", name, e));
    let json = serde_json::to_string_pretty(&design).expect("serialize failed");

    // Verify the design compiles correctly before writing
    let map = design.compile().expect("design compile failed");

    let path = dir.join(format!("{}.json", name));
    fs::write(&path, &json).unwrap_or_else(|e| panic!("write failed for {:?}: {}", path, e));
    eprintln!(
        "wrote {} ({} cells, {} stations, {} relays, {} blocks, {} zones, {} KB)",
        path.display(),
        map.graph.cell_count(),
        map.stations.len(),
        map.relays.len(),
        map.blocks.len(),
        map.zones.len(),
        json.len() / 1024,
    );
}

#[test]
#[ignore]
fn generate_all_fixtures() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    fs::create_dir_all(&dir).unwrap();

    // -----------------------------------------------------------------------
    // 1. Java codebase configs
    // -----------------------------------------------------------------------

    // 1a. Default config (Rust defaults, matching Java defaults)
    write_fixture(&dir, "default_rtp", &GeneratorConfig::default());

    // 1b. smoke-test-rtp.json from Java
    write_fixture(
        &dir,
        "java_smoke_rtp",
        &GeneratorConfig {
            grid_width: 20,
            grid_depth: 20,
            cell_size_mm: 1000.0,
            station_count: 4,
            station_layout: StationLayout::SingleBottom,
            highways_per_edge: 2,
            rows_per_block: 4,
            rack_columns: 2,
            cross_aisle_width: 1,
            system_type: SystemType::Rtp,
        },
    );

    // 1c. smoke-test-relay.json from Java
    write_fixture(
        &dir,
        "java_smoke_relay",
        &GeneratorConfig {
            grid_width: 20,
            grid_depth: 20,
            cell_size_mm: 1000.0,
            station_count: 1,
            station_layout: StationLayout::SingleBottom,
            highways_per_edge: 2,
            rows_per_block: 4,
            rack_columns: 2,
            cross_aisle_width: 1,
            system_type: SystemType::Relay,
        },
    );

    // 1d. smoke-test-shuttle.json from Java (50x30, relay, 2 stations, 3 rack cols)
    write_fixture(
        &dir,
        "java_smoke_shuttle",
        &GeneratorConfig {
            grid_width: 50,
            grid_depth: 30,
            cell_size_mm: 1000.0,
            station_count: 2,
            station_layout: StationLayout::SingleBottom,
            highways_per_edge: 3,
            rows_per_block: 10,
            rack_columns: 3,
            cross_aisle_width: 1,
            system_type: SystemType::Relay,
        },
    );

    // 1e. integration-test-config.json from Java (22x26, RTP, 3 highways)
    write_fixture(
        &dir,
        "java_integration_rtp",
        &GeneratorConfig {
            grid_width: 22,
            grid_depth: 26,
            cell_size_mm: 1000.0,
            station_count: 4,
            station_layout: StationLayout::SingleBottom,
            highways_per_edge: 3,
            rows_per_block: 10,
            rack_columns: 2,
            cross_aisle_width: 1,
            system_type: SystemType::Rtp,
        },
    );

    // 1f. relay-test-config.json from Java (30x30, relay, 2 stations)
    write_fixture(
        &dir,
        "java_relay_30x30",
        &GeneratorConfig {
            grid_width: 30,
            grid_depth: 30,
            cell_size_mm: 1000.0,
            station_count: 2,
            station_layout: StationLayout::SingleBottom,
            highways_per_edge: 3,
            rows_per_block: 10,
            rack_columns: 2,
            cross_aisle_width: 1,
            system_type: SystemType::Relay,
        },
    );

    // -----------------------------------------------------------------------
    // 2. StationLayout variants
    // -----------------------------------------------------------------------

    // 2a. SingleTop
    write_fixture(
        &dir,
        "layout_single_top",
        &GeneratorConfig {
            grid_width: 20,
            grid_depth: 20,
            station_count: 3,
            station_layout: StationLayout::SingleTop,
            ..Default::default()
        },
    );

    // 2b. SingleLeft
    write_fixture(
        &dir,
        "layout_single_left",
        &GeneratorConfig {
            grid_width: 20,
            grid_depth: 20,
            station_count: 2,
            station_layout: StationLayout::SingleLeft,
            ..Default::default()
        },
    );

    // 2c. SingleRight
    write_fixture(
        &dir,
        "layout_single_right",
        &GeneratorConfig {
            grid_width: 20,
            grid_depth: 20,
            station_count: 2,
            station_layout: StationLayout::SingleRight,
            ..Default::default()
        },
    );

    // 2d. OppositeVertical
    write_fixture(
        &dir,
        "layout_opposite_vertical",
        &GeneratorConfig {
            grid_width: 20,
            grid_depth: 20,
            station_count: 6,
            station_layout: StationLayout::OppositeVertical,
            ..Default::default()
        },
    );

    // 2e. OppositeHorizontal
    write_fixture(
        &dir,
        "layout_opposite_horizontal",
        &GeneratorConfig {
            grid_width: 20,
            grid_depth: 20,
            station_count: 4,
            station_layout: StationLayout::OppositeHorizontal,
            ..Default::default()
        },
    );

    // 2f. FourEdge
    write_fixture(
        &dir,
        "layout_four_edge",
        &GeneratorConfig {
            grid_width: 30,
            grid_depth: 30,
            station_count: 8,
            station_layout: StationLayout::FourEdge,
            ..Default::default()
        },
    );

    // 2g. Custom layout
    write_fixture(
        &dir,
        "layout_custom",
        &GeneratorConfig {
            grid_width: 30,
            grid_depth: 30,
            station_count: 5,
            station_layout: StationLayout::Custom(vec![
                StationEdgeSpec { edge: StationEdge::Bottom, count: 3, spacing: 5 },
                StationEdgeSpec { edge: StationEdge::Right, count: 2, spacing: 8 },
            ]),
            ..Default::default()
        },
    );

    // -----------------------------------------------------------------------
    // 3. Client requirement scenarios
    // -----------------------------------------------------------------------

    // 3a. Cencora-like: 56x34 grid, 10 stations (closest parametric approximation)
    write_fixture(
        &dir,
        "cencora_like",
        &GeneratorConfig {
            grid_width: 56,
            grid_depth: 34,
            cell_size_mm: 1380.0, // Cencora cells are 1380mm tall
            station_count: 10,
            station_layout: StationLayout::SingleBottom,
            highways_per_edge: 3,
            rows_per_block: 10,
            rack_columns: 2,
            cross_aisle_width: 1,
            system_type: SystemType::Relay,
        },
    );

    // 3b. Phase 0 HTM-only: relay system with 10 pick stations, suitable for 400 bots
    write_fixture(
        &dir,
        "phase0_htm_only",
        &GeneratorConfig {
            grid_width: 60,
            grid_depth: 40,
            cell_size_mm: 1000.0,
            station_count: 10,
            station_layout: StationLayout::SingleBottom,
            highways_per_edge: 3,
            rows_per_block: 10,
            rack_columns: 2,
            cross_aisle_width: 1,
            system_type: SystemType::Relay,
        },
    );

    // 3c. Scale test: 100x100 with 20 stations
    write_fixture(
        &dir,
        "scale_100x100",
        &GeneratorConfig {
            grid_width: 100,
            grid_depth: 100,
            cell_size_mm: 1000.0,
            station_count: 20,
            station_layout: StationLayout::OppositeVertical,
            highways_per_edge: 3,
            rows_per_block: 10,
            rack_columns: 2,
            cross_aisle_width: 1,
            system_type: SystemType::Relay,
        },
    );

    // 3d. Large scale: 200x200 with 40 stations (max grid, production-scale)
    write_fixture(
        &dir,
        "scale_200x200",
        &GeneratorConfig {
            grid_width: 200,
            grid_depth: 200,
            cell_size_mm: 1000.0,
            station_count: 40,
            station_layout: StationLayout::FourEdge,
            highways_per_edge: 3,
            rows_per_block: 10,
            rack_columns: 2,
            cross_aisle_width: 1,
            system_type: SystemType::Relay,
        },
    );

    // 3e. 500-bot scenario: grid large enough to support 500 bots with 10 stations
    write_fixture(
        &dir,
        "scenario_500_bots",
        &GeneratorConfig {
            grid_width: 80,
            grid_depth: 60,
            cell_size_mm: 1000.0,
            station_count: 10,
            station_layout: StationLayout::SingleBottom,
            highways_per_edge: 3,
            rows_per_block: 10,
            rack_columns: 2,
            cross_aisle_width: 1,
            system_type: SystemType::Relay,
        },
    );

    // -----------------------------------------------------------------------
    // 4. Edge cases
    // -----------------------------------------------------------------------

    // 4a. Minimum grid (5x5) -- likely zero rack blocks
    write_fixture(
        &dir,
        "edge_minimum_grid",
        &GeneratorConfig {
            grid_width: 5,
            grid_depth: 5,
            cell_size_mm: 1000.0,
            station_count: 1,
            station_layout: StationLayout::SingleBottom,
            highways_per_edge: 1,
            rows_per_block: 3,
            rack_columns: 1,
            cross_aisle_width: 1,
            system_type: SystemType::Rtp,
        },
    );

    // 4b. Single station
    write_fixture(
        &dir,
        "edge_single_station",
        &GeneratorConfig {
            grid_width: 15,
            grid_depth: 15,
            cell_size_mm: 1000.0,
            station_count: 1,
            station_layout: StationLayout::SingleBottom,
            highways_per_edge: 2,
            rows_per_block: 5,
            rack_columns: 2,
            cross_aisle_width: 1,
            system_type: SystemType::Rtp,
        },
    );

    // 4c. Maximum stations that fit on one edge (50 on a 200-wide grid)
    write_fixture(
        &dir,
        "edge_max_stations",
        &GeneratorConfig {
            grid_width: 200,
            grid_depth: 30,
            cell_size_mm: 1000.0,
            station_count: 40,
            station_layout: StationLayout::SingleBottom,
            highways_per_edge: 3,
            rows_per_block: 10,
            rack_columns: 2,
            cross_aisle_width: 1,
            system_type: SystemType::Rtp,
        },
    );

    // 4d. Narrow grid: 5x50
    write_fixture(
        &dir,
        "edge_narrow_5x50",
        &GeneratorConfig {
            grid_width: 5,
            grid_depth: 50,
            cell_size_mm: 1000.0,
            station_count: 1,
            station_layout: StationLayout::SingleBottom,
            highways_per_edge: 1,
            rows_per_block: 10,
            rack_columns: 1,
            cross_aisle_width: 1,
            system_type: SystemType::Rtp,
        },
    );

    // 4e. Narrow grid: 50x5
    write_fixture(
        &dir,
        "edge_narrow_50x5",
        &GeneratorConfig {
            grid_width: 50,
            grid_depth: 5,
            cell_size_mm: 1000.0,
            station_count: 2,
            station_layout: StationLayout::SingleBottom,
            highways_per_edge: 1,
            rows_per_block: 3,
            rack_columns: 2,
            cross_aisle_width: 1,
            system_type: SystemType::Rtp,
        },
    );

    // 4f. Large cell size (2000mm)
    write_fixture(
        &dir,
        "edge_large_cell_size",
        &GeneratorConfig {
            grid_width: 20,
            grid_depth: 20,
            cell_size_mm: 2000.0,
            station_count: 4,
            station_layout: StationLayout::SingleBottom,
            highways_per_edge: 3,
            rows_per_block: 10,
            rack_columns: 2,
            cross_aisle_width: 1,
            system_type: SystemType::Rtp,
        },
    );

    // 4g. Small cell size (500mm)
    write_fixture(
        &dir,
        "edge_small_cell_size",
        &GeneratorConfig {
            grid_width: 20,
            grid_depth: 20,
            cell_size_mm: 500.0,
            station_count: 4,
            station_layout: StationLayout::SingleBottom,
            highways_per_edge: 3,
            rows_per_block: 10,
            rack_columns: 2,
            cross_aisle_width: 1,
            system_type: SystemType::Rtp,
        },
    );

    // 4h. Maximum highways per edge (6)
    write_fixture(
        &dir,
        "edge_max_highways",
        &GeneratorConfig {
            grid_width: 30,
            grid_depth: 30,
            cell_size_mm: 1000.0,
            station_count: 2,
            station_layout: StationLayout::SingleBottom,
            highways_per_edge: 6,
            rows_per_block: 10,
            rack_columns: 2,
            cross_aisle_width: 1,
            system_type: SystemType::Rtp,
        },
    );

    // 4i. Maximum rows per block (20)
    write_fixture(
        &dir,
        "edge_max_rows_per_block",
        &GeneratorConfig {
            grid_width: 30,
            grid_depth: 50,
            cell_size_mm: 1000.0,
            station_count: 2,
            station_layout: StationLayout::SingleBottom,
            highways_per_edge: 3,
            rows_per_block: 20,
            rack_columns: 2,
            cross_aisle_width: 1,
            system_type: SystemType::Rtp,
        },
    );

    // 4j. Maximum rack columns (5)
    write_fixture(
        &dir,
        "edge_max_rack_columns",
        &GeneratorConfig {
            grid_width: 40,
            grid_depth: 30,
            cell_size_mm: 1000.0,
            station_count: 2,
            station_layout: StationLayout::SingleBottom,
            highways_per_edge: 3,
            rows_per_block: 10,
            rack_columns: 5,
            cross_aisle_width: 1,
            system_type: SystemType::Rtp,
        },
    );

    // 4k. Cross aisle width 2
    write_fixture(
        &dir,
        "edge_cross_aisle_2",
        &GeneratorConfig {
            grid_width: 20,
            grid_depth: 30,
            cell_size_mm: 1000.0,
            station_count: 2,
            station_layout: StationLayout::SingleBottom,
            highways_per_edge: 3,
            rows_per_block: 5,
            rack_columns: 2,
            cross_aisle_width: 2,
            system_type: SystemType::Rtp,
        },
    );

    // 4l. Relay on minimum viable grid (smallest grid that produces both blocks and relays)
    write_fixture(
        &dir,
        "edge_relay_small",
        &GeneratorConfig {
            grid_width: 10,
            grid_depth: 10,
            cell_size_mm: 1000.0,
            station_count: 1,
            station_layout: StationLayout::SingleBottom,
            highways_per_edge: 1,
            rows_per_block: 3,
            rack_columns: 1,
            cross_aisle_width: 1,
            system_type: SystemType::Relay,
        },
    );

    eprintln!("\nAll fixtures generated successfully.");
}
