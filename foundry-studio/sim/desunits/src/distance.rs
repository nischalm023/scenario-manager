//! Within desunits, `distance` is the DISTANCE VOCABULARY: the types every
//! subsystem uses to say *where* something is along one axis and *how far*
//! apart two such places are. Three of them, each the distance twin of a
//! [`simtime`](crate::simtime) type:
//!
//! | Type | Means | Backed by | Time twin |
//! |---|---|---|---|
//! | [`Position`] | an absolute coordinate along one axis | `u64` mm | `SimTime` |
//! | [`Distance`] | an unsigned span between two positions | `u64` mm | `SimDuration` |
//! | [`DistanceDelta`] | a signed displacement (correction, offset, error) | `i64` mm | `SimOffset` |
//!
//! A bare `u64`/`i64`/`f64` whose unit lives in a `_mm` or `_m` suffix is the
//! smell these replace: millimeters passed where meters are read compile
//! silently and answer 1000× wrong, a signed correction stored `as u64`
//! wraps without a diagnostic, and a position difference has no typed home
//! for its sign. Migration of existing raw bindings is a separate effort.
//!
//! Depends on nothing but `std` and `serde`.
//!
//! # Overflow policy
//!
//! Binary `+`, `-`, `*`, and `/` panic on overflow, underflow, inverted
//! subtraction, or division by zero, under `#[track_caller]`. Unlike
//! `SimDuration::MAX`, distances have no "indefinite" sentinel to preserve
//! through saturation. Except for `Position - Position`, these operations
//! have `checked_` methods that return `None` where the operator panics.
//! [`Position::abs_diff`] gives an exact unsigned gap in either order;
//! [`Position::diff`] gives a signed gap, clamped to the `i64` bounds.
//!
//! The signed-range operations follow `simtime`: converting [`Distance`]
//! to [`DistanceDelta`] clamps at `i64::MAX`, and negating a
//! [`DistanceDelta`] of `i64::MIN` saturates to `i64::MAX`.
//!
//! # Deferred, deliberately
//!
//! The following are known extension points. They are documented here so a
//! contributor does not invent an incompatible version before a unified
//! design is agreed. None of them has a stub, placeholder, or empty variant
//! in this module; `deferred_types_are_documentation_only` enforces that
//! mechanically.
//!
//! - **`Speed`**: a `Distance` per `SimDuration`. Needs `f64` backing
//!   because the quotient of two integer quantities is not generally an
//!   integer.
//! - **`Acceleration`**: a `Speed` per `SimDuration`. Same `f64` argument.
//! - **`DistanceRange`**: a half-open interval `[start: Position, end:
//!   Position)`, the twin of `TimeRange`. When introduced it must carry
//!   `TimeRange`'s invariant-enforcement pattern forward: private fields,
//!   fallible construction (`try_new` / `new`), and validated
//!   deserialization through a `#[serde(try_from)]` repr type.
//! - **`Distance::quantize_floor`**: round a distance down to the nearest
//!   multiple of a resolution, the twin of `SimDuration::quantize_floor`.
//!   Useful for cell-grid alignment and slot-pitch snapping, but no caller
//!   needs it yet.

use std::fmt;
use std::ops::{Add, Div, Mul, Neg, Sub};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Scalars
// ---------------------------------------------------------------------------

/// Absolute coordinate along one axis, in millimeters.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Position(u64);

/// Unsigned span between two positions, in millimeters.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Distance(u64);

/// Signed displacement in millimeters: corrections, offsets, errors.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DistanceDelta(i64);

impl Position {
    pub const ZERO: Self = Self(0);
    pub const MAX: Self = Self(u64::MAX);

    /// Construct from millimeters.
    #[inline]
    pub const fn millimeter(mm: u64) -> Self {
        Self(mm)
    }

    /// The raw millimeter count.
    #[inline]
    pub fn as_millimeter(self) -> u64 {
        self.0
    }

    /// `None` past [`Self::MAX`], where `+` panics.
    #[inline]
    pub fn checked_add(self, d: Distance) -> Option<Self> {
        self.0.checked_add(d.0).map(Self)
    }

    /// `None` below [`Self::ZERO`], where `-` panics.
    #[inline]
    pub fn checked_sub(self, d: Distance) -> Option<Self> {
        self.0.checked_sub(d.0).map(Self)
    }

    /// `None` below [`Self::ZERO`] or past [`Self::MAX`], where `+` panics.
    #[inline]
    pub fn checked_offset(self, delta: DistanceDelta) -> Option<Self> {
        if delta.0 >= 0 {
            self.0.checked_add(delta.0 as u64).map(Self)
        } else {
            self.0.checked_sub(delta.0.unsigned_abs()).map(Self)
        }
    }

