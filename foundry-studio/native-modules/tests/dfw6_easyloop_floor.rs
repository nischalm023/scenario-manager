//! What stands on the ground an EasyLoop occupies, whether a robot can reach
//! it, and which way it leaves.
//!
//! Nothing tiles inside a machine any more. Each of DFW6's four is a frame
//! carrying the `go.easyloop` envelope, with forty-six cells declared one at
//! a time and posed in that frame, and every arc into, out of and inside it
//! asserted by an explicit `stitch` (Daniel, 2026-09-04: "there are cells under
//! the whole easyloop. That's wrong"). So the questions this file asks changed
//! shape: not "do the loop's own tiles stand on the station floor's" -- there is
//! no loop lattice and no station floor -- but "are these forty-six the only
//! cells inside the envelope, and can a robot reach every one of them".
//!
//! The last question is the one a MIRRORED insert makes worth asking here: two
//! of the four machines are the DWG block reflected about its own origin, so
//! every direction inside them -- which way a port faces, which way an HTM
//! leaves a pick place -- is the block's own direction with the frame's `scale
//! [-1, 1]` applied to it, and a reading taken on the floor is the only one
//! that can tell a machine stated right from one stated backwards.
//!
//! `every_overlapping_pair_of_drawn_tiles_on_dfw6` at the foot is a GATE now
//! rather than a census. It ran `#[ignore]`d while it counted a document defect
//! that could not be fixed from here; the defect was the two lattices, they are
//! gone, and it pins zero. It still prints every pair it finds, so a regression
//! names its own cells rather than a number.

use desgraph::StationType;
use dessim::model::SimDuration;
use dessim::worldmap::WorldMap;
use native_modules::migration::flows::ttp::demand::loop_collect::approach_to_collection;
use rustc_hash::FxHashMap;
use std::collections::BTreeSet;
use std::collections::{HashSet, VecDeque};
use std::path::Path;

use desgraph::{CellId, CellKind};
use dessim::worldmap;

/// Two inventory ports, eight holding cells, four order boundary cells, two
/// six-row blocks of three cells, and two presentation cells.
const CELLS_PER_MACHINE: usize = 52;

/// Every machine's envelope, as the document states it, in millimetres.
///
/// Read off the map rather than written down here: the envelope is a rectangle
/// in the FLOOR frame on the machine's own entity, so a mirrored machine's
/// rectangle is already resolved and this test cannot disagree with the
/// document about where a machine is.
fn footprints(w: &WorldMap) -> Vec<[f64; 4]> {
    let mut out: Vec<[f64; 4]> = w.equipment.iter().filter(|e| e.part == "envelope").map(|e| e.rect_mm).collect();
    assert_eq!(out.len(), 4, "four EasyLoops, four envelopes");
    out.sort_by(|a, b| a[0].total_cmp(&b[0]));
    out
}

fn load() -> WorldMap {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/walmart_dfw6.json");
    worldmap::load(&p).expect("loads")
}

/// Every cell inside a loop can be driven to from the rest of the warehouse.
///
/// Before the loop was stitched to the floor, 66 of loop 0's 107 cells were
/// unreachable: its whole lattice was an island drawing floor no robot could
/// enter, which is what a broken-looking floor actually was.
#[test]
fn no_easyloop_cell_is_an_island() {
    let w = load();
    let g = &w.map.graph;
    let start = w.map.stations.iter().flat_map(|s| s.presentation_cells()).next().expect("a station");
    let mut seen = HashSet::new();
    let mut queue = VecDeque::new();
    seen.insert(start);
    queue.push_back(start);
    while let Some(cell) = queue.pop_front() {
        for (next, _) in g.neighbors(cell) {
            if seen.insert(next) {
                queue.push_back(next);
            }
        }
    }
    for (n, [x, y, dx, dy]) in footprints(&w).into_iter().enumerate() {
        let inside: Vec<_> = g
            .cells()
            .filter(|c| c.world_pos.is_some_and(|p| p.x_mm >= x && p.x_mm <= x + dx && p.y_mm >= y && p.y_mm <= y + dy))
            .collect();
        // An EXACT count now, not a lower bound. It was a lower bound because
        // the envelope was a bounding box with station floor inside it as well
        // as the loop's own lattice, so the four machines disagreed (95, 92,
        // ...). Nothing tiles inside a machine any more, so what stands in the
        // envelope is what the document declares and nothing else -- which is
        // the ruling this whole change is, made falsifiable.
        assert_eq!(inside.len(), CELLS_PER_MACHINE, "machine {n} declares its cells and no others");
        let islanded = inside.iter().filter(|c| !seen.contains(&c.id)).count();
        assert_eq!(islanded, 0, "machine {n}: {islanded} of {} cells unreachable", inside.len());
    }
}

