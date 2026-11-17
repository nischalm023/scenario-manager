//! The DFW6 fixture as world-protocol documents, pinned.
//!
//! Both documents were emitted once by `gen_dfw6_worldproto.py` (arrangement
//! from the sheet groups, dimensions from the DWG, joined at the verified
//! +5,575.7mm datum) and NOTHING REGENERATES THEM: that generator is not in
//! this repository, its inputs are not either, and the file at the tip is its
//! output plus fourteen hand patches. `walmart_dfw6.json` is HAND-MANAGED, on
//! cencora's footing -- edited only by a byte-proved script, never
//! regenerated -- and the script permitted to edit it is
//! `scripts/reauthor_dfw6_easyloops.py` (`dfw6-provenance.md`, 2026-09-04).
//! This file hard-expects both documents, because a loader that returns `None`
//! on a missing artifact passes vacuously and this repo has been burned by
//! exactly that.
//!
//! The generator decision that made two documents of it is what stands under a
//! bay. `legs` tiles a travel lane beneath every run; `blade` tiles nothing
//! there. Under the corrected geometry (racks back-to-back, VTM-only aisles,
//! no HTM corridors -- the sheet's [680,818|818,682|1227] period) that
//! decision is EXISTENTIAL: with blade uprights no relay can bind any cell an
//! HTM could stand on, and dessim refuses the whole map as storage no lane
//! serves. The canary is no longer a graph delta; it is whether the map loads
//! at all. (SPEC-GAPS 74 is the entry asking the format to carry this.)
//!
//! They differ in a SECOND thing since 2026-09-04, and deliberately: only
//! `walmart_dfw6.json` was re-authored. Its four EasyLoops are declared cell by
//! cell -- four `go.easyloop` envelopes, eight `pick` places, 1,517 entities --
//! while `walmart_dfw6_blade_uprights_canary.json` keeps the stamped
//! `template:easyloop` shape at 856. The canary is not a second copy of the
//! site to keep in step; it exists to be REFUSED at load, and the refusal is
//! about uprights, so re-authoring it would cost a byte-proved edit to a
//! hand-managed document to change nothing the test reads.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, from_slice};

use dessim::events::EventSink;
use dessim::pathfinding::holding_cells;
use dessim::runner::RunnerHooks;
use dessim::scenario::ScenarioConfig;
use dessim::{FHSippPathfinder, FidelityMode, SimDuration};
use native_modules as _;

use desgraph::{CellKind, StationType};
use desmap::OutboundRole;
use desmap::zone::ZoneHolds;
use dessim::scenario_gen::RackUse;
use dessim::worldmap::{self, WorldMap};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn load(name: &str) -> WorldMap {
    worldmap::load(&fixture(name)).expect("the committed DFW6 document loads")
}

