//! Robot facing at graph nodes. Cardinal values keep their existing identity;
//! curved links carry their tangent headings at millidegree precision.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Orientation {
    North,
    East,
    South,
    West,
    Heading(Heading),
}

/// A normalized noncardinal heading measured clockwise from East in map
/// coordinates (+y is South). Construct through `Orientation::from_degrees`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct Heading(u32);

impl TryFrom<u32> for Heading {
    type Error = &'static str;
    fn try_from(value: u32) -> Result<Self, Self::Error> {
        if value >= 360_000 || value % 90_000 == 0 {
            Err("noncardinal heading must be within one turn and use cardinal names on axes")
        } else {
            Ok(Self(value))
        }
    }
}

impl From<Heading> for u32 {
    fn from(value: Heading) -> Self {
        value.0
    }
}

impl Orientation {
    pub fn from_degrees(degrees: f64) -> Self {
        assert!(degrees.is_finite(), "a robot heading must be finite");
        match (degrees.rem_euclid(360.0) * 1000.0).round() as u32 % 360_000 {
            0 => Self::East,
            90_000 => Self::South,
            180_000 => Self::West,
            270_000 => Self::North,
            angle => Self::Heading(Heading(angle)),
        }
    }

    pub fn degrees(self) -> f64 {
        self.millidegrees() as f64 / 1000.0
    }

    /// Chassis heading while translating backward along a geometric path.
    pub(super) fn opposite(self) -> Self {
        Self::from_degrees(self.degrees() + 180.0)
    }

    fn millidegrees(self) -> u32 {
        match self {
            Self::North => 270_000,
            Self::East => 0,
            Self::South => 90_000,
            Self::West => 180_000,
            Self::Heading(Heading(angle)) => angle,
        }
    }

    pub fn angle_millidegrees_to(self, other: Self) -> u32 {
        let diff = self.millidegrees().abs_diff(other.millidegrees());
        diff.min(360_000 - diff)
    }

    /// Number of quarter-turns covering the angle, preserving cardinal counts.
    /// Motion timing uses the exact angle, through a drive's `rotation_cost`.
    pub fn turns_to(self, other: Self) -> u8 {
        self.angle_millidegrees_to(other).div_ceil(90_000) as u8
    }
}

impl Default for Orientation {
    fn default() -> Self {
        Self::East
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_precision_is_canonical_and_cardinal_serialization_is_unchanged() {
        assert_eq!(Orientation::from_degrees(-90.0), Orientation::North);
        assert_eq!(Orientation::from_degrees(360.000_000_1), Orientation::East);
        assert_eq!(Orientation::from_degrees(45.000_000_1), Orientation::from_degrees(405.0));
        assert_eq!(serde_json::to_string(&Orientation::North).unwrap(), "\"North\"");
        let diagonal = Orientation::from_degrees(45.0);
        let encoded = serde_json::to_string(&diagonal).unwrap();
        assert_eq!(serde_json::from_str::<Orientation>(&encoded).unwrap(), diagonal);
        assert!(serde_json::from_str::<Orientation>(r#"{"Heading":90000}"#).is_err());
        assert_eq!(diagonal.angle_millidegrees_to(Orientation::East), 45_000);
    }
}