// RETIRED, 2026-09-04: `the_loop_lattice_does_not_stand_on_the_station_floor`.
//
// It asked whether the loop's own [770, 850] mm lattice stood on the station
// floor's [852.8, 926.7] mm one. Neither lattice exists: the machine's cells
// are declared one at a time, and `station floor` is deleted. The question it
// asked cannot be asked of this document, so there is nothing here to re-point
// at something else.
//
// It was also passing vacuously by the end. It measured CENTRE DISTANCE, and
// the two lattices it named interleaved -- their rectangles overlapped by up to
// 409 x 815 mm while their centres stayed more than 230 mm apart, so it went
// green while the census below counted 116 pairs of stated tiles claiming one
// patch of ground. What replaces it is that census, promoted to a gate and
// pinned at zero: it measures RECTANGLES, which is the measurement that would
// have caught the 116.

/// Each loop hands inventory totes over at two cells: the DROPOFF cell where an
/// HTM sets a tote on the belt and the COLLECTION cell where one lifts the returning
/// tote off. `way_kind` gives a cell whose entity carries `port` the kind
/// `CellKind::Exchange`, which desvis paints sea green.
///
/// `stations.rs` used to take the SINK port as the station's presentation cell
/// and push `CellKind::Presentation` over it, so the DROPOFF cell lost its kind
/// and its colour and only one of the two showed. It now binds both ports as
/// `dropoff_cell` and `collection_cell` and leaves a port cell's kind alone.
#[test]
fn every_easyloop_has_a_load_and_an_unload_cell() {
    let w = load();
    for (n, [x, y, dx, dy]) in footprints(&w).into_iter().enumerate() {
        let exchange = w
            .map
            .graph
            .cells()
            .filter(|c| c.kind == CellKind::Exchange)
            .filter_map(|c| c.world_pos)
            .filter(|p| p.x_mm >= x && p.x_mm <= x + dx && p.y_mm >= y && p.y_mm <= y + dy)
            .count();
        assert_eq!(exchange, 2, "machine {n} should have a load and an dropoff cell");
    }
}

/// Nothing enters a machine except through its four declared doors.
///
/// Every machine used to stand inside `station floor`, stitched to it by
/// CONTACT, so an HTM could enter anywhere along a machine's perimeter and
/// cross it on a path the real machine has no lane for -- which is the
/// complaint this whole change answers (Daniel, 2026-09-04: "I don't want bots
/// just flying through the easyloop station in a non real path"). Every arc is
/// asserted now, and per machine these four are all of them: the two ports and
/// two order entries and two order exits.
///
/// A structural reading of "no HTM path crosses a machine off its lanes": a
/// path is a walk over arcs, so if the only arcs crossing an envelope are the
/// six doors, no path can enter any other way whatever the router does.
#[test]
fn no_arc_enters_a_machine_except_through_its_declared_doors() {
    let w = load();
    let g = &w.map.graph;
    for (n, [x, y, dx, dy]) in footprints(&w).into_iter().enumerate() {
        let inside = |c: CellId| {
            g.cell(c)
                .and_then(|c| c.world_pos)
                .is_some_and(|p| p.x_mm >= x && p.x_mm <= x + dx && p.y_mm >= y && p.y_mm <= y + dy)
        };
        let mut doors: BTreeSet<CellId> = BTreeSet::new();
        for cell in g.cells() {
            for (next, _) in g.neighbors(cell.id) {
                match (inside(cell.id), inside(next)) {
                    (true, false) => {
                        doors.insert(cell.id);
                    }
                    (false, true) => {
                        doors.insert(next);
                    }
                    _ => {}
                }
            }
        }
        assert_eq!(
            doors.len(),
            6,
            "machine {n}: six doors: two inventory ports, two order entries, two order exits, got {doors:?}"
        );
    }
    // And the two ports are two of each machine's four.
    let mut ports = 0;
    for s in &w.map.stations {
        if let (Some(drop), Some(collect)) = (s.dropoff_cell, s.collection_cell) {
            assert!(inside_any(&w, drop) && inside_any(&w, collect), "a port stands in its own machine");
            ports += 1;
        }
    }
    assert_eq!(ports, 8, "eight EasyLoop stations, four handoff pairs");
}

