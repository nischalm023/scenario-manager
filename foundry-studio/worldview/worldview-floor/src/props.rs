//! Shared machinery for STATION PROPS: the decorative 3D models of installed
//! equipment that desvis draws beside the cells a solver actually runs on.
//!
//! A prop is measured off a DWG block and assembled here into oriented boxes
//! for the `rack_frames` instanced-box pipeline. This module owns the two
//! things every prop needs and none of them should own privately:
//!
//! 1. **The height ruling.** No DWG block of this site's equipment carries any
//!    height — they are plan views, drawn at z=0 with no thickness. So heights
//!    come from one ergonomic rule (2026-08-27): read against an
//!    average-height operator, this is waist-height equipment, and every
//!    surface a carrier can sit on is at [`Z_COUNTER`], 36 inches.
//! 2. **The conveyor vocabulary.** Decks, guard rails, legs, curves, ramps and
//!    presentation stands, all placed by centreline in a block's own local
//!    millimetres and converted once, on the way out, by [`Builder`].
//!
//! Individual props (see [`crate::easyloop`], and desvis's own prop modules)
//! are then nothing but measurements plus an assembly order, which is the
//! point: a measurement that turns out to be wrong should be a one-line edit
//! in one place, not a shape to re-derive.
//!
//! Where a prop's own reading of its drawing disagrees with the shared
//! vocabulary, it says so through an option rather than by assembling boxes by
//! hand: [`Legs`] is the first, and it decides whether a conveyor run draws the
//! understructure nobody measured.

use worldview_render::{RackBox, RackBoxKind as K, RenderFrame};

/// Counter height, 36 inches. **The scale ruling of 2026-08-27:** read against
/// an average-height operator, this equipment is waist-height, so every surface
/// you could set a carrier down on shares this height.
///
/// It is the only Z with a stated basis, and the basis is ergonomic, not
/// measured. What it cannot make waist-height is a *carrier standing on* a
/// counter: a 400mm tote on a 914mm surface tops out at 1,314mm, which is
/// geometry, not a choice. Carriers, operators and pick-to-light heads are the
/// things allowed above this line; each prop's own test holds the rest below.
pub const Z_COUNTER: f64 = 914.0;

/// Top of roller on a conveyor deck, and of any presentation surface.
///
/// **Flush by necessity where a carrier crosses between them.** The EasyLoop's
/// transverse rollers drag a tote sideways off the belt into a seat; a step
/// between belt and seat is a step they cannot make.
pub const Z_DECK_TOP: f64 = Z_COUNTER;
/// Top of the understructure — a deck's own thickness below its surface.
pub const Z_LEG_TOP: f64 = Z_COUNTER - 120.0;
/// Top of a side guard rail: a low lip, the one thing allowed just over the
/// counter line because a deck with no lip sheds carriers.
pub const Z_GUARD_TOP: f64 = Z_COUNTER + 50.0;

/// Guard rail thickness.
pub const GUARD_T: f64 = 40.0;
/// Leg section, and how far apart legs stand along a run.
pub const LEG_SECTION: f64 = 90.0;
pub const LEG_PITCH: f64 = 1200.0;
/// Thickness of a presentation stand's top deck.
pub const STAND_T: f64 = 70.0;
/// Depth of a conveyor roller: how far below the counter line its bed sits.
pub const ROLLER_D: f64 = 60.0;

/// Carrier height. The repo's only tote height: the headroom `rack_geometry`
/// reserves for a tote above the top storage beam.
pub const TOTE_H: f64 = 400.0;
/// Carrier footprint, long axis by short. The size of the vendor's own
/// `Robots-RB-K50H_L(650450)` symbol, and the footprint of every presented
/// tote and box in these drawings.
pub const CARRIER: [f64; 2] = [650.0, 450.0];

/// What a conveyor RUN stands on, when it is drawn at all.
///
/// The understructure of a belt is the one part of these machines nobody
/// measured: the blocks are plan views at z=0, so a leg's section, its pitch
/// and its very existence are all read off [`LEG_SECTION`] and [`LEG_PITCH`],
/// which are assumptions. Drawn under a deck they are also the parts that read
/// worst — a row of posts crowding the floor a robot is supposed to drive on.
/// So a prop that has nothing to say about what holds its belts up says
/// [`Legs::None`] and draws the running surface alone.
///
/// This governs conveyor RUNS only. A presentation stand's four corner legs
/// (see [`Builder::stand`]) are not understructure — they are what makes the
/// stand a stand rather than a crate — and are drawn either way.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Legs {
    /// Posts at [`LEG_PITCH`] along a straight run and a roller bed, under
    /// every step of a ramp, and at every third chord of a curve.
    UnderRuns,
    /// Nothing under the deck.
    None,
}

