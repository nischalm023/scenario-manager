//! Within `worldview-floor`, `rack` is the RENDER-SIDE RACK MODEL: it turns a
//! design's rack prototypes + placements and the compiled map's per-slot
//! geometry (`desmap::Zone::slots`) into what the renderer draws — physical
//! bay frames (uprights, beams, slot dividers, diving-board tabs) and the
//! per-slot tote positions — falling back to the legacy uniform
//! `vertical_tote_capacity` cell-extrusion model for maps without slots.
//!
//! THE FRAME: metres, via [`worldview_render::RenderFrame`], like everything else
//! drawn. A slot/bay x still maps by `t = (x - x_min) / (x_max - x_min)`
//! over the zone's ENTIRE compiled slot set (both sides share one frame)
//! onto the run's span — but that span is now the run's real first-to-last
//! cell CENTRE in metres, which is what dessim's `topology.rs` interpolates
//! onto too. Previously this span was measured in grid indices, so the two
//! agreed only on a uniform axis; in metres they agree always.
//!
//! The residual stretch is the gap between the steel's own length and the
//! run's centre-to-centre span: −1.7% on calibrated cencora and −0.25% on a
//! dimensionally-derived map, i.e. under a fifth of a cell over 56 metres.
//! (The ~18% this doc used to cite predates `row_centers_mm` calibration.)
//! Keeping the interpolation rather than drawing raw slot mm is what holds
//! racks and totes together, since the sim's own rack positions are
//! interpolated the same way. Bay ENDPOINTS may extrapolate slightly outside
//! [x_min, x_max] — the steel overhangs the end cell centres but stays inside
//! the zone's footprint.

use desgraph::ZoneId;
use desmap::WarehouseMap;
use dessim::scenario_gen::Racking;
use dessim::worldmap::Steel;
use worldview_render::{
    RenderFrame,
    rack::{RackBox, RackBoxKind, RackVisual},
};

use std::collections::BTreeMap;

use std::collections::BTreeSet;

/// Rendered headroom above the top storage beam — one tote height. VTMs
/// (and their carried totes) ride at this envelope top.
const TOTE_HEADROOM_MM: f64 = 400.0;

// Bay frame proportions, in METRES. These were fractions of a cell, which
// stopped meaning anything once a 645mm rack row and a 1227mm aisle could
// share a drawing. The values are the old fractions taken against a nominal
// 850mm cell, so the steel keeps the proportions it had on cencora while
// becoming a real physical size everywhere else.
/// Across-aisle inset of the steel inside its rack column's footprint.
const FOOT_INSET_M: f32 = 0.068;
/// Upright (bay leg) thickness along the run.
const UPRIGHT_THICKNESS_M: f32 = 0.043;
/// Beam depth below its slot floor (totes sit flush on the beam top).
const BEAM_THICKNESS_M: f32 = 0.051;
/// Slot-boundary divider thickness along the run.
const DIVIDER_THICKNESS_M: f32 = 0.021;
/// The real tote: 650 x 450mm, the drawing's own robot-block rectangle.
/// Mirrored from `shaders/totes.wgsl`, which draws it — kept here because the
/// board is sized from it, and `board_is_at_least_tote_sized` holds the two
/// together.
const TOTE_DEPTH_M: f32 = 0.65;
const TOTE_WIDTH_M: f32 = 0.45;

/// Diving-board tab: reach from the face into the bay, width along the run,
/// thickness below the board floor.
///
/// A board carries a tote, so it is at least tote-sized. The document states
/// where a board sits (400mm in from the face it is served at) but never how
/// big it is, so this is desvis's own hardware figure — and it was 408 x 425,
/// smaller than the 650 x 450 tote it presents, which left the tote
/// overhanging its own board by 121mm at each end.
const TAB_REACH_M: f32 = TOTE_DEPTH_M;
const TAB_WIDTH_M: f32 = TOTE_WIDTH_M;
const TAB_THICKNESS_M: f32 = 0.043;