/// Whether a cell stands inside any machine's envelope.
fn inside_any(w: &WorldMap, cell: CellId) -> bool {
    let Some(p) = w.map.graph.cell(cell).and_then(|c| c.world_pos) else { return false };
    footprints(w).into_iter().any(|[x, y, dx, dy]| p.x_mm >= x && p.x_mm <= x + dx && p.y_mm >= y && p.y_mm <= y + dy)
}

/// Each EasyLoop's belt takes the circuit time the document declares, and no
/// other station has a belt at all.
///
/// `go.easyloop.loop_transit_secs` has been in the template since it was
/// written and was read by nothing: the value is 45, and `config.operator`'s
/// unrelated default of 9 made the neighbouring `time_per_pick_secs` look
/// wired when it was not. The tote's ride between the dropoff and collection cells is
/// the only place a circuit time can show up, so this is where it becomes
/// falsifiable -- change the document to 30 and this fails.
#[test]
fn each_easyloop_belt_runs_for_the_circuit_the_document_declares() {
    let w = load();
    let loops = &w.conveyors;
    let pick: Vec<_> = w.map.stations.iter().filter(|s| s.station_type == StationType::Pick).collect();
    // Eight, not four: an EasyLoop is two stations sharing one conveyor
    // (DESIGN.md §1), and the document used to pack each machine into one.
    assert_eq!(pick.len(), 8, "DFW6's four EasyLoops carry two stations each");
    for s in &pick {
        let conveyor = loops.of_station(s.id).expect("an EasyLoop station feeds a belt");
        assert_eq!(
            loops.transit(conveyor),
            Some(SimDuration::from_secs(45.0)),
            "machine {conveyor:?} should carry the document's 45s circuit"
        );
        assert!(s.dropoff_cell.is_some() && s.collection_cell.is_some(), "and both handoff cells");
    }
    // One belt per machine, not one per station. With four EasyLoops each
    // holding one station today, this is 4; when a machine's two stations are
    // split out it must stay 4, which is the point of keying by machine.
    let belts: BTreeSet<_> = pick.iter().filter_map(|s| loops.of_station(s.id)).collect();
    // Four belts for eight stations: this is the assertion that the split did
    // not accidentally hand each half a conveyor of its own.
    assert_eq!(belts.len(), 4, "four EasyLoops, so four belts, shared two stations each");
    for s in w.map.stations.iter().filter(|s| s.station_type != StationType::Pick) {
        assert!(!loops.serves(s.id), "station {:?} presents directly and has no belt", s.id);
    }
}

