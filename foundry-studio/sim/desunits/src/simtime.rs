//! Within dessim, `simtime` is the SIMULATION CLOCK VOCABULARY: the types
//! every subsystem uses to say *when* something happens and *how long* it
//! takes. Four of them:
//!
//! | Type | Means | Backed by |
//! |---|---|---|
//! | [`SimTime`] | an absolute instant on the sim clock | `u64` ms |
//! | [`SimDuration`] | an unsigned elapsed span | `u64` ms |
//! | [`SimOffset`] | a signed displacement (bias, lead/lag, quote error) | `i64` ms |
//! | [`TimeRange`] | a half-open interval `[start, end)`, optionally open-ended | two `SimTime`s |
//!
//! Per coding standards §11 these are the only legal time types in
//! signatures, fields, and locals. A bare `u64 ms` or `f64 secs` is a
//! violation, and the `_ms` / `_secs` suffix is the smell that flags one.
//!
//! The module owns no domain concepts — no cells, bots, or reservations —
//! and depends on nothing else in the crate, so any module may use it
//! without creating a cycle.
//!
//! # Arithmetic saturates, and that has a sharp edge
//!
//! Every operator on [`SimTime`] and [`SimDuration`] saturates rather than
//! wrapping or panicking. This is forced by [`SimDuration::MAX`] doubling as
//! an "indefinite" sentinel (parking tails, open-ended holds): saturation
//! keeps `sentinel + anything` at the sentinel.
//!
//! The edge is subtraction. `a - b` with `b > a` cannot go negative, so it
//! yields [`SimDuration::ZERO`] — which reads identically to "these happened
//! at the same instant". In a simulator whose central question is "did this
//! arrive before that", that silently converts an ordering bug into a
//! plausible zero. Prefer the operations that can express the difference:
//!
//! | Instead of | Prefer | Because |
//! |---|---|---|
//! | `a - b` (SimTime) | [`SimTime::checked_elapsed_since`] | `None` rather than a silent zero when `b > a` |
//! | `a - b` (SimTime) | [`SimTime::diff`] | signed [`SimOffset`] when the sign carries meaning |
//! | a magnitude, either order | [`SimTime::abs_diff`] / [`SimDuration::abs_diff`] | exact over the whole `u64` range, order-independent |
//! | `t - d` | [`SimTime::checked_sub`] | `None` rather than clamping to [`SimTime::ZERO`] |
//! | `d1 - d2` | [`SimDuration::checked_sub`] | same |
//! | raw `i64` ms | [`SimOffset`] | a signed bias is still a time quantity |
//!
//! # `EOT` is exceedable
//!
//! [`SimTime::EOT`] is `u64::MAX - 1` while [`SimDuration::MAX`] is
//! `u64::MAX`, and addition saturates at `u64::MAX`. So
//! `SimTime::ZERO + SimDuration::MAX` is a `SimTime` strictly greater than
//! the documented "last valid simulation time", and no comparison against
//! `EOT` catches it. Do not treat `<= EOT` as a total bound.
//!
//! Clamping addition to `EOT` would change the value stored by every
//! reachable saturating path, parking tails included, so it is a behavioral
//! change rather than a tidy-up. `eot_is_exceedable_today` pins the current
//! answer so that change has to be made deliberately.

use std::fmt;
use std::ops::{Add, AddAssign, Div, Mul, Neg, Rem, Sub};
use std::time::Duration;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Scalars
// ---------------------------------------------------------------------------

/// Absolute simulation time in milliseconds.
///
/// Field is private — construct via `SimTime::ms()`, `SimTime::seconds()`,
/// `SimTime::ZERO`, or `SimTime::EOT`.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SimTime(u64);
// serialize as u64 milliseconds with serde, flattened

/// Duration in milliseconds.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SimDuration(u64);

/// Signed displacement in milliseconds.
///
/// The typed home for quantities that are genuinely allowed to be negative:
/// scheduling biases, quote-vs-actual deltas, lead/lag. Without it such values
/// have nowhere to live but a raw `i64`, which §11 forbids.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SimOffset(i64);

impl SimTime {
    pub const ZERO: Self = Self(0);
    /// Last valid simulation time. Used for parking ("until end of simulation").
    /// Not a sentinel for "open-ended"; use `Option<SimTime>` for that.
    pub const EOT: Self = Self(u64::MAX - 1);

    /// Construct from milliseconds.
    pub const fn ms(ms: u64) -> Self {
        Self(ms)
    }

    /// Construct from seconds.
    ///
    /// `const`, so second-valued constants need not be written as `f64`. The
    /// multiply saturates: a bare `s * 1000` would be the one unguarded
    /// arithmetic in an otherwise panic-free type, and it wraps silently in
    /// release, the mandated build mode.
    pub const fn seconds(s: u64) -> Self {
        Self(s.saturating_mul(1000))
    }

    pub fn saturating_add(self, dur: SimDuration) -> Self {
        Self(self.0.saturating_add(dur.0))
    }

    pub fn saturating_sub(self, dur: SimDuration) -> Self {
        Self(self.0.saturating_sub(dur.0))
    }

    pub fn as_ms(self) -> u64 {
        self.0
    }

    /// `None` on overflow, where [`Self::saturating_add`] clamps.
    pub fn checked_add(self, dur: SimDuration) -> Option<Self> {
        self.0.checked_add(dur.0).map(Self)
    }

    /// `None` when `dur` reaches back past [`Self::ZERO`], where
    /// [`Self::saturating_sub`] clamps to zero.
    pub fn checked_sub(self, dur: SimDuration) -> Option<Self> {
        self.0.checked_sub(dur.0).map(Self)
    }

    /// Elapsed time since `earlier`, or `None` if `earlier` is actually
    /// later than `self`.
    ///
    /// The honest form of `self - earlier`: the `Sub` impl silently yields
    /// [`SimDuration::ZERO`] for an inverted pair, which reads identically
    /// to "these happened at the same instant."
    pub fn checked_elapsed_since(self, earlier: SimTime) -> Option<SimDuration> {
        self.0.checked_sub(earlier.0).map(SimDuration)
    }

