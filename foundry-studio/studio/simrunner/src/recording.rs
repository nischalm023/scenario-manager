//! Within `simrunner`, `recording` is what a finished `.worldview` says in the runner contract's words (spec 08
//! section 3.5): every metric the run declared with its final value, then the summary's figures the end block states
//! (desvis's finished Status rows, named as `studio_metrics` names them), every declared metric's values at each
//! slice end, and how and where in simtime the run stopped. `simulate` derives an attempt's `metrics.json` and
//! `series.json` from the recording it just wrote, so the two files never disagree with the replay; the tests hold
//! `fixtures/canned/` (three hours of a repository world, made by `desvis fixtures/canned/spec.json --headless
//! --threads 1 --simtime 3h --time-slice 10`) to the same function.

use std::io::Cursor;

use dessim::metrics::profile::SummaryStats;
use desunits::{SimDuration, SimTime};
use studio_metrics::{figures, recorded_clock};
use studio_protocol::{
    Aggregation, DeclaredMetric, FORMAT, MetricBasis, MetricSeries, MetricUnit, MetricsFile, SeriesFile, SeriesKind,
    SeriesPoint,
};
use worldview::container::index::IndexRecord;
use worldview::frames::SCALARS;
use worldview::frames::scalars::{AggregationClass, MetricUnit as RecordedUnit, ScalarsDelta, ScalarsKeyframe};
use worldview::frames::state::ScalarsState;
use worldview::{Declare, Descriptors, End, Error as WorldviewError, Payload, Reader, Scenario, Sections, Tag};

/// What the recording's end block says of how the run stopped, and where its last slice ends in simtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stop {
    pub reason: String,
    pub at: SimTime,
}

/// A recording's two files and how it stopped: every metric it declared with its final value, from the end block
/// when there is one, and every metric's values at each slice end, running totals for those that accumulate and the
/// readings stated for the rest.
pub fn derive(recording: &[u8]) -> Result<(MetricsFile, SeriesFile, Stop), WorldviewError> {
    let mut reader = Reader::from_bytes(recording)?;
    let scenario = Scenario::read_from(&mut reader)?;
    let width = scenario.slice_width;
    let mut descriptors = Descriptors::from_scenario(&scenario);
    let records: Vec<IndexRecord> = reader.index().records().to_vec();
    // A slice ends where the next frame begins: the next slice, or for the run's last the end block, stamped at the
    // run's stop, as dessim's own reader reads a recording (`sim/dessim/src/metrics/worldview/read.rs`). A recording
    // cut short has no end block, and one made before slices stopped at the run's stop can begin its last slice at the
    // end block; either's last slice ends its width on, so no two slices close at one instant.
    let starts: Vec<u64> = records.iter().filter(|record| record.tag == Tag::Delta).map(|record| record.t.0).collect();
    let stop = records.iter().find(|record| record.tag == Tag::End).map(|record| record.t.0);
    let last_ends = starts.last().map(|&last| stop.filter(|stop| *stop > last).unwrap_or(last + width.as_ms()));
    let mut ends = starts.iter().skip(1).copied().chain(last_ends);
    let mut reached = 0;
    let mut state = ScalarsState::default();
    state.describe(&descriptors);
    // One point list per slot, filled as the slices close.
    let mut points: Vec<Vec<SeriesPoint>> = Vec::new();
    let mut end: Option<End> = None;
    let mut started = false;
    for record in &records {
        match record.tag {
            Tag::Declare => {
                descriptors.push(&reader.payload_at::<Declare>(record)?);
                state.describe(&descriptors);
            }
            // The first keyframe states the run's start; every later one restates what the deltas reached.
            Tag::Keyframe if !started => {
                started = true;
                let stated = section::<ScalarsKeyframe>(&mut reader, record)?.unwrap_or_default();
                state = ScalarsState::from_keyframe(&stated);
                state.describe(&descriptors);
                close(&mut points, &descriptors, &state.values, &stated.values, SimTime::ms(record.t.0));
            }
            Tag::Delta => {
                let stated = section::<ScalarsDelta>(&mut reader, record)?.unwrap_or_default();
                state.apply(&stated);
                // A slice's values become true at its end.
                reached = ends.next().unwrap_or(record.t.0);
                close(&mut points, &descriptors, &state.values, &stated.values, SimTime::ms(reached));
            }
            Tag::End => {
                let frame = reader.frame_at(record.offset)?;
                end = Some(End::from_bytes(&frame.contents(record.offset)?, record.offset)?);
            }
            _ => {}
        }
    }
    let basis = if end.is_some() { MetricBasis::EndBlock } else { MetricBasis::Rebuilt };
    let finals = end.as_ref().map_or(&state.values, |end| &end.cumulative_scalars);
    let mut metrics: Vec<DeclaredMetric> = descriptors
        .metrics
        .iter()
        .enumerate()
        .map(|(slot, descriptor)| DeclaredMetric {
            metric: descriptor.name.clone(),
            label: descriptor.label.clone(),
            description: descriptor.description.clone(),
            unit: unit(descriptor.semantics.unit),
            clock: recorded_clock(&descriptor.name, unit(descriptor.semantics.unit)),
            aggregation: aggregation(descriptor.semantics.aggregation),
            denominator: descriptor.semantics.denominator_description.clone(),
            value: finals.get(slot).copied().flatten(),
            whole_run: false,
        })
        .collect();
    // The summary's figures, after the declared metrics so the headline rule's first four stay the declared ones, each
    // the whole run's by definition. A summary that does not read refuses the recording rather than store it without
    // them.
    if let Some(end) = &end {
        let summary: SummaryStats = end.summary.read("summary statistics")?;
        metrics.extend(figures(&summary).into_iter().map(|figure| DeclaredMetric {
            metric: figure.metric.to_owned(),
            label: figure.label.to_owned(),
            description: figure.description.to_owned(),
            unit: figure.unit,
            clock: figure.clock,
            aggregation: figure.aggregation,
            denominator: figure.denominator.map(str::to_owned),
            value: Some(figure.value),
            whole_run: true,
        }));
    }
    let series = descriptors
        .metrics
        .iter()
        .enumerate()
        .map(|(slot, descriptor)| MetricSeries {
            metric: descriptor.name.clone(),
            kind: if descriptor.semantics.aggregation.accumulates() {
                SeriesKind::Running
            } else {
                SeriesKind::Readings
            },
            points: points.get(slot).cloned().unwrap_or_default(),
        })
        .collect();
    let stopped = Stop {
        reason: end.as_ref().map_or_else(|| "the recording has no end".to_owned(), |end| end.stop_reason.clone()),
        at: SimTime::ms(reached),
    };
    Ok((
        MetricsFile { v: FORMAT, basis, metrics },
        SeriesFile { v: FORMAT, slice_sim_ms: SimDuration::ms(width.as_ms()), end_sim_ms: stopped.at, metrics: series },
        stopped,
    ))
}