/// A dropoff is crossed north to south, a collection south to north, the pair
/// stands 840 mm apart, and TWO of the four machines have their dropoff west of
/// their collection while two have it east.
///
/// The opposed directions are what make the pair a circulation rather than two
/// dead ends: in off the highway, through the machine, back out to it. The
/// collection is one-way in both senses because a handoff cell is one robot's
/// standing spot, and two HTMs meeting head-on at an ingress is a deadlock
/// rather than a race.
///
/// THE DROPOFF LEAVES TWO WAYS, ruled 2026-09-04: south into the inventory lane,
/// and back north onto the apron cell it was entered from. It is the lane's
/// ENTRY cell and not a step of the lane's path, so the corridor restriction
/// does not delete its floor arcs -- and a robot that has set its tote on the
/// belt and drawn a task somewhere else leaves that way instead of standing on a
/// handover with nowhere to go. `minimal_easyloop.rs` states the rule in full
/// and `minimal_easyloop_collision.rs` is what fails when it is broken.
///
/// The two-and-two is what mirroring MEANS. The site plan inserts the DWG block
/// twice at scale (1, 1) and twice at (-1, 1), and the document records that on
/// machines 3 and 4's frames; an insert mirrored about its own origin swaps the
/// two belts, so the dropoff of a mirrored machine stands east of its
/// collection. This used to assert "west" for all four, which was true of the
/// document and false of the site.
///
/// 840 mm is the distance between the twin belt's two vertical arms in the
/// block, so each port stands on its own belt's centreline. Mirroring does not
/// change it -- an extent has no direction.
#[test]
fn each_handoff_is_one_way_and_the_pair_circulates() {
    let w = load();
    let g = &w.map.graph;
    let mut machines = BTreeSet::new();
    let (mut unmirrored, mut mirrored) = (0, 0);
    for s in &w.map.stations {
        let (Some(drop), Some(coll)) = (s.dropoff_cell, s.collection_cell) else { continue };
        let dp = g.cell(drop).and_then(|c| c.world_pos).expect("placed");
        let cp = g.cell(coll).and_then(|c| c.world_pos).expect("placed");
        let apart = (cp.x_mm - dp.x_mm).hypot(cp.y_mm - dp.y_mm);
        assert!((apart - 840.0).abs() < 0.05, "the ports are {apart:.1}mm apart, and the drawing says 840");
        if dp.x_mm < cp.x_mm {
            unmirrored += 1;
        } else {
            mirrored += 1;
        }

        for (cell, southbound) in [(drop, true), (coll, false)] {
            let here = g.cell(cell).and_then(|c| c.world_pos).expect("placed");
            let out: Vec<_> = g.neighbors(cell).iter().map(|(n, _)| *n).collect();
            let ins: Vec<_> = g.cells().filter(|c| g.neighbors(c.id).iter().any(|(n, _)| *n == cell)).collect();
            assert_eq!(ins.len(), 1, "{cell:?} is entered exactly one way");
            let from = ins[0].world_pos.expect("placed");
            if southbound {
                assert_eq!(out.len(), 1, "the dropoff continues only into the under-conveyor queue");
                assert!(from.y_mm > here.y_mm, "dropoff entered from the apron");
                let to = g.cell(out[0]).and_then(|c| c.world_pos).expect("placed");
                assert!(to.y_mm < here.y_mm, "dropoff continues under the conveyor");
                assert!(!out.contains(&ins[0].id), "no early exit onto the apron");
            } else {
                assert_eq!(out.len(), 1, "{cell:?} leaves exactly one way");
                let to = g.cell(out[0]).and_then(|c| c.world_pos).expect("placed");
                assert!(from.y_mm < here.y_mm, "collection entered from the SOUTH");
                assert!(to.y_mm > here.y_mm, "collection exits NORTH");
            }
        }
        machines.insert((drop, coll));
    }
    assert_eq!(machines.len(), 4, "four EasyLoops, each with one handoff pair");
    // Eight stations, two per machine, so each machine is counted twice.
    assert_eq!(
        (unmirrored, mirrored),
        (4, 4),
        "two machines inserted at scale (1, 1) and two at (-1, 1), two stations each"
    );
}

/// The collection approach is eight under-conveyor holding cells plus dropoff.
#[test]
fn each_machines_lane_is_entered_at_its_own_dropoff() {
    let w = load();
    let mut machines = BTreeSet::new();
    for s in w.map.stations.iter().filter(|s| s.station_type == StationType::Pick) {
        let collect = s.collection_cell.expect("an EasyLoop station names a collection cell");
        let dropoff = s.dropoff_cell.expect("and a dropoff");
        let lane = s.handoff_queues.iter().find(|q| q.into == collect).expect("a declared lane serves it");
        assert_eq!(
            lane.entry_from,
            Some(dropoff),
            "the arc into this machine's inventory lane comes off its own dropoff {dropoff:?}"
        );
        let approach = approach_to_collection(s).expect("the machine has a way in");
        assert_eq!(approach.len(), 9, "eight lane slots and the dropoff: {approach:?}");
        assert_eq!(approach.last(), Some(&dropoff), "with the dropoff as the cell it is entered from");
        assert!(!approach.contains(&collect), "the collection cell is checked on its own, not as a step of the way in");
        machines.insert(collect);
    }
    assert_eq!(machines.len(), 4, "four EasyLoops, four collection cells");
}

