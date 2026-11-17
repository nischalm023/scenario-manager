//! The carton line as a 3D PROP: the roller conveyor that DFW6 installs as
//! both a **box induction** and an **extraction** station, measured off the
//! `carton_induction` and `Extraction_conveyor` blocks of `DFW6 Pilot trim
//! down.dwg`.
//!
//! **One machine, two station types.** The two blocks are separate definitions
//! — 6,239 and 6,188 entities, and each labelled `INDUCTION` in its own MTEXT —
//! but they agree on every intrinsic dimension: deck 500mm between the inner
//! rails, frame 747mm across the outer pair, a five-strand comb 1,070mm long at
//! 107mm pitch, a robot row 850mm off the deck centreline. They differ only in
//! which END the comb is on: west on the induction block, east on the
//! extraction block. So this module measures the machine once and installs it
//! twice, and [`the_two_blocks_are_one_machine`] is the test that holds that
//! claim honest.
//!
//! **Both blocks are drawn in INCHES**, inserted at scale 25.4, and keep their
//! ~6,200 geometry entities on the vendor's `1轮廓实线层` layer with exactly one
//! entity on layer 0. Every number here is already millimetres. Note that
//! `dfw6-block-extents.json` reports them as 243 x 39mm and 239 x 35mm: that
//! extraction measured layer 0 only, and is wrong for both.
//!
//! **XY is MEASURED. Z comes from the counter-height ruling** in
//! [`worldview_floor::props`] — these are plan views, all z=0, no thickness, and
//! unlike the EasyLoop there is not even a stray remark on record about this
//! machine's heights.
//!
//! **Placement is CHOSEN for the inductions and MEASURED for the extractions**,
//! and neither comes from these blocks: all seven inserts of the two sit on
//! detail sheets, none on the site plan. Commit 5c4deb5e7 already records both
//! — the induction positions as *"CHOSEN, not surveyed"* and the extraction
//! positions as measured off the map image in `Walmart Dallas.docx`. So each
//! prop anchors on its own station cell; the document declares no equipment
//! envelope for either type.

use worldview_floor::props::{Builder, CARRIER, ROLLER_D, TOTE_H, Z_COUNTER, Z_DECK_TOP};
use worldview_render::{RackBox, RackBoxKind as K, RenderFrame};

// ── Intrinsic dimensions: identical in both blocks ─────────────────────────

/// Roller deck width, between the inner rail pair.
const DECK_W: f64 = 500.0;
/// Overall frame width, outer rail to outer rail.
const FRAME_W: f64 = 747.0;
/// The comb's run along the machine, its strand pitch, and its strand count.
/// Strands are drawn 15mm wide, invisible at site zoom, so [`STRAND_W`] widens
/// them to something that reads as a strand.
const COMB_LEN: f64 = 1070.0;
const COMB_PITCH: f64 = 107.0;
const COMB_STRANDS: usize = 5;
const STRAND_W: f64 = 40.0;
/// Cross-roller pitch, and the width one roller is drawn at.
///
/// Measured off the sections the blocks detail (rollers in 50/70 alternating
/// pairs) and extended along the whole deck: a roller conveyor does not stop
/// having rollers where the draughtsman stopped drawing them. The pitch is
/// measured; the extension is a judgement.
const ROLLER_PITCH: f64 = 120.0;
const ROLLER_W: f64 = 50.0;
/// How far off the deck centreline the robot row stands — the site's own
/// lattice pitch, in both blocks.
///
/// Not drawn; the sim draws real robots. Kept because it is what states which
/// side of the machine faces the traffic, and because a robot row on the site
/// lattice pitch is the evidence that these blocks belong to this site.
const ROBOT_ROW_OFFSET: f64 = 850.0;

/// One installed carton line: the measurements that differ between the two
/// blocks, plus how it stands on the floor.
struct CartonLine {
    /// Overall length of the drawn machine, along the deck.
    machine_len: f64,
    /// The deck's run within the machine.
    deck: [f64; 2],
    /// The comb's run within the machine.
    comb: [f64; 2],
    /// Which way the machine's own +x points once installed, in degrees
    /// counter-clockwise from east.
    heading_deg: f64,
}

/// Box induction, from `carton_induction`. Local origin is the south-west
/// corner of that block's geometry; the comb is at its WEST end.
///
/// Installed east-west (heading 0), which is the axis the south station band
/// runs on: DFW6 reads PICK PICK BI BI PICK PICK west to east.
const INDUCTION: CartonLine =
    CartonLine { machine_len: 6182.5, deck: [1004.0, 6158.0], comb: [65.0, 1135.0], heading_deg: 0.0 };