/// Accumulates prop boxes, converting a block's local millimetres into the
/// render frame on the way in.
///
/// Every method takes local millimetres and a yaw in DEGREES, because that is
/// what a drawing is read in; nothing outside this type sees render metres.
pub struct Builder<'a> {
    /// The block's local origin, in world millimetres.
    origin: [f64; 2],
    /// How the whole assembly is turned about that origin, in degrees. Lets one
    /// measured machine be installed at any heading without the module that
    /// measured it doing trigonometry: the same carton line runs east-west as a
    /// box induction and north-south as an extraction.
    heading_deg: f64,
    /// Whether this assembly's conveyor runs draw an understructure.
    legs: Legs,
    frame: &'a RenderFrame,
    out: Vec<RackBox>,
}

impl<'a> Builder<'a> {
    /// A builder whose local axes are the world's: local +x is east, +y north.
    pub fn new(origin: [f64; 2], frame: &'a RenderFrame) -> Self {
        Self::new_oriented(origin, 0.0, frame)
    }

    /// A builder whose local axes are turned `heading_deg` counter-clockwise
    /// about `origin`.
    pub fn new_oriented(origin: [f64; 2], heading_deg: f64, frame: &'a RenderFrame) -> Self {
        Self { origin, heading_deg, legs: Legs::UnderRuns, frame, out: Vec::new() }
    }

    /// Say what this assembly's conveyor runs stand on. Defaults to
    /// [`Legs::UnderRuns`].
    pub fn with_legs(mut self, legs: Legs) -> Self {
        self.legs = legs;
        self
    }

    /// One leg post, centred at `cx, cy` and spanning `z0..z1`, if this
    /// assembly draws them at all. The single gate every run-drawing method
    /// passes its posts through.
    fn run_leg(&mut self, cx: f64, cy: f64, z0: f64, z1: f64, width: f64, yaw_deg: f64) {
        if self.legs == Legs::None {
            return;
        }
        self.push(cx, cy, z0, z1, LEG_SECTION, width, yaw_deg, K::ConveyorLeg);
    }

    pub fn into_boxes(self) -> Vec<RackBox> {
        self.out
    }

    /// One box, by its XY centre in block-local millimetres, its z span, its
    /// footprint before rotation, and its yaw in degrees about that centre.
    #[allow(clippy::too_many_arguments)]
    pub fn push(&mut self, cx: f64, cy: f64, z0: f64, z1: f64, sx: f64, sy: f64, yaw_deg: f64, kind: K) {
        // Turn the point about the local origin, then turn the box on itself by
        // the same amount, so an assembly heading needs no trigonometry at the
        // call sites.
        let h = self.heading_deg.to_radians();
        let (ch, sh) = (h.cos(), h.sin());
        let (rx, ry) = (cx * ch - cy * sh, cx * sh + cy * ch);
        let [wx, wy] = self.frame.world_mm(self.origin[0] + rx, self.origin[1] + ry);
        let (hx, hy) = ((sx / 2000.0) as f32, (sy / 2000.0) as f32);
        self.out.push(RackBox {
            min: [wx - hx, wy - hy, self.frame.z_of_mm(z0)],
            size: [hx * 2.0, hy * 2.0, self.frame.z_of_mm(z1 - z0)],
            yaw: ((yaw_deg + self.heading_deg) as f32).to_radians(),
            kind,
        });
    }

    /// One axis-aligned box from `[x0, x1, y0, y1]` in block-local millimetres.
    pub fn push_rect(&mut self, r: [f64; 4], z0: f64, z1: f64, kind: K) {
        self.push((r[0] + r[1]) / 2.0, (r[2] + r[3]) / 2.0, z0, z1, r[1] - r[0], r[3] - r[2], 0.0, kind);
    }

