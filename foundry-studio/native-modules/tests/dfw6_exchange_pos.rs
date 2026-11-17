//! Where is a relay's exchange point, really?
use dessim::worldmap;
use std::path::Path;

#[test]
fn exchange_points_versus_their_steel() {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/walmart_dfw6.json");
    let w = worldmap::load(&p).expect("loads");
    let ex = &w.racking.relay_exchange_pos;
    let xs: Vec<f64> = ex.values().map(|p| p.x_mm).collect();
    let ys: Vec<f64> = ex.values().map(|p| p.y_mm).collect();
    let mn = |v: &Vec<f64>| v.iter().cloned().fold(f64::INFINITY, f64::min);
    let mx = |v: &Vec<f64>| v.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    eprintln!("exchange points: {}", ex.len());
    eprintln!("  x range {:.1} .. {:.1} mm", mn(&xs), mx(&xs));
    eprintln!("  y range {:.1} .. {:.1} mm", mn(&ys), mx(&ys));
    let sxs: Vec<f64> = w.steel.iter().map(|s| s.rect_mm[0]).collect();
    let sys: Vec<f64> = w.steel.iter().map(|s| s.rect_mm[1]).collect();
    eprintln!("steel min corners:");
    eprintln!("  x range {:.1} .. {:.1} mm", mn(&sxs), mx(&sxs));
    eprintln!("  y range {:.1} .. {:.1} mm", mn(&sys), mx(&sys));
    for (id, p) in ex.iter().take(3) {
        eprintln!("  relay {:?} exchange ({:.1}, {:.1})", id, p.x_mm, p.y_mm);
    }

    eprintln!("--- exchange vs stand (first 6 relays) ---");
    for r in w.map.relays.iter().take(6) {
        let e = ex.get(&r.id);
        let st = w.map.graph.cell(r.htm_access).and_then(|c| c.world_pos);
        match (e, st) {
            (Some(e), Some(st)) => eprintln!(
                "  relay {:?} ex.x={:.1} stand.x={:.1}  ex>=stand? {}  side={:?}",
                r.id,
                e.x_mm,
                st.x_mm,
                e.x_mm >= st.x_mm,
                r.side
            ),
            _ => eprintln!("  relay {:?} missing", r.id),
        }
    }
}