    /// Unsigned distance between two instants, whichever is later.
    ///
    /// Exact over the whole `u64` range, where [`Self::diff`] clamps at
    /// `i64::MAX` to fit a sign. Prefer this whenever only the magnitude is
    /// wanted — for ranking keys and error magnitudes it is the direct
    /// replacement for `if a >= b { a - b } else { b - a }`, and unlike bare
    /// `Sub` it cannot saturate the answer to zero.
    pub fn abs_diff(self, other: SimTime) -> SimDuration {
        SimDuration(self.0.abs_diff(other.0))
    }

    /// Signed displacement from `other` to `self`. Positive when `self` is
    /// later. Safe at `EOT` (the difference is computed unsigned, then
    /// signed), but **clamps at `i64::MAX` / `i64::MIN`** where the true gap
    /// exceeds the signed range. When the sign is not needed, use
    /// [`Self::abs_diff`], which is exact.
    pub fn diff(self, other: SimTime) -> SimOffset {
        if self.0 >= other.0 {
            SimOffset(i64::try_from(self.0 - other.0).unwrap_or(i64::MAX))
        } else {
            SimOffset(i64::try_from(other.0 - self.0).map(|v| -v).unwrap_or(i64::MIN))
        }
    }
}

impl SimDuration {
    pub const ZERO: Self = Self(0);
    pub const MAX: Self = Self(u64::MAX);

    pub const fn ms(ms: u64) -> Self {
        Self(ms)
    }

    /// Saturating. See [`SimTime::seconds`] for why.
    pub const fn seconds(s: u64) -> Self {
        Self(s.saturating_mul(1000))
    }

    /// Construct from fractional seconds, ceilinged to whole ms so a real
    /// sub-ms phase never rounds to zero. Negative inputs clamp to zero.
    pub fn from_secs(secs: f64) -> Self {
        Self((secs * 1000.0).ceil().max(0.0) as u64)
    }

    pub fn as_ms(self) -> u64 {
        self.0
    }

    pub fn saturating_sub(self, rhs: SimDuration) -> Self {
        Self(self.0.saturating_sub(rhs.0))
    }

    pub fn saturating_add(self, rhs: SimDuration) -> Self {
        Self(self.0.saturating_add(rhs.0))
    }

    /// `None` on underflow, where [`Self::saturating_sub`] clamps to zero.
    pub fn checked_sub(self, rhs: SimDuration) -> Option<Self> {
        self.0.checked_sub(rhs.0).map(Self)
    }

    /// `None` on overflow, where [`Self::saturating_add`] clamps.
    pub fn checked_add(self, rhs: SimDuration) -> Option<Self> {
        self.0.checked_add(rhs.0).map(Self)
    }

    /// `None` on overflow, where [`Mul`] clamps.
    pub fn checked_mul(self, rhs: u64) -> Option<Self> {
        self.0.checked_mul(rhs).map(Self)
    }

    /// `None` when `rhs` is [`Self::ZERO`], where [`Div`] panics.
    pub fn checked_div(self, rhs: SimDuration) -> Option<u64> {
        self.0.checked_div(rhs.0)
    }

    /// Unsigned distance between two durations, whichever is larger.
    ///
    /// The unsigned sibling of [`SimTime::diff`], for the `if a >= b { a - b }
    /// else { b - a }` idiom. Order-independent, so unlike bare `Sub` it
    /// cannot silently saturate the answer to zero.
    pub fn abs_diff(self, other: SimDuration) -> SimDuration {
        Self(self.0.abs_diff(other.0))
    }

    /// Round down to the nearest multiple of `resolution`.
    ///
    /// # Panics
    /// If `resolution` is [`Self::ZERO`].
    #[track_caller]
    pub fn quantize_floor(self, resolution: SimDuration) -> Self {
        assert!(resolution.0 != 0, "quantize_floor resolution must be non-zero");
        Self((self.0 / resolution.0) * resolution.0)
    }
}

impl SimOffset {
    pub const ZERO: Self = Self(0);

    pub const fn ms(ms: i64) -> Self {
        Self(ms)
    }

    pub const fn seconds(s: i64) -> Self {
        Self(s.saturating_mul(1000))
    }

    pub fn as_ms(self) -> i64 {
        self.0
    }

    /// Construct from fractional seconds, rounded to the nearest millisecond.
    ///
    /// Rounding to nearest is the only rule that is right in both directions.
    /// `4.001 * 1000.0` is `4001.0000000000005` in binary floating point, so a
    /// `ceil` resolves a whole-millisecond value to 4002; an `as i64`
    /// truncation resolves `1.001` to 1000. Both are off by a millisecond on
    /// ordinary operator input, and this value feeds a selection comparator
    /// where one millisecond decides ties.
    ///
    /// A nonzero input never resolves to [`Self::ZERO`]: a displacement under
    /// half a millisecond becomes 1 ms and keeps its sign, because turning a
    /// configured bias into "no bias" would be indistinguishable from a
    /// deliberate zero.
    ///
    /// Non-finite input yields [`Self::ZERO`]. Callers taking operator input
    /// should reject `nan`/`inf` themselves rather than lean on that, since a
    /// neutral bias is not a meaningful reading of a malformed number.
    ///
    /// This deliberately does **not** share [`SimDuration::from_secs`]'s rule,
    /// which ceils so a sub-millisecond *phase* never vanishes. Same intent
    /// about not vanishing, different rounding, because a duration has no sign
    /// to lose and is not compared at millisecond granularity here.
    pub fn from_secs(secs: f64) -> Self {
        if !secs.is_finite() {
            return Self::ZERO;
        }
        // Nearest, so representation noise cannot inflate a whole-ms value.
        let rounded = (secs * 1000.0).round().abs();
        // ...but nearest sends sub-half-ms to zero, so floor a nonzero input at 1.
        let magnitude = if secs == 0.0 { 0.0 } else { rounded.max(1.0) };
        // Saturate rather than wrap on absurd inputs; -i64::MAX is safe to negate.
        let clamped = magnitude.min(i64::MAX as f64) as i64;
        if secs.is_sign_negative() { Self(-clamped) } else { Self(clamped) }
    }

    pub fn is_negative(self) -> bool {
        self.0 < 0
    }

