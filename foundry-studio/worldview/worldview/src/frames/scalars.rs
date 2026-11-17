//! The chart values: one slot per metric, per slice.
//!
//! A scalars frame is one positional array, and the metric descriptors — the
//! ones the scenario block states plus every one declared since — say what
//! each slot counts. A keyframe carries what the descriptor's aggregation
//! class says the metric is at that instant: a running sum for a counter, the
//! latest reading for a gauge. A delta carries one slice's own values.
//!
//! A slot is nil when the producer has no value for that metric in that
//! slice, which is not the same as zero: a run states some metrics only while
//! the thing they measure exists, and a reader has to rebuild exactly the
//! ones that were stated. Positions are only ever appended, so a frame
//! written before a metric was declared is shorter than one written after,
//! and the trailing positions it lacks were simply not yet declared.

use serde::{Deserialize, Serialize};

use crate::container::payload::Payload;
use crate::frames::byte_enum;

byte_enum! {
    /// The base unit a metric's values are in, before any presentation.
    MetricUnit as "metric unit" {
        Count = 1,
        DurationMilliseconds = 2,
        DistanceMillimeters = 3,
        Ratio = 4,
        RatePerHour = 5,
    }
}

byte_enum! {
    /// How a metric's raw values may be combined across scopes and periods —
    /// and, for a reader, whether a slice's value adds onto the running total
    /// or replaces the reading before it.
    AggregationClass as "aggregation class" {
        Additive = 1,
        Interval = 2,
        Ratio = 3,
        SnapshotGauge = 4,
        DistinctCount = 5,
    }
}

impl AggregationClass {
    /// Whether a slice's value adds onto what came before it. A gauge is the
    /// newest reading and nothing else; everything else accumulates.
    pub fn accumulates(self) -> bool {
        self != Self::SnapshotGauge
    }
}

/// What a metric means beyond its identity: its unit, what its denominator
/// counts when it has one, and how it aggregates.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MetricSemantics {
    pub unit: MetricUnit,
    pub denominator_description: Option<String>,
    pub aggregation: AggregationClass,
}

/// A metric's self-description. Their order fixes each value's slot in every
/// scalars array, for the whole run.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MetricDescriptor {
    pub name: String,
    pub label: String,
    pub description: String,
    pub semantics: MetricSemantics,
}

/// Every metric's value at this frame's instant.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScalarsKeyframe {
    pub values: Vec<Option<f64>>,
}

/// One slice's own contribution to every metric.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScalarsDelta {
    pub values: Vec<Option<f64>>,
}

impl Payload for ScalarsKeyframe {
    const NAME: &'static str = "scalars keyframe";
}

impl Payload for ScalarsDelta {
    const NAME: &'static str = "scalars delta";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scalars_frame_is_one_positional_array_and_an_absent_value_is_nil() {
        let keyframe = ScalarsKeyframe { values: vec![Some(1.0), None, Some(0.0)] };
        let bytes = keyframe.to_bytes().expect("encodes");
        assert_eq!(bytes[0], 0x91, "the payload is a one-element array holding the values");
        assert_eq!(bytes[1], 0x93, "three slots");
        let read = ScalarsKeyframe::from_bytes(&bytes, 0).expect("decodes");
        assert_eq!(read, keyframe);
        assert_eq!(read.values[1], None, "a metric with nothing to say stays absent, not zero");
    }

    #[test]
    fn a_descriptor_states_its_unit_and_its_class_as_bytes() {
        let descriptor = MetricDescriptor {
            name: "picks_total".to_owned(),
            label: "Picks".to_owned(),
            description: "Units picked".to_owned(),
            semantics: MetricSemantics {
                unit: MetricUnit::Count,
                denominator_description: None,
                aggregation: AggregationClass::Additive,
            },
        };
        let bytes = rmp_serde::to_vec(&descriptor).expect("encodes");
        assert_eq!(bytes[0], 0x94, "four positions");
        assert_eq!(
            &bytes[bytes.len() - 4..],
            &[0x93, 1, 0xc0, 1],
            "the semantics: unit byte, no denominator, class byte"
        );
        assert_eq!(rmp_serde::from_slice::<MetricDescriptor>(&bytes).expect("decodes"), descriptor);
        assert!(AggregationClass::Additive.accumulates());
        assert!(!AggregationClass::SnapshotGauge.accumulates());
    }
}