/// Extraction, from `Extraction_conveyor`. The same machine with the comb at
/// its EAST end.
///
/// Installed NORTH-SOUTH (heading 270, so the machine's +x points south and its
/// robot side faces east). Two reasons, and they agree:
///
/// 1. **East-west does not fit.** Extraction stations 1 and 2 stand 5,921mm
///    apart and this machine is 6,065mm long, so two east-west lines centred on
///    them would overlap by 144mm. [`extraction_lines_only_fit_north_south`]
///    pins that.
/// 2. **It is the rotation the block is inserted at.** All five
///    `Extraction_conveyor` inserts in the DWG are at rot 270.
///
/// **OPEN QUESTION: this may be the wrong machine for the role.** At the three
/// extraction station positions the SITE PLAN draws something else entirely —
/// three identical cells of one `Autobagger` (1,928 x 2,136mm, at 1:1 mm scale)
/// plus one `人工` manual station, two `AIO HS Mount top v3` and three
/// `Robots-OTH-Standard Storage Bin`. The correspondence is tight: those cells
/// sit at model x 64,224 / 70,251 / 79,700 against stations at 62,043 / 67,964
/// / 77,508 — a consistent ~2.2m offset — and their spacing (6,027 / 9,449)
/// matches the stations' (5,921 / 9,544) within ~100mm. Their robot rows land
/// 362mm and 1,216mm north of each station point.
///
/// So an extraction station at DFW6 is probably an autobagger pack-out cell,
/// with a roller line at most one part of it. `Extraction_conveyor` is a
/// detail-sheet block whose own MTEXT reads `INDUCTION`, and a site plan
/// outranks a detail sheet. Kept for now because a takeaway conveyor is a
/// plausible part of such a cell and this is the measured geometry we have;
/// flagged so nobody mistakes it for settled.
const EXTRACTION: CartonLine =
    CartonLine { machine_len: 6065.0, deck: [0.0, 5061.0], comb: [4930.0, 6000.0], heading_deg: 270.0 };

/// Build the prop for one box induction, at its station cell's world position.
pub(crate) fn induction_prop(station_mm: [f64; 2], frame: &RenderFrame) -> Vec<RackBox> {
    carton_line(&INDUCTION, station_mm, frame)
}

/// Build the prop for one extraction station, with its SOUTH end on the north
/// edge of the cell the robot presents at.
///
/// Not centred on the station point like the induction line. Extraction runs
/// north-south (heading 270, so local +x is world -y) and its comb -- the
/// robot handoff -- is at local x 4,930..6,000, the south end. Putting that
/// end against the station's north edge puts the handoff immediately in front
/// of the robot and the machine's body beyond it, instead of the machine
/// straddling the cell the robot stands in.
///
/// `edge_mm` is that north edge: the presentation cell's centre plus half its
/// depth.
pub(crate) fn extraction_prop(edge_mm: [f64; 2], frame: &RenderFrame) -> Vec<RackBox> {
    let anchor = [edge_mm[0], edge_mm[1] + EXTRACTION.machine_len / 2.0];
    carton_line(&EXTRACTION, anchor, frame)
}