    /// Magnitude, discarding sign.
    pub fn magnitude(self) -> SimDuration {
        SimDuration(self.0.unsigned_abs())
    }
}

// ---------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------

impl From<SimDuration> for SimOffset {
    /// Saturates at [`i64::MAX`] for durations beyond signed range
    /// ([`SimDuration::MAX`] among them).
    fn from(d: SimDuration) -> Self {
        Self(i64::try_from(d.0).unwrap_or(i64::MAX))
    }
}

impl From<Duration> for SimDuration {
    /// A [`Duration`] is a length, so it converts to a length. Truncates to
    /// whole milliseconds and saturates.
    ///
    /// There is deliberately no `From<Duration> for SimTime`. Turning a length
    /// into an instant is `epoch + length`, not a change of units, and hiding
    /// the epoch inside a conversion is how a configured "run for 10 hours"
    /// silently becomes "stop at t=10h" with the `SimTime::ZERO` assumption
    /// unstated. Write that addition out at the call site.
    fn from(d: Duration) -> Self {
        Self(d.as_millis().min(u64::MAX as u128) as u64)
    }
}

// ---------------------------------------------------------------------------
// Operators. Saturating throughout; see the module doc for why.
// ---------------------------------------------------------------------------

impl Add<SimDuration> for SimTime {
    type Output = SimTime;
    fn add(self, rhs: SimDuration) -> SimTime {
        // Use saturating math because SimDuration::MAX is used as an
        // "indefinite" sentinel (for example, parking tails).
        SimTime(self.0.saturating_add(rhs.0))
    }
}

impl Sub<SimDuration> for SimTime {
    type Output = SimTime;
    fn sub(self, rhs: SimDuration) -> SimTime {
        SimTime(self.0.saturating_sub(rhs.0))
    }
}

impl Sub for SimTime {
    type Output = SimDuration;
    fn sub(self, rhs: SimTime) -> SimDuration {
        SimDuration(self.0.saturating_sub(rhs.0))
    }
}

impl Add<SimOffset> for SimTime {
    type Output = SimTime;
    /// Saturating in both directions: a negative offset clamps at
    /// [`SimTime::ZERO`], a positive one at `u64::MAX`.
    fn add(self, rhs: SimOffset) -> SimTime {
        if rhs.0 >= 0 {
            SimTime(self.0.saturating_add(rhs.0 as u64))
        } else {
            SimTime(self.0.saturating_sub(rhs.0.unsigned_abs()))
        }
    }
}

impl Add for SimDuration {
    type Output = SimDuration;
    fn add(self, rhs: SimDuration) -> SimDuration {
        // Use saturating math because SimDuration::MAX is used as an
        // "indefinite" sentinel (for example, parking tails).
        SimDuration(self.0.saturating_add(rhs.0))
    }
}

impl Sub for SimDuration {
    type Output = SimDuration;
    fn sub(self, rhs: SimDuration) -> SimDuration {
        SimDuration(self.0.saturating_sub(rhs.0))
    }
}

impl AddAssign for SimDuration {
    fn add_assign(&mut self, rhs: SimDuration) {
        self.0 = self.0.saturating_add(rhs.0);
    }
}

impl Mul<u64> for SimDuration {
    type Output = SimDuration;
    fn mul(self, rhs: u64) -> SimDuration {
        SimDuration(self.0.saturating_mul(rhs))
    }
}

impl Div<SimDuration> for SimDuration {
    type Output = u64;
    /// # Panics
    /// If `rhs` is [`SimDuration::ZERO`]. Use [`SimDuration::checked_div`]
    /// when the divisor is caller-supplied.
    #[track_caller]
    fn div(self, rhs: SimDuration) -> u64 {
        assert!(rhs.0 != 0, "SimDuration divided by ZERO");
        self.0 / rhs.0
    }
}

impl Rem<SimDuration> for SimDuration {
    type Output = SimDuration;
    /// # Panics
    /// If `rhs` is [`SimDuration::ZERO`].
    #[track_caller]
    fn rem(self, rhs: SimDuration) -> SimDuration {
        assert!(rhs.0 != 0, "SimDuration remainder by ZERO");
        SimDuration(self.0 % rhs.0)
    }
}

impl Add for SimOffset {
    type Output = SimOffset;
    fn add(self, rhs: SimOffset) -> SimOffset {
        SimOffset(self.0.saturating_add(rhs.0))
    }
}

impl Sub for SimOffset {
    type Output = SimOffset;
    fn sub(self, rhs: SimOffset) -> SimOffset {
        SimOffset(self.0.saturating_sub(rhs.0))
    }
}

impl Neg for SimOffset {
    type Output = SimOffset;
    fn neg(self) -> SimOffset {
        SimOffset(self.0.saturating_neg())
    }
}

// ---------------------------------------------------------------------------
// Interval
// ---------------------------------------------------------------------------

/// Construction rejected because `end` precedes `start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("TimeRange end {end:?} precedes start {start:?}")]
pub struct TimeRangeError {
    pub start: SimTime,
    pub end: SimTime,
}

