//! Trapezoidal motion profile: full phase parameters for sampling
//! `position(t)` and `velocity(t)` across a single cell traversal.
//!
//! [`trapezoidal_profile`] is the new public entry point that surfaces
//! the *complete* profile constructed from the same six branches as the
//! original `trapezoidal_cost` (see `cost.rs` for the thin wrapper).
//! Phase 4 of the kinematic playback rasterization spec consumes this
//! struct to interpolate robot positions at observer-driven sample
//! times; the search itself only ever asked for `(total_time,
//! exit_velocity)`.
//!
//! All six branches collapse to the same parametric shape:
//!
//! - accel from `entry_velocity_mm_s` to `peak_velocity_mm_s` over
//!   `accel_time_s` covering `accel_distance_mm`,
//! - cruise at `peak_velocity_mm_s` for `cruise_time_s` covering
//!   `cruise_distance_mm`,
//! - decel from `peak_velocity_mm_s` to `exit_velocity_mm_s` over
//!   `decel_time_s` covering `decel_distance_mm`.
//!
//! Each phase may have zero duration. Position is the piecewise
//! integral; velocity the piecewise derivative.

use serde::{Deserialize, Serialize};

use super::{KinematicError, KinematicParams, Velocity};

/// Full trapezoidal profile for a single cell traversal.
///
/// All time fields sum to `total_time_s`; all distance fields sum to
/// `total_distance_mm` (modulo f64 rounding). Any of the three phase
/// times may be zero; degenerate cases (zero distance, pure cruise,
/// pure accel, pure decel) are normalised into the same struct shape
/// rather than represented as variants.
///
/// `acceleration_mm_s2` and `deceleration_mm_s2` are echoed from the
/// input [`KinematicParams`] so the position/velocity samplers don't
/// need a second parameter — they're not strictly required to evaluate
/// the profile (the time/distance triples are enough), but they're
/// useful for cross-checking and for sampling sub-phase derivatives.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrapezoidalProfile {
    pub entry_velocity_mm_s: f64,
    pub exit_velocity_mm_s: f64,
    /// Velocity during the cruise phase (also the peak of the accel
    /// phase and the start of the decel phase). For triangular peak
    /// it's `v_p`; for trapezoidal-with-cruise it's `v_max`; for
    /// degenerate cruise / decel-3b it's `entry_velocity_mm_s`.
    pub peak_velocity_mm_s: f64,
    pub accel_time_s: f64,
    pub cruise_time_s: f64,
    pub decel_time_s: f64,
    pub accel_distance_mm: f64,
    pub cruise_distance_mm: f64,
    pub decel_distance_mm: f64,
    pub total_time_s: f64,
    pub total_distance_mm: f64,
    pub acceleration_mm_s2: f64,
    pub deceleration_mm_s2: f64,
}

impl TrapezoidalProfile {
    /// Degenerate "no motion" profile. Used for zero-length cells and
    /// as the starting point for the case-by-case construction in
    /// [`trapezoidal_profile`].
    fn zero(entry_velocity_mm_s: f64, params: &KinematicParams) -> Self {
        Self {
            entry_velocity_mm_s,
            exit_velocity_mm_s: entry_velocity_mm_s,
            peak_velocity_mm_s: entry_velocity_mm_s,
            accel_time_s: 0.0,
            cruise_time_s: 0.0,
            decel_time_s: 0.0,
            accel_distance_mm: 0.0,
            cruise_distance_mm: 0.0,
            decel_distance_mm: 0.0,
            total_time_s: 0.0,
            total_distance_mm: 0.0,
            acceleration_mm_s2: params.acceleration_mm_s2,
            deceleration_mm_s2: params.deceleration_mm_s2,
        }
    }
}

