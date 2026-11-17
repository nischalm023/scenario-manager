//! `trapezoidal_cost`, a thin wrapper over [`trapezoidal_profile`] that the
//! `Trapezoidal` drive model prices moves with.
//!
//! The search itself only needs `(traversal_duration, exit_velocity)`;
//! the full profile is consumed by Phase 4 visualization. Keeping the
//! cost wrapper around preserves every existing call-site signature.

use crate::SimDuration;

use super::{KinematicError, KinematicParams, Velocity, profile::trapezoidal_profile};

/// Compute traversal time across a cell with a trapezoidal accel/cruise/decel
/// profile.
///
/// Thin wrapper over [`trapezoidal_profile`] — extracts
/// `(profile.total_time_s, profile.exit_velocity_mm_s)` and converts to
/// `SimDuration` / `Velocity`. Public signature unchanged from the
/// pre-Phase-2 implementation; all clamp behavior (target clamped DOWN
/// for short-cell accel, UP for short-cell decel) is preserved by the
/// underlying profile constructor.
pub fn trapezoidal_cost(
    entry_v: Velocity,
    target_exit_v: Velocity,
    cell_length_mm: u64,
    params: &KinematicParams,
) -> Result<(SimDuration, Velocity), KinematicError> {
    let profile = trapezoidal_profile(entry_v, target_exit_v, cell_length_mm, params)?;
    Ok((seconds_to_simduration(profile.total_time_s), Velocity(profile.exit_velocity_mm_s)))
}

/// Convert a duration in seconds (f64) to integer-millisecond `SimDuration`.
/// Round to nearest millisecond; never produce a negative duration.
fn seconds_to_simduration(t_seconds: f64) -> SimDuration {
    debug_assert!(t_seconds >= 0.0, "negative kinematic time: {}", t_seconds);
    let ms = (t_seconds * 1000.0).round().max(0.0) as u64;
    SimDuration::ms(ms)
}