/// The invariants, and the number that settles an open question: 153,550
/// storage positions -- EXACTLY the spec figure the v5 fixture only reached
/// 89% of. The missing 11% was never the sidedness question; it was the
/// phantom 1,325mm corridors the DWG-centric decode drew between aisle units,
/// floor the sheet says is rack backs.
#[test]
fn dfw6_carries_the_spec_invariants() {
    let w = load("walmart_dfw6.json");
    // 28 aisles: the 20 storage aisles, plus the order buffer's 8. Pinned by
    // what each aisle HOLDS rather than by a total, because the two blocks
    // differ in nothing else -- same steel, same bay geometry.
    let storage: Vec<_> = w.map.zones.iter().filter(|z| z.holds == ZoneHolds::Inventory).collect();
    let buffer: Vec<_> = w.map.zones.iter().filter(|z| z.holds == ZoneHolds::Order).collect();
    assert_eq!(storage.len(), 20, "20 storage aisles");
    // EIGHT, not nine: the buffer is the nine easternmost storage aisles'
    // 17 flanking runs minus the two westernmost, and dropping those merges
    // the two aisles that flanked the resulting gap. 15 runs, no hole.
    assert_eq!(buffer.len(), 8, "8 order-buffer aisles");
    // The buffer is worked by grippers; storage by the standard vacuum VTM.
    assert!(
        buffer.iter().all(|z| z.vtm_configuration.as_deref() == Some("gripper")),
        "buffer aisles are gripper-served"
    );
    assert!(storage.iter().all(|z| z.vtm_configuration.as_deref() != Some("gripper")), "storage aisles are not");
    // Four EasyLoops expose eight pick positions. Post-pick roles separate
    // five extraction stations, three sealer drop-offs and one return pickup.
    assert_eq!(w.map.stations.len(), 25);
    let count = |kind| w.map.stations.iter().filter(|s| s.station_type == kind).count();
    assert_eq!(
        (
            count(StationType::Pick),
            count(StationType::Putaway),
            count(StationType::BoxInduction),
            count(StationType::Extraction),
            count(StationType::ContainerPickup)
        ),
        (8, 6, 2, 8, 1)
    );

    // The three extraction stations stand on the putaway floor NORTH of the
    // order buffer, at x MEASURED from the "Walmart Dallas.docx" map image
    // (2026-08-18, 168.1mm/px): clustered west-center, the middle one ON the
    // dedicated highway through the buffer corridor. Tolerance is a cell: the
    // presentation snaps to the floor tiling.
    let mut extract_x: Vec<f64> = w
        .map
        .stations
        .iter()
        .filter(|s| s.outbound_role == Some(OutboundRole::RouteDispense))
        .filter_map(|s| w.map.graph.cell(s.position).and_then(|c| c.world_pos).map(|p| p.x_mm))
        .collect();
    extract_x.sort_by(|a, b| a.partial_cmp(b).unwrap());
    // Where the drawing puts them, and where one of them actually stands.
    //
    // Station 2 is displaced 2,392 mm east of its drawn column. The buffer
    // band has a 1.87 m void there — between `buffer travel lane 3` (ends
    // 661783) and `buffer travel lane 4` (starts 680441) — so its queue's
    // entry cell had no inbound edge at all: a robot could leave the station
    // and never enter it, and close-out discovered that as `no path exists`
    // mid-run. It was moved onto the next column with floor beneath it
    // (scripts/move_stranded_extraction_station.py).
    //
    // The displacement is pinned, not tolerated: close the void, move the
    // station back to the drawing, and this expectation must return to zero.
    // A wider tolerance would have hidden both the void and the move.
    const DRAWING_X: [f64; 3] = [61_190.0, 67_111.0, 76_655.0];
    const DISPLACED_EAST: [f64; 3] = [0.0, 2_392.0, 0.0];
    for ((got, drawn), shift) in extract_x.iter().zip(DRAWING_X).zip(DISPLACED_EAST) {
        let want = drawn + shift;
        let why = if shift == 0.0 { "drawing" } else { "drawing + displacement (buffer void)" };
        assert!((got - want).abs() < 900.0, "extraction at x {got:.0}, {why} says {want:.0}");
    }
    for st in w.map.stations.iter().filter(|s| s.outbound_role == Some(OutboundRole::RouteDispense)) {
        let pos = w.map.graph.cell(st.position).and_then(|c| c.world_pos).expect("extraction station cell is placed");
        assert!(pos.y_mm > 80_316.0, "extraction {} at y {:.0}, want north of the highway", st.id.0, pos.y_mm);
    }

    // The south band reads west to east PICK PICK BI BI PICK PICK (ruled
    // 2026-08-19): both box inductions sit at the THIRDS of the gap between
    // the inner two pick stations, not inside a pair. Placement is CHOSEN --
    // the DWG's two carton_induction blocks are on detail sheets, not the site
    // plan; the count of two is the DWG's.
    let mut pick_x: Vec<f64> = w
        .map
        .stations
        .iter()
        .filter(|s| s.station_type == StationType::Pick)
        .filter_map(|s| w.map.graph.cell(s.position).and_then(|c| c.world_pos).map(|p| p.x_mm))
        .collect();
    pick_x.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut box_x: Vec<f64> = w
        .map
        .stations
        .iter()
        .filter(|s| s.station_type == StationType::BoxInduction)
        .filter_map(|s| w.map.graph.cell(s.position).and_then(|c| c.world_pos).map(|p| p.x_mm))
        .collect();
    box_x.sort_by(|a, b| a.partial_cmp(b).unwrap());
    // EL EL BI BI EL EL along the row, still. The indices moved because each
    // machine now contributes TWO pick stations, so the pair bracketing the
    // box inductions is the east station of machine 2 and the west station of
    // machine 3 -- [3] and [4] of eight, where it used to be [1] and [2] of
    // four. The layout claim is unchanged.
    assert!(
        pick_x[3] < box_x[0] && box_x[0] < box_x[1] && box_x[1] < pick_x[4],
        "both box inductions belong between the two middle machines: {box_x:?} vs {pick_x:?}"
    );

    // Putaway stations stand in the first row past the 7-lane north
    // highway: 7 x 850 + half a lane = 6,375mm from the racks' north edge
    // (Vinay's summary; the earlier literal 5,950 turned out to be the
    // highway depth itself).
    // The STORAGE block's north edge. The order buffer is steel as well and
    // stands north of the putaway stations, so the bound is taken over the
    // 21-bay runs only -- a buffer run is one bay, so run length separates
    // them cleanly.
    let storage_run_mm = 21.0 * 2_751.6 - (2_751.6 - 2_532.8);
    let rack_north = w
        .steel
        .iter()
        .filter(|s| s.rect_mm[1] + s.rect_mm[3] < storage_run_mm + 20_000.0)
        .map(|s| s.rect_mm[1] + s.rect_mm[3])
        .fold(f64::NEG_INFINITY, f64::max);
    for st in w.map.stations.iter().filter(|s| s.station_type == StationType::Putaway) {
        let pos = w.map.graph.cell(st.position).and_then(|c| c.world_pos).expect("putaway station cell is placed");
        let standoff = pos.y_mm - rack_north;
        // Highway (5,950) + the order-buffer band (2,532.8) + 3 queue rows +
        // half the presentation row: the rotated cluster presents at its far
        // end, queue pointing at storage.
        assert!(
            (standoff - 11_457.8).abs() < 5.0,
            "putaway station {} presents {standoff:.1}mm past the north highway, want 11,457.8",
            st.id.0
        );
        assert!(pos.y_mm > rack_north, "putaway stations are NORTH of the block");
        // West of the order buffer (2026-08-17), so their approach is the
        // open floor in the buffer's own band and the buffer never stands
        // between a putaway station and the highway. 57,659 is the westmost
        // buffer run; the stations spread evenly below it rather than
        // bunching, so their queues do not contend.
        assert!(pos.x_mm < 57_659.0, "putaway station {} sits at x {:.0}, want west of the buffer", st.id.0, pos.x_mm);
    }

    // Pick stations mirror the putaways: first row south of the 7-lane
    // south highway ("about 2 bay widths", per the 2026-08-14 ruling that
    // supersedes the DWG's 12,129 loop row).
    //
    // The cell measured is the loop's INVENTORY INGRESS — where an HTM sets a
    // tote on the belt — because that is the cell a station binds when it
    // declares a sink port, and because it is the loop's northern edge: it
    // says where the machine STANDS, which is what this invariant is about.
    //
    // It reads the presentation cells, not `position`. `position` is the station
    // pose, which sits at the far end of the loop and moves whenever the
    // order presentation moves — it moved 1,700mm on 2026-08-28 when the two
    // presentation cells went onto the machine's inner seats. Pinning the
    // pose pinned an incidental; the prose here always claimed the ingress.
    let rack_front = w.steel.iter().map(|s| s.rect_mm[1]).fold(f64::INFINITY, f64::min);
    let picks = w.map.stations.iter().filter(|s| s.station_type == StationType::Pick);
    for (st, point) in picks.flat_map(|s| s.presentation_points.iter().map(move |point| (s, point))) {
        let at = |cell| w.map.graph.cell(cell).and_then(|c| c.world_pos).expect("pick station cell is placed");
        let standoff = rack_front - at(point.presentation_cell).y_mm;
        assert!(
            (standoff - 6_417.0).abs() < 450.0,
            "pick station {} ingress is {standoff:.1}mm south of the racks, want ~6,417",
            st.id.0
        );
        // And the order presentation is DEEPER in, one lattice (7 rows of
        // 850mm) south of the ingress. Without this the check above would
        // pass on a loop that had collapsed to its ingress row.
        let depth = at(point.presentation_cell).y_mm - at(st.position).y_mm;
        assert!(
            (depth - 5_950.0).abs() < 450.0,
            "pick station {} presents {depth:.1}mm inside the loop, want ~5,950",
            st.id.0
        );
    }
    // Evenly spread, not bunched: the smallest gap between adjacent putaway
    // stations is most of the nominal spacing.
    let mut put_x: Vec<f64> = w
        .map
        .stations
        .iter()
        .filter(|s| s.station_type == StationType::Putaway)
        .filter_map(|s| w.map.graph.cell(s.position).and_then(|c| c.world_pos).map(|p| p.x_mm))
        .collect();
    put_x.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let min_gap = put_x.windows(2).map(|w| w[1] - w[0]).fold(f64::INFINITY, f64::min);
    // DoO Figure 5.2 groups decant to the west of the post-pick handoffs.
    // Five floor columns between approaches leaves independent service lanes.
    assert!((min_gap - 4_264.0).abs() < 5.0, "decant approach spacing: {min_gap:.0}mm");

    assert_eq!(w.map.fleet_config().htms, 230, "September 12 ruling: use the DoO draft HTM fleet");
    // 2 vacuum VTMs per storage aisle, plus one gripper per buffer aisle.
    assert_eq!(w.map.vtm_count(), 20 * 2 + 8, "2 VTMs per aisle, 1 gripper per buffer aisle");
    // 21 storage bays per rack (ruled 2026-08-14: rows 1..21 from the north,
    // the DWG pilot having trimmed one bay from each end): 37 x (19 std x
    // 230 + 2 tunnel x 120) = 170,570. The spec sheet's 153,550 is exactly
    // TWO standard bays per rack less -- possibly two of the relay-free
    // rows are slot-less; open question for Vinay. What the ruling nails
    // beyond doubt: 14 relay rows x 4 boards x 37 racks = 2,072 diving
    // boards, the spec figure exactly, which the old 19-row alternation
    // never explained.
    //
    // The order buffer adds 15 single standard bays, 230 slots each. Those
    // are ORDER positions, not storage, so they are counted apart -- the
    // 170,570 above is what the spec sheet's figure should be compared to.
    let buffer_bays = 15;
    assert_eq!(
        w.racking.columns.len(),
        170_570 + buffer_bays * 230,
        "21 storage bays per rack, plus the buffer's {buffer_bays}"
    );
}