/// Construct a [`TrapezoidalProfile`] for traversing `cell_length_mm`
/// from `entry_v` toward `target_exit_v`, bounded by `params`.
///
/// Same six-branch logic as the original `trapezoidal_cost`:
///
/// - **Zero distance** (`cell_length_mm == 0`): degenerate profile.
/// - **Degenerate cruise** (`target == entry > 0`): pure cruise.
/// - **Pure-accel clamp** (`target > entry`, distance too short):
///   exit clamped DOWN to `sqrt(entry² + 2 a d)`.
/// - **Decel 3a** (`target < entry`, brake distance > cell): exit
///   clamped UP to `sqrt(entry² - 2 dec d)`.
/// - **Decel 3b** (`target < entry`, room to spare): cruise at entry
///   then decel to target.
/// - **Triangular peak** (`target ≥ entry`, peak ≤ v_max): accel to
///   `v_p`, decel to target, no cruise.
/// - **Trapezoidal with cruise** (peak > v_max): accel to v_max,
///   cruise at v_max, decel to target.
pub fn trapezoidal_profile(
    entry_v: Velocity,
    target_exit_v: Velocity,
    cell_length_mm: u64,
    params: &KinematicParams,
) -> Result<TrapezoidalProfile, KinematicError> {
    // Finiteness comes first: NaN compares false against everything, so the
    // `<= 0.0` checks below would silently pass NaN through into the profile
    // math. inf would survive the comparisons too and produce inf/NaN cell
    // costs. Configuration bug either way.
    if !params.acceleration_mm_s2.is_finite()
        || !params.deceleration_mm_s2.is_finite()
        || !params.max_velocity_mm_s.is_finite()
    {
        return Err(KinematicError::NonFiniteParam);
    }
    if params.acceleration_mm_s2 <= 0.0 {
        return Err(KinematicError::NonPositiveAcceleration);
    }
    if params.deceleration_mm_s2 <= 0.0 {
        return Err(KinematicError::NonPositiveDeceleration);
    }
    if params.max_velocity_mm_s <= 0.0 {
        return Err(KinematicError::NonPositiveMaxVelocity);
    }

    let a = params.acceleration_mm_s2;
    let dec = params.deceleration_mm_s2;
    let v_max = params.max_velocity_mm_s;
    let d = cell_length_mm as f64;

    // Zero-distance: no motion, exit equals entry. Used by callers that
    // synthesise pseudo-cells (e.g., dwell-only goal states); not normally
    // emitted by the move successor.
    if d <= 0.0 {
        return Ok(TrapezoidalProfile::zero(entry_v.0, params));
    }

    let v_in_raw = entry_v.0;
    let v_out_raw = target_exit_v.0;

    if v_in_raw < 0.0 || v_out_raw < 0.0 {
        // Negative velocities aren't a thing in this domain. Coerce to zero
        // rather than infect the math; debug builds catch the upstream bug.
        debug_assert!(v_in_raw >= 0.0 && v_out_raw >= 0.0, "negative velocity entry={} target={}", v_in_raw, v_out_raw);
    }
    let v_in = v_in_raw.max(0.0);
    let v_out_target = v_out_raw.max(0.0).min(v_max);

    if v_in > v_max + 1e-6 {
        return Err(KinematicError::EntryExceedsMaxVelocity);
    }
    let v_in = v_in.min(v_max); // tolerate sub-epsilon overshoot from f64 round-trip

    // Pure accel limit: max v reachable from v_in over distance d.
    // v_reach² = v_in² + 2 a d
    let v_reach_accel_sq = v_in * v_in + 2.0 * a * d;
    let v_reach_accel = v_reach_accel_sq.sqrt();

    // Case 1 (degenerate cruise): target == entry, both > 0. The robot
    // enters and exits at the same nonzero velocity, so the simplest
    // physically realisable trajectory is pure cruise the whole cell.
    // Note: target == entry == 0 deliberately falls through to the
    // triangular peak below, which finds the only feasible trajectory
    // (accel to a peak, then decel back to zero).
    if (v_out_target - v_in).abs() < 1e-9 && v_in > 0.0 {
        let cruise_time = d / v_in;
        return Ok(TrapezoidalProfile {
            entry_velocity_mm_s: v_in,
            exit_velocity_mm_s: v_in,
            peak_velocity_mm_s: v_in,
            accel_time_s: 0.0,
            cruise_time_s: cruise_time,
            decel_time_s: 0.0,
            accel_distance_mm: 0.0,
            cruise_distance_mm: d,
            decel_distance_mm: 0.0,
            total_time_s: cruise_time,
            total_distance_mm: d,
            acceleration_mm_s2: a,
            deceleration_mm_s2: dec,
        });
    }

    // Case 2 (acceleration, target > entry): clamp DOWN if cell is too
    // short to reach the requested target via pure acceleration. Profile
    // is a single accel phase ending at the clamped velocity.
    if v_out_target > v_in && v_reach_accel < v_out_target {
        let v_actual = v_reach_accel;
        let accel_time = (v_actual - v_in) / a;
        return Ok(TrapezoidalProfile {
            entry_velocity_mm_s: v_in,
            exit_velocity_mm_s: v_actual,
            peak_velocity_mm_s: v_actual,
            accel_time_s: accel_time,
            cruise_time_s: 0.0,
            decel_time_s: 0.0,
            accel_distance_mm: d,
            cruise_distance_mm: 0.0,
            decel_distance_mm: 0.0,
            total_time_s: accel_time,
            total_distance_mm: d,
            acceleration_mm_s2: a,
            deceleration_mm_s2: dec,
        });
    }

    // Case 3 (deceleration, target < entry).
    if v_out_target < v_in {
        let d_d = (v_in * v_in - v_out_target * v_out_target) / (2.0 * dec);
        if d_d > d {
            // 3a: insufficient distance to brake all the way down to
            // target. sqrt argument is guaranteed >= 0 because d_d > d
            // ==> (v_in² - v_out²)/(2 dec) > d ==> v_in² - 2 dec d >
            // v_out² >= 0.
            let v_actual_sq = v_in * v_in - 2.0 * dec * d;
            let v_actual = v_actual_sq.max(0.0).sqrt();
            let decel_time = (v_in - v_actual) / dec;
            return Ok(TrapezoidalProfile {
                entry_velocity_mm_s: v_in,
                exit_velocity_mm_s: v_actual,
                peak_velocity_mm_s: v_in,
                accel_time_s: 0.0,
                cruise_time_s: 0.0,
                decel_time_s: decel_time,
                accel_distance_mm: 0.0,
                cruise_distance_mm: 0.0,
                decel_distance_mm: d,
                total_time_s: decel_time,
                total_distance_mm: d,
                acceleration_mm_s2: a,
                deceleration_mm_s2: dec,
            });
        }
        // 3b: cruise at entry for the leftover distance, then decel to
        // target. v_in > v_out_target >= 0 ==> v_in > 0, so cruise time
        // is finite.
        let cruise_dist = d - d_d;
        let cruise_time = cruise_dist / v_in;
        let decel_time = (v_in - v_out_target) / dec;
        return Ok(TrapezoidalProfile {
            entry_velocity_mm_s: v_in,
            exit_velocity_mm_s: v_out_target,
            peak_velocity_mm_s: v_in,
            accel_time_s: 0.0,
            cruise_time_s: cruise_time,
            decel_time_s: decel_time,
            accel_distance_mm: 0.0,
            cruise_distance_mm: cruise_dist,
            decel_distance_mm: d_d,
            total_time_s: cruise_time + decel_time,
            total_distance_mm: d,
            acceleration_mm_s2: a,
            deceleration_mm_s2: dec,
        });
    }

    // Case C (target >= entry): triangular peak first, fall back to
    // trapezoid with cruise when the peak exceeds v_max.
    //
    // Triangular: v_p² = (2 a dec d + dec v_in² + a v_out²) / (a + dec)
    let v_p_sq = (2.0 * a * dec * d + dec * v_in * v_in + a * v_out_target * v_out_target) / (a + dec);
    let v_p = v_p_sq.sqrt();

    if v_p <= v_max {
        // Triangular: accel from v_in to v_p, decel from v_p to v_out.
        let accel_time = (v_p - v_in) / a;
        let decel_time = (v_p - v_out_target) / dec;
        // Distances from kinematics: d_a = (v_p² - v_in²)/(2a),
        // d_d = (v_p² - v_out²)/(2 dec). Their sum equals d up to f64
        // rounding (the v_p formula was derived from exactly that
        // constraint).
        let accel_distance = (v_p * v_p - v_in * v_in) / (2.0 * a);
        let decel_distance = (v_p * v_p - v_out_target * v_out_target) / (2.0 * dec);
        return Ok(TrapezoidalProfile {
            entry_velocity_mm_s: v_in,
            exit_velocity_mm_s: v_out_target,
            peak_velocity_mm_s: v_p,
            accel_time_s: accel_time,
            cruise_time_s: 0.0,
            decel_time_s: decel_time,
            accel_distance_mm: accel_distance,
            cruise_distance_mm: 0.0,
            decel_distance_mm: decel_distance,
            total_time_s: accel_time + decel_time,
            total_distance_mm: d,
            acceleration_mm_s2: a,
            deceleration_mm_s2: dec,
        });
    }

    // Trapezoidal with cruise at v_max.
    // d_a = (v_max² - v_in²) / (2 a)
    // d_d = (v_max² - v_out²) / (2 dec)
    // d_c = d - d_a - d_d  (>= 0 because v_p > v_max means we have room)
    let d_a = (v_max * v_max - v_in * v_in) / (2.0 * a);
    let d_d = (v_max * v_max - v_out_target * v_out_target) / (2.0 * dec);
    let d_c = d - d_a - d_d;
    debug_assert!(d_c > -1e-6, "d_c negative: d={d} d_a={d_a} d_d={d_d}");
    let d_c = d_c.max(0.0);
    let accel_time = (v_max - v_in) / a;
    let cruise_time = d_c / v_max;
    let decel_time = (v_max - v_out_target) / dec;
    Ok(TrapezoidalProfile {
        entry_velocity_mm_s: v_in,
        exit_velocity_mm_s: v_out_target,
        peak_velocity_mm_s: v_max,
        accel_time_s: accel_time,
        cruise_time_s: cruise_time,
        decel_time_s: decel_time,
        accel_distance_mm: d_a,
        cruise_distance_mm: d_c,
        decel_distance_mm: d_d,
        total_time_s: accel_time + cruise_time + decel_time,
        total_distance_mm: d,
        acceleration_mm_s2: a,
        deceleration_mm_s2: dec,
    })
}