/// The bay whose rectangle an exchange point sits on. The point lies on an
/// edge, so containment is tested with a millimetre of slack.
fn bay_at(steel: &[Steel], x_mm: f64, y_mm: f64) -> Option<&Steel> {
    const SLACK: f64 = 1.0;
    steel.iter().find(|s| {
        x_mm >= s.rect_mm[0] - SLACK
            && x_mm <= s.rect_mm[0] + s.rect_mm[2] + SLACK
            && y_mm >= s.rect_mm[1] - SLACK
            && y_mm <= s.rect_mm[1] + s.rect_mm[3] + SLACK
    })
}

/// A board's tab in render space: `TAB_REACH_M` from the face the exchange
/// point sits on, perpendicular, INTO the bay; `TAB_WIDTH_M` along that face.
///
/// The face is whichever of the bay's four edges the exchange point is
/// nearest, so this holds at any orientation — nothing assumes depth runs
/// along x.
fn tab_box(frame: &RenderFrame, bay: &Steel, ex_x: f64, ex_y: f64) -> ([f32; 2], [f32; 2]) {
    let [bx, by, bw, bh] = bay.rect_mm;
    let d = [ex_x - bx, bx + bw - ex_x, ex_y - by, by + bh - ex_y];
    let face = d.iter().enumerate().min_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).map(|(i, _)| i).unwrap_or(0);

    let reach = (TAB_REACH_M as f64) * 1000.0;
    let half = (TAB_WIDTH_M as f64) * 1000.0 / 2.0;
    let (x0, y0, x1, y1) = match face {
        0 => (ex_x, ex_y - half, ex_x + reach, ex_y + half),
        1 => (ex_x - reach, ex_y - half, ex_x, ex_y + half),
        2 => (ex_x - half, ex_y, ex_x + half, ex_y + reach),
        _ => (ex_x - half, ex_y - reach, ex_x + half, ex_y),
    };
    let [rx0, ry0] = frame.world_mm(x0, y0);
    let [rx1, ry1] = frame.world_mm(x1, y1);
    ([rx0.min(rx1), ry0.min(ry1)], [(rx1 - rx0).abs(), (ry1 - ry0).abs()])
}

