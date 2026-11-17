//! Within `crucible`, `series` is A CHART'S POINTS: what each named metric was
//! worth at each slice of a run, from its start to an instant a browser names.
//!
//! It exists because a chart wants the whole run at once and an attachment
//! delivers it one slice at a time. Reading every scalars section server-side
//! is one pass over a file the server already has, against an attachment that
//! would have to send every frame of the run to say the same thing. A browser
//! asks for this once and then extends each series from the scalars section of
//! every delta its attachment brings.
//!
//! The values are the RUN'S, not the slice's: a scalars delta carries one
//! slice's own contribution, and what a chart plots is what that contribution
//! added up to — which is [`ScalarsState`]'s job, and is done here by the same
//! code a browser applies to the deltas it receives.
//!
//! What a slice STATED is kept beside what it added up to. A run states some
//! metrics only while the thing they measure exists, and the accumulated value
//! of a metric a slice said nothing about is the value it had before — drawing
//! it again would put a point on a chart for a slice that reported none.

use std::fs::File;
use std::path::Path;

use worldview::container::index::IndexRecord;
use worldview::frames::SCALARS;
use worldview::frames::scalars::{ScalarsDelta, ScalarsKeyframe};
use worldview::frames::state::ScalarsState;
use worldview::{Declare, Descriptors, Error as WorldviewError, Ms, Payload, Reader, Scenario, Sections, Tag};

use crate::protocol::MetricSeries;

/// Every asked-for metric's points, from the run's start to `until`.
///
/// A metric the run never declared comes back named and empty: a chart that
/// asked for six things should be told which of them this run has nothing to
/// say about, rather than handed five and left to work out which is missing.
pub fn of(
    recording: &Path,
    metrics: &[String],
    until: Option<u64>,
    most_points: usize,
) -> Result<Vec<MetricSeries>, WorldviewError> {
    let mut reader = Reader::open(recording)?;
    let edge = reader.live_edge().unwrap_or(Ms(u64::MAX));
    let scenario = Scenario::read_from(&mut reader)?;
    let width = Ms(scenario.slice_width.as_ms());
    let mut descriptors = Descriptors::from_scenario(&scenario);
    let horizon = until.map_or(Ms(u64::MAX), Ms);

    let records: Vec<IndexRecord> =
        reader.index().records().iter().take_while(|record| record.t <= horizon).copied().collect();
    let mut values = ScalarsState::default();
    values.describe(&descriptors);
    // The running value at each instant, and — position by position — whether
    // the frame that produced it said anything about that metric at all.
    let mut walked: Vec<(Ms, Vec<Option<f64>>)> = Vec::new();
    for record in records {
        match record.tag {
            Tag::Declare => {
                descriptors.push(&reader.payload_at::<Declare>(&record)?);
                values.describe(&descriptors);
            }
            Tag::Keyframe => {
                let stated = section::<ScalarsKeyframe>(&mut reader, &record)?.unwrap_or_default();
                values = ScalarsState::from_keyframe(&stated);
                values.describe(&descriptors);
                // A keyframe restates what the deltas before it already
                // reached, so it only adds a point where nothing stands yet —
                // which is the run's first instant, and a seek's.
                if walked.last().map(|(at, _)| *at) != Some(record.t) {
                    walked.push((record.t, only_stated(&values.values, &stated.values)));
                }
            }
            Tag::Delta => {
                let stated = section::<ScalarsDelta>(&mut reader, &record)?.unwrap_or_default();
                values.apply(&stated);
                // A delta stamped at the start of its slice states what that
                // slice did, so what it makes true is true at the slice's END —
                // or at the run's end, for the last slice, which a run that
                // stopped part way through one does not reach.
                let ends = record.t.0.saturating_add(width.0).min(edge.0);
                walked.push((Ms(ends), only_stated(&values.values, &stated.values)));
            }
            _ => {}
        }
    }

    // A delta's value becomes true at the END of its slice, which can be past
    // the instant asked for; a series stops where it was asked to.
    walked.retain(|(at, _)| *at <= horizon);

    // Naming nothing asks for everything: a browser holds the metric list from
    // the scenario block, and making it repeat a hundred names to get a hundred
    // series is a message nobody gains by.
    let asked: Vec<String> = match metrics.is_empty() {
        true => descriptors.metrics.iter().map(|metric| metric.name.clone()).collect(),
        false => metrics.to_vec(),
    };

    Ok(asked
        .iter()
        .map(|name| MetricSeries {
            metric: name.clone(),
            points: match descriptors.metric_slot(name) {
                Some(slot) => bucketed(points(&walked, slot), most_points),
                None => Vec::new(),
            },
        })
        .collect())
}

/// The running values, blanked wherever the frame that produced them stated
/// nothing: a slot the slice said nothing about keeps its accumulated value in
/// the state, and that value is not a reading of this slice.
fn only_stated(running: &[Option<f64>], stated: &[Option<f64>]) -> Vec<Option<f64>> {
    running
        .iter()
        .enumerate()
        .map(|(slot, value)| match stated.get(slot).copied().flatten() {
            Some(_) => *value,
            None => None,
        })
        .collect()
}

/// One frame's scalars section, decoded. `None` when the frame carries none,
/// which a filtered frame can and a file's cannot.
fn section<P: Payload>(reader: &mut Reader<File>, record: &IndexRecord) -> Result<Option<P>, WorldviewError> {
    let frame = reader.frame_at(record.offset)?;
    let sections = Sections::decode(frame.contents(record.offset)?, record.offset)?;
    sections.get(SCALARS).map(|bytes| P::from_bytes(bytes, record.offset)).transpose()
}

/// One metric's slot, over every slice that stated a value for it. A slice that
/// stated nothing is skipped rather than drawn as zero: a run states some
/// metrics only while the thing they measure exists.
fn points(walked: &[(Ms, Vec<Option<f64>>)], slot: usize) -> Vec<(u64, f64)> {
    walked.iter().filter_map(|(at, values)| values.get(slot).copied().flatten().map(|value| (at.0, value))).collect()
}

/// Thin a series down to what a chart can draw, by averaging.
///
/// A two-hour run is seven hundred slices and fits; a two-day one does not, and
/// the choice there is between an unbounded reply and a mean. The mean is over
/// the instants too, so a bucket's point sits in the middle of the span it
/// stands for rather than at one end of it.
fn bucketed(points: Vec<(u64, f64)>, most: usize) -> Vec<(u64, f64)> {
    if points.len() <= most {
        return points;
    }
    let per = points.len().div_ceil(most);
    points
        .chunks(per)
        .map(|bucket| {
            let count = bucket.len() as f64;
            let at = bucket.iter().map(|(at, _)| *at as f64).sum::<f64>() / count;
            (at as u64, bucket.iter().map(|(_, value)| *value).sum::<f64>() / count)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::bucketed;

    #[test]
    fn a_series_longer_than_a_chart_can_draw_is_averaged_into_that_many_buckets() {
        let raw: Vec<(u64, f64)> = (0..10_000u64).map(|slice| (slice * 10_000, slice as f64)).collect();
        let drawn = bucketed(raw.clone(), 2_000);
        assert_eq!(drawn.len(), 2_000, "one point per bucket");
        assert_eq!(drawn[0], (20_000, 2.0), "the mean of the five slices it stands for, instant and value alike");
        assert_eq!(bucketed(raw[..2_000].to_vec(), 2_000).len(), 2_000, "a series that fits is untouched");
    }
}