    /// Unsigned gap in either order. Exact, and never panics.
    #[inline]
    pub fn abs_diff(self, other: Position) -> Distance {
        Distance(self.0.abs_diff(other.0))
    }

    /// Positive when `self` is further along. Clamps at `i64::MAX` /
    /// `i64::MIN`; [`Self::abs_diff`] is exact when the sign is not needed.
    #[inline]
    pub fn diff(self, other: Position) -> DistanceDelta {
        if self.0 >= other.0 {
            DistanceDelta(i64::try_from(self.0 - other.0).unwrap_or(i64::MAX))
        } else {
            DistanceDelta(i64::try_from(other.0 - self.0).map(|v| -v).unwrap_or(i64::MIN))
        }
    }
}

impl Distance {
    pub const ZERO: Self = Self(0);
    pub const MAX: Self = Self(u64::MAX);

    /// Construct from millimeters.
    #[inline]
    pub const fn millimeter(mm: u64) -> Self {
        Self(mm)
    }

    /// The raw millimeter count.
    #[inline]
    pub fn as_millimeter(self) -> u64 {
        self.0
    }

    /// `None` past [`Self::MAX`], where `+` panics.
    #[inline]
    pub fn checked_add(self, rhs: Distance) -> Option<Self> {
        self.0.checked_add(rhs.0).map(Self)
    }

    /// `None` below [`Self::ZERO`], where `-` panics.
    #[inline]
    pub fn checked_sub(self, rhs: Distance) -> Option<Self> {
        self.0.checked_sub(rhs.0).map(Self)
    }

    /// `None` past [`Self::MAX`], where `*` panics.
    #[inline]
    pub fn checked_mul(self, rhs: u64) -> Option<Self> {
        self.0.checked_mul(rhs).map(Self)
    }

    /// `None` for a zero divisor, where `/` panics.
    #[inline]
    pub fn checked_div(self, rhs: u64) -> Option<Self> {
        self.0.checked_div(rhs).map(Self)
    }

    /// Unsigned gap in either order. Never panics.
    #[inline]
    pub fn abs_diff(self, other: Distance) -> Distance {
        Self(self.0.abs_diff(other.0))
    }
}

impl DistanceDelta {
    pub const ZERO: Self = Self(0);

    /// Construct from signed millimeters.
    #[inline]
    pub const fn millimeter(mm: i64) -> Self {
        Self(mm)
    }

    /// The raw signed millimeter count.
    #[inline]
    pub fn as_millimeter(self) -> i64 {
        self.0
    }

    /// Strictly below zero.
    #[inline]
    pub fn is_negative(self) -> bool {
        self.0 < 0
    }

    /// Magnitude, discarding sign. Exact: `i64::MIN` yields 2^63 mm.
    #[inline]
    pub fn magnitude(self) -> Distance {
        Distance(self.0.unsigned_abs())
    }
}

// ---------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------

impl From<Distance> for DistanceDelta {
    /// Saturates at `i64::MAX`.
    #[inline]
    fn from(d: Distance) -> Self {
        Self(i64::try_from(d.0).unwrap_or(i64::MAX))
    }
}

// ---------------------------------------------------------------------------
// Operators. Binary operations panic; signed negation saturates.
// ---------------------------------------------------------------------------

// Out of line so the inlined operators carry only a branch, not the
// formatting. `track_caller` keeps the location at the operator's caller.
#[cold]
#[inline(never)]
#[track_caller]
fn out_of_range(message: fmt::Arguments<'_>) -> ! {
    panic!("{message}")
}

impl Add<Distance> for Position {
    type Output = Position;
    /// Panics past [`Position::MAX`].
    #[inline]
    #[track_caller]
    fn add(self, rhs: Distance) -> Position {
        match self.0.checked_add(rhs.0) {
            Some(v) => Position(v),
            None => out_of_range(format_args!("Position + Distance overflow: {self:?} + {rhs:?}")),
        }
    }
}

impl Sub<Distance> for Position {
    type Output = Position;
    /// Panics below [`Position::ZERO`].
    #[inline]
    #[track_caller]
    fn sub(self, rhs: Distance) -> Position {
        match self.0.checked_sub(rhs.0) {
            Some(v) => Position(v),
            None => out_of_range(format_args!("Position - Distance underflow: {self:?} - {rhs:?}")),
        }
    }
}

impl Sub for Position {
    type Output = Distance;
    /// Panics when `rhs` is further along than `self`.
    #[inline]
    #[track_caller]
    fn sub(self, rhs: Position) -> Distance {
        match self.0.checked_sub(rhs.0) {
            Some(v) => Distance(v),
            None => out_of_range(format_args!("Position - Position: lhs {self:?} < rhs {rhs:?} (inverted)")),
        }
    }
}