/// The machine itself, in its own local millimetres: +x along the deck, +y
/// toward the robot side, origin at mid-length on the deck centreline.
///
/// The station point is taken as that origin — the machine's working centre —
/// which is the convention the EasyLoop's station pose was moved to as well.
fn carton_line(line: &CartonLine, station_mm: [f64; 2], frame: &RenderFrame) -> Vec<RackBox> {
    let mut b = Builder::new_oriented(station_mm, line.heading_deg, frame);
    // Local x measured from mid-length, so the station point is the centre.
    let mid = line.machine_len / 2.0;
    let (deck0, deck1) = (line.deck[0] - mid, line.deck[1] - mid);
    let (comb0, comb1) = (line.comb[0] - mid, line.comb[1] - mid);

    // The roller bed: frame the width of the outer rail pair, rollers the width
    // of the inner one, tops at the counter line.
    b.roller_bed([deck0, 0.0], [deck1, 0.0], DECK_W, FRAME_W, ROLLER_PITCH, ROLLER_W);

    // The five-strand comb: the robot handoff.
    for i in 0..COMB_STRANDS {
        let dy = (i as f64 - (COMB_STRANDS as f64 - 1.0) / 2.0) * COMB_PITCH;
        b.push_rect(
            [comb0, comb1, dy - STRAND_W / 2.0, dy + STRAND_W / 2.0],
            Z_DECK_TOP - ROLLER_D,
            Z_DECK_TOP,
            K::ConveyorTransfer,
        );
    }
    // Its housing. Slimmer than the bed's frame and stopping short of the
    // floor, so the handoff module reads as a module and not as a wall.
    b.push_rect(
        [comb0 + 60.0, comb1 - 60.0, -DECK_W / 2.0, DECK_W / 2.0],
        Z_DECK_TOP - ROLLER_D - 180.0,
        Z_DECK_TOP - ROLLER_D,
        K::ConveyorTransfer,
    );
    for cx in [comb0 + 140.0, comb1 - 140.0] {
        b.push_rect(
            [cx - 45.0, cx + 45.0, -DECK_W / 2.0 + 30.0, DECK_W / 2.0 - 30.0],
            0.0,
            Z_DECK_TOP - ROLLER_D - 180.0,
            K::ConveyorLeg,
        );
    }

    // Cartons: one on the comb at the handoff, three along the run, so the
    // line's direction and working height read at a glance. Illustrative — the
    // blocks draw no cartons and no carton dimension is on record, so they take
    // the vendor's own carrier footprint like every other box in these
    // drawings.
    let step = (deck1 - deck0) / 4.0;
    for i in 0..4 {
        let cx = if i == 0 { (comb0 + comb1) / 2.0 } else { deck0 + step * i as f64 };
        b.push(cx, 0.0, Z_COUNTER, Z_COUNTER + TOTE_H, CARRIER[0], CARRIER[1], 0.0, K::OrderTote);
    }

    b.into_boxes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use worldview_floor::props::{Z_GUARD_TOP, allowed_above_counter, prop_bounds};

    /// DFW6's box induction 1 and extraction stations 1 and 2, world millimetres.
    const INDUCTION_1: [f64; 2] = [45_490.0, 8_650.5];
    const EXTRACTION_1: [f64; 2] = [62_043.0, 85_824.0];
    const EXTRACTION_2: [f64; 2] = [67_964.0, 85_824.0];

    fn frame() -> RenderFrame {
        RenderFrame::from_extent_mm(1_000_000.0, 1_000_000.0)
    }

    /// The claim this module is built on: the two blocks are one machine.
    ///
    /// Each block's own rail pairs are checked against its own symmetry axis —
    /// `carton_induction` about 495.8, `Extraction_conveyor` about 433.4, both
    /// to 0.05mm across six pairs — and the two then agree on deck and frame
    /// width. If a future re-measurement breaks either, modelling them from one
    /// set of numbers stops being justified.
    #[test]
    fn the_two_blocks_are_one_machine() {
        let induction_rails: [(f64, f64); 6] =
            [(122.3, 869.3), (152.3, 839.3), (184.8, 806.8), (220.8, 770.8), (227.8, 763.8), (245.8, 745.8)];
        let extraction_rails: [(f64, f64); 6] =
            [(59.9, 806.9), (89.9, 776.9), (122.4, 744.4), (158.4, 708.4), (165.4, 701.4), (183.4, 683.4)];
        for (rails, axis, who) in
            [(induction_rails, 495.8, "carton_induction"), (extraction_rails, 433.4, "Extraction_conveyor")]
        {
            for (a, z) in rails {
                assert!(((a + z) / 2.0 - axis).abs() < 0.05, "{who}: rail pair ({a}, {z}) centres on {axis}");
            }
            // Inner pair is the deck, outer pair the frame — in both blocks.
            let (inner_a, inner_z) = rails[5];
            let (outer_a, outer_z) = rails[0];
            assert!((inner_z - inner_a - DECK_W).abs() < 0.05, "{who}: deck width is {DECK_W}");
            assert!((outer_z - outer_a - FRAME_W).abs() < 0.05, "{who}: frame width is {FRAME_W}");
        }
        // And the comb is the same length in both.
        for line in [&INDUCTION, &EXTRACTION] {
            assert!((line.comb[1] - line.comb[0] - COMB_LEN).abs() < 0.05, "the comb is {COMB_LEN}mm long");
        }
    }

    /// The comb spans essentially an HTM's width, which is why it is modelled as
    /// the robot handoff rather than as more belt.
    #[test]
    fn the_comb_is_sized_for_a_robot() {
        let span = (COMB_STRANDS as f64 - 1.0) * COMB_PITCH + 15.0; // 15mm strands, as drawn
        assert!((span - 460.0).abs() < 30.0, "the comb spans {span:.0}mm, about an HTM's 460mm width");
        assert_eq!(ROBOT_ROW_OFFSET, 850.0, "the robot row is one site lattice step off the centreline");
    }

    /// Extraction is installed north-south because east-west does not fit.
    ///
    /// This is the measurement that forced the orientation, so it is pinned
    /// rather than left as a comment: stations 1 and 2 are closer together than
    /// the machine is long.
    #[test]
    fn extraction_lines_only_fit_north_south() {
        let spacing = EXTRACTION_2[0] - EXTRACTION_1[0];
        assert!(
            spacing < EXTRACTION.machine_len,
            "east-west would overlap: {spacing:.0}mm apart, {:.0}mm long",
            EXTRACTION.machine_len
        );
        // North-south, the machines are only their frame width apart in x.
        let f = frame();
        let [x0, _, x1, _] = prop_bounds(&extraction_prop(EXTRACTION_1, &f));
        let across = (x1 - x0) as f64 * 1000.0;
        assert!(
            across < spacing,
            "north-south, the line is {across:.0}mm across and clears its neighbour at {spacing:.0}mm"
        );
        // And it really is turned: north-south means it is longer in y than x.
        let [_, y0, _, y1] = prop_bounds(&extraction_prop(EXTRACTION_1, &f));
        assert!(y1 - y0 > x1 - x0, "the extraction line runs north-south");
    }

    /// Induction runs east-west along the south station band, and its two
    /// stations clear each other on that axis.
    #[test]
    fn induction_lines_run_east_west_and_clear() {
        let f = frame();
        let [x0, y0, x1, y1] = prop_bounds(&induction_prop(INDUCTION_1, &f));
        assert!(x1 - x0 > y1 - y0, "the induction line runs east-west");
        let spacing = 52_953.5 - 45_490.0;
        let clearance = spacing - (x1 - x0) as f64 * 1000.0;
        assert!(clearance > 500.0, "the two inductions clear each other by {clearance:.0}mm");
    }

    /// An extraction line sits ENTIRELY NORTH of the cell the robot presents
    /// at, with its comb -- the handoff -- against that cell's north edge.
    /// Centred on the station point instead, the machine straddled the cell
    /// the robot stands in.
    #[test]
    fn an_extraction_line_starts_at_the_station_and_runs_north() {
        let f = frame();
        let edge = [61_400.0_f64, 87_098.0_f64];
        let boxes = extraction_prop(edge, &f);
        assert!(!boxes.is_empty(), "the prop has geometry");
        let mut y0 = f32::MAX;
        let mut y1 = f32::MIN;
        for b in &boxes {
            y0 = y0.min(b.min[1]);
            y1 = y1.max(b.min[1] + b.size[1]);
        }
        let edge_m = (edge[1] / 1000.0) as f32;
        assert!(
            (y0 - edge_m).abs() < 0.30,
            "the machine's south end meets the station's north edge: {y0:.2}m vs {edge_m:.2}m"
        );
        assert!(y1 > edge_m, "and the body runs north of it");
        let len = ((y1 - y0) * 1000.0) as f64;
        assert!(
            (len - EXTRACTION.machine_len).abs() < 400.0,
            "the whole machine is drawn: {len:.0}mm against {:.0}mm",
            EXTRACTION.machine_len
        );
    }

    /// An INDUCTION line is centred on its station, at the machine's working
    /// centre. Extraction is not: it aligns its south end to the station's
    /// north edge, which `an_extraction_line_starts_at_the_station_and_runs_north`
    /// covers. The two anchor differently because the robot meets them
    /// differently -- an induction hands a box out along its length, an
    /// extraction takes one in at its comb.
    #[test]
    fn an_induction_prop_is_centred_on_its_station() {
        let f = frame();
        for (station, boxes) in [(INDUCTION_1, induction_prop(INDUCTION_1, &f))] {
            let [x0, y0, x1, y1] = prop_bounds(&boxes);
            let cx = (x0 + x1) as f64 / 2.0 * 1000.0;
            let cy = (y0 + y1) as f64 / 2.0 * 1000.0;
            // The drawn centre can sit a little off the machine's mid-length,
            // because the comb housing and the bed's end legs are not
            // symmetric about it. 150mm is that slack, not a placement error.
            assert!((cx - station[0]).abs() < 150.0, "centred in x, off by {:.0}mm", cx - station[0]);
            assert!((cy - station[1]).abs() < 150.0, "centred in y, off by {:.0}mm", cy - station[1]);
        }
    }

    /// Waist-height equipment, same ruling as every other prop: only a carton
    /// standing on the deck is allowed above the counter line.
    #[test]
    fn machine_is_counter_height() {
        let ceiling = (Z_GUARD_TOP / 1000.0) as f32 + 1e-4;
        let f = frame();
        for boxes in [induction_prop(INDUCTION_1, &f), extraction_prop(EXTRACTION_1, &f)] {
            for b in boxes {
                assert!(b.min[2] >= 0.0, "{:?} starts at or above the floor", b.kind);
                assert!(b.size[2] > 0.0, "{:?} has positive height", b.kind);
                if allowed_above_counter(b.kind) {
                    continue;
                }
                assert!(
                    b.min[2] + b.size[2] <= ceiling,
                    "{:?} tops out at {:.0}mm, above the {:.0}mm counter line",
                    b.kind,
                    (b.min[2] + b.size[2]) * 1000.0,
                    Z_GUARD_TOP
                );
            }
        }
    }
}