/// The row of empty order totes: the southernmost bay of every run, the steel
/// an HTM coming off an Easy Loop reaches first.
///
/// Where an order tote lives used to be nowhere -- the pool was the tail of a
/// shuffled slot ordering, so it moved whenever the ordering did and vanished
/// entirely when the count reached scenario generation as zero. This pins the
/// PLACE, arithmetically rather than by transcription:
///
///   * 37 rack columns, each beginning with a one-bay run at the block's
///     southern face;
///   * a standard bay is 5 slot columns along the run x 2 deep x 23 storage
///     levels = 230 positions, so the row is 37 x 230 = 8,510;
///   * the bay stands at y = 17,844.1 and its 5 slot columns sit at the half
///     of the 506.6mm slot pitch and every pitch after, so the row's slots
///     run from y = 18,097.4 to y = 20,124.0.
///
/// Nothing is created or destroyed: the row comes OUT of the storage total,
/// which is why the census still sums to what it did before.
#[test]
fn the_order_tote_row_is_the_southernmost_bay_of_every_run() {
    let w = load("walmart_dfw6.json");

    let depot: Vec<_> = w.racking.columns.iter().filter(|c| c.uses == RackUse::OrderDepot).collect();
    let buffer = w.racking.columns.iter().filter(|c| c.uses == RackUse::OrderBuffer).count();
    let stock = w.racking.columns.iter().filter(|c| c.uses == RackUse::Inventory).count();

    const RUNS: usize = 37;
    const SLOTS_PER_BAY: usize = 5 * 2 * 23;
    assert_eq!(depot.len(), RUNS * SLOTS_PER_BAY, "one bay of every run, 230 positions each");
    assert_eq!(buffer, 15 * SLOTS_PER_BAY, "the buffer is unchanged: 15 single bays");
    assert_eq!(stock + depot.len() + buffer, w.racking.columns.len(), "every position is one of the three");
    assert_eq!(stock, 170_570 - RUNS * SLOTS_PER_BAY, "the row comes out of storage, not out of nowhere");

    // It is a ROW: one band of y, spanning the whole width of the block.
    let mut ys: Vec<f64> = depot.iter().filter_map(|c| c.world_pos).map(|p| p.y_mm).collect();
    ys.sort_by(f64::total_cmp);
    ys.dedup_by(|a, b| (*a - *b).abs() < 1.0);
    assert_eq!(ys.len(), 5, "five slot columns along a bay: {ys:?}");
    assert!((ys[0] - 18_097.4).abs() < 1.0, "the row's first slot column sits at 18,097.4, not {:.1}", ys[0]);
    assert!((ys[4] - 20_124.0).abs() < 1.0, "its last at 20,124.0, not {:.1}", ys[4]);

    // And it is the SOUTHERNMOST steel: no stock position stands south of it.
    let stock_front = w
        .racking
        .columns
        .iter()
        .filter(|c| c.uses == RackUse::Inventory)
        .filter_map(|c| c.world_pos)
        .map(|p| p.y_mm)
        .fold(f64::INFINITY, f64::min);
    assert!(
        stock_front > ys[4],
        "stock starts at y {stock_front:.1}, which is not north of the row's last slot column {:.1}",
        ys[4]
    );

    // Every run has a piece of it: an order tote is a short trip from wherever
    // a robot happens to be, not a pilgrimage to one aisle.
    let depot_x: BTreeSet<i64> = depot.iter().filter_map(|c| c.world_pos).map(|p| p.x_mm.round() as i64).collect();
    assert_eq!(depot_x.len(), RUNS * 2, "two slot depths per run, all {RUNS} runs");
}