    /// A straight belt run along its centreline `a` to `c`: deck, a guard on
    /// each long edge, and legs at [`LEG_PITCH`].
    pub fn straight_belt(&mut self, a: [f64; 2], c: [f64; 2], deck_w: f64) {
        let (dx, dy) = (c[0] - a[0], c[1] - a[1]);
        let len = dx.hypot(dy);
        let yaw = dy.atan2(dx).to_degrees();
        let (mx, my) = ((a[0] + c[0]) / 2.0, (a[1] + c[1]) / 2.0);
        self.push(mx, my, Z_LEG_TOP, Z_DECK_TOP, len, deck_w, yaw, K::ConveyorDeck);
        // Guards ride the deck's two long edges, offset along its normal.
        let (nx, ny) = (-dy / len, dx / len);
        for s in [1.0, -1.0] {
            let off = s * (deck_w / 2.0 - GUARD_T / 2.0);
            self.push(mx + nx * off, my + ny * off, Z_DECK_TOP, Z_GUARD_TOP, len, GUARD_T, yaw, K::ConveyorGuard);
        }
        let n = ((len / LEG_PITCH).floor() as usize).max(1);
        for i in 0..=n {
            let t = i as f64 / n as f64;
            self.run_leg(a[0] + dx * t, a[1] + dy * t, 0.0, Z_LEG_TOP, deck_w - 60.0, yaw);
        }
    }

    /// A roller bed along its centreline: a frame the full `frame_w`, guards,
    /// legs, and the cross rollers that are its running surface.
    ///
    /// Distinct from [`Self::straight_belt`] in the one way that matters: on a
    /// roller conveyor the ROLLERS are the surface, so their tops sit at
    /// [`Z_DECK_TOP`] and the frame carrying them sits [`ROLLER_D`] below. A
    /// belt drawn with a slab at deck height and rollers under it hides the
    /// rollers and puts the carrier a roller's depth too high.
    #[allow(clippy::too_many_arguments)]
    pub fn roller_bed(&mut self, a: [f64; 2], c: [f64; 2], deck_w: f64, frame_w: f64, pitch: f64, roller_w: f64) {
        let (dx, dy) = (c[0] - a[0], c[1] - a[1]);
        let len = dx.hypot(dy);
        let yaw = dy.atan2(dx).to_degrees();
        let (mx, my) = ((a[0] + c[0]) / 2.0, (a[1] + c[1]) / 2.0);
        let bed_top = Z_DECK_TOP - ROLLER_D;

        self.push(mx, my, bed_top - 120.0, bed_top, len, frame_w, yaw, K::PresentDeck);
        // Guards stand on the frame's outer edges, up past the roller tops.
        let (nx, ny) = (-dy / len, dx / len);
        for s in [1.0, -1.0] {
            let off = s * (frame_w / 2.0 - GUARD_T / 2.0);
            self.push(mx + nx * off, my + ny * off, bed_top, Z_GUARD_TOP, len, GUARD_T, yaw, K::ConveyorGuard);
        }
        let n = ((len / LEG_PITCH).floor() as usize).max(1);
        for i in 0..=n {
            let t = i as f64 / n as f64;
            self.run_leg(a[0] + dx * t, a[1] + dy * t, 0.0, bed_top - 120.0, frame_w - 60.0, yaw);
        }
        // The rollers themselves, tops flush with the counter line.
        let rolls = (len / pitch).floor() as usize;
        for i in 0..=rolls {
            let t = (i as f64 * pitch + pitch / 2.0) / len;
            if t > 1.0 {
                break;
            }
            self.push(a[0] + dx * t, a[1] + dy * t, bed_top, Z_DECK_TOP, roller_w, deck_w, yaw, K::ConveyorRoller);
        }
    }

