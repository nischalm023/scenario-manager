//! Minimal-map topology and flow checks. The authoring scripts share EasyLoop
//! cells and links with DFW6; presentation cells are declared, with curvature
//! on their approach and departure links. Motion tests remain distinct from
//! these map checks because graph geometry alone does not implement steering.

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};

use rustc_hash::FxHashSet;
use serde_json::Value;

use desgraph::{CellId, CellKind, DestGate, StationType};
use desmap::ZoneHolds;
use dessim::SimConfig;
use dessim::composition::build;
use dessim::events::{CycleKind, EventSink, ResourceEventKind, SimEvent, VtmCycleDirection};
use dessim::model::{ExitPath, RackId, ToteDesignation, ToteId, ToteLocation};
use dessim::observer::output::MetricsOutput;
use dessim::runner::{RunnerHooks, resolve_scenario_with_racking, run};
use dessim::scenario_gen::RackUse;
use dessim::scenario_gen::ScenarioData;
use dessim::sim_types::SimResult;
use dessim::worldmap::{self, Equipment, WorldMap};
use native_modules as _;

const WORLD: &str = "minimal_easyloop.json";
const SPEC: &str = "minimal_easyloop_spec.json";

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn load() -> WorldMap {
    worldmap::load(&fixture(WORLD)).expect("the generated document loads")
}

#[test]
fn charging_cells_require_explicit_document_markers() {
    let mut source: Value = serde_json::from_str(&std::fs::read_to_string(fixture(WORLD)).unwrap()).unwrap();
    let south_floor = source["entities"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entity| entity["id"] == "532cc9b2-5538-57d2-ac75-f7d528b814ec")
        .expect("south open floor remains in the document");
    south_floor["components"]["tiling"]["derives"]["charging"] = serde_json::json!({});
    let scratch = std::env::temp_dir().join("minimal-easyloop-explicit-charging.json");
    std::fs::write(&scratch, serde_json::to_string(&source).unwrap()).unwrap();
    let w = worldmap::load(&scratch).unwrap();
    assert_eq!(w.map.graph.cells().filter(|cell| cell.kind == CellKind::Charging).count(), 32);
    assert_eq!(w.floors.iter().filter(|floor| floor.kind == CellKind::Charging).count(), 1);
    assert_eq!(w.map.graph.cell_count(), load().map.graph.cell_count(), "markers add no physical cells");
}

// ---------------------------------------------------------------------------
// (a) the census
// ---------------------------------------------------------------------------