/// The canary. Legs tiles a lane under all 33 runs; blade does not. If these
/// two documents ever load to the same graph, the uprights decision has
/// stopped reaching the simulator and measured clearance has been lost in
/// translation somewhere between the generator and the map.
#[test]
fn blade_uprights_make_the_warehouse_inoperable() {
    // Legs: every exchange point stands on its own under-rack cell.
    let legs = load("walmart_dfw6.json");
    let stands: BTreeSet<_> = legs.map.relays.iter().map(|r| r.htm_access).collect();
    assert_eq!(legs.map.relays.len(), stands.len(), "legs binds one stand per relay");

    // Blade: the HTM stands under the board, and with a full blade there is
    // no under-rack floor -- the aisle across the face is out of reach and
    // VTM-only besides. dessim must refuse the map outright: storage no lane
    // serves. If this ever LOADS, the uprights decision stopped reaching the
    // simulator.
    let err = match worldmap::load(&fixture("walmart_dfw6_blade_uprights_canary.json")) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("blade must refuse to load under the corrected geometry"),
    };
    // The refusal comes from the reader's relay binder rather than dessim's
    // downstream storage check, and it is the better error of the two: it names
    // the relay, its reach, and that its neighbours on the same run did bind.
    assert!(err.contains("handoff shelf with no floor in front of it"), "the refusal must say why: {err}");
}