    /// One 90-degree corner as chorded boxes following the centreline, from the
    /// arc's 180-degree point to its 270-degree point.
    pub fn curved_belt(&mut self, centre: [f64; 2], radius: f64, deck_w: f64, segs: usize) {
        use std::f64::consts::{FRAC_PI_2, PI};
        let step = FRAC_PI_2 / segs as f64;
        // Chords overlap slightly so the corner reads as one surface rather
        // than a fan of wedges with gaps at the outer radius.
        let chord = 2.0 * radius * (step / 2.0).sin() * 1.04;
        for i in 0..segs {
            let mid = PI + step * (i as f64 + 0.5);
            let (px, py) = (centre[0] + radius * mid.cos(), centre[1] + radius * mid.sin());
            let yaw = mid.to_degrees() + 90.0;
            self.push(px, py, Z_LEG_TOP, Z_DECK_TOP, chord, deck_w, yaw, K::ConveyorDeck);
            for r in [radius + deck_w / 2.0 - GUARD_T / 2.0, radius - deck_w / 2.0 + GUARD_T / 2.0] {
                let (gx, gy) = (centre[0] + r * mid.cos(), centre[1] + r * mid.sin());
                self.push(gx, gy, Z_DECK_TOP, Z_GUARD_TOP, chord, GUARD_T, yaw, K::ConveyorGuard);
            }
            if i % 3 == 0 {
                self.run_leg(px, py, 0.0, Z_LEG_TOP, deck_w - 60.0, yaw);
            }
        }
    }

    /// A ramped deck section of `len`, stepping from `z_far` at its far end to
    /// `z_near` at `y_near`, drawn in `steps` boxes because a box pipeline
    /// cannot slope a face. Each step carries its own leg and guards, so the
    /// ramp reads as belt rather than as a stack of slabs.
    #[allow(clippy::too_many_arguments)]
    pub fn ramp(&mut self, cx: f64, y_near: f64, len: f64, steps: usize, z_near: f64, z_far: f64, deck_w: f64) {
        let seg = len / steps as f64;
        for i in 0..steps {
            // Step 0 is nearest `y_near` and closest to `z_near`.
            let t = (i as f64 + 0.5) / steps as f64;
            let top = z_near + (z_far - z_near) * t;
            let cy = y_near - seg * (i as f64 + 0.5);
            self.push(cx, cy, top - 120.0, top, deck_w, seg, 0.0, K::ConveyorRamp);
            self.run_leg(cx, cy, 0.0, top - 120.0, deck_w - 60.0, 0.0);
            for s in [1.0, -1.0] {
                let off = s * (deck_w / 2.0 - GUARD_T / 2.0);
                self.push(cx + off, cy, top, top + 50.0, GUARD_T, seg, 0.0, K::ConveyorGuard);
            }
        }
    }

    /// A presentation stand: a thin top deck on four corner legs. A solid
    /// pedestal at this size reads as a crate, not a stand.
    pub fn stand(&mut self, cx: f64, cy: f64, sx: f64, sy: f64, yaw_deg: f64) {
        self.push(cx, cy, Z_COUNTER - STAND_T, Z_COUNTER, sx, sy, yaw_deg, K::PresentStand);
        let (ca, sa) = (yaw_deg.to_radians().cos(), yaw_deg.to_radians().sin());
        for dx in [-(sx / 2.0 - STAND_T), sx / 2.0 - STAND_T] {
            for dy in [-(sy / 2.0 - STAND_T), sy / 2.0 - STAND_T] {
                let (lx, ly) = (cx + dx * ca - dy * sa, cy + dx * sa + dy * ca);
                self.push(lx, ly, 0.0, Z_COUNTER - STAND_T, STAND_T, STAND_T, yaw_deg, K::ConveyorLeg);
            }
        }
    }
}

/// Corner-exact XY bounds of a prop's boxes, in render metres. A yawed box
/// reaches beyond its own `min + size`, so a plain min/max over corners is the
/// only honest extent — used by the props' placement tests.
pub fn prop_bounds(boxes: &[RackBox]) -> [f32; 4] {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for b in boxes {
        let (cx, cy) = (b.min[0] + b.size[0] / 2.0, b.min[1] + b.size[1] / 2.0);
        let (c, s) = (b.yaw.cos().abs(), b.yaw.sin().abs());
        let hx = (b.size[0] * c + b.size[1] * s) / 2.0;
        let hy = (b.size[0] * s + b.size[1] * c) / 2.0;
        x0 = x0.min(cx - hx);
        x1 = x1.max(cx + hx);
        y0 = y0.min(cy - hy);
        y1 = y1.max(cy + hy);
    }
    [x0, y0, x1, y1]
}

/// Every box a prop may put above [`Z_COUNTER`]: a carrier standing ON a
/// counter, a person, and a signal a person has to read. Shared so each prop's
/// counter-height test states the same exemptions.
pub fn allowed_above_counter(kind: K) -> bool {
    matches!(kind, K::InventoryTote | K::OrderTote | K::Operator | K::PickToLight)
}