/// One of each place the Walmart-era flows need, and no second one of
/// anything that would let a flow pass by accident on the spare.
///
/// The numbers are read off the loaded document, not off the generator: a
/// generator change that silently drops a station has to fail here.
#[test]
fn the_document_carries_one_of_every_place_a_walmart_era_flow_needs() {
    let w = load();

    // FIVE stations, and the split matters more than the total: the two picks
    // are not declared: they are what the EasyLoop stamp mints, so their
    // presence is the machine's presence.
    let by_kind = |t: StationType| w.map.stations.iter().filter(|s| s.station_type == t).count();
    assert_eq!(
        (
            by_kind(StationType::Pick),
            by_kind(StationType::Putaway),
            by_kind(StationType::BoxInduction),
            by_kind(StationType::Extraction),
        ),
        (2, 1, 1, 1),
        "one EasyLoop's two pick stations, one inventory induction, one box induction, one extraction"
    );
    assert_eq!(w.map.stations.len(), 5, "and nothing else stands on this floor");

    // ONE machine: an EasyLoop is two pick stations sharing one belt
    // (specs/easyloop/DESIGN.md section 1), so the belt count is the machine
    // count and two stations on one belt is the stamp working.
    let belts: BTreeSet<u32> = w.map.stations.iter().filter_map(|s| s.conveyor_group).collect();
    assert_eq!(belts.len(), 1, "one belt, so one machine");
    assert_eq!(
        w.map.stations.iter().filter(|s| s.conveyor_group.is_some()).count(),
        2,
        "both of the machine's pick stations ride it"
    );

    // Three direct-service presentations and the machine's two order places.
    let cells_of = |k: CellKind| w.map.graph.cells().filter(|c| c.kind == k).count();
    assert_eq!(cells_of(CellKind::Presentation), 5, "all operator presentation cells");
    // The machine's doorway: the two cells its ports mint, which are the only
    // places the inventory half hands a tote over a belt. Both keep `Exchange`
    // even though the inventory lane runs over the dropoff -- what the cell IS
    // is a belt handover, and a lane passing over it does not change that. The
    // other handoff cells are the relay access cells, where a tote changes
    // hands with a shelf.
    let relay_access: FxHashSet<CellId> = w.map.relays.iter().map(|r| r.htm_access).collect();
    assert!(relay_access.iter().all(|&cell| w.map.graph.cell(cell).unwrap().kind == CellKind::Exchange));
    assert_eq!(
        cells_of(CellKind::Exchange) - relay_access.len(),
        2,
        "beside the relay access cells, the EasyLoop's dropoff and collection ports"
    );
    let south_floor: Vec<_> = w
        .index
        .cell_entities
        .iter()
        .filter(|(_, entity)| entity.to_string().starts_with("532cc9b2-5538-57d2-ac75-f7d528b814ec/"))
        .map(|(cell, _)| w.map.graph.cell(*cell).unwrap())
        .collect();
    assert_eq!(south_floor.len(), 32, "the old strip keeps all of its traversable floor");
    assert!(south_floor.iter().all(|cell| cell.kind == CellKind::Open));
    assert_eq!(cells_of(CellKind::Charging), 0, "this fixture authors no charging infrastructure");
    assert!(cells_of(CellKind::Highway) > 0, "a through route exists");
    // 266 stated floor cells plus eleven rack anchors; VTM positions add no cells.
    assert_eq!(w.map.graph.cell_count(), 277, "the whole floor");

    // TWO aisles, told apart by what they hold and who works them -- the
    // distinction the order buffer IS.
    let storage: Vec<_> = w.map.zones.iter().filter(|z| z.holds == ZoneHolds::Inventory).collect();
    let buffer: Vec<_> = w.map.zones.iter().filter(|z| z.holds == ZoneHolds::Order).collect();
    assert_eq!((storage.len(), buffer.len()), (1, 1), "one stock aisle, one order-buffer aisle");
    assert_eq!(buffer[0].vtm_configuration.as_deref(), Some("gripper"), "the buffer is gripper-served");
    assert_ne!(storage[0].vtm_configuration.as_deref(), Some("gripper"), "stock is not");

    // Steel, by what each run holds. 55 storage positions in all: six stock
    // bays, a two-bay order-tote depot in the STOCK aisle, and a three-bay
    // buffer in the ORDER aisle -- five shelves each.
    let uses = |u: RackUse| w.racking.columns.iter().filter(|c| c.uses == u).count();
    assert_eq!(
        (uses(RackUse::Inventory), uses(RackUse::OrderDepot), uses(RackUse::OrderBuffer)),
        (30, 10, 15),
        "stock positions, empty-order-tote depot positions, order-buffer positions"
    );
    assert_eq!(w.racking.columns.len(), 55, "and no position belongs to no run");

    // Every bay's handoff shelf bound a relay: eight along the stock aisle,
    // three along the buffer's. Storage no lane serves is refused outright,
    // so a zero here would not reach this assertion -- the split is what it
    // pins.
    assert_eq!(w.map.relays.len(), 11, "one relay per bay across both aisles");
    assert_eq!((storage[0].relays.len(), buffer[0].relays.len()), (8, 3), "each aisle's own");

    // The fleet, and the two numbers that are load-bearing rather than
    // decorative: eight HTMs (see `a_buffer_bound_order_...` below for why not
    // six) and a gripper of the buffer's own.
    assert_eq!(w.map.fleet_config().htms, 8, "eight floor robots");
    assert_eq!(w.map.vtm_count(), 3, "two climbing the stock aisle, one the buffer's");
    assert_eq!((storage[0].vtm_count, buffer[0].vtm_count), (2, 1), "each aisle's own");

    // Ten empty ORDER totes, one per depot position. Close-out has nothing to
    // load an order into if this is zero, and the failure then reads as "no
    // order completed" rather than as the missing pool.
    assert_eq!((w.carriers.inventory, w.carriers.order), (0, 10), "an order-tote pool, no seeded stock carriers");
}

