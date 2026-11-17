//! Integer frame steps and headings used by Foundry's warehouse generators.
//!
//! Placement preserves exact integer arithmetic for quarter turns and rounds
//! other angles half-up onto the application's step lattice. Overflow is a
//! refusal, never a wrapped coordinate. These application geometry rules do
//! not belong to the generic WPS generator contract.

use serde::{Deserialize, Serialize};
use std::fmt;

/// An integer count of the steps of some frame — every length the format
/// carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default, Hash)]
#[serde(transparent)]
pub struct Steps(pub i64);

impl Steps {
    /// How many whole tiles of `pitch` fit in this extent, or `None` when the
    /// extent is not a positive exact multiple. SPEC §5 forbids a remainder
    /// strip along the far edge, and with step counts the test is plain
    /// arithmetic with nothing to round.
    ///
    /// `u64` and not `usize`, because a generator is compiled for
    /// `wasm32-wasip2` where `usize` is 32 bits: a quotient above four billion
    /// would have been truncated on the way out, and a region wider than that
    /// many tiles would have derived no cells at all rather than the cells the
    /// 64-bit reference reader derives.
    pub fn whole_tiles(self, pitch: Steps) -> Option<u64> {
        if self.0 <= 0 || pitch.0 <= 0 || self.0 % pitch.0 != 0 {
            return None;
        }
        Some((self.0 / pitch.0) as u64)
    }

    /// SPEC §5's one rounding rule, applied to a ratio: `numerator/denominator`
    /// rounded **half-up**, which is `floor(v + 1/2)` and therefore toward `+∞`
    /// on a tie, on both sides of zero.
    ///
    /// A ratio rather than a value because that is the shape every computed
    /// length in this crate has — half a pitch, a third of a span — and because
    /// a ratio of integers rounds exactly where the same quotient as a float
    /// rounds however the division left it.
    pub fn round_half_up(numerator: i64, denominator: i64) -> Option<Steps> {
        if denominator <= 0 {
            return None;
        }
        // floor((n/d) + 1/2) = floor((2n + d) / 2d), with a floor division
        // rather than Rust's truncation, which would round a negative tie the
        // wrong way.
        let doubled = numerator.checked_mul(2)?.checked_add(denominator)?;
        let divisor = denominator.checked_mul(2)?;
        Some(Steps(doubled.div_euclid(divisor)))
    }

    pub fn checked_add(self, other: Steps) -> Option<Steps> {
        self.0.checked_add(other.0).map(Steps)
    }

    pub fn checked_sub(self, other: Steps) -> Option<Steps> {
        self.0.checked_sub(other.0).map(Steps)
    }

    pub fn checked_mul(self, factor: i64) -> Option<Steps> {
        self.0.checked_mul(factor).map(Steps)
    }

    pub fn checked_neg(self) -> Option<Steps> {
        self.0.checked_neg().map(Steps)
    }

    /// The same half-up rule applied to a value that already went through
    /// trigonometry. Private, and the only float this type is ever built from:
    /// a length reaches a float in exactly one situation, a rotation that is not
    /// a quarter turn, and [`Degrees::place`] is the only caller.
    ///
    /// `None` for a value no step count can hold, rather than the saturation a
    /// bare cast would leave standing as a coordinate.
    fn of_rounded(value: f64) -> Option<Steps> {
        let landed = (value + 0.5).floor();
        // `i64::MAX as f64` is 2^63, one past the largest step count, so the
        // upper test is strict: at exactly 2^63 an `as` cast saturates and
        // hands back a coordinate the value never named. NaN fails both
        // comparisons and each infinity fails one, so finiteness needs no test.
        ((i64::MIN as f64) <= landed && landed < (i64::MAX as f64)).then_some(Steps(landed as i64))
    }
}

/// Degrees counter-clockwise from `+x` (SPEC §5).
///
/// An angle is not a length: it takes its unit from no frame and the format
/// lets it be fractional, so this is the one quantity here that holds a float.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct Degrees(pub f64);

impl Degrees {
    /// This heading as a count of quarter turns, when it is an exact multiple of
    /// 90°.
    ///
    /// A right angle permutes and negates coordinates, which is exact on the
    /// step lattice; every other angle has to go through `sin`/`cos` and lands
    /// between steps. A caller that may only act on geometry v0 can write down —
    /// an axis-aligned rectangle, an axis-aligned pitch — asks this first and
    /// refuses what comes back empty.
    pub fn quarter_turns(self) -> Option<u8> {
        let turned = self.0.rem_euclid(360.0);
        [0.0, 90.0, 180.0, 270.0].iter().position(|exact| *exact == turned).map(|i| i as u8)
    }

    /// Two headings applied one after the other, kept in `[0, 360)` so that a
    /// composed heading is a canonical number rather than, say, `360`.
    pub fn compose(self, other: Degrees) -> Degrees {
        Degrees((self.0 + other.0).rem_euclid(360.0))
    }