impl Add<DistanceDelta> for Position {
    type Output = Position;
    /// Panics below [`Position::ZERO`] or past [`Position::MAX`].
    #[inline]
    #[track_caller]
    fn add(self, rhs: DistanceDelta) -> Position {
        match self.checked_offset(rhs) {
            Some(v) => v,
            None if rhs.0 < 0 => {
                out_of_range(format_args!("Position + DistanceDelta: result below zero: {self:?} + {rhs:?}"))
            }
            None => out_of_range(format_args!("Position + DistanceDelta: result overflow: {self:?} + {rhs:?}")),
        }
    }
}

impl Add for Distance {
    type Output = Distance;
    /// Panics past [`Distance::MAX`].
    #[inline]
    #[track_caller]
    fn add(self, rhs: Distance) -> Distance {
        match self.0.checked_add(rhs.0) {
            Some(v) => Distance(v),
            None => out_of_range(format_args!("Distance + Distance overflow: {self:?} + {rhs:?}")),
        }
    }
}

impl Sub for Distance {
    type Output = Distance;
    /// Panics below [`Distance::ZERO`].
    #[inline]
    #[track_caller]
    fn sub(self, rhs: Distance) -> Distance {
        match self.0.checked_sub(rhs.0) {
            Some(v) => Distance(v),
            None => out_of_range(format_args!("Distance - Distance underflow: {self:?} - {rhs:?}")),
        }
    }
}

impl Mul<u64> for Distance {
    type Output = Distance;
    /// Panics past [`Distance::MAX`].
    #[inline]
    #[track_caller]
    fn mul(self, rhs: u64) -> Distance {
        match self.0.checked_mul(rhs) {
            Some(v) => Distance(v),
            None => out_of_range(format_args!("Distance * u64 overflow: {self:?} * {rhs}")),
        }
    }
}

impl Div<u64> for Distance {
    type Output = Distance;
    /// Integer division toward zero. Panics on a zero divisor.
    #[inline]
    #[track_caller]
    fn div(self, rhs: u64) -> Distance {
        match self.0.checked_div(rhs) {
            Some(v) => Distance(v),
            None => out_of_range(format_args!("Distance / u64: division by zero: {self:?} / 0")),
        }
    }
}

impl Neg for DistanceDelta {
    type Output = DistanceDelta;
    /// `-i64::MIN` has no `i64` representation; saturates to `i64::MAX`.
    #[inline]
    fn neg(self) -> DistanceDelta {
        DistanceDelta(self.0.checked_neg().unwrap_or(i64::MAX))
    }
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_mm_as_meters(self.0, f)
    }
}

impl fmt::Display for Distance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_mm_as_meters(self.0, f)
    }
}

impl fmt::Display for DistanceDelta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 < 0 {
            f.write_str("-")?;
        }
        fmt_mm_as_meters(self.0.unsigned_abs(), f)
    }
}

impl fmt::Debug for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "pos({})", self.0)
    }
}

impl fmt::Debug for Distance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "d({})", self.0)
    }
}

impl fmt::Debug for DistanceDelta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 < 0 {
            return write!(f, "delta(-{})", self.0.unsigned_abs());
        }
        write!(f, "delta({})", self.0)
    }
}

