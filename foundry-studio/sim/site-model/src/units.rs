//! The protocol's scalar quantities, each with its unit fixed in the type.
//!
//! Every length in the format is an integer count of the **steps** of the frame
//! it is stated in, and only the primary frame's `unit_mm` says what a step is
//! worth physically (SPEC §4, §5). This module is where that promise is cashed:
//! a step count cannot be added to a heading, a heading cannot be measured in
//! seconds, and the one rounding rule the spec states — half-up, for a value
//! that is computed rather than authored — lives here so that no other module
//! can improvise a second one. Because a length is an integer, divisibility and
//! pin checks are exact comparisons with nothing to round.
//!
//! The test-only `JsonSchema` derives below are read by one thing, the schema
//! sync test in [`crate::components`]: a component's field census cannot be
//! derived without one on every type its fields are made of.

use serde::{Deserialize, Serialize};
use std::fmt;

/// An integer count of the steps of some frame — every length the format
/// carries, and every length resolved into the primary frame.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default, Hash,
)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct Steps(pub i64);

/// One axis of a position, in the steps of the frame that states it. Only
/// [`crate::frames::FrameGraph`] may turn one into [`Steps`] of the primary
/// frame, because only the frame chain knows what this frame's step is worth.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default, Hash,
)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct Coord(pub i64);

/// Degrees counter-clockwise from `+x`, per SPEC §5. An angle is not a length
/// and takes its unit from no frame, so it stays fractional.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize, Default)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct Degrees(pub f64);

/// Seconds of simulated time. Times are plain seconds and may be fractional
/// (SPEC §5).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct Secs(pub f64);

impl Steps {
    /// SPEC §5's one rounding rule, in the one place that implements it: a
    /// value that lands between steps rounds **half-up**, `floor(v + 0.5)`.
    /// Everything computed rather than authored comes through here — a cell
    /// centre at an odd pitch, a stamped internal's rotated corner, a position
    /// crossing frames — so a half-boundary tie lands the same way in every
    /// implementation.
    pub fn round_half_up(value: f64) -> Steps {
        Steps((value + 0.5).floor() as i64)
    }

    /// How many whole tiles of `pitch` fit in this extent, or `None` when the
    /// extent is not a positive exact multiple. SPEC §5 forbids remainder
    /// strips, and with step counts the test is plain arithmetic.
    pub fn whole_tiles(self, pitch: Steps) -> Option<usize> {
        if self.0 <= 0 || pitch.0 <= 0 || self.0 % pitch.0 != 0 {
            return None;
        }
        Some((self.0 / pitch.0) as usize)
    }

    /// Whether two points lie farther apart than `tolerance`. Decided on
    /// squared distances in whole steps, so no rounding can make a pin that
    /// misses look like one that holds (SPEC §3).
    pub fn beyond(a: [Steps; 2], b: [Steps; 2], tolerance: Steps) -> bool {
        let dx = i128::from(a[0].0 - b[0].0);
        let dy = i128::from(a[1].0 - b[1].0);
        let tolerance = i128::from(tolerance.0);
        dx * dx + dy * dy > tolerance * tolerance
    }

    /// The separation between two points, rounded by the one rule above. This
    /// is what a message reports; [`Steps::beyond`] is what decides.
    pub fn distance(a: [Steps; 2], b: [Steps; 2]) -> Steps {
        let dx = (a[0].0 - b[0].0) as f64;
        let dy = (a[1].0 - b[1].0) as f64;
        Steps::round_half_up(dx.hypot(dy))
    }
}

impl Degrees {
    /// The heading as a count of quarter turns when it is an exact multiple of
    /// 90°, so a right-angle rotation can be applied by permuting and negating
    /// components instead of running coordinates through `sin`/`cos` and
    /// leaving 1e-13 residues in every derived cell centre.
    pub fn quarter_turns(self) -> Option<u8> {
        let d = self.0.rem_euclid(360.0);
        if d == 0.0 {
            Some(0)
        } else if d == 90.0 {
            Some(1)
        } else if d == 180.0 {
            Some(2)
        } else if d == 270.0 {
            Some(3)
        } else {
            None
        }
    }