/// The lanes WORK, pinned as a regression test: a two-sim-minute run over the
/// legs document picks. This is the whole point of the under-rack lanes, and
/// it is exactly what silently broke twice on the way here -- once because
/// composition seeds initial inventory THROUGH the event sink (a None sink
/// means every tote stays empty and the run completes with zero events), and
/// once because idle HTMs squat with open-ended reservations wherever they
/// stop without an idle-relocation policy.
#[test]
#[cfg_attr(not(feature = "slow-gate"), ignore = "whole-fixture simulation; slow gate: scripts/test-slow-gate.sh")]
fn the_legs_document_picks() {
    use std::sync::{Arc, mpsc};
    let loaded = dessim::world_spec::load(&fixture("walmart_dfw6_generated_spec.json")).expect("spec loads");
    // The fleet comes from the document through the world_spec default --
    // this run exercises the same path a spec-driven desvis launch takes.
    let mut config = loaded.settings;
    assert_eq!(config.num_htms, 230, "world_spec defaults the full-workflow fleet from the document");
    // Ghost, deliberately: under CollisionDes the corrected map exposes a
    // DETERMINISTIC scheduling-imbalance abort (HtmStore reserve at 67.023s,
    // identical with 122 or 60 HTMs) -- the VTM/HTM decoupling gap the team
    // roadmap tracks, with a repro at last. This pin covers the map and the
    // lanes, not that scheduler.
    config.fidelity = FidelityMode::Ghost;
    config.sim_duration = Duration::from_secs(120);
    let mut data = dessim::runner::resolve_scenario_with_racking(
        loaded.world.map.clone(),
        loaded.world.racking.clone(),
        &loaded.scenario,
        &config,
    )
    .expect("scenario generation");
    let pathfinder = Arc::new(FHSippPathfinder::new(
        &data.map,
        (config.vtm.speed * 1000.0) as u64,
        SimDuration::seconds(1),
        holding_cells(&data.map),
    ));
    let (event_tx, event_rx) = mpsc::channel();
    let sink: Arc<dyn EventSink> = Arc::new(event_tx.clone());
    let (planner, model, bias) =
        dessim::composition::build(&mut data, &config, pathfinder, Some(sink)).expect("planner assembly");
    let (result, _metrics) = dessim::runner::run(
        planner,
        model,
        &data,
        config,
        event_tx,
        event_rx,
        bias,
        RunnerHooks { live: None, subscribers: vec![], recording: None },
    )
    .expect("the run completes");
    assert!(
        result.total_picks > 0,
        "a two-minute run over the legs document must pick; zero means the lanes \
         or the dispatch pipeline regressed"
    );
}