fn fmt_mm_as_meters(mm: u64, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let whole = mm / 1000;
    let frac = mm % 1000;
    if frac == 0 {
        write!(f, "{whole}m")
    } else if frac.is_multiple_of(100) {
        write!(f, "{whole}.{}m", frac / 100)
    } else if frac.is_multiple_of(10) {
        write!(f, "{whole}.{:02}m", frac / 10)
    } else {
        write!(f, "{whole}.{frac:03}m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(mm: u64) -> Position {
        Position::millimeter(mm)
    }

    fn d(mm: u64) -> Distance {
        Distance::millimeter(mm)
    }

    fn delta(mm: i64) -> DistanceDelta {
        DistanceDelta::millimeter(mm)
    }

    const I64_MAX_U: u64 = i64::MAX as u64;

    #[test]
    fn debug_renders_short_form() {
        assert_eq!(format!("{:?}", Position::ZERO), "pos(0)");
        assert_eq!(format!("{:?}", p(1_500)), "pos(1500)");
        assert_eq!(format!("{:?}", Position::MAX), "pos(18446744073709551615)");
        assert_eq!(format!("{:?}", d(5_000)), "d(5000)");
        assert_eq!(format!("{:?}", Distance::MAX), "d(18446744073709551615)");
        assert_eq!(format!("{:?}", delta(0)), "delta(0)");
        assert_eq!(format!("{:?}", delta(2_000)), "delta(2000)");
        assert_eq!(format!("{:?}", delta(-1)), "delta(-1)");
        assert_eq!(format!("{:?}", delta(i64::MAX)), "delta(9223372036854775807)");
        assert_eq!(format!("{:?}", delta(i64::MIN)), "delta(-9223372036854775808)");
    }

    #[test]
    fn display_renders_adaptive_meters() {
        assert_eq!(p(0).to_string(), "0m");
        assert_eq!(p(1_000).to_string(), "1m");
        assert_eq!(p(1).to_string(), "0.001m");
        assert_eq!(p(1_500).to_string(), "1.5m");
        assert_eq!(p(1_380).to_string(), "1.38m");
        assert_eq!(p(999).to_string(), "0.999m");
        assert_eq!(p(5_005).to_string(), "5.005m");
        assert_eq!(Position::MAX.to_string(), "18446744073709551.615m");

        assert_eq!(d(0).to_string(), "0m");
        assert_eq!(d(5_000).to_string(), "5m");
        assert_eq!(d(1).to_string(), "0.001m");
        assert_eq!(d(1_500).to_string(), "1.5m");
        assert_eq!(d(1_380).to_string(), "1.38m");
        assert_eq!(d(999).to_string(), "0.999m");
        assert_eq!(Distance::MAX.to_string(), "18446744073709551.615m");

        assert_eq!(delta(0).to_string(), "0m");
        assert_eq!(delta(1).to_string(), "0.001m");
        assert_eq!(delta(-1_500).to_string(), "-1.5m");
        assert_eq!(delta(-1).to_string(), "-0.001m");
        assert_eq!(delta(i64::MAX).to_string(), "9223372036854775.807m");
        assert_eq!(delta(i64::MIN).to_string(), "-9223372036854775.808m");
    }

    // Wire format is a bare integer so stored maps and event streams survive
    // a field migrating from `u64` to one of these types.

    #[test]
    fn json_wire_format_is_a_bare_integer() {
        assert_eq!(serde_json::to_string(&p(1_000)).unwrap(), "1000");
        assert_eq!(serde_json::to_string(&d(1_000)).unwrap(), "1000");
        assert_eq!(serde_json::to_string(&delta(-500)).unwrap(), "-500");
    }

    #[test]
    fn json_round_trips_representative_values() {
        for v in [0, 1, 1_000, u64::MAX] {
            let json = serde_json::to_string(&p(v)).unwrap();
            assert_eq!(serde_json::from_str::<Position>(&json).unwrap().as_millimeter(), v);
            let json = serde_json::to_string(&d(v)).unwrap();
            assert_eq!(serde_json::from_str::<Distance>(&json).unwrap().as_millimeter(), v);
        }
        for v in [0, 1, -1, 500, -500, i64::MAX, i64::MIN] {
            let json = serde_json::to_string(&delta(v)).unwrap();
            assert_eq!(serde_json::from_str::<DistanceDelta>(&json).unwrap().as_millimeter(), v);
        }
    }

    #[test]
    fn json_rejects_wrapped_objects() {
        assert!(serde_json::from_str::<Position>(r#"{"mm":1000}"#).is_err());
        assert!(serde_json::from_str::<Distance>(r#"{"mm":1000}"#).is_err());
        assert!(serde_json::from_str::<DistanceDelta>(r#"{"mm":-500}"#).is_err());
    }

    #[test]
    fn json_rejects_values_the_backing_integer_cannot_hold() {
        for bad in ["-1", "1000.0", "1.5", "18446744073709551616"] {
            assert!(serde_json::from_str::<Position>(bad).is_err(), "Position accepted {bad}");
            assert!(serde_json::from_str::<Distance>(bad).is_err(), "Distance accepted {bad}");
        }
        for bad in ["1000.0", "9223372036854775808", "-9223372036854775809"] {
            assert!(serde_json::from_str::<DistanceDelta>(bad).is_err(), "DistanceDelta accepted {bad}");
        }
    }

    #[test]
    fn msgpack_wire_format_matches_the_raw_primitive() {
        assert_eq!(rmp_serde::to_vec(&p(1_000)).unwrap(), rmp_serde::to_vec(&1_000u64).unwrap());
        assert_eq!(rmp_serde::to_vec(&d(1_000)).unwrap(), rmp_serde::to_vec(&1_000u64).unwrap());
        assert_eq!(rmp_serde::to_vec(&delta(-500)).unwrap(), rmp_serde::to_vec(&-500i64).unwrap());
    }

    #[test]
    fn msgpack_round_trips_representative_values() {
        for v in [0, 1, 1_000, u64::MAX] {
            let bytes = rmp_serde::to_vec(&p(v)).unwrap();
            assert_eq!(rmp_serde::from_slice::<Position>(&bytes).unwrap().as_millimeter(), v);
            let bytes = rmp_serde::to_vec(&d(v)).unwrap();
            assert_eq!(rmp_serde::from_slice::<Distance>(&bytes).unwrap().as_millimeter(), v);
        }
        for v in [0, 1, -1, 500, -500, i64::MAX, i64::MIN] {
            let bytes = rmp_serde::to_vec(&delta(v)).unwrap();
            assert_eq!(rmp_serde::from_slice::<DistanceDelta>(&bytes).unwrap().as_millimeter(), v);
        }
    }

    #[test]
    #[should_panic(expected = "Position + Distance overflow")]
    fn position_plus_distance_overflow_panics() {
        let _ = Position::MAX + d(1);
    }

    #[test]
    #[should_panic(expected = "Position - Distance underflow")]
    fn position_minus_distance_underflow_panics() {
        let _ = Position::ZERO - d(1);
    }

    #[test]
    #[should_panic(expected = "Position - Position: lhs pos(5) < rhs pos(10) (inverted)")]
    fn position_minus_position_inverted_panics() {
        let _ = p(5) - p(10);
    }

    #[test]
    #[should_panic(expected = "Position + DistanceDelta: result below zero")]
    fn position_plus_negative_delta_below_zero_panics() {
        let _ = p(5) + delta(-6);
    }

    #[test]
    #[should_panic(expected = "Position + DistanceDelta: result overflow")]
    fn position_plus_positive_delta_overflow_panics() {
        let _ = Position::MAX + delta(1);
    }

    #[test]
    #[should_panic(expected = "Distance + Distance overflow")]
    fn distance_plus_distance_overflow_panics() {
        let _ = Distance::MAX + d(1);
    }

    #[test]
    #[should_panic(expected = "Distance - Distance underflow")]
    fn distance_minus_distance_underflow_panics() {
        let _ = d(5) - d(6);
    }

    #[test]
    #[should_panic(expected = "Distance * u64 overflow")]
    fn distance_times_scalar_overflow_panics() {
        let _ = Distance::MAX * 2;
    }

    #[test]
    #[should_panic(expected = "Distance / u64: division by zero")]
    fn distance_div_by_zero_panics() {
        let _ = d(5) / 0;
    }

    #[test]
    fn position_arithmetic_in_range() {
        assert_eq!(p(10) + d(5), p(15));
        assert_eq!(p(10) - d(5), p(5));
        assert_eq!(p(10) - p(4), d(6));
        assert_eq!(p(10) - p(10), d(0));
        assert_eq!(p(10) + delta(5), p(15));
        assert_eq!(p(10) + delta(-5), p(5));
        assert_eq!(p(10) + delta(-10), Position::ZERO);
        assert_eq!(Position::MAX + d(0), Position::MAX);
        assert_eq!(Position::ZERO - d(0), Position::ZERO);
    }

    #[test]
    fn distance_arithmetic_in_range() {
        assert_eq!(d(10) + d(5), d(15));
        assert_eq!(d(10) - d(5), d(5));
        assert_eq!(d(10) - d(10), d(0));
        assert_eq!(d(10) * 3, d(30));
        assert_eq!(d(10) / 3, d(3));
        assert_eq!(d(0) / 3, d(0));
        assert_eq!(Distance::MAX * 1, Distance::MAX);
    }

    #[test]
    fn position_checked_ops_report_boundaries() {
        assert_eq!(Position::MAX.checked_add(d(0)), Some(Position::MAX));
        assert_eq!(Position::MAX.checked_add(d(1)), None);
        assert_eq!(p(u64::MAX - 1).checked_add(d(1)), Some(Position::MAX));
        assert_eq!(Position::ZERO.checked_sub(d(0)), Some(Position::ZERO));
        assert_eq!(Position::ZERO.checked_sub(d(1)), None);
        assert_eq!(p(1).checked_sub(d(1)), Some(Position::ZERO));
    }

    #[test]
    fn distance_checked_ops_report_boundaries() {
        assert_eq!(Distance::MAX.checked_add(d(0)), Some(Distance::MAX));
        assert_eq!(Distance::MAX.checked_add(d(1)), None);
        assert_eq!(Distance::ZERO.checked_sub(d(0)), Some(Distance::ZERO));
        assert_eq!(Distance::ZERO.checked_sub(d(1)), None);
        assert_eq!(Distance::MAX.checked_mul(1), Some(Distance::MAX));
        assert_eq!(Distance::MAX.checked_mul(2), None);
        assert_eq!(d(u64::MAX / 2).checked_mul(2), Some(d(u64::MAX - 1)));
        assert_eq!(d(10).checked_div(3), Some(d(3)));
        assert_eq!(d(10).checked_div(0), None);
    }

    #[test]
    fn checked_offset_reports_both_boundaries() {
        assert_eq!(p(5).checked_offset(delta(-5)), Some(Position::ZERO));
        assert_eq!(p(5).checked_offset(delta(-6)), None);
        assert_eq!(Position::MAX.checked_offset(delta(0)), Some(Position::MAX));
        assert_eq!(Position::MAX.checked_offset(delta(1)), None);
        assert_eq!(p(u64::MAX - 1).checked_offset(delta(1)), Some(Position::MAX));
        assert_eq!(Position::ZERO.checked_offset(delta(i64::MAX)), Some(p(I64_MAX_U)));
    }

    #[test]
    fn checked_offset_and_operator_agree_at_i64_min() {
        assert_eq!(Position::MAX.checked_offset(delta(i64::MIN)), Some(p(u64::MAX - (1u64 << 63))));
        assert_eq!(Position::MAX + delta(i64::MIN), p(u64::MAX - (1u64 << 63)));
        assert_eq!(p((1u64 << 63) - 1).checked_offset(delta(i64::MIN)), None);
    }

    #[test]
    fn abs_diff_is_order_independent() {
        assert_eq!(p(10).abs_diff(p(4)), d(6));
        assert_eq!(p(4).abs_diff(p(10)), d(6));
        assert_eq!(p(7).abs_diff(p(7)), Distance::ZERO);
        assert_eq!(Position::MAX.abs_diff(Position::ZERO), Distance::MAX);
        assert_eq!(d(10).abs_diff(d(4)), d(6));
        assert_eq!(d(4).abs_diff(d(10)), d(6));
        assert_eq!(d(7).abs_diff(d(7)), Distance::ZERO);
        assert_eq!(Distance::MAX.abs_diff(Distance::ZERO), Distance::MAX);
    }

    #[test]
    fn diff_is_signed_and_clamps_at_the_signed_range() {
        assert_eq!(p(10).diff(p(4)), delta(6));
        assert_eq!(p(4).diff(p(10)), delta(-6));
        assert_eq!(p(7).diff(p(7)), DistanceDelta::ZERO);
        assert!(!p(7).diff(p(7)).is_negative());
        assert_eq!(p(I64_MAX_U).diff(Position::ZERO), delta(i64::MAX));
        assert_eq!(Position::MAX.diff(Position::ZERO), delta(i64::MAX));
        assert_eq!(Position::ZERO.diff(p(I64_MAX_U)), delta(-i64::MAX));
        assert_eq!(Position::ZERO.diff(Position::MAX), delta(i64::MIN));
    }

    #[test]
    fn delta_sign_magnitude_and_negation() {
        assert!(delta(-1).is_negative());
        assert!(!DistanceDelta::ZERO.is_negative());
        assert!(!delta(1).is_negative());
        assert_eq!(delta(-500).magnitude(), d(500));
        assert_eq!(delta(500).magnitude(), d(500));
        assert_eq!(DistanceDelta::ZERO.magnitude(), Distance::ZERO);
        assert_eq!(-delta(500), delta(-500));
        assert_eq!(-delta(-500), delta(500));
        assert_eq!(-DistanceDelta::ZERO, DistanceDelta::ZERO);
    }

    #[test]
    fn delta_magnitude_is_exact_and_negation_saturates_at_i64_min() {
        assert_eq!(delta(i64::MIN).magnitude(), d(1u64 << 63));
        assert_eq!(delta(i64::MIN + 1).magnitude(), d(I64_MAX_U));
        assert_eq!(-delta(i64::MIN), delta(i64::MAX));
        assert_eq!(-(-delta(i64::MIN)), delta(-i64::MAX));
    }

    #[test]
    fn distance_converts_to_delta_saturating() {
        assert_eq!(DistanceDelta::from(d(500)), delta(500));
        assert_eq!(DistanceDelta::from(Distance::ZERO), DistanceDelta::ZERO);
        assert_eq!(DistanceDelta::from(d(I64_MAX_U)), delta(i64::MAX));
        assert_eq!(DistanceDelta::from(d(I64_MAX_U + 1)), delta(i64::MAX));
        assert_eq!(DistanceDelta::from(Distance::MAX), delta(i64::MAX));
    }

    #[test]
    fn deferred_types_are_documentation_only() {
        // The names are assembled at runtime so this test's own source does
        // not trip the check it performs.
        let forbidden: Vec<String> =
            [["Sp", "eed"], ["Accel", "eration"], ["Distance", "Range"]].iter().map(|parts| parts.concat()).collect();
        let is_ident = |c: char| c.is_alphanumeric() || c == '_';
        for (idx, line) in include_str!("distance.rs").lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            for token in line.split(|c: char| !is_ident(c)) {
                assert!(
                    !forbidden.iter().any(|f| f == token),
                    "distance.rs:{}: `{token}` is deferred and may appear only in documentation",
                    idx + 1
                );
            }
        }
    }

    mod properties {
        use super::*;
        use proptest::prelude::*;

        fn parse_meters(s: &str) -> u64 {
            let body = s.strip_suffix('m').expect("meters suffix");
            match body.split_once('.') {
                None => body.parse::<u64>().unwrap() * 1000,
                Some((whole, frac)) => {
                    assert!(!frac.is_empty() && frac.len() <= 3, "{s}: bad fraction width");
                    assert!(!frac.ends_with('0'), "{s}: trailing zero");
                    let scale = 10u64.pow(3 - frac.len() as u32);
                    whole.parse::<u64>().unwrap() * 1000 + frac.parse::<u64>().unwrap() * scale
                }
            }
        }

        fn position_and_smaller_distance() -> impl Strategy<Value = (u64, u64)> {
            any::<u64>().prop_flat_map(|p| (Just(p), 0..=p))
        }

        fn distance_and_safe_multiplier() -> impl Strategy<Value = (u64, u64)> {
            (1u64..=u64::MAX).prop_flat_map(|n| (0..=u64::MAX / n, Just(n)))
        }

        fn assert_double_negation(v: i64) {
            let expected = if v == i64::MIN { -i64::MAX } else { v };
            assert_eq!((-(-delta(v))).as_millimeter(), expected);
        }

        fn assert_signed_diff(a: u64, b: u64) {
            let expected = (i128::from(a) - i128::from(b)).clamp(i128::from(i64::MIN), i128::from(i64::MAX));
            assert_eq!(p(a).diff(p(b)).as_millimeter(), expected as i64, "a={a}, b={b}");
        }

        #[test]
        fn double_negation_at_signed_boundaries() {
            for v in [i64::MIN, i64::MIN + 1, -1, 0, 1, i64::MAX - 1, i64::MAX] {
                assert_double_negation(v);
            }
        }

        #[test]
        fn diff_at_signed_boundaries() {
            let positions = [0, 1, I64_MAX_U, I64_MAX_U + 1, I64_MAX_U + 2, u64::MAX];
            for a in positions {
                for b in positions {
                    assert_signed_diff(a, b);
                }
            }
        }

        proptest! {
            // Feature: distance-primitives, Property 1: Position subtraction–addition round-trip
            #[test]
            fn position_sub_then_add_round_trips((p_raw, d_raw) in position_and_smaller_distance()) {
                prop_assert_eq!((p(p_raw) - d(d_raw) + d(d_raw)).as_millimeter(), p_raw);
            }

            // Feature: distance-primitives, Property 2: Position abs_diff commutativity
            #[test]
            fn position_abs_diff_commutes_and_is_reflexive(a in any::<u64>(), b in any::<u64>()) {
                prop_assert_eq!(p(a).abs_diff(p(b)), p(b).abs_diff(p(a)));
                prop_assert_eq!(p(a).abs_diff(p(a)), Distance::ZERO);
                prop_assert_eq!(d(a).abs_diff(d(b)), d(b).abs_diff(d(a)));
                prop_assert_eq!(d(a).abs_diff(d(a)), Distance::ZERO);
            }

            // Feature: distance-primitives, Property 3: Distance subtraction–addition round-trip
            #[test]
            fn distance_sub_then_add_round_trips((d1, d2) in position_and_smaller_distance()) {
                prop_assert_eq!((d(d1) - d(d2) + d(d2)).as_millimeter(), d1);
            }

            // Feature: distance-primitives, Property 4: Distance multiplication–division round-trip
            #[test]
            fn distance_mul_then_div_round_trips((d_raw, n) in distance_and_safe_multiplier()) {
                prop_assert_eq!((d(d_raw) * n / n).as_millimeter(), d_raw);
            }

            // Feature: distance-primitives, Property 5: DistanceDelta double-negation identity
            #[test]
            fn delta_double_negation_is_identity_except_at_i64_min(v in any::<i64>()) {
                assert_double_negation(v);
            }

            // Feature: distance-primitives, Property 6: Distance-to-DistanceDelta conversion in-range
            #[test]
            fn distance_to_delta_is_exact_in_signed_range(v in 0..=I64_MAX_U) {
                prop_assert_eq!(DistanceDelta::from(d(v)).as_millimeter(), v as i64);
            }

            // Feature: distance-primitives, Property 7: serde round-trip
            #[test]
            fn serde_round_trips(u in any::<u64>(), i in any::<i64>()) {
                let json = serde_json::to_string(&p(u)).unwrap();
                prop_assert_eq!(serde_json::from_str::<Position>(&json).unwrap().as_millimeter(), u);
                let json = serde_json::to_string(&d(u)).unwrap();
                prop_assert_eq!(serde_json::from_str::<Distance>(&json).unwrap().as_millimeter(), u);
                let json = serde_json::to_string(&delta(i)).unwrap();
                prop_assert_eq!(serde_json::from_str::<DistanceDelta>(&json).unwrap().as_millimeter(), i);

                let bytes = rmp_serde::to_vec(&p(u)).unwrap();
                prop_assert_eq!(rmp_serde::from_slice::<Position>(&bytes).unwrap().as_millimeter(), u);
                let bytes = rmp_serde::to_vec(&d(u)).unwrap();
                prop_assert_eq!(rmp_serde::from_slice::<Distance>(&bytes).unwrap().as_millimeter(), u);
                let bytes = rmp_serde::to_vec(&delta(i)).unwrap();
                prop_assert_eq!(rmp_serde::from_slice::<DistanceDelta>(&bytes).unwrap().as_millimeter(), i);
            }

            // Feature: distance-primitives, Property 8: checked_add/checked_sub agree with panicking operators
            #[test]
            fn checked_ops_agree_with_raw_integer_arithmetic(a in any::<u64>(), b in any::<u64>()) {
                prop_assert_eq!(p(a).checked_add(d(b)).map(Position::as_millimeter), a.checked_add(b));
                prop_assert_eq!(p(a).checked_sub(d(b)).map(Position::as_millimeter), a.checked_sub(b));
                prop_assert_eq!(d(a).checked_add(d(b)).map(Distance::as_millimeter), a.checked_add(b));
                prop_assert_eq!(d(a).checked_sub(d(b)).map(Distance::as_millimeter), a.checked_sub(b));
                prop_assert_eq!(d(a).checked_mul(b).map(Distance::as_millimeter), a.checked_mul(b));
                prop_assert_eq!(d(a).checked_div(b).map(Distance::as_millimeter), a.checked_div(b));
                if let Some(v) = p(a).checked_add(d(b)) {
                    prop_assert_eq!(p(a) + d(b), v);
                }
                if let Some(v) = p(a).checked_sub(d(b)) {
                    prop_assert_eq!(p(a) - d(b), v);
                    prop_assert_eq!(p(a) - p(b), d(a - b));
                }
                if let Some(v) = d(a).checked_add(d(b)) {
                    prop_assert_eq!(d(a) + d(b), v);
                }
                if let Some(v) = d(a).checked_sub(d(b)) {
                    prop_assert_eq!(d(a) - d(b), v);
                }
                if let Some(v) = d(a).checked_mul(b) {
                    prop_assert_eq!(d(a) * b, v);
                }
                if let Some(v) = d(a).checked_div(b) {
                    prop_assert_eq!(d(a) / b, v);
                }
            }

            // Feature: distance-primitives, Property 8: checked_offset agrees with Position + DistanceDelta
            #[test]
            fn checked_offset_agrees_with_operator(a in any::<u64>(), i in any::<i64>()) {
                let expected = if i >= 0 { a.checked_add(i as u64) } else { a.checked_sub(i.unsigned_abs()) };
                prop_assert_eq!(p(a).checked_offset(delta(i)).map(Position::as_millimeter), expected);
                if let Some(v) = p(a).checked_offset(delta(i)) {
                    prop_assert_eq!(p(a) + delta(i), v);
                }
            }

            // Feature: distance-primitives, Property 9: Position::diff matches the clamped signed gap
            #[test]
            fn diff_matches_clamped_signed_difference(a in any::<u64>(), b in any::<u64>()) {
                assert_signed_diff(a, b);
            }

            // Feature: distance-primitives, Property 10: Display round-trip
            #[test]
            fn display_round_trips_through_a_meters_parser(u in any::<u64>(), i in any::<i64>()) {
                prop_assert_eq!(parse_meters(&d(u).to_string()), u);
                prop_assert_eq!(parse_meters(&p(u).to_string()), u);
                let shown = delta(i).to_string();
                let unsigned = shown.strip_prefix('-').unwrap_or(&shown);
                prop_assert_eq!(shown.starts_with('-'), i < 0);
                prop_assert_eq!(parse_meters(unsigned), i.unsigned_abs());
            }
        }
    }
}