    /// Rotate a point about the origin by this heading. Right angles are exact;
    /// anything else goes through trigonometry, and the caller rounds what
    /// comes back by SPEC §5's one rule.
    pub fn rotate(self, p: [f64; 2]) -> [f64; 2] {
        match self.quarter_turns() {
            Some(0) => p,
            Some(1) => [-p[1], p[0]],
            Some(2) => [-p[0], -p[1]],
            Some(3) => [p[1], -p[0]],
            _ => {
                let r = self.0.to_radians();
                let (s, c) = r.sin_cos();
                [p[0] * c - p[1] * s, p[0] * s + p[1] * c]
            }
        }
    }

    /// Rotate a point stated in whole steps, rounding the result back onto the
    /// step lattice by SPEC §5's one rule.
    pub fn rotate_steps(self, p: [Steps; 2]) -> [Steps; 2] {
        let r = self.rotate([p[0].0 as f64, p[1].0 as f64]);
        [Steps::round_half_up(r[0]), Steps::round_half_up(r[1])]
    }

    /// Compose two headings, keeping the sum in `[0, 360)` so that a stamped
    /// internal's heading is a canonical number rather than, say, `360`.
    pub fn compose(self, other: Degrees) -> Degrees {
        Degrees((self.0 + other.0).rem_euclid(360.0))
    }
}

macro_rules! display_scalar {
    ($t:ty, $suffix:literal) => {
        impl fmt::Display for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}{}", self.0, $suffix)
            }
        }
    };
}

display_scalar!(Steps, " steps");
display_scalar!(Coord, "");
display_scalar!(Degrees, "°");
display_scalar!(Secs, " s");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_divisibility_admits_only_whole_multiples() {
        assert_eq!(Steps(29700).whole_tiles(Steps(900)), Some(33));
        assert_eq!(Steps(1300).whole_tiles(Steps(650)), Some(2));
        assert_eq!(Steps(1350).whole_tiles(Steps(900)), None);
        assert_eq!(Steps(0).whole_tiles(Steps(900)), None);
        assert_eq!(Steps(-900).whole_tiles(Steps(900)), None);
    }

    /// Half-up means toward `+∞` on a tie, on both sides of zero: that is what
    /// makes the rule one rule rather than two.
    #[test]
    fn the_one_rounding_rule_takes_every_tie_upward() {
        assert_eq!(Steps::round_half_up(5749.5), Steps(5750));
        assert_eq!(Steps::round_half_up(5749.4), Steps(5749));
        assert_eq!(Steps::round_half_up(-0.5), Steps(0));
        assert_eq!(Steps::round_half_up(-1.5), Steps(-1));
        assert_eq!(Steps::round_half_up(-1.6), Steps(-2));
    }

    /// A pin is decided in whole steps, never on a rounded distance: 30-40-50
    /// clears a tolerance of 50 and 31-40 does not, even though both round to
    /// the same reported number.
    #[test]
    fn a_pin_is_decided_before_the_distance_is_rounded() {
        let origin = [Steps(0), Steps(0)];
        assert!(!Steps::beyond(origin, [Steps(30), Steps(40)], Steps(50)));
        assert!(Steps::beyond(origin, [Steps(31), Steps(40)], Steps(50)));
        assert_eq!(Steps::distance(origin, [Steps(31), Steps(40)]), Steps(51));
    }

    #[test]
    fn right_angle_rotation_is_exact() {
        assert_eq!(Degrees(180.0).rotate([340.0, 1750.0]), [-340.0, -1750.0]);
        assert_eq!(Degrees(90.0).rotate([1.0, 0.0]), [0.0, 1.0]);
        assert_eq!(Degrees(-90.0).rotate([1.0, 0.0]), [0.0, -1.0]);
        assert_eq!(Degrees(45.0).quarter_turns(), None);
        assert_eq!(
            Degrees(180.0).rotate_steps([Steps(340), Steps(1750)]),
            [Steps(-340), Steps(-1750)]
        );
    }

    #[test]
    fn headings_compose_into_the_half_open_circle() {
        assert_eq!(Degrees(180.0).compose(Degrees(180.0)), Degrees(0.0));
        assert_eq!(Degrees(270.0).compose(Degrees(180.0)), Degrees(90.0));
    }
}
