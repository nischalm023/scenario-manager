//! Work scoring: proximity-weighted utility functions for dispatch decisions.
//!
//! Lifted from actors/dispatcher/matching/scoring.rs and generalized.
//! The scoring function determines priority when multiple work candidates
//! compete for the same resource (bot, relay, VTM).

use crate::config::VtmScoringConfig;

/// Score a VTM work candidate (forward retrieval or return to rack).
///
/// Returns a utility score where higher = better.
///
/// The type_weight creates a natural forward/return balance based on relay
/// zone pressure (p). Forward dominates at low pressure (p < ~0.48),
/// Return dominates at high pressure. This prevents zone deadlock by
/// prioritizing returns when zones are filling up.
///
/// ```text
/// score = proximity * type_weight + age_bonus
///
/// Forward type_weight = (1 - p²) / (1 + contention_penalty * vtms_in_flight)
/// Return type_weight  = return_base + return_urgency * p²
/// age_bonus           = min(unclaimed_secs / decay_secs, max_bonus)
/// ```
pub fn score_vtm_work(
    proximity: f64,
    pressure: f64,
    is_forward: bool,
    unclaimed_secs: f64,
    vtms_in_flight: u32,
    config: &VtmScoringConfig,
) -> f64 {
    let p = pressure.clamp(0.0, 1.0);

    let type_weight = if is_forward {
        let base = 1.0 - p * p;
        base / (1.0 + config.zone_contention_penalty * vtms_in_flight as f64)
    } else {
        config.return_base + config.return_urgency * p * p
    };

    let age_bonus = (unclaimed_secs / config.starvation_decay_secs).min(config.starvation_max_bonus);

    proximity * type_weight + age_bonus
}

/// Compute zone pressure: ratio of occupied+reserved to capacity.
///
/// `reservation_discount` controls how much reserved-but-empty slots
/// count toward pressure (0.7 = reserved slots count as 70% full).
pub fn zone_pressure(occupied: u32, reserved: u32, capacity: u32, reservation_discount: f64) -> f64 {
    if capacity == 0 {
        return 0.0;
    }
    let used = occupied as f64 + reservation_discount * reserved as f64;
    (used / capacity as f64).clamp(0.0, 1.0)
}

/// Check if a zone is at deadlock risk.
///
/// Returns true if free capacity <= headroom (RAW count, not discounted).
/// Forward work should be blocked when this returns true.
/// Headroom is capped at capacity-1 so zones with capacity <= headroom
/// are not permanently blocked (adversary review H1).
pub fn is_deadlock_risk(occupied: u32, reserved: u32, capacity: u32, headroom: u32) -> bool {
    let effective_headroom = headroom.min(capacity.saturating_sub(1));
    let free = capacity.saturating_sub(occupied + reserved);
    free <= effective_headroom
}

/// Normalize proximity: 0.0 = max_dist away, 1.0 = at the target.
pub fn normalize_proximity(distance_mm: f64, max_travel_dist: f64) -> f64 {
    if max_travel_dist <= 0.0 {
        return 0.0;
    }
    (1.0 - distance_mm / max_travel_dist).clamp(0.0, 1.0)
}

/// Score a relay for VTM forward cycle.
///
/// Balances VTM travel distance (60%) with HTM accessibility (40%).
/// Lower score = better relay.
pub fn score_relay_for_forward(vtm_distance: f64, htm_distance: f64) -> f64 {
    0.6 * vtm_distance + 0.4 * htm_distance
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> VtmScoringConfig {
        VtmScoringConfig::default()
    }

    #[test]
    fn crossover_at_048() {
        let cfg = default_config();
        let fwd = score_vtm_work(1.0, 0.48, true, 0.0, 0, &cfg);
        let ret = score_vtm_work(1.0, 0.48, false, 0.0, 0, &cfg);
        assert!((fwd - ret).abs() < 0.05, "Crossover gap at p=0.48: fwd={fwd:.3}, ret={ret:.3}");
    }

    #[test]
    fn forward_dominates_low_pressure() {
        let cfg = default_config();
        let fwd = score_vtm_work(1.0, 0.0, true, 0.0, 0, &cfg);
        let ret = score_vtm_work(1.0, 0.0, false, 0.0, 0, &cfg);
        assert!(fwd > ret, "Forward should dominate at p=0: fwd={fwd}, ret={ret}");
    }

    #[test]
    fn return_dominates_high_pressure() {
        let cfg = default_config();
        let fwd = score_vtm_work(1.0, 0.9, true, 0.0, 0, &cfg);
        let ret = score_vtm_work(1.0, 0.9, false, 0.0, 0, &cfg);
        assert!(ret > fwd, "Return should dominate at p=0.9: fwd={fwd}, ret={ret}");
    }

    #[test]
    fn age_bonus_growth_and_cap() {
        let cfg = default_config();
        let s0 = score_vtm_work(0.5, 0.5, true, 0.0, 0, &cfg);
        let s30 = score_vtm_work(0.5, 0.5, true, 30.0, 0, &cfg);
        let s120 = score_vtm_work(0.5, 0.5, true, 120.0, 0, &cfg);
        assert!(s30 > s0, "Age bonus should increase score");
        assert!((s120 - s30).abs() < 0.5 + 0.01, "Age bonus should cap");
    }

    #[test]
    fn zone_contention_penalty_reduces_forward() {
        let cfg = default_config();
        let s0 = score_vtm_work(1.0, 0.3, true, 0.0, 0, &cfg);
        let s3 = score_vtm_work(1.0, 0.3, true, 0.0, 3, &cfg);
        assert!(s3 < s0, "Contention penalty should reduce forward score");
    }

    #[test]
    fn contention_does_not_affect_return() {
        let cfg = default_config();
        let s0 = score_vtm_work(1.0, 0.7, false, 0.0, 0, &cfg);
        let s3 = score_vtm_work(1.0, 0.7, false, 0.0, 3, &cfg);
        assert_eq!(s0, s3, "Contention should not affect return scores");
    }

    #[test]
    fn zone_pressure_calculation() {
        assert_eq!(zone_pressure(5, 3, 20, 0.7), (5.0 + 0.7 * 3.0) / 20.0);
        assert_eq!(zone_pressure(0, 0, 0, 0.7), 0.0);
        assert_eq!(zone_pressure(20, 0, 20, 0.7), 1.0);
    }

    #[test]
    fn deadlock_risk_detection() {
        assert!(!is_deadlock_risk(5, 3, 20, 2)); // 12 free > 2
        assert!(is_deadlock_risk(17, 1, 20, 2)); // 2 free <= 2
        assert!(is_deadlock_risk(19, 1, 20, 2)); // 0 free <= 2
    }

    #[test]
    fn proximity_normalization() {
        assert_eq!(normalize_proximity(0.0, 150_000.0), 1.0);
        assert_eq!(normalize_proximity(150_000.0, 150_000.0), 0.0);
        assert!((normalize_proximity(75_000.0, 150_000.0) - 0.5).abs() < 0.001);
    }
}
