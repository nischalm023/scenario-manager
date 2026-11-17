//! Can dessim load the layered DFW6 decomposition?
use dessim::worldmap;
use std::path::Path;

#[test]
fn layered_probe_loads() {
    let base = Path::new("/Users/kristinabrenneman/work/greyorange/for-daniel/layered-probe");
    for candidate in ["scenario.json", "world.json"] {
        let p = base.join(candidate);
        match worldmap::load(&p) {
            Ok(w) => eprintln!(
                "{candidate}: OK cells={} relays={} stations={} steel={}",
                w.map.graph.cell_count(),
                w.map.relays.len(),
                w.map.stations.len(),
                w.steel.len()
            ),
            Err(e) => eprintln!("{candidate}: ERR {e}"),
        }
    }
}