/// Every cell an HTM has business on can be driven to from a station.
///
/// The machine's cells are the reason this matters. They are declared, not
/// tiled, and every arc touching them is one an author asserted -- so a missing
/// stitch does not read as an error anywhere, it reads as a cell no robot can
/// reach, and an order tote could then never be driven to a presentation cell
/// while every count above still passed. The flood fill is what says the thirty
/// arcs are all there.
///
/// Aisle and Relay cells are excluded, and for the same reason: an aisle is
/// the ladder a climbing VTM works from and a relay is the shelf it sets a
/// tote on. Neither is floor. What the relays owe instead is an HTM access
/// cell on the reachable side, which is asserted below.
#[test]
fn no_cell_an_htm_needs_is_an_island() {
    use std::collections::VecDeque;

    let w = load();
    let g = &w.map.graph;
    let start = w.map.stations.iter().flat_map(|s| s.presentation_cells()).next().expect("a station stands somewhere");
    let mut seen = BTreeSet::new();
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
    let stranded: Vec<_> = g
        .cells()
        .filter(|c| !matches!(c.kind, CellKind::Aisle | CellKind::Relay))
        .filter(|c| !seen.contains(&c.id))
        .map(|c| c.id)
        .collect();
    assert!(stranded.is_empty(), "{} cells are unreachable from a station: {stranded:?}", stranded.len());

    // And the places the flows name outright.
    for station in &w.map.stations {
        assert!(
            station.presentation_cells().all(|cell| seen.contains(&cell)),
            "station {:?} presents on an island",
            station.id
        );
    }
    for relay in &w.map.relays {
        assert!(seen.contains(&relay.htm_access), "relay {:?} has no side an HTM can drive to", relay.id);
    }
}

/// No two declared tiles stand on one patch of ground.
///
/// The machine used to tile its own floor at [770, 850] inside a band tiled at
/// the same pitch but a different phase, and the band carved three holes to
/// keep them apart. There is no tiling inside the machine now: its thirty cells
/// are declared, each states its own 695 mm extent, and the bands carve out the
/// whole envelope -- so the claim to measure is the general one, over
/// RECTANGLES rather than centre distances, the way `dfw6_easyloop_floor.rs`
/// takes its census.
///
/// Only cells whose extent the DOCUMENT stated are measured. A cell that states
/// none inherits SPEC section 5's 900 mm default, which nobody declared, and
/// counting that square as floor is what made the DFW6 census read 57 false
/// pairs.
#[test]
fn no_two_declared_tiles_claim_one_patch_of_ground() {
    let w = load();
    let tiles: Vec<(CellId, [f64; 4])> = w
        .map
        .graph
        .cells()
        .filter(|c| w.cell_footprint_stated.get(c.id.0 as usize).copied().unwrap_or(false))
        .filter_map(|c| {
            let p = c.world_pos?;
            let [sx, sy] = w.cell_sizes_mm.get(c.id.0 as usize).copied()?;
            Some((c.id, [p.x_mm - sx / 2.0, p.y_mm - sy / 2.0, sx, sy]))
        })
        .collect();
    assert_eq!(tiles.len(), 266, "every cell the document states, band tiles and machine cells alike");

    // Thinner than a millimetre in either direction is two tiles meeting at an
    // edge a rounding step put on the wrong side; nothing stands on that.
    const SEAM_MM: f64 = 1.0;
    let mut overlapping = Vec::new();
    for (i, (a, ra)) in tiles.iter().enumerate() {
        for (b, rb) in &tiles[i + 1..] {
            let ox = (ra[0] + ra[2]).min(rb[0] + rb[2]) - ra[0].max(rb[0]);
            let oy = (ra[1] + ra[3]).min(rb[1] + rb[3]) - ra[1].max(rb[1]);
            if ox > SEAM_MM && oy > SEAM_MM {
                overlapping.push((*a, *b, ox, oy));
            }
        }
    }
    assert!(overlapping.is_empty(), "{} pairs of tiles overlap: {overlapping:?}", overlapping.len());
}

// ---------------------------------------------------------------------------
// (e) the machine, as essential cells
// ---------------------------------------------------------------------------

/// The machine's envelope, and the cells standing inside it.
fn machine_envelope(w: &WorldMap) -> [f64; 4] {
    let envelope: Vec<&Equipment> = w.equipment.iter().filter(|e| e.part == "envelope").collect();
    assert_eq!(envelope.len(), 1, "one machine, one envelope");
    envelope[0].rect_mm
}

fn inside(rect: [f64; 4], p: [f64; 2]) -> bool {
    p[0] >= rect[0] && p[0] <= rect[0] + rect[2] && p[1] >= rect[1] && p[1] <= rect[1] + rect[3]
}

