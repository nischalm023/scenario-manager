use desgraph::types::{CellId, RobotType, ZoneId};
use desgraph::{Cell, Edge};

use super::{at_quarter_turn_rate, at_top_speed, debug_assert_within_speed, may_rotate_on, may_traverse, reverses_on};
use crate::SimDuration;
use crate::drive::kinematics::{HtmKinematics, KinematicParams, Velocity, trapezoidal_cost};
use crate::drive::{DriveModel, MotionKind};

/// Accelerate, cruise, decelerate. Neither the entry nor the target velocity may exceed the top
/// speed: an entry above it is refused by the profile, a target above it would be clamped.
///
/// One robot type in one load state: `robot_type` and `carrying_cargo` say which, and decide
/// the edges and turns the robot may take.
#[derive(Clone, Copy, Debug)]
pub struct Trapezoidal {
    pub params: KinematicParams,
    pub turn_duration: SimDuration,
    pub reversal_duration: Option<SimDuration>,
    pub robot_type: RobotType,
    pub carrying_cargo: bool,
}

impl Trapezoidal {
    /// An HTM's trapezoidal drive in one load state, from the figures the run states for its
    /// HTMs.
    pub fn htm(figures: &HtmKinematics, carrying_cargo: bool) -> Self {
        Self {
            params: if carrying_cargo { figures.loaded } else { figures.unloaded },
            turn_duration: figures.turn_duration_for(carrying_cargo),
            reversal_duration: figures.reversal_duration_for(carrying_cargo),
            robot_type: RobotType::Htm,
            carrying_cargo,
        }
    }
}

impl DriveModel for Trapezoidal {
    fn kind(&self) -> MotionKind {
        MotionKind::Kinematic
    }

    fn max_speed(&self) -> Velocity {
        Velocity(self.params.max_velocity_mm_s)
    }

    /// Panics on params the profile rejects (zero or non-finite accel, decel or top speed, or
    /// entry above top speed).
    fn kinematic_cost(
        &self,
        entry_v: Velocity,
        target_exit_v: Velocity,
        cell_length_mm: u64,
    ) -> (SimDuration, Velocity) {
        debug_assert_within_speed("trapezoidal", self.max_speed(), entry_v, target_exit_v);
        // TODO: return an error instead of panicking on params the profile rejects.
        trapezoidal_cost(entry_v, target_exit_v, cell_length_mm, &self.params)
            .expect("trapezoidal_cost: invalid kinematic params (zero accel/decel/v_max or v_in > v_max)")
    }
    fn rotation_cost(&self, angle_millidegrees: u32) -> SimDuration {
        at_quarter_turn_rate(self.turn_duration, angle_millidegrees)
    }

    fn reversal_duration_for(&self) -> Option<SimDuration> {
        self.reversal_duration
    }
    fn min_travel_time(&self, distance_mm: u64) -> SimDuration {
        at_top_speed(self.max_speed(), distance_mm)
    }

    fn may_traverse(&self, edge: &Edge, dest: CellId, dest_zone: ZoneId) -> bool {
        may_traverse(self.robot_type, self.carrying_cargo, edge, dest, dest_zone)
    }

    fn may_rotate_on(&self, cell: &Cell) -> bool {
        may_rotate_on(self.robot_type, cell)
    }

    fn reverses_on(&self, edge: &Edge) -> bool {
        reverses_on(self.robot_type, edge)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "trapezoidal drive at")]
    fn trapezoidal_refuses_a_target_above_its_top_speed() {
        let params =
            KinematicParams { max_velocity_mm_s: 1500.0, acceleration_mm_s2: 1000.0, deceleration_mm_s2: 1000.0 };
        Trapezoidal {
            params,
            turn_duration: SimDuration::ms(1000),
            reversal_duration: None,
            robot_type: RobotType::Htm,
            carrying_cargo: false,
        }
        .kinematic_cost(Velocity::ZERO, Velocity(2000.0), 4500);
    }

    #[test]
    fn trapezoidal_is_the_trapezoid_profile() {
        let params =
            KinematicParams { max_velocity_mm_s: 1500.0, acceleration_mm_s2: 1000.0, deceleration_mm_s2: 1000.0 };
        assert_eq!(
            Trapezoidal {
                params,
                turn_duration: SimDuration::ms(1000),
                reversal_duration: None,
                robot_type: RobotType::Htm,
                carrying_cargo: false,
            }
            .kinematic_cost(Velocity::ZERO, Velocity(1500.0), 4500),
            trapezoidal_cost(Velocity::ZERO, Velocity(1500.0), 4500, &params).unwrap(),
        );
    }
}