// The v3 grid-rack builder (RackVisual::build + Builder) died with the v3
// compiler; this is the only thing that builds one now.
/// Build from a world document's stated layout: slots, boards, and steel
/// as the loader carried them, no grid in the middle. Every slot arrives
/// with its own position and height, so the frames render measured
/// geometry -- including tunnel bays, whose 12 shelves share the standard
/// top beam instead of stopping at half height (extruding by shelf COUNT
/// drew exactly that lie).
pub fn from_worldmap(map: &WarehouseMap, racking: &Racking, steel: &[Steel]) -> RackVisual {
    let frame = RenderFrame::default();

    // Slots per anchor cell (render space, for totes) and the raw mm per
    // cell (for the frame pass).
    // ordered: the map this builds is `RackVisual`'s, which travels.
    let mut cell_slots: BTreeMap<u32, Vec<[f32; 3]>> = BTreeMap::new();
    // ordered: the frame pass walks it, and the frames it pushes travel in that order.
    let mut by_cell_mm: BTreeMap<u32, Vec<([f64; 2], f64)>> = BTreeMap::new();
    for col in &racking.columns {
        let (Some(w), Some(floor_m)) = (col.world_pos, col.floor_m) else { continue };
        let [x, y] = frame.world_mm(w.x_mm, w.y_mm);
        let z = frame.z_of_mm(floor_m * 1000.0);
        cell_slots.entry(col.cell.0).or_default().push([x, y, z]);
        by_cell_mm.entry(col.cell.0).or_default().push(([w.x_mm, w.y_mm], floor_m * 1000.0));
    }
    // Bottom-up, then along the run (render y descends along the run),
    // then front-to-back within a slot pair.
    for slots in cell_slots.values_mut() {
        slots.sort_by(|a, b| a[2].total_cmp(&b[2]).then(b[1].total_cmp(&a[1])).then(a[0].total_cmp(&b[0])));
    }

    let mut frames: Vec<RackBox> = Vec::new();
    let mut top_z: f32 = 0.0;

    // One bay of steel per anchor cell, joined by containment: the bay
    // whose rectangle holds the cell's slot centroid. Adjacent bays abut,
    // so their shared uprights coincide -- harmless overdraw, exactly as
    // the v3 builder's per-side dedup produces at bay boundaries.
    for slots_mm in by_cell_mm.values() {
        let n = slots_mm.len() as f64;
        let cx = slots_mm.iter().map(|(p, _)| p[0]).sum::<f64>() / n;
        let cy = slots_mm.iter().map(|(p, _)| p[1]).sum::<f64>() / n;
        let Some(bay) = steel.iter().find(|s| {
            cx >= s.rect_mm[0]
                && cx <= s.rect_mm[0] + s.rect_mm[2]
                && cy >= s.rect_mm[1]
                && cy <= s.rect_mm[1] + s.rect_mm[3]
        }) else {
            continue;
        };
        let [ax0, ay0] = frame.world_mm(bay.rect_mm[0], bay.rect_mm[1]);
        let [ax1, ay1] = frame.world_mm(bay.rect_mm[0] + bay.rect_mm[2], bay.rect_mm[1] + bay.rect_mm[3]);
        let (x0, x1) = (ax0.min(ax1) + FOOT_INSET_M, ax0.max(ax1) - FOOT_INSET_M);
        let (y0, y1) = (ay0.min(ay1), ay0.max(ay1));

        let mut floor_mm: BTreeSet<i64> = BTreeSet::new();
        let mut ys: BTreeSet<i64> = BTreeSet::new();
        for (p, f) in slots_mm {
            floor_mm.insert(f.round() as i64);
            ys.insert(p[1].round() as i64);
        }
        let Some(&top_mm) = floor_mm.iter().next_back() else { continue };
        let bay_top_z = frame.z_of_mm(top_mm as f64);
        top_z = top_z.max(bay_top_z);

        // Storage beams: one shelf slab per level, spanning the bay.
        for &mm in &floor_mm {
            let z = frame.z_of_mm(mm as f64);
            frames.push(RackBox {
                min: [x0, y0, z - BEAM_THICKNESS_M],
                size: [x1 - x0, y1 - y0, BEAM_THICKNESS_M],
                yaw: 0.0,
                kind: RackBoxKind::Beam,
            });
        }

        // Slot-boundary dividers at midpoints of adjacent slot rows,
        // lowest shelf to top beam.
        let bottom_z = frame.z_of_mm(*floor_mm.iter().next().unwrap() as f64) - BEAM_THICKNESS_M;
        let slot_ys: Vec<i64> = ys.into_iter().collect();
        for pair in slot_ys.windows(2) {
            let [_, ry] = frame.world_mm(cx, (pair[0] + pair[1]) as f64 / 2.0);
            frames.push(RackBox {
                min: [x0, ry - DIVIDER_THICKNESS_M / 2.0, bottom_z],
                size: [x1 - x0, DIVIDER_THICKNESS_M, bay_top_z - bottom_z],
                yaw: 0.0,
                kind: RackBoxKind::Divider,
            });
        }

        // Bay legs at the run ends, floor to top beam.
        for leg_y in [y0, y1 - UPRIGHT_THICKNESS_M] {
            frames.push(RackBox {
                min: [x0, leg_y, 0.0],
                size: [x1 - x0, UPRIGHT_THICKNESS_M, bay_top_z],
                yaw: 0.0,
                kind: RackBoxKind::Upright,
            });
        }
    }

    // Board tabs and relay tote anchors. A diving board lives INSIDE the
    // steel (ruling 2026-08-20): the tab runs from the face it is served
    // at, into the bay.
    //
    // Which way that is comes from the BAY, not from the robot. The
    // exchange point sits on one edge of the bay's own rectangle, so
    // inward is the perpendicular from that edge toward the bay's centre.
    // Inferring it from the cell an HTM stands on inverts wherever the HTM
    // stands UNDER the rack rather than in an aisle — DFW6's under-rack
    // travel lanes, where it put all 2,132 tabs outside the steel.
    let mut board_mm: BTreeSet<i64> = BTreeSet::new();
    // ordered: as above.
    let mut relay_tote_pos: BTreeMap<u32, [f32; 3]> = BTreeMap::new();
    for relay in &map.relays {
        let Some(ex) = racking.relay_exchange_pos.get(&relay.id) else { continue };
        let Some(stand) = map.graph.cell(relay.htm_access).and_then(|c| c.world_pos) else { continue };
        let Some(boards) = racking.board_floors_m.get(&ZoneId(relay.block.0)) else { continue };
        let Some(&b0) = boards.first() else { continue };
        let _ = stand;
        let Some(bay) = bay_at(steel, ex.x_mm, ex.y_mm) else { continue };
        let ([tab_x, tab_y], [tab_w, tab_h]) = tab_box(&frame, bay, ex.x_mm, ex.y_mm);
        for f in boards {
            board_mm.insert((f * 1000.0).round() as i64);
            let z = frame.z_of_mm(f * 1000.0);
            frames.push(RackBox {
                min: [tab_x, tab_y, z - TAB_THICKNESS_M],
                size: [tab_w, tab_h, TAB_THICKNESS_M],
                yaw: 0.0,
                kind: RackBoxKind::BoardTab,
            });
        }
        // On the board, not on the rack edge. `ex` is the exchange point,
        // which sits ON the served face; the shader centres a tote on its
        // position, so anchoring there hung half of every relay tote
        // outside the steel and off the tab drawn inward above. The tab's
        // own midpoint is where the tote rests.
        //
        // Keyed by stand cell because that is all `ToteVisualState::AtRelay`
        // carries. Sound while one relay binds one stand cell — DFW6:
        // 2,132 relays, 2,132 distinct stand cells. A map where two relays
        // shared a stand would need the relay's identity threaded through
        // the visual state.
        relay_tote_pos.entry(relay.htm_access.0).or_insert([
            tab_x + tab_w / 2.0,
            tab_y + tab_h / 2.0,
            frame.z_of_mm(b0 * 1000.0),
        ]);
    }

    let board_floors_z: Vec<f32> = board_mm.iter().map(|&mm| frame.z_of_mm(mm as f64)).collect();
    RackVisual {
        steel_mm: steel.iter().map(|bay| bay.rect_mm).collect(),
        uniform_levels: 0,
        top_z: top_z + frame.z_of_mm(TOTE_HEADROOM_MM),
        board_floors_z,
        frames,
        cell_slots,
        relay_tote_pos,
        easyloop_belts: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use worldview_render::rack::LEVEL_HEIGHT;

    use super::*;

    /// Every board in the document draws a tab. The tab loop skips a relay
    /// whose zone lost its board-floor stack, and that skip is SILENT -- it
    /// once hid 14 of 20 zones' boards (the aisle merge kept the first
    /// draft's empty stack). One tab per board or the map is lying visually.
    #[test]
    fn every_board_draws_a_tab() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sim/dessim/tests/fixtures/walmart_dfw6.json");
        let w = dessim::worldmap::load(&fixture).expect("loads");
        let rv = from_worldmap(&w.map, &w.racking, &w.steel);
        let tabs = rv.frames.iter().filter(|f| f.kind == RackBoxKind::BoardTab).count();
        assert_eq!(tabs, w.map.relays.len(), "one tab per board");
    }

    /// A relay tote rests ON its board, inside the steel. Anchoring it at the
    /// exchange point put it on the rack edge — half outside, and off the tab
    /// drawn inward beside it.
    #[test]
    fn every_relay_tote_rests_on_its_tab() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sim/dessim/tests/fixtures/walmart_dfw6.json");
        let w = dessim::worldmap::load(&fixture).expect("loads");
        let rv = from_worldmap(&w.map, &w.racking, &w.steel);
        assert!(!rv.relay_tote_pos.is_empty(), "the map has relay tote anchors");

        let tabs: Vec<&RackBox> = rv.frames.iter().filter(|f| f.kind == RackBoxKind::BoardTab).collect();
        for anchor in rv.relay_tote_pos.values() {
            let [x, y, _] = *anchor;
            // Not merely over a tab — at its MIDPOINT. The old anchor sat on
            // the tab's edge, which is the rack face, so a containment check
            // alone would accept the bug it is here to catch.
            let centred = tabs.iter().any(|t| {
                let mid_x = t.min[0] + t.size[0] / 2.0;
                let mid_y = t.min[1] + t.size[1] / 2.0;
                (x - mid_x).abs() < 1e-3 && (y - mid_y).abs() < 1e-3
            });
            assert!(centred, "relay tote at {x}, {y} is not centred on any board tab");
        }
    }

    /// A board is part of the bay, so its tab lies within some bay's steel.
    /// This is what silently inverted: inferring "inward" from where the HTM
    /// stands puts every tab outside wherever the HTM stands UNDER the rack.
    #[test]
    fn htm_presentation_cells_are_under_rendered_relay_boards() {
        let frame = RenderFrame::default();
        for (name, expected) in [("walmart_dfw6.json", 2_132), ("cencora_v3.json", 684)] {
            let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sim/dessim/tests/fixtures").join(name);
            let w = dessim::worldmap::load(&fixture).expect("loads");
            let rv = from_worldmap(&w.map, &w.racking, &w.steel);
            let tabs: Vec<_> = rv.frames.iter().filter(|f| f.kind == RackBoxKind::BoardTab).collect();
            assert_eq!(w.map.relays.len(), expected);
            let outside: Vec<_> = w
                .map
                .relays
                .iter()
                .filter_map(|relay| {
                    let p = w.map.cell_world_pos(relay.htm_access).unwrap();
                    let [x, y] = frame.world_mm(p.x_mm, p.y_mm);
                    let under = tabs.iter().any(|tab| {
                        x >= tab.min[0]
                            && x <= tab.min[0] + tab.size[0]
                            && y >= tab.min[1]
                            && y <= tab.min[1] + tab.size[1]
                    });
                    (!under).then_some((relay.id, relay.htm_access, [x, y]))
                })
                .collect();
            assert!(
                outside.is_empty(),
                "{name}: {} HTM relay stands are not beneath a drawn board; first examples: {:?}",
                outside.len(),
                &outside[..outside.len().min(3)]
            );
        }
    }

    #[test]
    fn no_board_tab_falls_outside_the_steel() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sim/dessim/tests/fixtures/walmart_dfw6.json");
        let w = dessim::worldmap::load(&fixture).expect("loads");
        let frame = RenderFrame::default();
        let rv = from_worldmap(&w.map, &w.racking, &w.steel);

        let rects: Vec<[f32; 4]> = w
            .steel
            .iter()
            .map(|s| {
                let [ax0, ay0] = frame.world_mm(s.rect_mm[0], s.rect_mm[1]);
                let [ax1, ay1] = frame.world_mm(s.rect_mm[0] + s.rect_mm[2], s.rect_mm[1] + s.rect_mm[3]);
                [ax0.min(ax1), ay0.min(ay1), ax0.max(ax1), ay0.max(ay1)]
            })
            .collect();

        let tabs: Vec<&RackBox> = rv.frames.iter().filter(|f| f.kind == RackBoxKind::BoardTab).collect();
        let outside = tabs
            .iter()
            .filter(|t| {
                let (x0, y0) = (t.min[0], t.min[1]);
                let (x1, y1) = (t.min[0] + t.size[0], t.min[1] + t.size[1]);
                !rects.iter().any(|r| x0 >= r[0] - 1e-3 && x1 <= r[2] + 1e-3 && y0 >= r[1] - 1e-3 && y1 <= r[3] + 1e-3)
            })
            .count();
        assert_eq!(outside, 0, "{outside} of {} board tabs are outside every bay", tabs.len());
    }

    /// A board holds a tote. Draw it smaller and the tote overhangs its own
    /// board — which is what 408 x 425 against a 650 x 450 tote did, by 121mm
    /// at each end along the reach.
    #[test]
    fn board_is_at_least_tote_sized() {
        const { assert!(TAB_REACH_M >= TOTE_DEPTH_M, "the board's reach is shorter than the tote it carries") };
        const { assert!(TAB_WIDTH_M >= TOTE_WIDTH_M, "the board is narrower than the tote it carries") };
        // And it must still fit the shallowest bay DFW6 has (1,498mm).
        const { assert!(TAB_REACH_M < 1.498, "the board's reach exceeds the shallowest bay") };
    }

    #[test]
    fn uniform_ladder_matches_legacy_model() {
        let rv = RackVisual::uniform(8);
        assert_eq!(rv.uniform_levels, 8);
        assert_eq!(rv.top_z, 9.0 * LEVEL_HEIGHT);
        assert!(rv.frames.is_empty());
        // Uniform in-rack totes keep the legacy cell-center ladder path.
        assert_eq!(rv.in_rack_slot_pos(42, 0), None);
        assert_eq!(rv.relay_tote_z(), 0.01);
    }

    /// The bays' steel comes out in anchor-cell order, each bay's legs around
    /// the slots of the cell it is built for: the frames travel in a floor's
    /// bytes, so their order is the cells', never a hash map's
    /// (reviews-impl/astra-t12.md finding 9).
    #[test]
    fn the_bays_steel_comes_out_in_anchor_cell_order() {
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sim/dessim/tests/fixtures/minimal_putaway.json");
        let w = dessim::worldmap::load(&fixture).expect("loads");
        let frame = RenderFrame::default();
        let mut slots: BTreeMap<u32, Vec<[f64; 2]>> = BTreeMap::new();
        for col in &w.racking.columns {
            if let (Some(at), Some(_)) = (col.world_pos, col.floor_m) {
                slots.entry(col.cell.0).or_default().push([at.x_mm, at.y_mm]);
            }
        }
        let centroids: Vec<[f32; 2]> = slots
            .values()
            .map(|at| {
                at.iter().fold([0.0, 0.0], |sum, p| [sum[0] + p[0], sum[1] + p[1]]).map(|sum| sum / at.len() as f64)
            })
            .filter(|&[x, y]| {
                w.steel.iter().any(|bay| {
                    let [bx, by, bw, bh] = bay.rect_mm;
                    (bx..=bx + bw).contains(&x) && (by..=by + bh).contains(&y)
                })
            })
            .map(|[x, y]| frame.world_mm(x, y))
            .collect();
        let rv = from_worldmap(&w.map, &w.racking, &w.steel);
        let legs: Vec<&RackBox> = rv.frames.iter().filter(|f| f.kind == RackBoxKind::Upright).collect();
        assert!(centroids.len() > 2 && legs.len() == 2 * centroids.len(), "two legs for each anchor cell's bay");
        for (at, (pair, [x, y])) in legs.chunks(2).zip(&centroids).enumerate() {
            let (x0, x1) = (pair[0].min[0] - FOOT_INSET_M, pair[0].min[0] + pair[0].size[0] + FOOT_INSET_M);
            let (y0, y1) = (pair[0].min[1], pair[1].min[1] + pair[1].size[1]);
            assert!((x0..=x1).contains(x) && (y0..=y1).contains(y), "bay {at} is not the {at}th anchor cell's");
        }
    }

    // The cencora_v3 explicit-geometry pins died with the v3 compiler;
    // the from_worldmap path is pinned by every_board_draws_a_tab above
    // and by the worldproto suites.
}