fn machine_cells(w: &WorldMap) -> Vec<CellId> {
    let rect = machine_envelope(w);
    w.map.graph.cells().filter(|c| c.world_pos.is_some_and(|p| inside(rect, [p.x_mm, p.y_mm]))).map(|c| c.id).collect()
}

/// Only individually declared machine cells occupy the envelope: eight inventory
/// holding cells, two independent six-row waiting blocks, and two turns.
#[test]
fn only_the_machines_essential_cells_stand_inside_its_envelope() {
    let w = load();
    let cells = machine_cells(&w);
    assert_eq!(cells.len(), 52, "the machine's declared cells, and nothing else inside the envelope");

    // Every one of them is a cell the DOCUMENT declares, not one a generator
    // produced: a derived identity here would mean a tiling reached under the
    // machine, which is the thing the envelope is carved out of the bands to
    // prevent.
    for cell in &cells {
        let entity = w.index.cell_entities.get(cell).unwrap_or_else(|| panic!("{cell:?} stands for no entity"));
        assert!(!entity.is_derived(), "{entity} is derived, so something tiles inside the envelope");
    }

    // Each presentation has six waiting locations, separate from travel lanes.
    let waiting = cells
        .iter()
        .filter(|c| {
            w.index.cell_entities.get(c).and_then(|e| w.world.entities.get(e)).is_some_and(|e| e.has("waiting"))
        })
        .count();
    assert_eq!(waiting, 12, "two six-cell waiting blocks");
}

/// The two ports stand 840 mm apart, one under each belt centreline.
///
/// 840 is `V_EAST_C - V_WEST_C`, the distance between the twin belt's two
/// vertical arms in the DWG block, and the number `easyloop-dwg/README`
/// section 2 states outright. Two earlier numbers are retired here: 1,705.6 mm,
/// which `scripts/separate_easyloop_handoff_cells.py` spread the pair to
/// because the loader inferred both ports' approaches onto one apron tile and
/// the resulting turnstile deadlocked under collision fidelity; and 770 mm,
/// which was the pitch of the robot lattice this authoring deletes and which
/// left each port 35 mm off its own belt. The machine declares its lane now, so
/// no approach is inferred and the pair is safe at the drawing's spacing.
#[test]
fn the_two_ports_stand_where_the_drawing_puts_them() {
    let w = load();
    let station = w.map.stations.iter().find(|s| s.dropoff_cell.is_some()).expect("the machine");
    let at = |c| w.map.graph.cell(c).and_then(|x| x.world_pos).expect("a port stands somewhere");
    let drop = at(station.dropoff_cell.expect("a dropoff"));
    let collect = at(station.collection_cell.expect("a collection cell"));
    let apart = (collect.x_mm - drop.x_mm).hypot(collect.y_mm - drop.y_mm);
    assert!((apart - 840.0).abs() < 0.05, "the ports are {apart:.1}mm apart, and the drawing says 840");
}

