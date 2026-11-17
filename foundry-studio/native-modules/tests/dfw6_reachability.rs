//! Diagnostic: is DFW6's cell graph connected from stations to rack relays?
//!
//! Temporary. Answers whether the map's floor topology, not its workload, is
//! why a run cannot dispatch.

use std::collections::VecDeque;
use std::path::Path;

use desgraph::{CellId, StationType};
use desmap::{PresentationPoint, Station};
use dessim::worldmap;
use rustc_hash::FxHashSet;

/// Every lane of every station, beside its station.
fn lanes(stations: &[Station]) -> impl Iterator<Item = (&Station, &PresentationPoint)> {
    stations.iter().flat_map(|station| station.presentation_points.iter().map(move |lane| (station, lane)))
}

#[test]
fn dfw6_station_to_relay_reachability() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/walmart_dfw6.json");
    let w = worldmap::load(&path).expect("loads");

    let total = w.map.graph.cell_count();
    eprintln!("cells={total} edges={}", w.map.graph.edge_count());
    eprintln!("relays={} stations={}", w.map.relays.len(), w.map.stations.len());

    // Forward BFS from every station presentation cell.
    let seeds: Vec<CellId> = w.map.stations.iter().flat_map(|s| s.presentation_cells()).collect();
    eprintln!("station seed cells: {}", seeds.len());

    let mut seen: FxHashSet<CellId> = seeds.iter().copied().collect();
    let mut q: VecDeque<CellId> = seeds.iter().copied().collect();
    while let Some(c) = q.pop_front() {
        for (n, _e) in w.map.graph.neighbors(c) {
            if seen.insert(n) {
                q.push_back(n);
            }
        }
    }
    eprintln!("reachable FROM stations: {} / {total}  ({:.1}%)", seen.len(), 100.0 * seen.len() as f64 / total as f64);

    // How many relay HTM-access cells can a station actually reach?
    let htm_cells: Vec<CellId> = w.map.relays.iter().map(|r| r.htm_access).collect();
    let reachable_relays = htm_cells.iter().filter(|c| seen.contains(c)).count();
    eprintln!(
        "relay htm_access cells reachable from a station: {reachable_relays} / {}  ({:.1}%)",
        htm_cells.len(),
        100.0 * reachable_relays as f64 / htm_cells.len() as f64
    );

    // Reverse direction: from a relay cell, can we get back to a station?
    if let Some(&start) = htm_cells.first() {
        let mut seen2: FxHashSet<CellId> = FxHashSet::default();
        seen2.insert(start);
        let mut q2: VecDeque<CellId> = VecDeque::from(vec![start]);
        while let Some(c) = q2.pop_front() {
            for (n, _e) in w.map.graph.neighbors(c) {
                if seen2.insert(n) {
                    q2.push_back(n);
                }
            }
        }
        eprintln!("from relay[0]: {} cells reachable", seen2.len());
        for (s, lane) in lanes(&w.map.stations) {
            let ok = seen2.contains(&lane.presentation_cell);
            eprintln!(
                "  station {:?} kind={:?} presentation={:?} reachable_from_relay0={}",
                s.id, s.station_type, lane.presentation_cell, ok
            );
        }
    }

    // In/out degree of the pick stations' presentation cells and their queues.
    eprintln!("--- pick station cell degrees ---");
    for (st, lane) in lanes(&w.map.stations).filter(|(s, _)| s.station_type == StationType::Pick) {
        let out = w.map.graph.neighbors(lane.presentation_cell).len();
        let inn = w.map.graph.edges().filter(|(_, to, _)| *to == lane.presentation_cell).count();
        eprintln!(
            "  station {:?} presentation={:?} out_degree={out} in_degree={inn} queue_len={}",
            st.id,
            lane.presentation_cell,
            lane.queue_path.len()
        );
        for q in &lane.queue_path {
            let qo = w.map.graph.neighbors(*q).len();
            let qi = w.map.graph.edges().filter(|(_, to, _)| to == q).count();
            eprintln!("      queue cell {q:?} out={qo} in={qi}");
        }
    }

    // Can an HTM at an Easy Loop reach an extraction station?
    eprintln!("--- pick -> extraction ---");
    for (pick, lane) in lanes(&w.map.stations).filter(|(s, _)| s.station_type == StationType::Pick) {
        let mut seen3: FxHashSet<CellId> = FxHashSet::default();
        seen3.insert(lane.presentation_cell);
        let mut q3: VecDeque<CellId> = VecDeque::from(vec![lane.presentation_cell]);
        while let Some(c) = q3.pop_front() {
            for (n, _e) in w.map.graph.neighbors(c) {
                if seen3.insert(n) {
                    q3.push_back(n);
                }
            }
        }
        let reach: Vec<_> = w
            .map
            .stations
            .iter()
            .filter(|s| s.station_type == StationType::Extraction)
            .map(|s| (s.id, s.presentation_cells().all(|cell| seen3.contains(&cell))))
            .collect();
        eprintln!("  pick {:?} reaches {} cells; extraction: {:?}", pick.id, seen3.len(), reach);
    }

    eprintln!("--- extraction station cells ---");
    for (st, lane) in lanes(&w.map.stations).filter(|(s, _)| s.station_type == StationType::Extraction) {
        let out = w.map.graph.neighbors(lane.presentation_cell).len();
        let inn = w.map.graph.edges().filter(|(_, to, _)| *to == lane.presentation_cell).count();
        let pos = w.map.graph.cell(lane.presentation_cell).and_then(|c| c.world_pos);
        // component size reachable from this cell
        let mut seen: FxHashSet<CellId> = FxHashSet::default();
        seen.insert(lane.presentation_cell);
        let mut q: VecDeque<CellId> = VecDeque::from(vec![lane.presentation_cell]);
        while let Some(c) = q.pop_front() {
            for (n, _) in w.map.graph.neighbors(c) {
                if seen.insert(n) {
                    q.push_back(n);
                }
            }
        }
        eprintln!(
            "  {:?} cell={:?} out={out} in={inn} component={} pos={:?} queue_len={}",
            st.id,
            lane.presentation_cell,
            seen.len(),
            pos,
            lane.queue_path.len()
        );
    }

    eprintln!("--- extraction edges ---");
    for (st, lane) in lanes(&w.map.stations).filter(|(s, _)| s.station_type == StationType::Extraction) {
        let c = lane.presentation_cell;
        let outs: Vec<_> = w.map.graph.neighbors(c).into_iter().map(|(n, _)| n).collect();
        let ins: Vec<_> = w.map.graph.edges().filter(|(_, to, _)| *to == c).map(|(f, _, _)| f).collect();
        eprintln!("  {:?} cell={c:?} out->{outs:?} in<-{ins:?}", st.id);
        for q in &lane.queue_path {
            let qo: Vec<_> = w.map.graph.neighbors(*q).into_iter().map(|(n, _)| n).collect();
            let qi: Vec<_> = w.map.graph.edges().filter(|(_, to, _)| to == q).map(|(f, _, _)| f).collect();
            eprintln!("      q {q:?} out->{qo:?} in<-{qi:?}");
        }
    }
}