/// The departure continues the same curved U-turn, including on mirrored machines.
#[test]
fn each_pick_place_continues_its_curve_to_the_return_lane() {
    let w = load();
    let g = &w.map.graph;
    let mut count = 0;
    for s in w.map.stations.iter().filter(|s| s.station_type == StationType::Pick) {
        let here = g.cell(s.order_presentation_cell.unwrap()).and_then(|c| c.world_pos).expect("placed");
        let outgoing = g.neighbors(s.order_presentation_cell.unwrap());
        assert_eq!(outgoing.len(), 1, "one continued turn after dwell");
        let (to, edge) = outgoing[0];
        let arc = edge.curve.expect("curvature lives on the departure link");
        let end = g.cell(to).and_then(|c| c.world_pos).expect("placed");
        let direction = arc.end_tangent(end);
        assert!(direction[0].abs() < 1e-6 && direction[1] > 0.999, "return heads toward waiting");
        assert!(arc.length_mm(here, end) > (end.x_mm - here.x_mm).hypot(end.y_mm - here.y_mm));
        count += 1;
    }
    assert_eq!(count, 8);
}

/// A machine's belt accepts INVENTORY totes, and says so itself.
///
/// The conveyance is the inventory half: a tote rides out to a seat, is
/// worked, and rides back to be collected for back-to-store. An ORDER carrier
/// never touches it -- it stays on its robot and moves between the machine's
/// two stations.
///
/// Both ports declare `of: [{kind: tote, designation: inventory}]` and nothing
/// read it. Because an EasyLoop station's presentation cell IS its dropoff
/// cell, close-out's empty order tote was delivered there, boarded onto the
/// inventory belt, and left through the collection port. `LoopDeposit` now
/// filters on this, so the declaration is load-bearing rather than decorative.
#[test]
fn a_belt_accepts_only_the_carrier_its_port_declares() {
    let w = load();
    let mut belted = 0;
    for s in &w.map.stations {
        if s.dropoff_cell.is_none() {
            assert!(s.conveyance_accepts.is_none(), "no belt, nothing to accept");
            continue;
        }
        assert_eq!(
            s.conveyance_accepts.as_deref(),
            Some("inventory"),
            "station {:?} feeds a belt that must state what it carries",
            s.id
        );
        belted += 1;
    }
    assert_eq!(belted, 8, "eight EasyLoop stations across four machines");
}

/// An order carrier presents somewhere other than the belt's dropoff cell.
///
/// An EasyLoop hands an inventory tote to its conveyance at one cell and
/// presents an ORDER carrier at another: `present:N`, declared
/// `for: order_carrier`, the horseshoe head. They are metres apart, and
/// the station presents at the belt's dropoff cell, so close-out was
/// delivering its order tote straight onto the conveyance.
#[test]
fn an_order_presents_at_the_order_point_not_at_the_belt() {
    let w = load();
    let g = &w.map.graph;
    let mut checked = 0;
    for s in &w.map.stations {
        let Some(dropoff) = s.dropoff_cell else {
            continue;
        };
        let order = s.order_presentation_cell.expect("a belt station presents orders somewhere");
        assert_ne!(order, dropoff, "an order must not present on the belt's dropoff cell");
        assert!(s.presentation_cells().all(|cell| cell != order), "nor where inventory is handed over");
        let op = g.cell(order).and_then(|c| c.world_pos).expect("placed");
        let dp = g.cell(dropoff).and_then(|c| c.world_pos).expect("placed");
        // The horseshoe head sits deep in the machine, south of the handoffs.
        assert!(op.y_mm < dp.y_mm, "the order point is south of the belt handoff");
        checked += 1;
    }
    assert_eq!(checked, 8, "eight EasyLoop stations");
}