/// Half-open range `[start, end)` in absolute simulation time.
///
/// `end`: `Some(t)` = closed, ending at `t`. `None` = open-ended, meaning
/// the end is not yet determined (a successor task has not finalized it).
/// An open-ended range extends past everything after `start`.
///
/// A range with `end == Some(start)` is **empty**: it contains no instant
/// and overlaps nothing.
///
/// # Invariant
///
/// `start <= end`, enforced at construction and unbypassable afterwards: the
/// fields are private, [`Self::try_new`] is fallible, [`Self::new`] panics,
/// and deserialization validates rather than trusts.
///
/// That strictness is load-bearing. An inverted range would report three
/// mutually contradictory things and no method would detect the state:
/// `is_empty()` false (it claims to be a real range), `duration()` saturated
/// to [`SimDuration::ZERO`] (indistinguishable from a genuinely zero-width
/// range), and `contains()` false for every instant. [`Self::overlaps`] is
/// worse — it compares the widened `end` against the other range's `start`,
/// so with `end` below `start` it answers disjointness from the wrong
/// endpoint and returns a confident wrong bool on the pathfinder's hot path.
///
/// Contrast `std::ops::Range`, which is public-field precisely because it has
/// no invariant to violate: `3..1` is simply empty and every operation stays
/// well-defined. Inverted values here are representable but meaningless,
/// which is when construction control earns its keep.
///
/// # Open-endedness
///
/// `end: None` means the range extends past every representable instant,
/// **including `u64::MAX`** — not "ends at the maximum". So
/// [`Self::contains`] is true at `u64::MAX` for any open-ended range, and two
/// open-ended ranges always overlap, even one starting at `u64::MAX`
/// (`two_open_ended_ranges_at_the_max_instant_overlap` pins this). Modelling
/// unbounded as a finite maximum instead would lose the top instant to
/// half-open exclusivity; the `Option` cannot make that mistake, which is
/// also what keeps `overlaps` and `contains` in agreement.
///
/// [`Self::duration`] returns `None` when open-ended — there is no end to
/// measure to.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "TimeRangeRepr", try_from = "TimeRangeRepr")]
pub struct TimeRange {
    start: SimTime,
    end: Option<SimTime>,
}

impl TimeRange {
    /// Closed range `[start, end)`, or [`TimeRangeError`] if `end` precedes
    /// `start`. `start == end` is accepted and yields an empty range.
    pub fn try_new(start: SimTime, end: SimTime) -> Result<Self, TimeRangeError> {
        if end < start {
            return Err(TimeRangeError { start, end });
        }
        Ok(Self { start, end: Some(end) })
    }

    /// Closed range `[start, end)`, asserting the caller knows the ordering.
    ///
    /// # Panics
    /// If `end` precedes `start`. Prefer [`Self::try_new`] when either bound
    /// is derived from run state rather than a literal.
    #[track_caller]
    pub fn new(start: SimTime, end: SimTime) -> Self {
        match Self::try_new(start, end) {
            Ok(range) => range,
            Err(err) => panic!("{err}"),
        }
    }

    /// Open-ended range starting at `start`, end not yet determined.
    pub const fn open_ended(start: SimTime) -> Self {
        Self { start, end: None }
    }

    /// Build from an already-optional end: `None` yields an open-ended
    /// range, `Some` goes through [`Self::try_new`].
    ///
    /// For call sites that carry `Option<SimTime>` themselves and would
    /// otherwise re-derive the open-ended-versus-closed split inline.
    pub fn from_parts(start: SimTime, end: Option<SimTime>) -> Result<Self, TimeRangeError> {
        match end {
            Some(end) => Self::try_new(start, end),
            None => Ok(Self::open_ended(start)),
        }
    }

    /// Closed range of `duration` beginning at `start`.
    ///
    /// Infallible by construction: `start + duration >= start` always, so
    /// there is no ordering to validate. The addition saturates, so a duration
    /// large enough to overflow clamps the end rather than failing, and
    /// `SimDuration::MAX` yields a range ending at `u64::MAX` (see the `EOT`
    /// hazard in the module doc).
    pub fn from_duration(start: SimTime, duration: SimDuration) -> Self {
        Self { start, end: Some(start.saturating_add(duration)) }
    }

    /// The empty range at `at`. Contains nothing, overlaps nothing.
    pub const fn empty_at(at: SimTime) -> Self {
        Self { start: at, end: Some(at) }
    }

    pub const fn start(self) -> SimTime {
        self.start
    }

    pub const fn end(self) -> Option<SimTime> {
        self.end
    }

    pub const fn is_open_ended(self) -> bool {
        self.end.is_none()
    }

    /// True for a zero-width range. Open-ended ranges are never empty.
    pub fn is_empty(self) -> bool {
        self.end == Some(self.start)
    }

    /// Elapsed span, or `None` when open-ended (no end to measure to).
    pub fn duration(self) -> Option<SimDuration> {
        self.end.map(|end| end - self.start)
    }

    /// Is `at` inside `[start, end)`? Always false for an empty range.
    pub fn contains(self, at: SimTime) -> bool {
        if self.is_empty() || at < self.start {
            return false;
        }
        self.end.is_none_or(|end| at < end)
    }

    /// Close an open-ended range, or move a closed range's end.
    ///
    /// This is the "successor finalizes the predecessor's edge" operation:
    /// a task creates an open-ended hold, and its successor sets the end to
    /// its own departure time.
    pub fn with_end(self, end: SimTime) -> Result<Self, TimeRangeError> {
        Self::try_new(self.start, end)
    }

    /// Do two half-open ranges intersect? `[a, b) ∩ [c, d) ≠ ∅`
    ///
    /// `gap` widens both ranges' ends, so it is a required clearance between
    /// otherwise-disjoint ranges. Note the boundary: ranges separated by `s`
    /// are disjoint when `gap <= s`, so overlap needs `gap > s` and a
    /// separation of *exactly* `gap` is still disjoint.
    ///
    /// Open-ended ranges extend without bound. Empty ranges overlap nothing,
    /// including each other.
    pub fn overlaps(self, other: Self, gap: SimDuration) -> bool {
        if self.is_empty() || other.is_empty() {
            return false;
        }
        if let Some(end) = self.end
            && end.saturating_add(gap) <= other.start
        {
            return false;
        }
        if let Some(end) = other.end
            && end.saturating_add(gap) <= self.start
        {
            return false;
        }
        true
    }
}

/// Serde shape. Routing through this is what makes deserialization validate
/// the ordering invariant instead of trusting the input, which a derived
/// impl on private fields would not do.
#[derive(Serialize, Deserialize)]
struct TimeRangeRepr {
    start: SimTime,
    end: Option<SimTime>,
}

impl From<TimeRange> for TimeRangeRepr {
    fn from(range: TimeRange) -> Self {
        Self { start: range.start, end: range.end }
    }
}

impl TryFrom<TimeRangeRepr> for TimeRange {
    type Error = TimeRangeError;
    fn try_from(repr: TimeRangeRepr) -> Result<Self, Self::Error> {
        match repr.end {
            Some(end) => Self::try_new(repr.start, end),
            None => Ok(Self::open_ended(repr.start)),
        }
    }
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

impl fmt::Display for SimTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_ms_as_seconds(self.0, f)
    }
}