    /// A point stated in the axes of something placed at `anchor` with this
    /// heading, carried out into the frame that placement is stated in: rotated
    /// about the origin, offset by the anchor, landed on the step lattice.
    ///
    /// The rotation and the offset are one call because that is the expression
    /// the reference evaluates: it adds the anchor to the unrounded rotation and
    /// rounds the sum. An anchor is a whole count of steps, and rounding half-up
    /// commutes with adding a whole number, so the other ordering would land on
    /// the same coordinate — this is faithfulness to the reader's expression
    /// rather than a difference a document could show, and it is what lets the
    /// two be read side by side.
    ///
    /// At a quarter turn nothing is rounded at all — the answer is a
    /// permutation, a sign and an addition, in whole steps. `None` says the
    /// answer does not fit in a step count.
    pub fn place(self, point: [Steps; 2], anchor: [Steps; 2]) -> Option<[Steps; 2]> {
        let [x, y] = point;
        let turned = match self.quarter_turns() {
            Some(0) => [x, y],
            Some(1) => [y.checked_neg()?, x],
            Some(2) => [x.checked_neg()?, y.checked_neg()?],
            Some(3) => [y, x.checked_neg()?],
            _ => {
                let (sin, cos) = self.0.to_radians().sin_cos();
                let (x, y) = (x.0 as f64, y.0 as f64);
                return Some([
                    Steps::of_rounded(anchor[0].0 as f64 + (x * cos - y * sin))?,
                    Steps::of_rounded(anchor[1].0 as f64 + (x * sin + y * cos))?,
                ]);
            }
        };
        Some([anchor[0].checked_add(turned[0])?, anchor[1].checked_add(turned[1])?])
    }
}

impl fmt::Display for Degrees {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}°", self.0)
    }
}

/// The centre of tile `index` along an axis that starts at `origin` and repeats
/// every `pitch`: `origin + (index + 1/2) · pitch`, rounded by the rule above.
///
/// `None` says the tile's centre does not fit in a step count, which is a
/// document stating a region no floor has.
pub fn centre_of(origin: Steps, index: u64, pitch: Steps) -> Option<Steps> {
    let index = i64::try_from(index).ok()?;
    let offset = Steps::round_half_up(pitch.checked_mul(index)?.0.checked_mul(2)?.checked_add(pitch.0)?, 2)?;
    origin.checked_add(offset)
}