/// Distance travelled from the start of `profile` at time `t_seconds`.
/// Saturates at the boundaries: t < 0 returns 0, t > total_time returns
/// total_distance. Position is monotonically non-decreasing.
pub fn trapezoid_position(profile: &TrapezoidalProfile, t_seconds: f64) -> f64 {
    if !t_seconds.is_finite() || t_seconds <= 0.0 {
        return 0.0;
    }
    if t_seconds >= profile.total_time_s {
        return profile.total_distance_mm;
    }

    if t_seconds <= profile.accel_time_s {
        // s(t) = v_in * t + 0.5 * a * t²
        return profile.entry_velocity_mm_s * t_seconds + 0.5 * profile.acceleration_mm_s2 * t_seconds * t_seconds;
    }

    let cruise_end = profile.accel_time_s + profile.cruise_time_s;
    if t_seconds <= cruise_end {
        let t_c = t_seconds - profile.accel_time_s;
        return profile.accel_distance_mm + profile.peak_velocity_mm_s * t_c;
    }

    // Decel phase.
    let t_d = t_seconds - cruise_end;
    profile.accel_distance_mm + profile.cruise_distance_mm + profile.peak_velocity_mm_s * t_d
        - 0.5 * profile.deceleration_mm_s2 * t_d * t_d
}

