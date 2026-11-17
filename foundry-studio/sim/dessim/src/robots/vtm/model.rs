//! One catalog model's VTM motion. The chassis keeps its floor or rack-ladder
//! identity; the cycle composes its translation with a simultaneous lift and
//! operation-specific handling. Floor translation here is confined to the
//! assigned aisle. It does not model floor turns or collision reservations.

use super::motion::{self, CycleBreakdown, CycleTimes};

/// A validated, rest-to-rest translation profile, in metres and seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AxisMotion {
    Constant { speed_m_s: f64 },
    Trapezoidal { max_speed_m_s: f64, accel_m_s2: f64 },
}

impl AxisMotion {
    pub fn travel_secs(self, distance_m: f64) -> f64 {
        if distance_m <= 0.0 {
            return 0.0;
        }
        match self {
            Self::Constant { speed_m_s } => distance_m / speed_m_s,
            Self::Trapezoidal { max_speed_m_s, accel_m_s2 } => {
                motion::trapezoidal_time(distance_m, max_speed_m_s, accel_m_s2)
            }
        }
    }
}

/// Rack-attached chassis translation. The source ladder profile applies both
/// with and without a tote; it states no separate load profiles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LadderMotionModel {
    pub max_speed_m_s: f64,
    pub accel_m_s2: f64,
}

impl LadderMotionModel {
    fn travel_secs(self, distance_m: f64) -> f64 {
        motion::trapezoidal_time(distance_m, self.max_speed_m_s, self.accel_m_s2)
    }
}

/// A floor-driving VTM keeps the source's distinct empty and laden profiles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloorVtmMotionModel {
    pub empty: AxisMotion,
    pub loaded: AxisMotion,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VtmChassis {
    Floor(FloorVtmMotionModel),
    Ladder(LadderMotionModel),
}

impl VtmChassis {
    pub fn travel_secs(self, distance_m: f64, loaded: bool) -> f64 {
        match self {
            Self::Floor(floor) => if loaded { floor.loaded } else { floor.empty }.travel_secs(distance_m),
            Self::Ladder(ladder) => ladder.travel_secs(distance_m),
        }
    }
}

/// Handling follows the operation: a return picks at the relay and drops at
/// the rack, whereas a forward cycle picks at the rack and drops at the relay.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HandlingTimes {
    pub pick_secs: f64,
    pub drop_secs: f64,
    pub pick_alignment_secs: f64,
    pub drop_alignment_secs: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VtmMotion {
    pub chassis: VtmChassis,
    pub lift: AxisMotion,
    pub handling: HandlingTimes,
}

impl VtmMotion {
    pub fn forward_cycle(
        self,
        to_rack_m: f64,
        rack_to_relay_m: f64,
        tote_height_m: f64,
        relay_height_m: f64,
    ) -> CycleBreakdown {
        motion::forward_cycle_with_model(to_rack_m, rack_to_relay_m, tote_height_m, relay_height_m, self)
    }

    pub fn return_cycle(
        self,
        to_relay_m: f64,
        relay_to_rack_m: f64,
        tote_height_m: f64,
        relay_height_m: f64,
    ) -> CycleBreakdown {
        motion::return_cycle_with_model(to_relay_m, relay_to_rack_m, tote_height_m, relay_height_m, self)
    }
}

