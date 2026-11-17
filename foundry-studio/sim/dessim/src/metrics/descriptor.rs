//! Metric description vocabulary: what a metric MEANS, independent of how
//! it is computed (`reporting`) or transported (`streams`). Both consume
//! this module; neither owns it.

use serde::{Deserialize, Serialize};

/// Base unit retained by the canonical report before presentation conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MetricUnit {
    Count,
    DurationMilliseconds,
    DistanceMillimeters,
    Ratio,
    RatePerHour,
}

/// How raw metric values may be combined across scopes and periods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AggregationClass {
    /// Discrete counts, durations, or distances that can be summed.
    Additive,
    /// Duration intervals that must first be clipped to reporting boundaries.
    Interval,
    /// Numerator and denominator pairs that are summed before division.
    Ratio,
    /// Point-in-time state that cannot be summed without explicit integration.
    SnapshotGauge,
    /// Identity-based count requiring a distinct key within the selected scope.
    DistinctCount,
}

/// What a metric MEANS, beyond its identity and columns: the semantic
/// extension the metric family adds on top of the shared stream core.
///
/// Lives in exactly one struct so typed reports and metric streams can never
/// disagree about a metric's unit or aggregation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MetricSemantics {
    pub unit: MetricUnit,
    pub denominator_description: Option<String>,
    pub aggregation: AggregationClass,
}

/// Self-description a metric producer emits alongside its data: identity
/// plus [`MetricSemantics`].
///
/// Descriptors are owned by the report builder that computes the values they
/// describe and travel with the emitted report. There is no central list of
/// metrics; consumers discover metrics from emitted reports.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MetricDescriptor {
    pub id: String,
    pub label: String,
    pub description: String,
    pub semantics: MetricSemantics,
}

impl MetricDescriptor {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        description: impl Into<String>,
        unit: MetricUnit,
        denominator_description: Option<String>,
        aggregation: AggregationClass,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            description: description.into(),
            semantics: MetricSemantics { unit, denominator_description, aggregation },
        }
    }
}