/// Instantaneous velocity at time `t_seconds`. Saturates: t < 0 returns
/// 0; t > total_time returns 0 (the robot has stopped emitting motion
/// for this cell — exit velocity is the entry of the next cell).
pub fn trapezoid_velocity(profile: &TrapezoidalProfile, t_seconds: f64) -> f64 {
    if !t_seconds.is_finite() || t_seconds < 0.0 || t_seconds > profile.total_time_s {
        return 0.0;
    }

    if t_seconds <= profile.accel_time_s {
        // v(t) = v_in + a * t
        return profile.entry_velocity_mm_s + profile.acceleration_mm_s2 * t_seconds;
    }

    let cruise_end = profile.accel_time_s + profile.cruise_time_s;
    if t_seconds <= cruise_end {
        return profile.peak_velocity_mm_s;
    }

    // Decel phase: v(t) = peak - dec * (t - cruise_end)
    let t_d = t_seconds - cruise_end;
    profile.peak_velocity_mm_s - profile.deceleration_mm_s2 * t_d
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixed trapezoidal-math params, decoupled from the reference M5F's loaded
    /// figures (field-calibrated to the empty ones, 4500/2000).
    /// These subcase scenarios were tuned around v_max=1500 / a=dec=1000 — the ratios
    /// that select each trapezoid subcase — so they pin those values explicitly.
    fn loaded_test_params() -> KinematicParams {
        KinematicParams { max_velocity_mm_s: 1500.0, acceleration_mm_s2: 1000.0, deceleration_mm_s2: 1000.0 }
    }

    fn approx_eq(a: f64, b: f64, eps: f64) -> bool {
        (a - b).abs() < eps
    }

    /// For each profile, sample N intermediate t values and assert
    /// position is monotonic and velocity is in [0, peak].
    fn assert_profile_invariants(profile: &TrapezoidalProfile) {
        // Boundaries.
        assert!(
            approx_eq(trapezoid_position(profile, 0.0), 0.0, 1e-9),
            "position(0) = {}",
            trapezoid_position(profile, 0.0)
        );
        assert!(
            approx_eq(trapezoid_position(profile, profile.total_time_s), profile.total_distance_mm, 1e-6),
            "position(total_time) = {} expected {}",
            trapezoid_position(profile, profile.total_time_s),
            profile.total_distance_mm,
        );
        assert!(
            approx_eq(trapezoid_velocity(profile, 0.0), profile.entry_velocity_mm_s, 1e-6),
            "velocity(0) = {} expected {}",
            trapezoid_velocity(profile, 0.0),
            profile.entry_velocity_mm_s,
        );
        assert!(
            approx_eq(trapezoid_velocity(profile, profile.total_time_s), profile.exit_velocity_mm_s, 1e-6),
            "velocity(total_time) = {} expected {}",
            trapezoid_velocity(profile, profile.total_time_s),
            profile.exit_velocity_mm_s,
        );

        // Monotonic position + bounded velocity over 32 samples.
        let n = 32usize;
        let mut last = -1e-12;
        for i in 0..=n {
            let t = profile.total_time_s * (i as f64 / n as f64);
            let s = trapezoid_position(profile, t);
            let v = trapezoid_velocity(profile, t);
            assert!(s + 1e-9 >= last, "position regressed at t={t}: {s} < prev {last}");
            assert!(
                v >= -1e-9 && v <= profile.peak_velocity_mm_s + 1e-6,
                "velocity out of range at t={t}: v={v} peak={}",
                profile.peak_velocity_mm_s
            );
            last = s;
        }

        // Distances should sum to total.
        let sum = profile.accel_distance_mm + profile.cruise_distance_mm + profile.decel_distance_mm;
        assert!(
            approx_eq(sum, profile.total_distance_mm, 1e-6),
            "distance components {} != total {}",
            sum,
            profile.total_distance_mm,
        );
        // Times should sum to total.
        let tsum = profile.accel_time_s + profile.cruise_time_s + profile.decel_time_s;
        assert!(
            approx_eq(tsum, profile.total_time_s, 1e-9),
            "time components {} != total {}",
            tsum,
            profile.total_time_s,
        );
    }

    #[test]
    fn zero_distance_profile_is_pure_idle() {
        let p = loaded_test_params();
        let profile = trapezoidal_profile(Velocity(500.0), Velocity::ZERO, 0, &p).expect("ok");
        assert_eq!(profile.total_time_s, 0.0);
        assert_eq!(profile.total_distance_mm, 0.0);
        assert_eq!(profile.entry_velocity_mm_s, 500.0);
        // Per spec: position(t)=0, velocity(t)=entry for all t.
        for t in [-1.0, 0.0, 0.5, 5.0, 100.0] {
            assert_eq!(trapezoid_position(&profile, t), 0.0, "pos at {t}");
        }
        // Velocity sampler returns 0 outside [0, total_time]. With
        // total_time=0 the only in-range sample is t=0, where we return
        // entry velocity.
        assert_eq!(trapezoid_velocity(&profile, 0.0), 500.0);
        assert_eq!(trapezoid_velocity(&profile, 0.001), 0.0);
        assert_eq!(trapezoid_velocity(&profile, -0.001), 0.0);
    }

    #[test]
    fn degenerate_cruise_profile() {
        // Case 1: target == entry > 0. Pure cruise.
        let p = loaded_test_params();
        let profile = trapezoidal_profile(Velocity(1500.0), Velocity(1500.0), 1500, &p).expect("ok");
        assert_eq!(profile.peak_velocity_mm_s, 1500.0);
        assert_eq!(profile.accel_time_s, 0.0);
        assert_eq!(profile.decel_time_s, 0.0);
        assert!(approx_eq(profile.cruise_time_s, 1.0, 1e-9));
        assert!(approx_eq(profile.cruise_distance_mm, 1500.0, 1e-9));
        assert_profile_invariants(&profile);
    }

    #[test]
    fn pure_accel_clamp_profile() {
        // Case 2: entry=0, target=v_max=1500, d=100 mm. Distance too
        // short — clamp DOWN.
        let p = loaded_test_params();
        let profile = trapezoidal_profile(Velocity::ZERO, Velocity(1500.0), 100, &p).expect("ok");
        let v_actual = (200_000f64).sqrt();
        assert!(approx_eq(profile.exit_velocity_mm_s, v_actual, 1e-6));
        assert_eq!(profile.cruise_time_s, 0.0);
        assert_eq!(profile.decel_time_s, 0.0);
        assert!(profile.accel_time_s > 0.0);
        assert!(approx_eq(profile.accel_distance_mm, 100.0, 1e-9));
        assert_profile_invariants(&profile);
    }

    #[test]
    fn decel_3a_partial_brake_profile() {
        // Case 3a: entry=1500, target=0, d=1000. Brake distance
        // 1125 > 1000. Exit clamped UP to 500 mm/s.
        let p = loaded_test_params();
        let profile = trapezoidal_profile(Velocity(1500.0), Velocity::ZERO, 1000, &p).expect("ok");
        assert!(approx_eq(profile.exit_velocity_mm_s, 500.0, 1e-6));
        assert_eq!(profile.peak_velocity_mm_s, 1500.0);
        assert_eq!(profile.accel_time_s, 0.0);
        assert_eq!(profile.cruise_time_s, 0.0);
        assert!(approx_eq(profile.decel_time_s, 1.0, 1e-9));
        assert!(approx_eq(profile.decel_distance_mm, 1000.0, 1e-9));
        assert_profile_invariants(&profile);
    }

    #[test]
    fn decel_3b_cruise_then_brake_profile() {
        // Case 3b: entry=1500, target=0, d=2000. d_d=1125, cruise=875.
        let p = loaded_test_params();
        let profile = trapezoidal_profile(Velocity(1500.0), Velocity::ZERO, 2000, &p).expect("ok");
        assert_eq!(profile.exit_velocity_mm_s, 0.0);
        assert_eq!(profile.peak_velocity_mm_s, 1500.0);
        assert_eq!(profile.accel_time_s, 0.0);
        assert!(approx_eq(profile.cruise_distance_mm, 875.0, 1e-9));
        assert!(approx_eq(profile.decel_distance_mm, 1125.0, 1e-9));
        assert!(approx_eq(profile.cruise_time_s, 875.0 / 1500.0, 1e-9));
        assert!(approx_eq(profile.decel_time_s, 1.5, 1e-9));
        assert_profile_invariants(&profile);
    }

    #[test]
    fn triangular_peak_profile() {
        // Case C triangular: entry=0, target=0, d=1000. v_p=1000<v_max.
        let p = loaded_test_params();
        let profile = trapezoidal_profile(Velocity::ZERO, Velocity::ZERO, 1000, &p).expect("ok");
        assert!(approx_eq(profile.peak_velocity_mm_s, 1000.0, 1e-6));
        assert_eq!(profile.cruise_time_s, 0.0);
        assert_eq!(profile.cruise_distance_mm, 0.0);
        assert!(approx_eq(profile.accel_distance_mm, 500.0, 1e-6));
        assert!(approx_eq(profile.decel_distance_mm, 500.0, 1e-6));
        assert!(approx_eq(profile.total_time_s, 2.0, 1e-9));
        assert_profile_invariants(&profile);
    }

    #[test]
    fn trapezoidal_with_cruise_profile() {
        // Case C trapezoidal: entry=0, target=0, d=4500. v_p>v_max so cruise.
        // d_a = d_d = 1125, d_c = 2250.
        let p = loaded_test_params();
        let profile = trapezoidal_profile(Velocity::ZERO, Velocity::ZERO, 4500, &p).expect("ok");
        assert_eq!(profile.peak_velocity_mm_s, 1500.0);
        assert!(approx_eq(profile.accel_distance_mm, 1125.0, 1e-6));
        assert!(approx_eq(profile.decel_distance_mm, 1125.0, 1e-6));
        assert!(approx_eq(profile.cruise_distance_mm, 2250.0, 1e-6));
        assert!(approx_eq(profile.total_time_s, 4.5, 1e-9));
        assert_profile_invariants(&profile);
    }

    #[test]
    fn position_clamps_outside_total_time() {
        // Use the trapezoidal-with-cruise profile.
        let p = loaded_test_params();
        let profile = trapezoidal_profile(Velocity::ZERO, Velocity::ZERO, 4500, &p).expect("ok");

        assert_eq!(trapezoid_position(&profile, -1.0), 0.0);
        assert_eq!(trapezoid_position(&profile, -1e9), 0.0);
        assert!(approx_eq(trapezoid_position(&profile, profile.total_time_s + 0.5), profile.total_distance_mm, 1e-9,));
        assert!(approx_eq(trapezoid_position(&profile, 1e9), profile.total_distance_mm, 1e-9,));
    }

    #[test]
    fn velocity_clamps_outside_total_time() {
        let p = loaded_test_params();
        let profile = trapezoidal_profile(Velocity::ZERO, Velocity::ZERO, 4500, &p).expect("ok");

        assert_eq!(trapezoid_velocity(&profile, -1.0), 0.0);
        assert_eq!(trapezoid_velocity(&profile, -1e9), 0.0);
        assert_eq!(trapezoid_velocity(&profile, profile.total_time_s + 0.5), 0.0);
        assert_eq!(trapezoid_velocity(&profile, 1e9), 0.0);
    }

    #[test]
    fn trapezoidal_phase_continuity() {
        // Sample inside each phase and verify positions/velocities make
        // physical sense at the phase boundaries.
        let p = loaded_test_params();
        let profile = trapezoidal_profile(Velocity::ZERO, Velocity::ZERO, 4500, &p).expect("ok");

        // End of accel: position == accel_distance, velocity == peak.
        let t_a = profile.accel_time_s;
        assert!(approx_eq(trapezoid_position(&profile, t_a), profile.accel_distance_mm, 1e-6,));
        assert!(approx_eq(trapezoid_velocity(&profile, t_a), profile.peak_velocity_mm_s, 1e-6,));

        // End of cruise: position == accel + cruise distance, velocity == peak.
        let t_c = profile.accel_time_s + profile.cruise_time_s;
        assert!(approx_eq(
            trapezoid_position(&profile, t_c),
            profile.accel_distance_mm + profile.cruise_distance_mm,
            1e-6,
        ));
        assert!(approx_eq(trapezoid_velocity(&profile, t_c), profile.peak_velocity_mm_s, 1e-6,));
    }

    #[test]
    fn decel_3b_target_nonzero_profile() {
        // Subcase 3b with target>0: entry=1500, target=500, d=2000.
        let p = loaded_test_params();
        let profile = trapezoidal_profile(Velocity(1500.0), Velocity(500.0), 2000, &p).expect("ok");
        assert_eq!(profile.exit_velocity_mm_s, 500.0);
        assert_eq!(profile.peak_velocity_mm_s, 1500.0);
        assert_eq!(profile.accel_time_s, 0.0);
        assert!(approx_eq(profile.cruise_distance_mm, 1000.0, 1e-9));
        assert!(approx_eq(profile.decel_distance_mm, 1000.0, 1e-9));
        assert!(approx_eq(profile.total_time_s, 1.0 / 1.5 + 1.0, 1e-9));
        assert_profile_invariants(&profile);
    }

    #[test]
    fn nan_t_returns_safe_defaults() {
        let p = loaded_test_params();
        let profile = trapezoidal_profile(Velocity::ZERO, Velocity::ZERO, 4500, &p).expect("ok");
        // NaN propagating through position/velocity would produce NaN
        // sample data downstream. Saturate at the start of the profile
        // instead — same convention as negative t.
        assert_eq!(trapezoid_position(&profile, f64::NAN), 0.0);
        assert_eq!(trapezoid_velocity(&profile, f64::NAN), 0.0);
    }
}