/// The values at `at`, the end of a slice that stated `stated`: an accumulating metric's running total once it has
/// one, a gauge's reading only where this slice stated it.
fn close(
    points: &mut Vec<Vec<SeriesPoint>>,
    descriptors: &Descriptors,
    running: &[Option<f64>],
    stated: &[Option<f64>],
    at: SimTime,
) {
    points.resize(descriptors.metrics.len(), Vec::new());
    for (slot, descriptor) in descriptors.metrics.iter().enumerate() {
        let value = if descriptor.semantics.aggregation.accumulates() {
            running.get(slot).copied().flatten()
        } else {
            stated.get(slot).copied().flatten()
        };
        if let Some(value) = value {
            points[slot].push(SeriesPoint(at, Some(value)));
        }
    }
}

fn section<P: Payload>(reader: &mut Reader<Cursor<&[u8]>>, record: &IndexRecord) -> Result<Option<P>, WorldviewError> {
    let frame = reader.frame_at(record.offset)?;
    let sections = Sections::decode(frame.contents(record.offset)?, record.offset)?;
    sections.get(SCALARS).map(|bytes| P::from_bytes(bytes, record.offset)).transpose()
}

fn unit(unit: RecordedUnit) -> MetricUnit {
    match unit {
        RecordedUnit::Count => MetricUnit::Count,
        RecordedUnit::DurationMilliseconds => MetricUnit::DurationMs,
        RecordedUnit::DistanceMillimeters => MetricUnit::DistanceMm,
        RecordedUnit::Ratio => MetricUnit::Ratio,
        RecordedUnit::RatePerHour => MetricUnit::RatePerHour,
    }
}

fn aggregation(class: AggregationClass) -> Aggregation {
    match class {
        AggregationClass::Additive => Aggregation::Additive,
        AggregationClass::Interval => Aggregation::Interval,
        AggregationClass::Ratio => Aggregation::Ratio,
        AggregationClass::SnapshotGauge => Aggregation::SnapshotGauge,
        AggregationClass::DistinctCount => Aggregation::DistinctCount,
    }
}
