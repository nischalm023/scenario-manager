//! What the traffic overlay measures: each move a robot made in the slice,
//! with the speed profile it was driven by.
//!
//! A step list says which cells a mover reached and when, which is everything
//! playback needs and nothing a speed can be read from. The overlay bins
//! distance and time by speed, so it needs the profile the planner actually
//! drove each leg with — entry, exit and peak speed, and how long the
//! machine spent accelerating, cruising and slowing down. That is what these
//! records carry, one entry per robot that moved in the slice.
//!
//! The spans mirror the simulator's own per-move records, but nothing here
//! depends on the simulator: a consumer reads them with this crate alone.

use desgraph::types::CellId;
use desunits::{RobotId, SimTime};
use serde::{Deserialize, Serialize, Serializer};

use crate::container::payload::Payload;
use crate::frames::in_id_order;

/// The trapezoid a leg was driven as: three phases with their velocities,
/// their times and their distances, then the totals and the two accelerations
/// the planner was given.
///
/// Every field the planner computed travels, in the order it states them, and
/// a consumer reads what it needs rather than recomputing it. The phase times
/// sum to `total_time_s` and the phase distances to `total_distance_mm`,
/// modulo the rounding of the arithmetic that produced them — which is why
/// the totals are carried and not derived. Any phase may be zero: a move with
/// no cruise, or no motion at all, is this same shape with zeros in it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MotionProfile {
    pub entry_velocity_mm_s: f64,
    pub exit_velocity_mm_s: f64,
    /// The cruise velocity, which is also the peak of the accelerating phase
    /// and the start of the slowing one.
    pub peak_velocity_mm_s: f64,
    pub accel_time_s: f64,
    pub cruise_time_s: f64,
    pub decel_time_s: f64,
    pub accel_distance_mm: f64,
    pub cruise_distance_mm: f64,
    pub decel_distance_mm: f64,
    pub total_time_s: f64,
    pub total_distance_mm: f64,
    /// The accelerations the move was planned with, echoed from the robot's
    /// own parameters; a phase of zero time leaves no way to recover them.
    pub acceleration_mm_s2: f64,
    pub deceleration_mm_s2: f64,
}

/// One committed move: from where to where, between which instants, how far,
/// and how it was driven. The profile is nil for a move the planner did not
/// give one — the consumer then reads the move as one constant speed.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MotionSpan {
    pub source: CellId,
    pub destination: CellId,
    pub start: SimTime,
    pub end: SimTime,
    pub distance_mm: f64,
    pub profile: Option<MotionProfile>,
}

/// One robot's moves within a slice, in the order it made them.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct RobotMotion {
    pub robot: RobotId,
    pub spans: Vec<MotionSpan>,
}

/// Every robot that moved in the slice, in ascending id. A robot that made no
/// move has no entry.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct MotionDelta(pub Vec<RobotMotion>);

impl Serialize for RobotMotion {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        (self.robot, in_id_order(&self.spans, |span| (span.start.as_ms(), span.end.as_ms()))).serialize(serializer)
    }
}

impl Serialize for MotionDelta {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        in_id_order(&self.0, |motion| motion.robot).serialize(serializer)
    }
}

impl MotionDelta {
    /// One robot's moves in this slice, or `None` when it made none.
    pub fn get(&self, robot: RobotId) -> Option<&RobotMotion> {
        self.0.iter().find(|motion| motion.robot == robot)
    }
}

impl Payload for MotionDelta {
    const NAME: &'static str = "motion delta";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(start: u64, distance_mm: f64) -> MotionSpan {
        MotionSpan {
            source: CellId(1),
            destination: CellId(2),
            start: SimTime::ms(start),
            end: SimTime::ms(start + 1_000),
            distance_mm,
            profile: Some(MotionProfile {
                entry_velocity_mm_s: 0.0,
                exit_velocity_mm_s: 0.0,
                peak_velocity_mm_s: 1_000.0,
                accel_time_s: 0.5,
                cruise_time_s: 0.0,
                decel_time_s: 0.5,
                accel_distance_mm: 250.0,
                cruise_distance_mm: 0.0,
                decel_distance_mm: 250.0,
                total_time_s: 1.0,
                total_distance_mm: 500.0,
                acceleration_mm_s2: 2_000.0,
                deceleration_mm_s2: 2_000.0,
            }),
        }
    }

    #[test]
    fn a_motion_delta_is_robots_in_ascending_id_each_with_its_spans_in_start_order() {
        let delta = MotionDelta(vec![
            RobotMotion { robot: RobotId(5), spans: vec![span(2_000, 500.0)] },
            RobotMotion { robot: RobotId(1), spans: vec![span(3_000, 500.0), span(1_000, 500.0)] },
        ]);
        let bytes = delta.to_bytes().expect("encodes");
        assert_eq!(bytes[0], 0x92, "two entries, with no wrapper around them");
        assert_eq!(&bytes[1..3], &[0x92, 1], "the lower robot id first: two positions, then the id");
        let read = MotionDelta::from_bytes(&bytes, 0).expect("decodes");
        assert_eq!(read.0[0].robot, RobotId(1));
        assert_eq!(read.0[0].spans[0].start, SimTime::ms(1_000), "spans come back in start order");
        assert_eq!(read.get(RobotId(5)).expect("robot 5 moved").spans.len(), 1);
    }

    #[test]
    fn a_span_is_six_positions_and_its_profile_is_the_planners_thirteen() {
        let bytes = rmp_serde::to_vec(&span(0, 500.0)).expect("encodes");
        assert_eq!(bytes[0], 0x96, "six positions");
        assert_eq!(bytes[1..5], [1, 2, 0, 0xcd], "source, destination, the start instant, then a two-byte end");
        let read = rmp_serde::from_slice::<MotionSpan>(&bytes).expect("decodes");
        assert_eq!(read, span(0, 500.0));
        let profile = rmp_serde::to_vec(&read.profile.expect("a profile")).expect("encodes");
        assert_eq!(profile[0], 0x9d, "thirteen positions");
    }
}