impl fmt::Display for SimDuration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_ms_as_seconds(self.0, f)
    }
}

impl fmt::Display for SimOffset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 < 0 {
            f.write_str("-")?;
        }
        fmt_ms_as_seconds(self.0.unsigned_abs(), f)
    }
}

impl fmt::Display for TimeRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.end {
            Some(end) => write!(f, "[{},{})", self.start, end),
            None => write!(f, "[{},…)", self.start),
        }
    }
}

// Debug emits the test-helper constructor form (`t(N)` / `ms(N)` / `EOT` / `MAX`)
// so panic output from `assert_trace!` / `assert_waypoints!` pastes back into
// the fh_sipp test suite verbatim. Production logging gains brevity (no
// `SimTime(0)` wrapper).
//
// LOAD-BEARING BEYOND READABILITY: `observer::digest` hashes each event's
// `Debug` rendering to produce the G4 identity hash pinned in
// `perf/phase2-baseline/gate.txt`. Changing these impls, or changing an
// event field from a raw `u64` to one of these types, changes that hash
// with no change in behavior. Re-pin the baseline in the same commit.
impl fmt::Debug for SimTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if *self == Self::EOT {
            return f.write_str("EOT");
        }
        write!(f, "t({})", FmtUnderscored(self.0))
    }
}

impl fmt::Debug for SimDuration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if *self == Self::MAX {
            return f.write_str("MAX");
        }
        write!(f, "ms({})", FmtUnderscored(self.0))
    }
}

impl fmt::Debug for SimOffset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 < 0 {
            return write!(f, "off(-{})", FmtUnderscored(self.0.unsigned_abs()));
        }
        write!(f, "off({})", FmtUnderscored(self.0.unsigned_abs()))
    }
}

impl fmt::Debug for TimeRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.end {
            Some(end) => write!(f, "[{:?},{:?})", self.start, end),
            None => write!(f, "[{:?},…)", self.start),
        }
    }
}

struct FmtUnderscored(u64);

impl fmt::Display for FmtUnderscored {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 < 1000 {
            return write!(f, "{}", self.0);
        }
        let s = self.0.to_string();
        let len = s.len();
        for (i, ch) in s.chars().enumerate() {
            if i > 0 && (len - i).is_multiple_of(3) {
                f.write_str("_")?;
            }
            write!(f, "{ch}")?;
        }
        Ok(())
    }
}

