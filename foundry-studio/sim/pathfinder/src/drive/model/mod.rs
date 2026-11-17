//! Within `drive`, the models are the drives runs use today, [`Constant`] and [`Trapezoidal`],
//! and the arithmetic and admissibility rules both share.

mod constant;
mod trapezoidal;

pub use constant::Constant;
pub use trapezoidal::Trapezoidal;

use desgraph::types::{CellId, HtmTraversal, RobotType, ZoneId};
use desgraph::{Cell, Edge, HtmDriveDirection};

use crate::SimDuration;
use crate::drive::kinematics::Velocity;

/// The time to cover `distance_mm` at `top_speed` throughout, in whole mm/s and whole
/// milliseconds, both truncated: no move can be faster, so it bounds `kinematic_cost` from below.
fn at_top_speed(top_speed: Velocity, distance_mm: u64) -> SimDuration {
    SimDuration::ms(distance_mm * 1000 / (top_speed.mm_per_sec() as u64).max(1))
}

/// The time to turn `angle_millidegrees` when a quarter turn takes `quarter_turn`: the exact
/// angular fraction, rounded up to the next millisecond.
fn at_quarter_turn_rate(quarter_turn: SimDuration, angle_millidegrees: u32) -> SimDuration {
    SimDuration::ms((quarter_turn.as_ms() as u128 * angle_millidegrees as u128).div_ceil(90_000) as u64)
}

/// Whether a robot of `robot_type`, laden when `carrying_cargo`, may take `edge` toward `dest`,
/// which lies in `dest_zone`: both drives answer `DriveModel::may_traverse` with it.
fn may_traverse(robot_type: RobotType, carrying_cargo: bool, edge: &Edge, dest: CellId, dest_zone: ZoneId) -> bool {
    match robot_type {
        RobotType::Vtm => {
            if !edge.traversability.vtm_traversal {
                return false;
            }
        }
        RobotType::Htm => {
            if carrying_cargo {
                if edge.traversability.htm_traversal != HtmTraversal::Both {
                    return false;
                }
            } else if edge.traversability.htm_traversal == HtmTraversal::None {
                return false;
            }
        }
        RobotType::Rtp => {
            if !edge.traversability.rtp_traversal {
                return false;
            }
        }
    }
    if let Some(gate) = edge.dest_only {
        if !gate.admits(dest) {
            return false;
        }
    }
    if robot_type == RobotType::Vtm {
        if let Some(required_zone) = edge.vtm_zone_gate {
            if dest_zone != required_zone {
                return false;
            }
        }
    }
    true
}

/// An HTM may not turn in place on a cell that forbids it; any other robot may.
fn may_rotate_on(robot_type: RobotType, cell: &Cell) -> bool {
    !(robot_type == RobotType::Htm && cell.htm_rotation_forbidden)
}

/// An HTM drives an edge marked for reverse travel backwards; any other robot ignores the mark.
fn reverses_on(robot_type: RobotType, edge: &Edge) -> bool {
    robot_type == RobotType::Htm && edge.htm_drive == HtmDriveDirection::Reverse
}

/// A drive is never asked to enter or leave faster than it can go (within the profile's f64
/// round-trip tolerance); doing so means the caller has the wrong drive.
fn debug_assert_within_speed(drive: &str, max: Velocity, entry_v: Velocity, target_exit_v: Velocity) {
    const TOLERANCE_MM_S: f64 = 1e-6;
    debug_assert!(
        entry_v.mm_per_sec() <= max.mm_per_sec() + TOLERANCE_MM_S
            && target_exit_v.mm_per_sec() <= max.mm_per_sec() + TOLERANCE_MM_S,
        "{drive} drive at {max:?} asked to enter at {entry_v:?} or leave at {target_exit_v:?}",
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drive::DriveModel;
    use crate::drive::kinematics::KinematicParams;

    #[test]
    fn a_turn_costs_its_angle_at_the_quarter_turn_rate() {
        let drive = Constant {
            speed: Velocity(1000.0),
            turn_duration: SimDuration::ms(1000),
            reversal_duration: None,
            robot_type: RobotType::Htm,
            carrying_cargo: false,
        };
        assert_eq!(drive.rotation_cost(45_000), SimDuration::ms(500));
        assert_eq!(drive.rotation_cost(90_000), SimDuration::ms(1000));
        assert_eq!(drive.rotation_cost(1), SimDuration::ms(1));
    }

    #[test]
    fn min_travel_time_is_distance_over_top_speed() {
        let constant = Constant {
            speed: Velocity(1000.0),
            turn_duration: SimDuration::ms(1000),
            reversal_duration: None,
            robot_type: RobotType::Htm,
            carrying_cargo: false,
        };
        assert_eq!(constant.min_travel_time(1500), SimDuration::ms(1500));
        let params =
            KinematicParams { max_velocity_mm_s: 1500.9, acceleration_mm_s2: 1000.0, deceleration_mm_s2: 1000.0 };
        let trapezoidal = Trapezoidal {
            params,
            turn_duration: SimDuration::ms(1000),
            reversal_duration: None,
            robot_type: RobotType::Htm,
            carrying_cargo: false,
        };
        assert_eq!(trapezoidal.min_travel_time(4500), SimDuration::ms(3000));
        assert!(
            trapezoidal.min_travel_time(4500) <= trapezoidal.kinematic_cost(Velocity::ZERO, Velocity(1500.9), 4500).0
        );
    }
}