/// The surfaces a renderer draws are part of the map's contract now: real
/// axes (so a frame spans the true extent, not a uniform guess), one steel
/// rectangle per bay, one floor quad per tiled region. If any of these
/// counts drifts, the visualizer's picture and the solver's world have
/// stopped being the same thing.
#[test]
fn the_render_surfaces_match_the_survey() {
    let w = load("walmart_dfw6.json");

    // The cells span the real field. (GridAxes died with the v3 sunset:
    // the renderer frames the tiles' own bounding box now, so the pin
    // measures cell positions directly.)
    let (mut min_x, mut max_x) = (f64::INFINITY, f64::NEG_INFINITY);
    for cell in w.map.graph.cells() {
        if let Some(p) = cell.world_pos {
            min_x = min_x.min(p.x_mm);
            max_x = max_x.max(p.x_mm);
        }
    }
    let span = max_x - min_x;
    assert!((85_000.0..93_000.0).contains(&span), "the cells should span ~the 92.1m field, got {span:.0}mm");

    // One steel rectangle per bay: 37 storage runs x 21 bays, plus the
    // order buffer's 15 single-bay runs.
    assert_eq!(w.steel.len(), 37 * 21 + 15, "steel rectangles");
    // Every rectangle has real area and sits inside the field.
    for s in &w.steel {
        // Runs go along y: width is the rack DEPTH (1,498..1,580 per the
        // sheet), height the bay length (2,012 tunnel / 2,533 storage).
        assert!((1_400.0..1_700.0).contains(&s.rect_mm[2]) && s.rect_mm[3] > 1_900.0, "bay-sized: {:?}", s.rect_mm);
        assert!(s.rect_mm[0] >= -1.0 && s.rect_mm[0] + s.rect_mm[2] <= 92_113.0);
    }

    // Each of the 97 inventory corridors now owns one floor section per
    // existing steel bay. Cross-floor columns follow those corridor widths;
    // apron columns follow the 26 station doors. None tiles inside EasyLoop.
    let inventory_sections = (20 + 37 + 37 + 3) * 21;
    let cross_floor_sections = 2 * 106;
    let station_apron_sections = 104;
    let buffer_sections = 8 + 15 + 15 + 1;
    // South open floor and putaway floor, plus the south-side lane and three
    // surviving inter-station floor blocks (Daniel deleted the other three).
    let other_sections = 2 + 4;
    assert_eq!(
        w.floors.len(),
        inventory_sections + cross_floor_sections + station_apron_sections + buffer_sections + other_sections,
        "floor sections match the authored subdivisions"
    );
    assert!(w.floors.iter().all(|floor| floor.kind != CellKind::Charging), "no charger was authored");

    // The building envelope: the DWG's wall standoffs around the canonical
    // field. Bigger than the tiled field on every side, and the north wall
    // now has to clear the order buffer, which stands inside it.
    let b = w.building.expect("the document states the building");
    assert!(b[0] < -4_000.0 && b[1] < -3_000.0, "walls stand off south-west: {b:?}");
    assert!((104_000.0..105_000.0).contains(&b[2]), "width {b:?}");
    assert!((92_000.0..93_200.0).contains(&b[3]), "depth {b:?}");
    let steel_north = w.steel.iter().map(|s| s.rect_mm[1] + s.rect_mm[3]).fold(f64::NEG_INFINITY, f64::max);
    assert!(steel_north < b[1] + b[3], "the buffer stands inside the north wall");
}

/// Removing the old parking designation keeps the south strip's geometry
/// and cells. They are ordinary traversable floor, not charging equipment.
#[test]
fn the_south_strip_stays_ordinary_floor() {
    let w = load("walmart_dfw6.json");
    let strip: Vec<_> = w
        .index
        .cell_entities
        .iter()
        .filter(|(_, entity)| entity.to_string().starts_with("0a281c3e-03c4-5cba-8e3f-fd62b288822d/"))
        .map(|(cell, _)| w.map.graph.cell(*cell).unwrap())
        .collect();
    assert_eq!(strip.len(), 216, "108 columns by two rows remain in the map");
    assert!(strip.iter().all(|cell| cell.kind == CellKind::Open));
    assert!(w.map.graph.cells().all(|cell| cell.kind != CellKind::Charging));
}