// ---------------------------------------------------------------------------
// Tests (preserved verbatim from the pre-split kinematics.rs)
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

    #[test]
    fn cruise_only_full_speed_in_and_out() {
        // entry = exit = v_max, distance such that pure cruise is the answer.
        // 1500 mm at 1500 mm/s = 1.0 s (no accel/decel needed).
        let p = loaded_test_params();
        let (t, v) = trapezoidal_cost(Velocity(1500.0), Velocity(1500.0), 1500, &p).expect("ok");
        assert_eq!(v, Velocity(1500.0));
        assert!(approx_eq(t.as_ms() as f64 / 1000.0, 1.0, 1e-3), "t = {:?}", t);
    }

    #[test]
    fn full_trapezoid_zero_to_zero_long_cell() {
        // Loaded M5F: accel 1000, decel 1000, v_max 1500. From 0 to 0 over a
        // long distance — full trapezoid with cruise phase.
        // d_a = 1500² / (2*1000) = 1125 mm. Same for decel.
        // Pick d = 4500 mm: d_c = 4500 - 1125 - 1125 = 2250 mm.
        // t = 1500/1000 + 2250/1500 + 1500/1000 = 1.5 + 1.5 + 1.5 = 4.5 s.
        let p = loaded_test_params();
        let (t, v) = trapezoidal_cost(Velocity::ZERO, Velocity::ZERO, 4500, &p).expect("ok");
        assert_eq!(v, Velocity::ZERO);
        assert!(approx_eq(t.as_ms() as f64 / 1000.0, 4.5, 1e-3), "t = {:?}", t);
    }

    #[test]
    fn triangular_zero_to_zero_short_cell() {
        // Same params, but distance < 2 * d_a so no cruise fits.
        // d = 1000 mm. Triangular peak: v_p² = (2 * 1000 * 1000 * 1000 + 0 + 0) / 2000
        //   = 2e9 / 2000 = 1e6 → v_p = 1000 mm/s (< 1500 ⇒ triangular).
        // t = 1000/1000 + 1000/1000 = 2.0 s.
        let p = loaded_test_params();
        let (t, v) = trapezoidal_cost(Velocity::ZERO, Velocity::ZERO, 1000, &p).expect("ok");
        assert_eq!(v, Velocity::ZERO);
        assert!(approx_eq(t.as_ms() as f64 / 1000.0, 2.0, 1e-3), "t = {:?}", t);
    }

    #[test]
    fn pure_accel_clamps_target_when_distance_too_short() {
        // entry = 0, target = v_max = 1500, distance = 100 mm.
        // Required accel distance for v=1500: 1500²/(2*1000) = 1125 mm. Have 100.
        // Pure accel: v_actual = sqrt(0 + 2*1000*100) = sqrt(200_000) ≈ 447.2 mm/s.
        // t = 447.2/1000 ≈ 0.447 s.
        let p = loaded_test_params();
        let (t, v) = trapezoidal_cost(Velocity::ZERO, Velocity(1500.0), 100, &p).expect("ok");
        assert!(v.mm_per_sec() < 1500.0 && v.mm_per_sec() > 440.0, "v_actual = {:?}", v);
        assert!(approx_eq(v.mm_per_sec(), (200_000f64).sqrt(), 1e-3), "v = {:?}", v);
        assert!(approx_eq(t.as_ms() as f64 / 1000.0, v.mm_per_sec() / 1000.0, 1e-3), "t = {:?}", t);
    }

    #[test]
    fn pure_decel_clamps_target_up_when_distance_too_short() {
        // entry = 1500, target = 0, distance = 100 mm.
        // Required decel distance to reach 0: 1500²/(2*1000) = 1125 mm. Have 100.
        // Pure decel: v_actual = sqrt(1500² - 2*1000*100) = sqrt(2.25e6 - 2e5)
        //           = sqrt(2.05e6) ≈ 1431.8 mm/s.
        let p = loaded_test_params();
        let (t, v) = trapezoidal_cost(Velocity(1500.0), Velocity::ZERO, 100, &p).expect("ok");
        assert!(v.mm_per_sec() > 1430.0 && v.mm_per_sec() < 1500.0, "v_actual = {:?}", v);
        assert!(approx_eq(v.mm_per_sec(), (2_050_000f64).sqrt(), 1e-3), "v = {:?}", v);
        let expected_t = (1500.0 - v.mm_per_sec()) / 1000.0;
        assert!(approx_eq(t.as_ms() as f64 / 1000.0, expected_t, 1e-3), "t = {:?}", t);
    }

    #[test]
    fn decel_subcase_3a_insufficient_distance_partial_brake() {
        // Subcase 3a: entry too hot, can only decel partway through the cell.
        // entry = 1500 mm/s, target = 0, d = 1000 mm, dec = 1000 mm/s².
        // Brake distance d_d = 1500²/2000 = 1125 mm > d.
        // actual_exit = sqrt(1500² - 2*1000*1000) = sqrt(2.5e5) ≈ 500 mm/s.
        // time = (1500 - 500)/1000 = 1.0 s.
        // (Pre-fix this branch incorrectly returned (1.5 s, 0) via "decel to a
        // stop" which left part of the cell physically un-traversed.)
        let p = loaded_test_params();
        let (t, v) = trapezoidal_cost(Velocity(1500.0), Velocity::ZERO, 1000, &p).expect("ok");
        assert!(approx_eq(v.mm_per_sec(), 500.0, 1e-3), "v = {:?}", v);
        assert!(approx_eq(t.as_ms() as f64 / 1000.0, 1.0, 1e-3), "t = {:?}", t);
    }

    #[test]
    fn decel_subcase_3b_with_target_zero_extra_distance() {
        // Subcase 3b: enough room to brake to zero, with cruise distance left over.
        // entry = 1500 mm/s, target = 0, d = 2000 mm, dec = 1000 mm/s².
        // d_d = 1500²/2000 = 1125 mm. cruise = 2000 - 1125 = 875 mm.
        // time = 875/1500 + 1500/1000 = 0.5833... + 1.5 = 2.0833... s.
        let p = loaded_test_params();
        let (t, v) = trapezoidal_cost(Velocity(1500.0), Velocity::ZERO, 2000, &p).expect("ok");
        assert_eq!(v, Velocity::ZERO);
        let expected = 875.0 / 1500.0 + 1.5;
        assert!(approx_eq(t.as_ms() as f64 / 1000.0, expected, 1e-3), "t = {:?}, expected {}", t, expected);
    }

    #[test]
    fn decel_subcase_3b_target_nonzero_boundary() {
        // Subcase 3b boundary: brake distance exactly equals cell length, so
        // there is no cruise leg.
        // entry = 1500 mm/s, target = 500 mm/s, d = 1000 mm, dec = 1000 mm/s².
        // d_d = (1500² - 500²)/2000 = (2.25e6 - 2.5e5)/2000 = 1000 mm = d.
        // cruise = 0, time = 0 + (1500 - 500)/1000 = 1.0 s.
        let p = loaded_test_params();
        let (t, v) = trapezoidal_cost(Velocity(1500.0), Velocity(500.0), 1000, &p).expect("ok");
        assert_eq!(v, Velocity(500.0));
        assert!(approx_eq(t.as_ms() as f64 / 1000.0, 1.0, 1e-3), "t = {:?}", t);
    }

    #[test]
    fn decel_subcase_3b_target_nonzero_with_cruise() {
        // Subcase 3b: target > 0, and cell longer than brake distance.
        // entry = 1500 mm/s, target = 500 mm/s, d = 2000 mm, dec = 1000 mm/s².
        // d_d = (1500² - 500²)/2000 = 1000 mm. cruise = 2000 - 1000 = 1000 mm.
        // time = 1000/1500 + 1000/1000 = 0.6667 + 1.0 = 1.6667 s.
        let p = loaded_test_params();
        let (t, v) = trapezoidal_cost(Velocity(1500.0), Velocity(500.0), 2000, &p).expect("ok");
        assert_eq!(v, Velocity(500.0));
        let expected = 1000.0 / 1500.0 + 1.0;
        assert!(approx_eq(t.as_ms() as f64 / 1000.0, expected, 1e-3), "t = {:?}, expected {}", t, expected);
    }

    #[test]
    fn entry_exceeds_max_velocity_errors() {
        let p = loaded_test_params(); // v_max = 1500
        let r = trapezoidal_cost(Velocity(2000.0), Velocity::ZERO, 1000, &p);
        assert_eq!(r, Err(KinematicError::EntryExceedsMaxVelocity));
    }

    #[test]
    fn zero_acceleration_errors() {
        let bad = KinematicParams { max_velocity_mm_s: 1000.0, acceleration_mm_s2: 0.0, deceleration_mm_s2: 1000.0 };
        let r = trapezoidal_cost(Velocity::ZERO, Velocity::ZERO, 1000, &bad);
        assert_eq!(r, Err(KinematicError::NonPositiveAcceleration));
    }

    #[test]
    fn non_finite_param_errors_distinct_from_non_positive() {
        // The finiteness guard runs before the `<= 0.0` checks because
        // `NaN <= 0.0` is false: reordering the checks would silently let
        // NaN propagate through the profile math and produce NaN cell
        // costs. These cases lock the ordering down.
        let nan_v_max =
            KinematicParams { max_velocity_mm_s: f64::NAN, acceleration_mm_s2: 1000.0, deceleration_mm_s2: 1000.0 };
        assert_eq!(
            trapezoidal_cost(Velocity::ZERO, Velocity::ZERO, 1000, &nan_v_max),
            Err(KinematicError::NonFiniteParam),
        );

        let nan_accel =
            KinematicParams { max_velocity_mm_s: 1000.0, acceleration_mm_s2: f64::NAN, deceleration_mm_s2: 1000.0 };
        assert_eq!(
            trapezoidal_cost(Velocity::ZERO, Velocity::ZERO, 1000, &nan_accel),
            Err(KinematicError::NonFiniteParam),
        );

        let nan_decel =
            KinematicParams { max_velocity_mm_s: 1000.0, acceleration_mm_s2: 1000.0, deceleration_mm_s2: f64::NAN };
        assert_eq!(
            trapezoidal_cost(Velocity::ZERO, Velocity::ZERO, 1000, &nan_decel),
            Err(KinematicError::NonFiniteParam),
        );

        let pos_inf = KinematicParams {
            max_velocity_mm_s: f64::INFINITY,
            acceleration_mm_s2: 1000.0,
            deceleration_mm_s2: 1000.0,
        };
        assert_eq!(
            trapezoidal_cost(Velocity::ZERO, Velocity::ZERO, 1000, &pos_inf),
            Err(KinematicError::NonFiniteParam),
        );

        let neg_inf = KinematicParams {
            max_velocity_mm_s: 1000.0,
            acceleration_mm_s2: f64::NEG_INFINITY,
            deceleration_mm_s2: 1000.0,
        };
        assert_eq!(
            trapezoidal_cost(Velocity::ZERO, Velocity::ZERO, 1000, &neg_inf),
            Err(KinematicError::NonFiniteParam),
        );
    }

    #[test]
    fn zero_distance_returns_zero_time_and_entry_velocity() {
        let p = loaded_test_params();
        let (t, v) = trapezoidal_cost(Velocity(500.0), Velocity::ZERO, 0, &p).expect("ok");
        assert_eq!(t, SimDuration::ZERO);
        assert_eq!(v, Velocity(500.0));
    }
}