fn fmt_ms_as_seconds(ms: u64, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let secs = ms / 1000;
    let frac = ms % 1000;
    if frac == 0 {
        write!(f, "{}s", secs)
    } else if frac.is_multiple_of(100) {
        write!(f, "{}.{}s", secs, frac / 100)
    } else if frac.is_multiple_of(10) {
        write!(f, "{}.{:02}s", secs, frac / 10)
    } else {
        write!(f, "{}.{:03}s", secs, frac)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(ms: u64) -> SimTime {
        SimTime::ms(ms)
    }

    // --- Rendering. Debug is load-bearing for assert_trace! and the G4 hash.

    #[test]
    fn debug_renders_paste_back_form() {
        assert_eq!(format!("{:?}", SimTime::ms(0)), "t(0)");
        assert_eq!(format!("{:?}", SimTime::ms(1_500)), "t(1_500)");
        assert_eq!(format!("{:?}", SimTime::ms(1_234_567)), "t(1_234_567)");
        assert_eq!(format!("{:?}", SimTime::EOT), "EOT");
        assert_eq!(format!("{:?}", SimDuration::ms(300)), "ms(300)");
        assert_eq!(format!("{:?}", SimDuration::MAX), "MAX");
        assert_eq!(format!("{:?}", SimOffset::ms(-500)), "off(-500)");
        assert_eq!(format!("{:?}", SimOffset::ms(2_000)), "off(2_000)");
    }

    #[test]
    fn display_renders_adaptive_seconds() {
        assert_eq!(SimTime::ms(5_000).to_string(), "5s");
        assert_eq!(SimTime::ms(5_500).to_string(), "5.5s");
        assert_eq!(SimTime::ms(5_050).to_string(), "5.05s");
        assert_eq!(SimTime::ms(5_005).to_string(), "5.005s");
        assert_eq!(SimOffset::ms(-1_500).to_string(), "-1.5s");
    }

    // --- Wire format must not change: newtypes serialize as bare integers,
    // so recordings and crucible's browser protocol survive a §11
    // migration of any field from `u64` to `SimTime`.

    #[test]
    fn json_wire_format_is_a_bare_integer() {
        assert_eq!(serde_json::to_string(&SimTime::ms(1_500)).unwrap(), "1500");
        assert_eq!(serde_json::to_string(&SimDuration::ms(300)).unwrap(), "300");
        assert_eq!(serde_json::to_string(&SimOffset::ms(-42)).unwrap(), "-42");
        assert_eq!(serde_json::from_str::<SimTime>("1500").unwrap(), SimTime::ms(1_500));
    }

    #[test]
    fn msgpack_wire_format_matches_the_raw_primitive() {
        // The profile container is MessagePack; a newtype must encode
        // identically to a bare u64 or recordings stop round-tripping.
        assert_eq!(rmp_serde::to_vec(&SimTime::ms(1_500)).unwrap(), rmp_serde::to_vec(&1_500u64).unwrap());
        assert_eq!(rmp_serde::to_vec(&SimDuration::ms(300)).unwrap(), rmp_serde::to_vec(&300u64).unwrap());
    }

    // --- Saturating semantics, pinned so a later policy change is deliberate.

    #[test]
    fn sub_saturates_to_zero_on_inversion() {
        // The silent fallback this module documents. Pinned, not endorsed.
        assert_eq!(SimTime::ms(100) - SimTime::ms(500), SimDuration::ZERO);
        assert_eq!(SimDuration::ms(100) - SimDuration::ms(500), SimDuration::ZERO);
        assert_eq!(SimTime::ms(100) - SimDuration::ms(500), SimTime::ZERO);
    }

    #[test]
    fn eot_is_exceedable_today() {
        // Known hazard, documented in the module doc and deliberately not
        // fixed here: saturation lands one ms past "last valid sim time".
        let past_eot = SimTime::ZERO + SimDuration::MAX;
        assert!(past_eot > SimTime::EOT);
        assert_eq!(past_eot.as_ms(), u64::MAX);
    }

    #[test]
    fn seconds_saturates_instead_of_wrapping() {
        assert_eq!(SimTime::seconds(5).as_ms(), 5_000);
        assert_eq!(SimTime::seconds(u64::MAX).as_ms(), u64::MAX);
        assert_eq!(SimDuration::seconds(u64::MAX).as_ms(), u64::MAX);
    }

    // --- The honest alternatives.

    #[test]
    fn checked_elapsed_since_reports_inversion() {
        assert_eq!(SimTime::ms(500).checked_elapsed_since(t(100)), Some(SimDuration::ms(400)));
        assert_eq!(SimTime::ms(100).checked_elapsed_since(t(500)), None);
        assert_eq!(SimTime::ms(100).checked_elapsed_since(t(100)), Some(SimDuration::ZERO));
    }

    #[test]
    fn checked_ops_report_boundaries() {
        assert_eq!(SimTime::ms(100).checked_sub(SimDuration::ms(500)), None);
        assert_eq!(SimTime::ms(500).checked_sub(SimDuration::ms(100)), Some(t(400)));
        assert_eq!(SimTime::ms(u64::MAX).checked_add(SimDuration::ms(1)), None);
        assert_eq!(SimDuration::ms(100).checked_sub(SimDuration::ms(500)), None);
        assert_eq!(SimDuration::MAX.checked_add(SimDuration::ms(1)), None);
        assert_eq!(SimDuration::MAX.checked_mul(2), None);
        assert_eq!(SimDuration::ms(100).checked_div(SimDuration::ZERO), None);
        assert_eq!(SimDuration::ms(100).checked_div(SimDuration::ms(10)), Some(10));
    }

    #[test]
    fn diff_is_signed_and_safe_at_eot() {
        assert_eq!(SimTime::ms(500).diff(t(100)), SimOffset::ms(400));
        assert_eq!(SimTime::ms(100).diff(t(500)), SimOffset::ms(-400));
        assert_eq!(SimTime::ms(100).diff(t(100)), SimOffset::ZERO);
        // Would overflow a naive `self.0 as i64 - other.0 as i64`.
        assert_eq!(SimTime::EOT.diff(SimTime::ZERO), SimOffset::ms(i64::MAX));
        assert_eq!(SimTime::ZERO.diff(SimTime::EOT), SimOffset::ms(i64::MIN));
    }

    #[test]
    fn offset_moves_time_both_directions() {
        assert_eq!(SimTime::ms(1_000) + SimOffset::ms(500), t(1_500));
        assert_eq!(SimTime::ms(1_000) + SimOffset::ms(-500), t(500));
        // Negative past zero clamps rather than wrapping.
        assert_eq!(SimTime::ms(100) + SimOffset::ms(-500), SimTime::ZERO);
        // i64::MIN has no positive counterpart; unsigned_abs must not panic.
        assert_eq!(SimTime::ms(100) + SimOffset::ms(i64::MIN), SimTime::ZERO);
    }

    #[test]
    fn abs_diff_is_order_independent() {
        assert_eq!(SimDuration::ms(500).abs_diff(SimDuration::ms(100)), SimDuration::ms(400));
        assert_eq!(SimDuration::ms(100).abs_diff(SimDuration::ms(500)), SimDuration::ms(400));
        assert_eq!(SimDuration::ms(100).abs_diff(SimDuration::ms(100)), SimDuration::ZERO);
        // Where bare Sub would saturate the smaller-minus-larger case to zero.
        assert_eq!(SimDuration::ms(100) - SimDuration::ms(500), SimDuration::ZERO);
    }

    #[test]
    fn offset_magnitude_and_negation() {
        assert_eq!(SimOffset::ms(-400).magnitude(), SimDuration::ms(400));
        assert_eq!(SimOffset::ms(400).magnitude(), SimDuration::ms(400));
        assert_eq!(-SimOffset::ms(400), SimOffset::ms(-400));
        assert!(SimOffset::ms(-1).is_negative());
        assert!(!SimOffset::ZERO.is_negative());
        // SimDuration::MAX exceeds i64; conversion saturates rather than wrapping negative.
        assert_eq!(SimOffset::from(SimDuration::MAX), SimOffset::ms(i64::MAX));
    }

    #[test]
    fn wall_duration_converts_to_a_length_not_an_instant() {
        assert_eq!(SimDuration::from(Duration::from_millis(1_500)), SimDuration::ms(1_500));
        assert_eq!(SimDuration::from(Duration::from_secs(2)), SimDuration::ms(2_000));
        // Sub-millisecond truncates toward zero.
        assert_eq!(SimDuration::from(Duration::from_nanos(999_999)), SimDuration::ZERO);
        // Reaching an instant takes an explicit epoch, which is what callers
        // configuring a run length have to write.
        assert_eq!(SimTime::ZERO + SimDuration::from(Duration::from_secs(36_000)), t(36_000_000));
    }

    #[test]
    fn from_secs_ceilings_so_submillisecond_never_vanishes() {
        assert_eq!(SimDuration::from_secs(0.0001).as_ms(), 1);
        assert_eq!(SimDuration::from_secs(-5.0), SimDuration::ZERO);
        assert_eq!(SimDuration::from_secs(1.933).as_ms(), 1_933);
    }

    /// [`SimOffset::from_secs`] rounds to nearest, and a nonzero input never
    /// lands on zero.
    ///
    /// The `4.001` and `1.001` cases are the ones that discriminate: they are
    /// whole-millisecond inputs whose float product is not exact, so a `ceil`
    /// answers 4002 and a truncating `as i64` answers 1000. Both are wrong by a
    /// millisecond on input an operator would actually type, and this value
    /// breaks ties in `timing_key`.
    #[test]
    fn offset_from_secs_rounds_to_nearest_without_letting_nonzero_vanish() {
        // Float product is 4001.0000000000005 — a ceil would say 4002.
        assert_eq!(SimOffset::from_secs(4.001), SimOffset::ms(4_001));
        // Float product is just over 1001.0 — a truncating cast would say 1000.
        assert_eq!(SimOffset::from_secs(1.001), SimOffset::ms(1_001));
        assert_eq!(SimOffset::from_secs(-4.001), SimOffset::ms(-4_001), "and symmetrically below zero");

        // Sub-half-millisecond rounds to nearest = 0, so the floor kicks in and
        // the sign survives. A configured bias must not become "no bias".
        assert_eq!(SimOffset::from_secs(0.0001), SimOffset::ms(1));
        assert_eq!(SimOffset::from_secs(-0.0001), SimOffset::ms(-1));

        // Exact zero is the only input allowed to produce zero.
        assert_eq!(SimOffset::from_secs(0.0), SimOffset::ZERO);
        assert_eq!(SimOffset::from_secs(-0.0), SimOffset::ZERO);

        assert_eq!(SimOffset::from_secs(1.933), SimOffset::ms(1_933));
        assert_eq!(SimOffset::from_secs(-1.933), SimOffset::ms(-1_933));
    }

    /// Non-finite input cannot be read as a displacement, so it collapses to
    /// zero here rather than to a saturated bias that would silently win every
    /// comparison. Config boundaries reject it before this point; this pins the
    /// fallback so it stays boring.
    #[test]
    fn offset_from_secs_is_zero_on_non_finite_input() {
        assert_eq!(SimOffset::from_secs(f64::NAN), SimOffset::ZERO);
        assert_eq!(SimOffset::from_secs(f64::INFINITY), SimOffset::ZERO);
        assert_eq!(SimOffset::from_secs(f64::NEG_INFINITY), SimOffset::ZERO);
        // Finite but absurd saturates instead of wrapping.
        assert_eq!(SimOffset::from_secs(1e30), SimOffset::ms(i64::MAX));
        assert_eq!(SimOffset::from_secs(-1e30), SimOffset::ms(-i64::MAX));
    }

    #[test]
    fn quantize_floor_rounds_down() {
        assert_eq!(SimDuration::ms(1_250).quantize_floor(SimDuration::ms(500)), SimDuration::ms(1_000));
        assert_eq!(SimDuration::ms(1_000).quantize_floor(SimDuration::ms(500)), SimDuration::ms(1_000));
    }

    #[test]
    #[should_panic(expected = "quantize_floor resolution must be non-zero")]
    fn quantize_floor_rejects_zero_resolution() {
        SimDuration::ms(1_250).quantize_floor(SimDuration::ZERO);
    }

    #[test]
    #[should_panic(expected = "SimDuration divided by ZERO")]
    fn div_by_zero_panics_with_a_named_message() {
        let _ = SimDuration::ms(100) / SimDuration::ZERO;
    }

    // --- TimeRange: the invariant.

    #[test]
    fn inverted_construction_is_rejected() {
        let err = TimeRange::try_new(t(500), t(100)).unwrap_err();
        assert_eq!(err, TimeRangeError { start: t(500), end: t(100) });
        // The message carries both bounds in paste-back form.
        assert_eq!(err.to_string(), "TimeRange end t(100) precedes start t(500)");
    }

    #[test]
    #[should_panic(expected = "TimeRange end t(100) precedes start t(500)")]
    fn new_panics_on_inversion() {
        TimeRange::new(t(500), t(100));
    }

    #[test]
    fn zero_width_is_accepted_as_empty() {
        let range = TimeRange::try_new(t(100), t(100)).unwrap();
        assert!(range.is_empty());
        assert_eq!(range.duration(), Some(SimDuration::ZERO));
        assert_eq!(range, TimeRange::empty_at(t(100)));
    }

    #[test]
    fn deserialization_validates_rather_than_trusting() {
        // The hole a derived impl on private fields would leave open.
        let err = serde_json::from_str::<TimeRange>(r#"{"start":500,"end":100}"#).unwrap_err();
        assert!(err.to_string().contains("precedes start"), "got: {err}");

        let ok: TimeRange = serde_json::from_str(r#"{"start":100,"end":500}"#).unwrap();
        assert_eq!(ok, TimeRange::new(t(100), t(500)));

        let open: TimeRange = serde_json::from_str(r#"{"start":100,"end":null}"#).unwrap();
        assert!(open.is_open_ended());
    }

    #[test]
    fn from_parts_splits_on_the_option() {
        assert_eq!(TimeRange::from_parts(t(100), Some(t(500))).unwrap(), TimeRange::new(t(100), t(500)));
        assert_eq!(TimeRange::from_parts(t(100), None).unwrap(), TimeRange::open_ended(t(100)));
        assert!(TimeRange::from_parts(t(500), Some(t(100))).is_err(), "still validates");
    }

    #[test]
    fn from_duration_cannot_invert() {
        assert_eq!(TimeRange::from_duration(t(100), SimDuration::ms(400)), TimeRange::new(t(100), t(500)));
        assert_eq!(TimeRange::from_duration(t(100), SimDuration::ZERO), TimeRange::empty_at(t(100)));
        // Saturating: no ordering violation is reachable even at MAX.
        let huge = TimeRange::from_duration(t(100), SimDuration::MAX);
        assert_eq!(huge.end, Some(SimTime::ms(u64::MAX)));
        assert!(huge.end.unwrap() >= huge.start);
    }

    #[test]
    fn serde_round_trips_both_shapes() {
        for range in [TimeRange::new(t(100), t(500)), TimeRange::open_ended(t(100))] {
            let json = serde_json::to_string(&range).unwrap();
            assert_eq!(serde_json::from_str::<TimeRange>(&json).unwrap(), range);
        }
    }

    // --- TimeRange: accessors and containment.

    #[test]
    fn open_ended_has_no_measurable_duration() {
        let open = TimeRange::open_ended(t(100));
        assert!(open.is_open_ended());
        assert!(!open.is_empty());
        // No end to measure to, so None rather than a panic or a fake value.
        assert_eq!(open.duration(), None);
    }

    #[test]
    fn contains_is_half_open() {
        let range = TimeRange::new(t(100), t(500));
        assert!(!range.contains(t(99)));
        assert!(range.contains(t(100)), "start is inclusive");
        assert!(range.contains(t(499)));
        assert!(!range.contains(t(500)), "end is exclusive");

        let open = TimeRange::open_ended(t(100));
        assert!(!open.contains(t(99)));
        assert!(open.contains(t(100)));
        assert!(open.contains(SimTime::EOT));

        assert!(!TimeRange::empty_at(t(100)).contains(t(100)), "empty contains nothing");
    }

    #[test]
    fn with_end_closes_an_open_range() {
        let open = TimeRange::open_ended(t(100));
        assert_eq!(open.with_end(t(500)).unwrap(), TimeRange::new(t(100), t(500)));
        assert!(open.with_end(t(50)).is_err(), "cannot close before the start");
    }

    // --- TimeRange: overlap.

    #[test]
    fn zero_width_overlap_is_empty() {
        // A zero-width range is empty, so it overlaps nothing — not even a
        // range that strictly contains its instant, and not even itself.
        let zero = TimeRange::new(t(100), t(100));
        let containing = TimeRange::new(t(50), t(200));
        let exact = TimeRange::new(t(100), t(150));
        let open = TimeRange::open_ended(t(0));
        assert!(!zero.overlaps(containing, SimDuration::ZERO));
        assert!(!containing.overlaps(zero, SimDuration::ZERO));
        assert!(!zero.overlaps(exact, SimDuration::ZERO));
        assert!(!zero.overlaps(open, SimDuration::ZERO));
    }

    #[test]
    fn adjacent_half_open_ranges_do_not_overlap() {
        let a = TimeRange::new(t(100), t(200));
        let b = TimeRange::new(t(200), t(300));
        assert!(!a.overlaps(b, SimDuration::ZERO));
        assert!(!b.overlaps(a, SimDuration::ZERO));
    }

    #[test]
    fn overlapping_and_nested_ranges_intersect() {
        let a = TimeRange::new(t(100), t(300));
        assert!(a.overlaps(TimeRange::new(t(200), t(400)), SimDuration::ZERO));
        assert!(a.overlaps(TimeRange::new(t(150), t(200)), SimDuration::ZERO), "nested");
        assert!(a.overlaps(TimeRange::new(t(0), t(1_000)), SimDuration::ZERO), "containing");
        assert!(a.overlaps(a, SimDuration::ZERO), "self");
    }

    #[test]
    fn gap_must_strictly_exceed_the_separation() {
        // Separation here is 50ms (a ends at 200, b starts at 250).
        let a = TimeRange::new(t(100), t(200));
        let b = TimeRange::new(t(250), t(300));
        assert!(!a.overlaps(b, SimDuration::ZERO));
        assert!(!a.overlaps(b, SimDuration::ms(49)));
        assert!(!a.overlaps(b, SimDuration::ms(50)), "a gap equal to the separation is still disjoint");
        assert!(a.overlaps(b, SimDuration::ms(51)), "one ms more bridges it");
        assert!(b.overlaps(a, SimDuration::ms(51)), "symmetric");
    }

    #[test]
    fn open_ended_extends_without_bound() {
        let open = TimeRange::open_ended(t(100));
        assert!(open.overlaps(TimeRange::new(t(1_000_000), SimTime::EOT), SimDuration::ZERO));
        assert!(!open.overlaps(TimeRange::new(t(0), t(100)), SimDuration::ZERO), "ends at our start");
        assert!(open.overlaps(TimeRange::open_ended(t(999)), SimDuration::ZERO), "two open ranges");
    }

    #[test]
    fn two_open_ended_ranges_at_the_max_instant_overlap() {
        // `end: None` means "past every representable instant", so an
        // open-ended range contains u64::MAX and any two of them share it.
        // Flattening unbounded to the finite u64::MAX would answer "disjoint"
        // here, because the EXCLUSIVE `end <= other.start` test then reasons
        // about [s, u64::MAX) while the range actually includes u64::MAX.
        let max = SimTime::ms(u64::MAX);
        let unbounded = TimeRange::open_ended(t(1_000));
        let at_max = TimeRange::open_ended(max);

        // Both genuinely contain the instant, so they genuinely overlap.
        assert!(unbounded.contains(max), "[1s,∞) contains the max instant");
        assert!(at_max.contains(max), "[max,∞) contains its own start");
        assert!(unbounded.overlaps(at_max, SimDuration::ZERO), "sharing an instant means overlapping");
        assert!(at_max.overlaps(unbounded, SimDuration::ZERO), "symmetric");

        // A CLOSED range at the max instant is zero-width, hence empty, hence
        // overlaps nothing — this is why the divergent case is uniquely the
        // open-ended-vs-open-ended one.
        let empty_at_max = TimeRange::empty_at(max);
        assert!(empty_at_max.is_empty());
        assert!(!unbounded.overlaps(empty_at_max, SimDuration::ZERO));
        assert!(!empty_at_max.overlaps(unbounded, SimDuration::ZERO));
    }

    #[test]
    fn a_huge_gap_saturates_rather_than_wrapping() {
        // `end + gap` must clamp, not wrap. If it wrapped, the widened end
        // would land below the other range's start and the disjointness
        // check would report a false negative.
        let late = TimeRange::new(t(1_000), SimTime::EOT);
        let early = TimeRange::new(t(0), t(500));
        assert!(!late.overlaps(early, SimDuration::ZERO), "disjoint with no gap");
        assert!(late.overlaps(early, SimDuration::MAX), "a MAX gap must bridge them");
        assert!(early.overlaps(late, SimDuration::MAX), "symmetric");
    }

    #[test]
    fn range_display_and_debug_shapes() {
        assert_eq!(TimeRange::new(t(1_000), t(2_500)).to_string(), "[1s,2.5s)");
        assert_eq!(TimeRange::open_ended(t(1_000)).to_string(), "[1s,…)");
        assert_eq!(format!("{:?}", TimeRange::new(t(1_000), t(2_500))), "[t(1_000),t(2_500))");
        assert_eq!(format!("{:?}", TimeRange::open_ended(t(1_000))), "[t(1_000),…)");
    }
}