impl fmt::Display for Steps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} steps", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::{centre_of, Degrees, Steps};

    /// A right angle permutes and negates, and that is the whole of it: no
    /// trigonometry runs, so no residue reaches a coordinate. The values are the
    /// reference reader's own cases.
    #[test]
    fn a_quarter_turn_never_leaves_the_step_lattice() {
        let origin = [Steps(0), Steps(0)];
        assert_eq!(Degrees(0.0).place([Steps(340), Steps(1750)], origin), Some([Steps(340), Steps(1750)]));
        assert_eq!(Degrees(90.0).place([Steps(1), Steps(0)], origin), Some([Steps(0), Steps(1)]));
        assert_eq!(Degrees(-90.0).place([Steps(1), Steps(0)], origin), Some([Steps(0), Steps(-1)]));
        assert_eq!(Degrees(180.0).place([Steps(340), Steps(1750)], origin), Some([Steps(-340), Steps(-1750)]));
        assert_eq!(Degrees(360.0).quarter_turns(), Some(0));
        assert_eq!(Degrees(45.0).quarter_turns(), None);
    }

    /// The trigonometric branch, against coordinates worked out by hand rather
    /// than by evaluating the implementation's own expression a second time.
    ///
    /// cos 30° is 0.8660254 and sin 30° is 0.5; cos 45° and sin 45° are both
    /// 0.7071068. Each answer below is the anchor plus the rotated offset,
    /// rounded half-up, and the anchor is off the origin so that both terms are
    /// in play. The radius-1 case is the one where the fraction decides the
    /// answer by itself, the offset being less than a step on either axis.
    #[test]
    fn a_heading_that_is_no_quarter_turn_lands_where_the_arithmetic_says() {
        let anchor = [Steps(6200), Steps(300)];
        // 6200 + 866.0254 = 7066.0254 → 7066; 300 + 500 = 800.
        assert_eq!(Degrees(30.0).place([Steps(1000), Steps(0)], anchor), Some([Steps(7066), Steps(800)]));
        // 6200 + (866.0254 − 250) = 6816.0254; 300 + (500 + 433.0127) = 1233.0127.
        assert_eq!(Degrees(30.0).place([Steps(1000), Steps(500)], anchor), Some([Steps(6816), Steps(1233)]));
        // 6200 + 707.1068 = 6907.1068; 300 + 707.1068 = 1007.1068.
        assert_eq!(Degrees(45.0).place([Steps(1000), Steps(0)], anchor), Some([Steps(6907), Steps(1007)]));
        // 6200.7071 → 6201 and 300.7071 → 301: the rounding is the whole answer.
        assert_eq!(Degrees(45.0).place([Steps(1), Steps(0)], anchor), Some([Steps(6201), Steps(301)]));
    }

    /// A placement whose answer no step count can hold is no answer — including
    /// the one value a range test written with `<=` lets through, since
    /// `i64::MAX as f64` rounds up to 2^63 and a bare cast saturates there.
    #[test]
    fn a_placement_off_the_lattice_gets_no_answer() {
        assert_eq!(Degrees(0.0).place([Steps(1), Steps(0)], [Steps(i64::MAX), Steps(0)]), None);
        assert_eq!(Degrees(45.0).place([Steps(i64::MAX), Steps(0)], [Steps(i64::MAX), Steps(0)]), None);

        assert_eq!(Steps::of_rounded(9_223_372_036_854_775_808.0), None, "2^63, one past the largest step count");
        assert_eq!(
            Steps::of_rounded(9_223_372_036_854_774_784.0),
            Some(Steps(9_223_372_036_854_774_784)),
            "the largest float below it is a step count, and comes back as that one"
        );
        assert_eq!(Steps::of_rounded(-9_223_372_036_854_775_808.0), Some(Steps(i64::MIN)), "−2^63 is exactly i64::MIN");
        assert_eq!(Steps::of_rounded(-9_223_372_036_854_777_856.0), None, "and the next float below it is not");
        assert_eq!((Steps::of_rounded(f64::NAN), Steps::of_rounded(f64::INFINITY)), (None, None));
    }

    /// Composition stays in the half-open circle, so that a stamped internal's
    /// heading is one number rather than two spellings of the same angle.
    #[test]
    fn headings_compose_into_the_half_open_circle() {
        assert_eq!(Degrees(180.0).compose(Degrees(180.0)), Degrees(0.0));
        assert_eq!(Degrees(270.0).compose(Degrees(180.0)), Degrees(90.0));
        assert_eq!(Degrees(90.0).compose(Degrees(0.0)), Degrees(90.0));
        assert_eq!(Degrees(45.0).to_string(), "45°");
    }

    #[test]
    fn exact_divisibility_admits_only_whole_multiples() {
        assert_eq!(Steps(29700).whole_tiles(Steps(900)), Some(33));
        assert_eq!(Steps(1300).whole_tiles(Steps(650)), Some(2));
        assert_eq!(Steps(1350).whole_tiles(Steps(900)), None);
        assert_eq!(Steps(0).whole_tiles(Steps(900)), None);
        assert_eq!(Steps(-900).whole_tiles(Steps(900)), None);
    }

    /// Half-up means toward `+∞` on a tie, on both sides of zero: that is what
    /// makes the rule one rule rather than two. The reference reader's own
    /// cases, stated as the ratios they are.
    #[test]
    fn the_one_rounding_rule_takes_every_tie_upward() {
        assert_eq!(Steps::round_half_up(11499, 2), Some(Steps(5750)));
        assert_eq!(Steps::round_half_up(57494, 10), Some(Steps(5749)));
        assert_eq!(Steps::round_half_up(-1, 2), Some(Steps(0)));
        assert_eq!(Steps::round_half_up(-3, 2), Some(Steps(-1)));
        assert_eq!(Steps::round_half_up(-16, 10), Some(Steps(-2)));
        assert_eq!(Steps::round_half_up(1, 0), None);
    }

    /// An even pitch puts a centre half a pitch along and an odd one lands on a
    /// tie every time, which is the case the rounding rule is for.
    #[test]
    fn a_centre_sits_half_a_pitch_into_its_tile() {
        assert_eq!(centre_of(Steps(0), 0, Steps(900)), Some(Steps(450)));
        assert_eq!(centre_of(Steps(0), 1, Steps(900)), Some(Steps(1350)));
        assert_eq!(centre_of(Steps(6200), 0, Steps(900)), Some(Steps(6650)));
        assert_eq!(centre_of(Steps(0), 0, Steps(901)), Some(Steps(451)));
        assert_eq!(centre_of(Steps(0), 2, Steps(901)), Some(Steps(2253)));
        assert_eq!(centre_of(Steps(-1800), 0, Steps(900)), Some(Steps(-1350)));
    }

    /// Whatever a document says, the answer is a step count or nothing.
    #[test]
    fn a_region_no_floor_has_gets_no_answer() {
        assert_eq!(centre_of(Steps(0), u64::MAX, Steps(900)), None);
        assert_eq!(centre_of(Steps(i64::MAX), 1, Steps(900)), None);
    }
}