/// Compatibility boundary for old settings and small diagnostic fixtures.
/// Catalog models are constructed directly from their validated components.
impl From<CycleTimes> for VtmMotion {
    fn from(times: CycleTimes) -> Self {
        Self {
            chassis: VtmChassis::Ladder(LadderMotionModel {
                max_speed_m_s: times.chassis_speed,
                accel_m_s2: times.chassis_accel,
            }),
            lift: AxisMotion::Trapezoidal { max_speed_m_s: times.lift_speed, accel_m_s2: times.lift_accel },
            handling: HandlingTimes {
                pick_secs: times.pick_secs,
                drop_secs: times.drop_secs,
                pick_alignment_secs: times.pick_alignment_secs,
                drop_alignment_secs: times.drop_alignment_secs,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floor(empty: AxisMotion, loaded: AxisMotion) -> VtmMotion {
        VtmMotion {
            chassis: VtmChassis::Floor(FloorVtmMotionModel { empty, loaded }),
            lift: AxisMotion::Trapezoidal { max_speed_m_s: 2.0, accel_m_s2: 1.0 },
            handling: HandlingTimes {
                pick_secs: 1.0,
                drop_secs: 2.0,
                pick_alignment_secs: 0.25,
                drop_alignment_secs: 0.5,
            },
        }
    }

    #[test]
    fn constant_floor_uses_empty_then_laden_in_both_directions() {
        let model = floor(AxisMotion::Constant { speed_m_s: 2.0 }, AxisMotion::Constant { speed_m_s: 1.0 });
        let forward = model.forward_cycle(6.0, 10.0, 0.0, 0.0);
        let returning = model.return_cycle(6.0, 10.0, 0.0, 0.0);
        for cycle in [&forward, &returning] {
            assert_eq!(cycle.first_travel_secs, 3.0);
            assert_eq!(cycle.second_travel_secs, 10.0);
            assert_eq!(cycle.total_secs, 16.75);
        }
        assert_eq!(forward.rack_transfer_secs, returning.relay_transfer_secs);
        assert_eq!(forward.relay_alignment_secs, returning.rack_alignment_secs);
    }

    #[test]
    fn trapezoidal_floor_keeps_its_two_load_profiles() {
        let model = floor(
            AxisMotion::Trapezoidal { max_speed_m_s: 4.0, accel_m_s2: 2.0 },
            AxisMotion::Trapezoidal { max_speed_m_s: 1.0, accel_m_s2: 1.0 },
        );
        let cycle = model.forward_cycle(10.0, 10.0, 0.0, 0.0);
        assert_eq!(cycle.first_travel_secs, 4.5);
        assert_eq!(cycle.second_travel_secs, 11.0);
        assert_eq!(cycle.total_secs, 19.25);
        assert!(matches!(model.chassis, VtmChassis::Floor(_)));
    }

    #[test]
    fn ladder_is_distinct_and_lift_runs_concurrently() {
        let model = VtmMotion {
            chassis: VtmChassis::Ladder(LadderMotionModel { max_speed_m_s: 3.0, accel_m_s2: 2.0 }),
            ..floor(AxisMotion::Constant { speed_m_s: 2.0 }, AxisMotion::Constant { speed_m_s: 1.0 })
        };
        let cycle = model.forward_cycle(0.0, 0.0, 4.0, 0.0);
        assert_eq!(cycle.first_travel_secs, 4.0);
        assert_eq!(cycle.second_travel_secs, 4.0);
        assert_eq!(cycle.h_travel_secs, 0.0);
        assert_eq!(cycle.total_secs, 11.75);
        assert!(matches!(model.chassis, VtmChassis::Ladder(_)));
        assert_eq!(model.chassis.travel_secs(10.0, false), model.chassis.travel_secs(10.0, true));
    }

    #[test]
    fn legacy_cencora_profile_keeps_exact_cycle_arithmetic() {
        // Frozen pre-model cycle at 12 m per leg, 5 m shelf and 0.39 m relay.
        // Both horizontal legs dominate; the previous formula yields 5.5 s.
        let model = VtmMotion::from(CycleTimes {
            pick_secs: 6.2,
            drop_secs: 5.0,
            pick_alignment_secs: 3.0,
            drop_alignment_secs: 3.0,
            chassis_speed: 3.0,
            chassis_accel: 2.0,
            lift_speed: 3.0,
            lift_accel: 2.0,
        });
        let cycle = model.forward_cycle(12.0, 12.0, 5.0, 0.39);
        assert_eq!(cycle.first_travel_secs, 5.5);
        assert_eq!(cycle.second_travel_secs, 5.5);
        assert_eq!(cycle.total_secs, 28.2);
    }
}