/// Nothing enters the machine except through its four declared doors.
///
/// The machine used to stand inside a floor band that stitched every one of its
/// cells to the nearest band cell by CONTACT, so an HTM could enter it anywhere
/// along its perimeter and cross it on a path the real machine has no lane for.
/// Every arc is asserted now, and these four are all of them: the two ports and
/// two order entries and two order exits.
#[test]
fn no_arc_enters_the_machine_except_through_its_declared_doors() {
    use std::collections::BTreeSet;

    let w = load();
    let members: BTreeSet<_> = machine_cells(&w).into_iter().collect();
    let mut doors: BTreeSet<CellId> = BTreeSet::new();
    for cell in w.map.graph.cells() {
        for (next, _) in w.map.graph.neighbors(cell.id) {
            match (members.contains(&cell.id), members.contains(&next)) {
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
    assert_eq!(doors.len(), 6, "six doors: two inventory ports, two order entries and two order exits, got {doors:?}");
    let station = w.map.stations.iter().find(|s| s.dropoff_cell.is_some()).expect("the machine");
    assert!(doors.contains(&station.dropoff_cell.expect("a dropoff")), "the dropoff is a door");
    assert!(doors.contains(&station.collection_cell.expect("a collection cell")), "and so is the collection cell");
}

/// Deposit commits the HTM to the one-way route through collection.
#[test]
fn the_dropoff_has_no_early_exit_and_the_lane_is_gated_at_its_far_end() {
    let w = load();
    let members: BTreeSet<_> = machine_cells(&w).into_iter().collect();
    let station = w.map.stations.iter().find(|s| s.dropoff_cell.is_some()).expect("the machine");
    let dropoff = station.dropoff_cell.expect("a dropoff");
    let collect = station.collection_cell.expect("a collection cell");

    let out: Vec<(CellId, Option<DestGate>)> =
        w.map.graph.neighbors(dropoff).into_iter().map(|(n, e)| (n, e.dest_only)).collect();
    assert!(out.iter().all(|(n, _)| members.contains(n)), "no early exit after deposit: {out:?}");
    let inside = &out;

    // INTO the machine: exactly one arc, into the head of the inventory lane,
    // and OPEN -- a robot may drive in to wait for the belt as well as to meet
    // it, and what it is bound for is asked at the far end of the lane instead.
    assert_eq!(inside.len(), 1, "the dropoff has one way into the machine, the lane: {inside:?}");
    assert_eq!(inside[0].1, None, "a robot may drive into the lane to WAIT, so its head is not gated");

    // The far end IS gated: the arc out of the lane and onto the collection
    // cell admits a robot bound for either end of the belt and nobody else, so
    // a robot with no business inside cannot use the machine as a way across
    // the apron -- coming out the far end is the only thing it would be for.
    let last = w
        .map
        .graph
        .cells()
        .filter(|c| w.map.graph.neighbors(c.id).iter().any(|(n, _)| *n == collect))
        .map(|c| c.id)
        .collect::<Vec<_>>();
    assert_eq!(last.len(), 1, "the collection cell is entered exactly one way, from the lane: {last:?}");
    let serving = w.map.graph.neighbors(last[0]).into_iter().find(|(n, _)| *n == collect).expect("the arc").1;
    assert_eq!(
        serving.dest_only,
        Some(DestGate::Handovers([dropoff, collect])),
        "the arc onto the collection admits a robot bound for either end of the belt and nobody else"
    );
}

// ---------------------------------------------------------------------------
// (b) the run
// ---------------------------------------------------------------------------

/// A compact reading of one run: what finished, and how.
struct Run {
    events: Vec<SimEvent>,
    result: SimResult,
    metrics: MetricsOutput,
}

impl Run {
    fn go() -> Self {
        let (world, scenario, config) = common::load_world_spec(SPEC);
        let (events, result, metrics) = common::run_world_capture_all(&world, &scenario, config);
        Self { events, result, metrics }
    }

    fn cycles(&self, want: fn(&CycleKind) -> bool) -> usize {
        self.events.iter().filter(|e| matches!(e, SimEvent::CycleComplete(cc) if want(&cc.kind))).count()
    }

    /// Everything two runs of one document have to agree on: the counters, and
    /// the ordered list of what happened when.
    fn fingerprint(&self) -> (usize, Vec<(u64, String)>) {
        let s = &self.metrics.cumulative_scalars;
        let scalars = format!(
            "picks={} orders={} pres={}/{}/{}/{}",
            self.result.total_picks,
            s.orders_completed,
            s.pick_presentations,
            s.putaway_presentations,
            s.order_load_presentations,
            s.order_extract_presentations,
        );
        let mut trace = vec![(0u64, scalars)];
        for event in &self.events {
            match event {
                SimEvent::CycleComplete(cc) => {
                    trace.push((cc.sim_time_ms, format!("cycle {:?} at {:?}", cc.kind, cc.station_id)))
                }
                SimEvent::OrderComplete(oc) => trace.push((oc.sim_time_ms, format!("order {:?}", oc.order_id))),
                _ => {}
            }
        }
        (self.events.len(), trace)
    }
}

/// The preloaded scenario picks into order totes on HTMs, then extracts each
/// completed order. No separate order-load or stock-induction stage is present.
///
/// The three counters are asserted separately from the cycle kinds on purpose:
/// the counters are what a report would quote, the cycles are the physical
/// events that produced them, and a bug that increments one without the other
/// is exactly what this file is the early gate for.
#[test]
fn every_preloaded_order_is_picked_and_extracted() {
    let run = Run::go();
    let s = &run.metrics.cumulative_scalars;

    assert_eq!(run.result.total_picks, 24, "all twenty-four of the scenario's order picks");
    assert_eq!(s.orders_completed, 12, "all twelve orders");
    assert_eq!(s.order_load_presentations, 0, "items enter the carried order tote during picking");
    assert_eq!(s.order_extract_presentations, 12, "one extraction per completed order");
    assert_eq!(s.putaway_presentations, 0, "this composition preloads stock rather than inducting arrivals");
    assert_eq!(
        s.presentations,
        s.pick_presentations + s.putaway_presentations + s.order_load_presentations + s.order_extract_presentations,
        "every presentation is accounted for by exactly one kind"
    );

    // The physical events behind the counters.
    assert_eq!(run.cycles(|k| matches!(k, CycleKind::OrderLoad { .. })), 0, "no separate order loading");
    assert!(run.cycles(|k| matches!(k, CycleKind::OrderExtract { .. })) > 0, "and emptied at extraction");
    assert_eq!(run.cycles(|k| matches!(k, CycleKind::Putaway { .. })), 0, "stock was preloaded");
    assert!(run.cycles(|k| matches!(k, CycleKind::Pick { .. })) > 0, "and was picked out of it");

    // BOTH halves of the machine work. The workload is sized for this: at the
    // six orders this scenario started out with, dispatch put every one of
    // them on the first station and the second stood idle all run, which made
    // "an EasyLoop is two pick stations" a claim the census made and the run
    // never checked.
    let picked_at: BTreeSet<_> = run
        .events
        .iter()
        .filter_map(|e| match e {
            SimEvent::CycleComplete(cc) if matches!(cc.kind, CycleKind::Pick { .. }) => Some(cc.station_id),
            _ => None,
        })
        .collect();
    assert_eq!(picked_at.len(), 2, "both of the machine's pick stations picked, not just the first: {picked_at:?}");

    let deadlocks = run
        .events
        .iter()
        .filter(|e| matches!(e, SimEvent::Resource(r) if r.kind == ResourceEventKind::DeadlockHeadroomTriggered))
        .count();
    assert_eq!(deadlocks, 0, "the deadlock headroom alarm never fired");
    let watchdogs = run.events.iter().filter(|e| matches!(e, SimEvent::WatchdogTimeout(_))).count();
    assert_eq!(watchdogs, 0, "no order sat at a station past its watchdog");
}

// ---------------------------------------------------------------------------
// (c) determinism
// ---------------------------------------------------------------------------

/// The same document and the same spec, twice, produce the same run.
///
/// Not a general claim about the solver -- a floor under THIS fixture, so a
/// change that makes the early gate's numbers wobble is caught by the gate
/// itself rather than by a puzzled reader of two different reports.
#[test]
fn two_runs_of_the_same_document_agree_event_for_event() {
    let (events_a, trace_a) = Run::go().fingerprint();
    let (events_b, trace_b) = Run::go().fingerprint();
    assert_eq!(events_a, events_b, "the same number of events");
    assert_eq!(trace_a, trace_b, "the same cycles and completions, at the same times");
}

// ---------------------------------------------------------------------------
// (d) the order buffer, actually worked
// ---------------------------------------------------------------------------

/// The buffer detour, on the small map: pick station -> buffer relay -> buffer
/// rack -> dwell -> buffer relay -> extraction.
///
/// The generated orders state no exit path, so the run above takes the
/// straight-to-extraction route and the buffer band sits there proving
/// nothing. Here every order is declared `ExitPath::Buffer`, which is DFW6's
/// majority case, and the gripper has to file each closed order into the
/// order aisle and fetch it back out.
///
/// WHY EIGHT HTMs. At six this run wedges -- putaway staging holds four HTMs
/// carrying empty totes for the induction station while a `CloseOutRelease`
/// waits for one to fetch a tote back out of the buffer, and neither side
/// yields; the scheduler stops after 1,000 passes with no progress, ~57s of
/// wall time in. Eight clears it and so do ten and twelve. The wedge is the
/// same shape as greymatter-foundry#124's induction-policy gap, reproduced
/// here in a minute rather than in hours, which is worth knowing the map can
/// do.
#[test]
fn a_buffer_bound_order_rests_in_the_buffer_and_still_reaches_extraction() {
    use std::collections::{BTreeMap, BTreeSet};

    use desgraph::ZoneId;

    let (world, scenario, config) = common::load_world_spec(SPEC);
    let mut data = resolve_scenario_with_racking(world.map.clone(), world.racking.clone(), &scenario, &config)
        .expect("scenario generation");

    // Which racks are the buffer's, before anything moves.
    let buffer_zones: BTreeSet<ZoneId> =
        data.map.zones.iter().filter(|z| z.holds == ZoneHolds::Order).map(|z| z.id).collect();
    let cell_zone: BTreeMap<CellId, ZoneId> =
        data.zone_storage.iter().flat_map(|(&zid, cells)| cells.iter().map(move |&cid| (cid, zid))).collect();
    let buffer_racks: BTreeSet<RackId> = data
        .rack_cells
        .iter()
        .filter(|(_, cell)| cell_zone.get(cell).is_some_and(|z| buffer_zones.contains(z)))
        .map(|(&r, _)| r)
        .collect();
    assert_eq!(buffer_racks.len(), 15, "the buffer aisle's fifteen positions");

    // The buffer starts EMPTY. A slot holding a seeded tote is capacity a
    // closed order cannot use.
    assert!(
        !data
            .tote_placements
            .iter()
            .any(|tp| matches!(tp.location, ToteLocation::InRack { rack_id, .. } if buffer_racks.contains(&rack_id))),
        "no tote is seeded into buffer racking"
    );

    for order in &mut data.orders {
        order.exit_path = ExitPath::Buffer;
    }
    let order_totes: BTreeSet<ToteId> = data
        .tote_placements
        .iter()
        .filter(|tp| tp.designation == ToteDesignation::Order)
        .map(|tp| tp.tote_id)
        .collect();

    let (events, result, metrics) = run_prepared(data, config);

    let in_buffer: Vec<(VtmCycleDirection, RackId, ToteId)> = events
        .iter()
        .filter_map(|e| match e {
            SimEvent::VtmCycle(c) if buffer_racks.contains(&c.rack_id) => Some((c.direction, c.rack_id, c.tote_id)),
            _ => None,
        })
        .collect();
    let filed = in_buffer.iter().filter(|(d, _, _)| *d == VtmCycleDirection::Return).count();
    let fetched = in_buffer.iter().filter(|(d, _, _)| *d == VtmCycleDirection::Forward).count();

    let s = &metrics.cumulative_scalars;
    println!("[buffer] filed={filed} fetched={fetched} buffer_cycles={}", in_buffer.len());
    assert_eq!(result.total_picks, 24, "picking is untouched by the detour");
    assert_eq!(s.orders_completed, 12, "and every order still closes");
    assert_eq!(filed, 12, "the gripper filed all twelve closed orders into the buffer");
    assert!(fetched > 0, "and fetched at least one back out");
    assert!(fetched <= filed, "nothing comes out that never went in (fetched={fetched}, filed={filed})");

    // ONLY order totes rest here. The buffer is not overflow storage for
    // stock; that is what its zone's racking being exclusive buys.
    for (direction, rack, tote) in &in_buffer {
        assert!(order_totes.contains(tote), "an inventory tote {tote:?} moved {direction:?} in buffer rack {rack:?}");
    }

    // A detour, not a sink: what came back out was extracted.
    assert_eq!(s.order_load_presentations, 0, "buffer storage does not add an order-load operation");
    assert_eq!(
        s.order_extract_presentations as usize, fetched,
        "one extraction per tote fetched back out (extracts={}, fetched={fetched})",
        s.order_extract_presentations
    );
}

/// Run a scenario the caller has already resolved AND edited.
///
/// `common::run_world_capture_all` resolves from the spec and runs it; this
/// test has to reach in between those two steps to declare every order
/// buffer-bound, so it repeats the assembly rather than losing the edit.
fn run_prepared(mut data: ScenarioData, config: SimConfig) -> (Vec<SimEvent>, SimResult, MetricsOutput) {
    let pathfinder = common::fh_sipp_for_map(&data.map, &config);
    let (sub_tx, sub_rx) = mpsc::channel::<SimEvent>();
    let capture = std::thread::spawn(move || sub_rx.iter().collect::<Vec<_>>());
    let (event_tx, event_rx) = mpsc::channel();
    let sink: Arc<dyn EventSink> = Arc::new(event_tx.clone());
    let (scheduler, model, bias) = build(&mut data, &config, pathfinder, Some(sink)).expect("planner assembly");
    let (result, metrics) = run(
        scheduler,
        model,
        &data,
        config,
        event_tx,
        event_rx,
        bias,
        RunnerHooks { live: None, subscribers: vec![sub_tx], recording: None },
    )
    .expect("the run completes");
    (capture.join().unwrap(), result, metrics)
}