/// The putaway spec runs the phase2 model on this map: racks start with
/// empty totes and the four putaway stations feed them. Zero putaways means
/// the stations, the empty-tote seeding, or the induction pipeline regressed
/// -- or someone re-enabled putaway on a map without putaway stations, which
/// is the silent zero-event trap this file's sibling comment warns about.
#[test]
#[cfg_attr(not(feature = "slow-gate"), ignore = "whole-fixture simulation; slow gate: scripts/test-slow-gate.sh")]
fn the_putaway_spec_puts() {
    use std::sync::{Arc, mpsc};
    let loaded = dessim::world_spec::load(&fixture("walmart_dfw6_putaway_spec.json")).expect("spec loads");
    let mut config = loaded.settings;
    assert!(config.putaway.enabled, "the spec declares the putaway phase");
    config.sim_duration = Duration::from_secs(120);
    let mut data = dessim::runner::resolve_scenario_with_racking(
        loaded.world.map.clone(),
        loaded.world.racking.clone(),
        &loaded.scenario,
        &config,
    )
    .expect("scenario generation");
    let pathfinder = Arc::new(FHSippPathfinder::new(
        &data.map,
        (config.vtm.speed * 1000.0) as u64,
        SimDuration::seconds(1),
        holding_cells(&data.map),
    ));
    let (event_tx, event_rx) = mpsc::channel();
    let sink: Arc<dyn EventSink> = Arc::new(event_tx.clone());
    let (planner, model, bias) =
        dessim::composition::build(&mut data, &config, pathfinder, Some(sink)).expect("planner assembly");
    let (result, _metrics) = dessim::runner::run(
        planner,
        model,
        &data,
        config,
        event_tx,
        event_rx,
        bias,
        RunnerHooks { live: None, subscribers: vec![], recording: None },
    )
    .expect("the run completes");
    assert!(
        result.throughput_putaways_per_hour > 0.0,
        "a two-minute putaway-phase run must induct; zero means the putaway \
         stations or the empty-tote pipeline regressed"
    );
}

/// The spec files admit the documents through `world_spec` -- the S1 seam.
/// (What a RUN of them does is the standing pick-flow question tracked in the
/// workspace's WALMART.md §6a; this file pins the map, not the simulation.)
#[test]
fn the_spec_files_load_through_world_spec() {
    let loaded = dessim::world_spec::load(&fixture("walmart_dfw6_spec.json")).expect("spec loads");
    assert!(loaded.world.map.graph.cell_count() > 4_000);
}

#[test]
fn throughput_control_differs_from_canonical_only_by_named_workload_overrides() {
    let canonical_path = fixture("walmart_dfw6_spec.json");
    let control_path = fixture("walmart_dfw6_throughput_control_spec.json");
    let canonical: Value = from_slice(&fs::read(canonical_path).unwrap()).unwrap();
    let mut control: Value = from_slice(&fs::read(&control_path).unwrap()).unwrap();
    let order = control["settings"]["order"].as_object_mut().unwrap();
    assert_eq!(order.remove("release_all_at_start"), Some(Value::Bool(true)));
    assert_eq!(order.remove("direct_extraction"), Some(Value::Bool(true)));
    assert_eq!(control, canonical, "the throughput control must retain the canonical fleet, workload and inventory");

    let loaded = dessim::world_spec::load(&control_path).expect("throughput control loads");
    assert!(loaded.settings.order.release_all_at_start);
    assert!(loaded.settings.order.direct_extraction);
    assert_eq!(loaded.settings.operator.time_per_pick_secs, 10.0);
    assert_eq!(loaded.settings.order.extraction_secs, 9.0);
    assert_eq!(loaded.settings.max_orders, None);
}

/// The bay module the sheet describes: 10 slots per storage level (5 across
/// x 2 deep) on every bay, and relay bays carrying boards -- 2,072 in total,
/// which is Vinay's diving-board count for the site and what the assertion
/// below holds the document to.
/// Whether alternation matches the drawing is an A4 sheet question; this pin
/// holds the document to what it currently claims so a regen can't drift
/// silently.
#[test]
fn bays_carry_ten_slots_per_level() {
    use std::collections::BTreeMap;
    let w = load("walmart_dfw6.json");
    let mut per_cell: BTreeMap<u32, Vec<i64>> = BTreeMap::new();
    for c in &w.racking.columns {
        let floor = (c.floor_m.expect("document slots state heights") * 1000.0).round() as i64;
        assert!(c.world_pos.is_some(), "document slots state positions");
        per_cell.entry(c.cell.0).or_default().push(floor);
    }
    // 37 storage runs of 21 bays, plus the buffer's 15 one-bay runs.
    assert_eq!(per_cell.len(), 37 * 21 + 15, "one anchor cell per bay");
    for (cell, floors) in &per_cell {
        let mut distinct = floors.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(floors.len(), distinct.len() * 10, "bay at cell {cell}: 10 slots per level (5 across x 2 deep)");
        assert_eq!(*distinct.last().unwrap(), 9_400, "every bay tops out at the shared beam");
    }
    // Double-deep is spatial, not just a count: each bay's slots stand at
    // exactly two depth positions (the half-depth centres), back to back.
    let mut depths: BTreeMap<u32, BTreeSet<i64>> = BTreeMap::new();
    let mut alongs: BTreeMap<u32, BTreeSet<i64>> = BTreeMap::new();
    for c in &w.racking.columns {
        let p = c.world_pos.unwrap();
        depths.entry(c.cell.0).or_default().insert(p.x_mm.round() as i64);
        alongs.entry(c.cell.0).or_default().insert(p.y_mm.round() as i64);
    }
    for (cell, xs) in &depths {
        assert_eq!(xs.len(), 2, "bay at cell {cell}: two depth rows, got {xs:?}");
        assert_eq!(alongs[cell].len(), 5, "bay at cell {cell}: five along-run columns");
    }
    // The ruled row pattern lands on the spec's diving-board count EXACTLY:
    // 14 relay rows x 4 boards x 37 racks = 2,072. The order buffer adds its
    // own, 4 per single-bay run, which the spec figure does not cover.
    assert_eq!(w.map.relays.len(), 2_072 + 15 * 4, "the spec's diving boards, exactly");
}

