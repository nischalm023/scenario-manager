//! What a two-run, stacked-board, surveyed-shelf world document becomes.
//!
//! `minimal_putaway2` is one run beside one aisle with one board per bay and a
//! uniform floor. cencora is the opposite of all four: nineteen aisles with a
//! run on each side, three board positions per bay carrying two shelves each,
//! eleven surveyed shelf heights, and a floor cut into three bands at two
//! pitches. Every assumption the first fixture could not test is tested here.
//!
//! The committed world document originated in the desmap converter and now
//! includes audited corrections. These tests always load that committed copy.
//! Counts come from the source geometry and explicit access-cell additions.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use desgraph::CellId;
use dessim::worldmap::{self, WorldMap};

/// The COMMITTED conversion of cencora_v3, not the converter's build output.
///
/// These pins used to read `desmap2proto/target/converted/`, a gitignored
/// directory, and return `None` when it was absent -- so every test below
/// passed by skipping in any tree that had not just run the converter. The
/// document is committed beside this file precisely so the suite cannot go
/// vacuous. Do not overwrite its audited corrections with converter output.
fn cencora() -> WorldMap {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cencora_v3.json");
    worldmap::load(&path).expect("load the committed cencora_v3 world document")
}

/// 19 zones, each a `Dual` sandwich: two runs of 6 bays facing one aisle.
const ZONES: usize = 19;
const BAYS: usize = ZONES * 2 * 6;

#[test]
fn two_runs_share_one_aisle() {
    let world = cencora();
    assert_eq!(world.map.zones.len(), ZONES);
    for zone in &world.map.zones {
        // Only the twelve rack anchors belong to the zone's cell collection.
        assert_eq!(zone.cells.len(), 12);
        // 12 bays × 3 distinct board positions.
        assert_eq!(zone.relays.len(), 36);
        // One cohort of 2, hard-assigned to the sector that names both runs —
        // counted once for the zone, not once per run.
        assert_eq!(zone.vtm_count, 2);
    }
    assert_eq!(world.map.vtm_count(), ZONES * 2);
    assert_eq!(world.map.relays.len(), BAYS * 3);
}

#[test]
fn stacked_relay_levels_are_one_place_to_stand_and_two_shelves() {
    let world = cencora();
    // The bay carries relay levels at two heights, each with the same three
    // along-face positions: 2 x 3 x 228 bays = 1368 relay entities. Both levels
    // at one position bind the same explicit access cell, so six shelves stand over
    // three places to stand and each bay yields three dessim relays.
    let shelves: usize = world.index.relay_shelves.values().map(Vec::len).sum();
    assert_eq!(shelves, BAYS * 6, "the document's own relay entities");
    assert_eq!(world.map.relays.len(), BAYS * 3);
    assert_eq!(world.index.relay_shelves.values().map(Vec::len).max(), Some(2));
    for zone in &world.map.zones {
        let floors = world.racking.board_floors_m.get(&zone.id).expect("the zone's board stack");
        assert_eq!(floors, &vec![0.394, 0.952]);
    }
}

#[test]
fn each_board_stack_has_its_own_standing_cell() {
    let world = cencora();
    // Migrated at v0.7. Under v0.6 binding, relay pitch (645 mm) and cell pitch
    // (856 mm) were incommensurate and a run's 18 exchange points stood on 15
    // cells (570 for 38 runs, sharing allowed). The committed correction gives
    // each board stack an explicit cell and binding, matching legacy positions.
    let mut per_cell: BTreeMap<CellId, usize> = BTreeMap::new();
    for relay in &world.map.relays {
        *per_cell.entry(relay.htm_access).or_default() += 1;
    }
    assert_eq!(per_cell.len(), 684, "684 exchange points, 684 standing cells");
    assert_eq!(per_cell.values().copied().max(), Some(1));
    // Every relay of a run sits on a different bay from the one it shares a cell
    // with: the three exchange points of any one bay are three distinct cells.
    let mut per_bay: BTreeMap<CellId, BTreeSet<CellId>> = BTreeMap::new();
    for relay in &world.map.relays {
        per_bay.entry(relay.position).or_default().insert(relay.htm_access);
    }
    assert_eq!(per_bay.len(), BAYS);
    assert!(per_bay.values().all(|cells| cells.len() == 3));
}

#[test]
fn every_storage_position_is_one_slot_of_the_document() {
    let world = cencora();
    // 4 slot columns × 11 storage levels × 1 deep = 44 storage positions per
    // bay; the two relay levels of the same list hold none.
    assert_eq!(world.racking.columns.len(), BAYS * 44);
    assert_eq!(world.racking.vertical_tote_capacity, 1);
    // Each one names the slot entity it stands for, and no two share it.
    assert_eq!(world.index.storage_slots.len(), BAYS * 44);
    let distinct: BTreeSet<_> = world.index.storage_slots.values().collect();
    assert_eq!(distinct.len(), BAYS * 44);
}