/// No two cells DFW6 states a footprint for claim one patch of ground.
///
/// A GATE since 2026-09-04, and a census before that. It ran `#[ignore]`d while
/// it counted 116 pairs of the four machines' own tiles standing on the station
/// floor's -- a defect in the document that no renderer and no test could fix,
/// so pinning it would only have pinned the defect. The machines declare their
/// cells now and `station floor` is deleted, the 116 are zero, and the gate
/// this leaves behind is the one that would have caught them: it measures
/// RECTANGLES, which is what the retired centre-distance gate above did not.
///
/// It still PRINTS every pair, classified, before it asserts. A regression here
/// names its own cells and the entities that carry them rather than a number.
///
/// It measures what desvis DRAWS, which is not the same set the gate
/// measures. Three classes come out, and only the first is a document defect:
///
/// - TILE over TILE: two cells that each STATE a floor tile, claiming one
///   patch of ground. Two robots can stand on one spot, and the two quads
///   share desvis's declared-tile tier, where nothing can separate them.
/// - MARKER over TILE: a cell the document places as a POINT — a port's
///   Exchange cell, a presentation point, a cell dessim minted for a bay —
///   standing inside floor, which is what points are for. desvis draws the
///   point as a square anyway so it can be seen at all, and puts that square
///   a depth tier in front of the floor tile under it. The document is right
///   and the renderer separates them.
/// - MARKER over MARKER: two POINTS on one patch, most of them two cells
///   dessim minted at one docking centre. They share the marker tier, so the
///   tiering does NOT separate them; whether that matters is a question about
///   the two squares' colours, since same-colour duplicates are only redundant
///   drawing.
///
/// A cell is a TILE here when the DOCUMENT stated its extent, never because
/// its footprint is positive: the reader gives an unsized document cell SPEC
/// §5's 900mm default, so a port's Exchange cell arrives carrying a square
/// nobody declared. Reading the number counted 57 of these pairs as document
/// defects (Codex review, 2026-09-04).
#[test]
fn every_overlapping_pair_of_drawn_tiles_on_dfw6() {
    let w = load();
    // What desvis would draw: the document's own tile where it declares one,
    // and `build_grid_data`'s marker square where it declares a point.
    let nominal_mm = w.map.grid_bounds.map(|b| b.cell_size_mm).unwrap_or(1000.0);
    let marker_mm = nominal_mm * 0.6;

    struct Drawn {
        id: CellId,
        rect: [f64; 4], // min_x, min_y, w, h
        tiled: bool,
        /// What desvis colours the square by. Printed because it is what
        /// decides whether a MARKER over MARKER pair is worth anything: two
        /// squares of one kind at one centre are the same colour, so they are
        /// redundant drawing rather than a picture that lies.
        kind: CellKind,
    }
    let drawn: Vec<Drawn> = w
        .map
        .graph
        .cells()
        .filter_map(|c| {
            let p = c.world_pos?;
            // Size and provenance are separate questions, exactly as
            // `build_grid_data` asks them: the square comes from the map
            // (a minted cell has none and falls back to the nominal marker),
            // and whether it is FLOOR comes from the document.
            let sized = w.cell_sizes_mm.get(c.id.0 as usize).copied().filter(|s| s[0] > 0.0 && s[1] > 0.0);
            let [sx, sy] = sized.unwrap_or([marker_mm, marker_mm]);
            let tiled = w.cell_footprint_stated.get(c.id.0 as usize).copied().unwrap_or(false);
            Some(Drawn { id: c.id, rect: [p.x_mm - sx / 2.0, p.y_mm - sy / 2.0, sx, sy], tiled, kind: c.kind })
        })
        .collect();

    // Overlap by the RECTANGLES, not by centre distance: two tiles of
    // different pitches abut at arbitrary offsets, and abutting is fine.
    // Returned as the two overlap spans in mm, because their SMALLER one is
    // what separates the two things that happen here: neighbours in one
    // lattice whose shared edge lands a rounding step apart overlap by a
    // fraction of a millimetre, and two lattices claiming one patch overlap
    // by most of a tile.
    let overlap = |a: &[f64; 4], b: &[f64; 4]| -> Option<[f64; 2]> {
        let ox = (a[0] + a[2]).min(b[0] + b[2]) - a[0].max(b[0]);
        let oy = (a[1] + a[3]).min(b[1] + b[3]) - a[1].max(b[1]);
        (ox > 0.0 && oy > 0.0).then_some([ox, oy])
    };
    /// Overlap thinner than this in either direction is two tiles meeting at
    /// an edge that rounding put a step on the wrong side of. Nothing stands
    /// on it; it is a seam a pixel wide at most.
    const SEAM_MM: f64 = 1.0;
    let carrier =
        |id: CellId| w.index.cell_entities.get(&id).map_or_else(|| "minted (no entity)".to_string(), |e| e.to_string());

    // Bucketed by a coarse grid so this is not 13k^2 comparisons.
    let bucket_mm = 4_000.0;
    let mut buckets: FxHashMap<(i64, i64), Vec<usize>> = FxHashMap::default();
    for (i, d) in drawn.iter().enumerate() {
        let (x0, y0) = ((d.rect[0] / bucket_mm).floor() as i64, (d.rect[1] / bucket_mm).floor() as i64);
        let (x1, y1) = (
            ((d.rect[0] + d.rect[2]) / bucket_mm).floor() as i64,
            ((d.rect[1] + d.rect[3]) / bucket_mm).floor() as i64,
        );
        for bx in x0..=x1 {
            for by in y0..=y1 {
                buckets.entry((bx, by)).or_default().push(i);
            }
        }
    }
    // ordered: the census is read by a person and compared between runs, so
    // the pairs come out in cell-id order rather than in bucket order.
    let mut pairs: BTreeSet<(u32, u32)> = BTreeSet::new();
    for ids in buckets.values() {
        for (n, &i) in ids.iter().enumerate() {
            for &j in &ids[n + 1..] {
                if overlap(&drawn[i].rect, &drawn[j].rect).is_some() {
                    pairs.insert((drawn[i].id.0.min(drawn[j].id.0), drawn[i].id.0.max(drawn[j].id.0)));
                }
            }
        }
    }

    let by_id: FxHashMap<u32, &Drawn> = drawn.iter().map(|d| (d.id.0, d)).collect();
    let (mut tile_tile, mut marker_tile, mut marker_marker, mut seams) = (0, 0, 0, 0);
    println!("\n{} cells drawn; {} overlapping pairs\n", drawn.len(), pairs.len());
    for (a, b) in &pairs {
        let (da, db) = (by_id[a], by_id[b]);
        let [ox, oy] = overlap(&da.rect, &db.rect).expect("a pair overlaps");
        if ox.min(oy) < SEAM_MM {
            seams += 1;
            continue;
        }
        let class = match (da.tiled, db.tiled) {
            (true, true) => {
                tile_tile += 1;
                "TILE over TILE    "
            }
            (false, false) => {
                marker_marker += 1;
                "MARKER over MARKER"
            }
            _ => {
                marker_tile += 1;
                "MARKER over TILE  "
            }
        };
        println!(
            "{class}  cells {a:>5} x {b:>5}  overlap {ox:>7.1} x {oy:>7.1} mm  at ({:.0}, {:.0})\n    {a:>5}: {:?} {} [{:.0} x {:.0}]\n    {b:>5}: {:?} {} [{:.0} x {:.0}]",
            da.rect[0] + da.rect[2] / 2.0,
            da.rect[1] + da.rect[3] / 2.0,
            da.kind,
            carrier(da.id),
            da.rect[2],
            da.rect[3],
            db.kind,
            carrier(db.id),
            db.rect[2],
            db.rect[3],
        );
    }
    println!(
        "\n{tile_tile} TILE over TILE (a document defect), {marker_tile} MARKER over TILE (separated by the marker \
         tier), {marker_marker} MARKER over MARKER (two points on one patch, NOT separated), {seams} thinner than \
         {SEAM_MM}mm (two tiles meeting at an edge)"
    );
    // Only the first class is asserted. MARKER over TILE is what a point is
    // FOR -- a port's Exchange cell standing inside floor -- and MARKER over
    // MARKER is mostly two cells dessim minted at one docking centre, which is
    // a question about two squares' colours rather than about the document.
    assert_eq!(tile_tile, 0, "two cells the document sizes cannot claim one patch of ground");
}
