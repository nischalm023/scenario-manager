//! Diagnostic: do relays share the cell an HTM stands on to reach them?
use desgraph::CellId;
use dessim::worldmap;
use rustc_hash::FxHashMap;
use std::path::Path;

#[test]
fn relay_stand_cell_sharing() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/walmart_dfw6.json");
    let w = worldmap::load(&path).expect("loads");
    let mut by_stand: FxHashMap<CellId, Vec<u32>> = FxHashMap::default();
    for r in &w.map.relays {
        by_stand.entry(r.htm_access).or_default().push(r.id.0);
    }
    let shared: Vec<_> = by_stand.iter().filter(|(_, v)| v.len() > 1).collect();
    let lost: usize = shared.iter().map(|(_, v)| v.len() - 1).sum();
    eprintln!("relays={} distinct stand cells={}", w.map.relays.len(), by_stand.len());
    eprintln!("stand cells serving >1 relay: {}", shared.len());
    eprintln!("relays that would LOSE their tote anchor to or_insert: {lost}");
    for (cell, ids) in shared.iter().take(3) {
        eprintln!("  {cell:?} <- relays {ids:?}");
    }
}