#[test]
fn shelf_heights_are_the_survey_not_a_pitch() {
    let world = cencora();
    // The prototype's eleven surveyed fork-interface heights, 1580 mm to
    // 7577 mm, read straight off the level list.
    let mut heights: Vec<f64> = world.racking.columns.iter().filter_map(|r| r.floor_m).collect();
    heights.sort_by(f64::total_cmp);
    heights.dedup();
    assert_eq!(heights.first().copied(), Some(1.580));
    assert_eq!(heights.last().copied(), Some(7.577));
    assert_eq!(heights.len(), 11);
    for zone in &world.map.zones {
        assert_eq!(zone.zone_height_mm, 7577, "the topmost shelf, not the sum of them");
        assert_eq!(zone.real_shelf_count, 11);
    }
}

#[test]
fn the_aisles_travel_is_derived_where_the_old_format_had_to_state_it() {
    let world = cencora();
    // Six bays of 2140 mm steel span 12840 mm; the outermost slot columns sit
    // 320 mm in from each end, so the travel between them is 12200 mm. The old
    // fixture had to write that number by hand — its compiler could not know the
    // margin past the outermost position — and every zone here derives it.
    for zone in &world.map.zones {
        assert_eq!(zone.zone_width_mm, Some(12200));
    }
}

#[test]
fn a_floor_of_two_pitches_keeps_its_extent_and_axis_aligned_arcs() {
    // Migrated from `a_floor_of_two_pitches_still_makes_a_lattice_of_single_steps`
    // when cells stopped carrying grid positions: the ordinal lattice is gone,
    // so "every arc moves exactly one ordinal" is no longer expressible. What
    // survives of it physically is stated below in millimetres.
    let world = cencora();
    let bounds = world.map.grid_bounds.expect("the header extent");
    // desmap surveys 96 row centres, 18 of them the relay band's. Under v0.6
    // those were retiled at a lane pitch (93 rows); v0.7 puts a cell under each
    // relay stack, so the band's own rows are floor structure again and all 96
    // survive.
    assert_eq!(bounds.width, 153);
    assert_eq!(bounds.depth, 96);
    // The nominal step is what a move actually costs, not the tightest gap two
    // bands happen to leave at a seam.
    assert_eq!(bounds.cell_size_mm, 850.0);

    // What survives of the single-step property without ordinals: every arc is
    // axis-aligned -- it moves along exactly one millimetre axis -- and no arc
    // is degenerate. The heuristic no longer counts squares (it measures the
    // straight line in millimetres), so nothing needs the stronger claim.
    let mut arcs = 0usize;
    for (from, to, _) in world.map.graph.edges() {
        let a = world.map.graph.cell(from).unwrap().world_pos.unwrap();
        let b = world.map.graph.cell(to).unwrap().world_pos.unwrap();
        let (dx, dy) = ((a.x_mm - b.x_mm).abs(), (a.y_mm - b.y_mm).abs());
        assert!((dx > 0.5) != (dy > 0.5), "arc {from:?}->{to:?} moves ({dx:.1}, {dy:.1}) mm -- not axis-aligned");
        arcs += 1;
    }
    assert_eq!(arcs, world.map.graph.edge_count());
}

#[test]
fn the_cells_are_the_documents_plus_the_indices_dessim_needs() {
    let world = cencora();
    // Existing floor plus three explicitly declared HTM stands under each
    // bay's boards. Each bay also has a reporting anchor; VTMs add no cells.
    assert_eq!(world.map.graph.cell_count(), 8089 + 3 * BAYS + BAYS);
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    for cell in world.map.graph.cells() {
        *kinds.entry(format!("{:?}", cell.kind)).or_default() += 1;
    }
    assert_eq!(
        kinds,
        BTreeMap::from([
            // The three HTM stands under each bay are relay access cells.
            ("Exchange".into(), 3 * BAYS),
            // Floor cells less the 162 the 26 stations claim. Former parking
            // markings are ordinary floor; they create no chargers.
            ("Open".into(), 7927),
            ("Presentation".into(), 26),
            ("QueueCell".into(), 110),
            ("Relay".into(), BAYS),
            ("StationArea".into(), 26),
        ])
    );
    // 29446 flow arcs survive as one arc each, and the stitch links as two --
    // 672 of them at v0.7, every relay standing cell now stitched to its lane.
    // Minus 418: the queue discipline (one-way dest_only corridors through
    // each of the 26 stations' queues, single entry, single exit) drops every
    // other edge touching a queue or presentation cell -- the same
    // restriction v3 maps get from queue_edges::restrict_queue_htm.
    // The new stands each add a forward entry and reverse exit. The former
    // stands are ordinary travel cells again, retaining along-lane movement.
    assert_eq!(world.map.graph.edge_count(), 29446 + 672 * 2 - 418 + 3 * BAYS * 2);
    assert_eq!(world.map.stations.len(), 26);
}
