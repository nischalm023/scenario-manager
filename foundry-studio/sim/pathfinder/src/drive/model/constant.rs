use desgraph::types::{CellId, RobotType, ZoneId};
use desgraph::{Cell, Edge};

use super::{at_quarter_turn_rate, at_top_speed, debug_assert_within_speed, may_rotate_on, may_traverse, reverses_on};
use crate::SimDuration;
use crate::drive::kinematics::{HtmKinematics, Velocity};
use crate::drive::{DriveModel, MotionKind};

/// Constant speed, priced in whole mm/s (the speed is truncated). No acceleration: entry and
/// target velocities are ignored, and every move exits at the cruise speed. Neither may exceed
/// the cruise speed.
///
/// One robot type in one load state: `robot_type` and `carrying_cargo` say which, and decide
/// the edges and turns the robot may take.
#[derive(Clone, Copy, Debug)]
pub struct Constant {
    pub speed: Velocity,
    pub turn_duration: SimDuration,
    pub reversal_duration: Option<SimDuration>,
    pub robot_type: RobotType,
    pub carrying_cargo: bool,
}

impl Constant {
    /// An HTM's constant drive in one load state, from the figures the run states for its HTMs.
    /// The speed is taken in whole mm/s, as the search always priced it.
    // TODO: round rather than truncate.
    pub fn htm(figures: &HtmKinematics, carrying_cargo: bool) -> Self {
        Self {
            speed: Velocity((figures.constant_speed_mm_s(carrying_cargo) as u64) as f64),
            turn_duration: figures.turn_duration_for(carrying_cargo),
            reversal_duration: figures.reversal_duration_for(carrying_cargo),
            robot_type: RobotType::Htm,
            carrying_cargo,
        }
    }
}

impl DriveModel for Constant {
    fn kind(&self) -> MotionKind {
        MotionKind::Constant
    }

    fn max_speed(&self) -> Velocity {
        self.speed
    }

    fn kinematic_cost(
        &self,
        entry_v: Velocity,
        target_exit_v: Velocity,
        cell_length_mm: u64,
    ) -> (SimDuration, Velocity) {
        debug_assert_within_speed("constant", self.speed, entry_v, target_exit_v);
        let speed_mm_per_sec = self.speed.mm_per_sec() as u64;
        (SimDuration::ms(cell_length_mm * 1000 / speed_mm_per_sec), Velocity(speed_mm_per_sec as f64))
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
    fn constant_ignores_velocities_and_uses_raw_speed() {
        let (t, v) = Constant {
            speed: Velocity(1000.0),
            turn_duration: SimDuration::ms(1000),
            reversal_duration: None,
            robot_type: RobotType::Htm,
            carrying_cargo: false,
        }
        .kinematic_cost(Velocity::ZERO, Velocity::ZERO, 1000);
        assert_eq!(t, SimDuration::ms(1000));
        assert_eq!(v.mm_per_sec(), 1000.0);
    }

    #[test]
    fn constant_truncates_its_speed_to_whole_mm_per_s() {
        let (t, v) = Constant {
            speed: Velocity(1000.9),
            turn_duration: SimDuration::ms(1000),
            reversal_duration: None,
            robot_type: RobotType::Htm,
            carrying_cargo: false,
        }
        .kinematic_cost(Velocity::ZERO, Velocity::ZERO, 1000);
        assert_eq!(t, SimDuration::ms(1000));
        assert_eq!(v.mm_per_sec(), 1000.0);
    }

    #[test]
    fn constant_drives_are_built_in_whole_mm_per_s() {
        let htm =
            HtmKinematics { empty_constant_speed_mm_s: 2500.7, ..HtmKinematics::uniform(1000, SimDuration::ms(1000)) };
        assert_eq!(Constant::htm(&htm, false).speed, Velocity(2500.0));
        assert_eq!(Constant::htm(&htm, true).speed, Velocity(1000.0));
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "constant drive at")]
    fn constant_refuses_a_velocity_above_its_speed() {
        Constant {
            speed: Velocity(1000.0),
            turn_duration: SimDuration::ms(1000),
            reversal_duration: None,
            robot_type: RobotType::Htm,
            carrying_cargo: false,
        }
        .kinematic_cost(Velocity(1500.0), Velocity::ZERO, 1000);
    }
}