/// The tote population is the document's, not the spec's. `carrier_pool` is a
/// generator, so the reader expands it into one carrier per tote and the
/// census counts the expansion — a pool of 200 must arrive as 200 carriers.
#[test]
fn the_document_declares_its_order_tote_pool() {
    let w = load("walmart_dfw6.json");
    // One tote per position of the order-tote row: 37 runs x 230 positions.
    // The pool used to be the Dallas pilot's 200, a number for a 1,000-tote
    // building carried onto a 174,020-slot one; measured on the real
    // four-hour extract it bought 298 close-out cycles against 11,315
    // completed orders, which is close-out running and then starving. The row
    // is where order totes stand, so the row is how many there are.
    assert_eq!(w.carriers.order, 37 * 230, "one order tote per position of the row");
    assert_eq!(w.carriers.inventory, 0, "no inventory pool declared yet (mockput gap)");

    // And a spec-driven load carries it into the scenario, so nothing has to
    // repeat the number beside the map.
    let loaded = dessim::world_spec::load(&fixture("walmart_dfw6_smoke_spec.json")).expect("loads");
    match loaded.scenario {
        ScenarioConfig::Generated(g) => {
            assert_eq!(g.order_totes, 37 * 230, "the document's pool reaches scenario generation");
        }
        other => panic!("smoke spec should be Generated, got {other:?}"),
    }

    // EVERY scenario kind, and the one that matters most is the site extract:
    // it states demand and never a tote population, so it defaulted to zero
    // order totes. That is what made the four-hour DFW6 run finish with zero
    // order loads and zero extractions -- close-out asked for an empty
    // order-designated tote and the building contained none.
    let loaded = dessim::world_spec::load(&fixture("dfw6_real_spec.json")).expect("loads");
    match loaded.scenario {
        ScenarioConfig::SiteExtract(e) => {
            assert_eq!(e.order_totes, 37 * 230, "the document's pool reaches a site-extract run too");
        }
        other => panic!("the real spec should be a SiteExtract, got {other:?}"),
    }
}

/// Every order tote in the building stands in the row, and every tote in the
/// row is an order tote. This is the designation being a PLACE: nothing counts
/// totes into pools, so the two cannot drift apart.
///
/// The buffer stays empty of totes, which is a different statement about the
/// same racking -- a buffer slot is capacity for a closed order the run has
/// not produced yet, and a tote parked in one at t=0 spends it.
#[test]
fn order_totes_start_in_the_row_and_nowhere_else() {
    use dessim::model::{ToteDesignation, ToteLocation};
    use dessim::scenario_gen::TotePlacement;

    let loaded = dessim::world_spec::load(&fixture("walmart_dfw6_smoke_spec.json")).expect("loads");
    let data = dessim::runner::resolve_scenario_with_racking(
        loaded.world.map.clone(),
        loaded.world.racking.clone(),
        &loaded.scenario,
        &loaded.settings,
    )
    .expect("scenario generation");

    let uses_of = |placement: &TotePlacement| match placement.location {
        ToteLocation::InRack { rack_id, .. } => data.rack_use.get(&rack_id).copied(),
        _ => None,
    };

    let mut order = 0usize;
    for placement in &data.tote_placements {
        let uses = uses_of(placement).expect("a seeded tote starts in a rack the map knows");
        match placement.designation {
            ToteDesignation::Order => {
                order += 1;
                assert_eq!(uses, RackUse::OrderDepot, "order tote {:?} stands outside the row", placement.tote_id);
            }
            ToteDesignation::Inventory => {
                assert_eq!(uses, RackUse::Inventory, "stock tote {:?} stands on order racking", placement.tote_id);
            }
        }
    }
    // The document's pool is the row's own capacity, so the row starts full:
    // one empty order tote per position, and the free slots an order tote
    // lands in coming back from extraction are the ones its peers vacated.
    assert_eq!(order, 37 * 230, "the row is stocked to the document's declared pool");
}
